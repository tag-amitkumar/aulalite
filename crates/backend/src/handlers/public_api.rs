// crates/backend/src/handlers/public_api.rs
//! Read-only PUBLIC API under `/api/v1/*`, authenticated by a tenant-scoped API
//! key (`Authorization: Bearer ak_<prefix>_<secret>`) rather than a Firebase
//! token. This router is mounted in `lib.rs` OUTSIDE the `require_auth` layer
//! (like the mediamtx + stripe-webhook callbacks), so the standard
//! `RequestContext` is NOT present; instead each handler resolves an
//! `AuthedKey` from the bearer header via `db::api_keys::authenticate` and
//! scopes every read to that key's tenant.
//!
//! Surface (all GET, all scoped to the key's tenant):
//!   * GET /api/v1/courses                      — list courses in the tenant
//!   * GET /api/v1/courses/{id}                  — one course
//!   * GET /api/v1/courses/{id}/roster           — active members
//!   * GET /api/v1/courses/{id}/grades           — released numeric grades matrix
//!
//! Scopes: a key carries a `scopes TEXT[]`. The wildcard `"*"` or `"read"` grants
//! all read endpoints; finer scopes (`courses:read`, `roster:read`, `grades:read`)
//! gate the corresponding endpoint. A key with no usable scope gets 403.
//!
//! We reuse the EXISTING tenant-scoped read fns (`db::courses`, `db::course_detail`,
//! `db::gradebook`), passing the API key's tenant_id so RLS applies exactly as it
//! does for the authed app. No new read paths into the data are introduced.
//!
//! This module ALSO hosts the org-admin key-management endpoints (mint/list/
//! revoke) under `/v1/admin/api-keys`, exposed via `admin_routes()` which is
//! merged INSIDE the require_auth layer. They live here (not in a separate
//! module) so the public surface and its credentials stay in one place.
use axum::extract::{Extension, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::{routing, Json, Router};
use serde::{Deserialize, Serialize};
use sqlx::types::BigDecimal;
use uuid::Uuid;

use crate::context::RequestContext;
use crate::db;
use crate::db::api_keys::AuthedKey;
use crate::error::ApiError;
use crate::AppState;

const MAX_NAME_LEN: usize = 120;
/// Scopes a key may be granted. `*`/`read` are blanket read grants; the rest gate
/// individual `/api/v1/*` endpoints (see `has_scope`).
const ALLOWED_SCOPES: &[&str] = &["*", "read", "courses:read", "roster:read", "grades:read"];

/// PUBLIC API-key router. Mounted in `lib.rs` WITHOUT the require_auth layer.
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/courses", routing::get(list_courses))
        .route("/api/v1/courses/{id}", routing::get(get_course))
        .route("/api/v1/courses/{id}/roster", routing::get(get_roster))
        .route("/api/v1/courses/{id}/grades", routing::get(get_grades))
}

/// ADMIN router for minting/listing/revoking API keys. Merged INSIDE the
/// require_auth layer in `lib.rs`; gated to org-admin / platform-admin.
pub fn admin_routes() -> Router<AppState> {
    Router::new()
        .route("/v1/admin/api-keys", routing::get(list_keys).post(mint_key))
        .route("/v1/admin/api-keys/{id}", routing::delete(revoke_key))
}

// ---------------------------------------------------------------------------
// API-key authentication + scope gating
// ---------------------------------------------------------------------------

/// Resolve + authenticate the `ak_...` bearer token from the request headers.
/// 401 on a missing/invalid/revoked key. Does NOT check scopes (see `require_scope`).
async fn authed_key(state: &AppState, headers: &HeaderMap) -> Result<AuthedKey, ApiError> {
    let token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .ok_or_else(|| ApiError::Unauthorized("missing api key".into()))?;

    db::api_keys::authenticate(&state.pool, token)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::Unauthorized("invalid or revoked api key".into()))
}

/// True if `key` is allowed to use an endpoint requiring `needed`. `"*"` and
/// `"read"` are blanket read grants; otherwise the exact scope must be present.
fn has_scope(key: &AuthedKey, needed: &str) -> bool {
    key.scopes
        .iter()
        .any(|s| s == "*" || s == "read" || s == needed)
}

fn require_scope(key: &AuthedKey, needed: &str) -> Result<(), ApiError> {
    if has_scope(key, needed) {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
    }
}

// ---------------------------------------------------------------------------
// DTOs (stable external contract — keep field names/types frozen)
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct CourseDto {
    id: Uuid,
    slug: String,
    title: String,
    description: Option<String>,
    status: String,
    visibility: String,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<db::courses::CourseRow> for CourseDto {
    fn from(c: db::courses::CourseRow) -> Self {
        Self {
            id: c.id,
            slug: c.slug,
            title: c.title,
            description: c.description,
            status: c.status,
            visibility: c.visibility,
            created_at: c.created_at,
            updated_at: c.updated_at,
        }
    }
}

#[derive(Serialize)]
struct RosterMemberDto {
    user_id: Uuid,
    display_name: Option<String>,
    email: String,
    role: String,
}

#[derive(Serialize)]
struct GradeCellDto {
    assignment_id: Uuid,
    student_user_id: Uuid,
    numeric_grade: BigDecimal,
}

#[derive(Serialize)]
struct GradeAssignmentDto {
    id: Uuid,
    title: String,
    max_points: Option<i32>,
}

#[derive(Serialize)]
struct GradesDto {
    course_id: Uuid,
    assignments: Vec<GradeAssignmentDto>,
    grades: Vec<GradeCellDto>,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn list_courses(
    State(s): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<CourseDto>>, ApiError> {
    let key = authed_key(&s, &headers).await?;
    require_scope(&key, "courses:read")?;

    // Reuse the existing list path with the org-admin branch so the key sees the
    // whole tenant's catalogue (the key is a tenant-wide credential). The fn
    // resolves the tenant from a user_id; there is no API-key user, so we read
    // the tenant's courses directly through the tenant-scoped helper below.
    let rows = list_courses_for_tenant(&s.pool, key.tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(rows.into_iter().map(CourseDto::from).collect()))
}

async fn get_course(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<CourseDto>, ApiError> {
    let key = authed_key(&s, &headers).await?;
    require_scope(&key, "courses:read")?;

    let row = fetch_course_in_tenant(&s.pool, key.tenant_id, id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(CourseDto::from(row)))
}

async fn get_roster(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<RosterMemberDto>>, ApiError> {
    let key = authed_key(&s, &headers).await?;
    require_scope(&key, "roster:read")?;

    // Confirm the course is in the key's tenant before exposing its roster.
    let _course = fetch_course_in_tenant(&s.pool, key.tenant_id, id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::NotFound)?;

    let members = roster_in_tenant(&s.pool, key.tenant_id, id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(
        members
            .into_iter()
            .map(|m| RosterMemberDto {
                user_id: m.user_id,
                display_name: m.display_name,
                email: m.email,
                role: m.role,
            })
            .collect(),
    ))
}

async fn get_grades(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<GradesDto>, ApiError> {
    let key = authed_key(&s, &headers).await?;
    require_scope(&key, "grades:read")?;

    let _course = fetch_course_in_tenant(&s.pool, key.tenant_id, id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::NotFound)?;

    // Reuse the gradebook loader (tenant-scoped, RLS-applied) and project just
    // the released-grade matrix into the public contract.
    let data = db::gradebook::load_gradebook(&s.pool, key.tenant_id, id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let assignments = data
        .assignments
        .into_iter()
        .map(|a| GradeAssignmentDto {
            id: a.id,
            title: a.title,
            max_points: a.max_points,
        })
        .collect();
    let grades = data
        .grades
        .into_iter()
        .map(|g| GradeCellDto {
            assignment_id: g.assignment_id,
            student_user_id: g.student_user_id,
            numeric_grade: g.numeric_grade,
        })
        .collect();

    Ok(Json(GradesDto {
        course_id: id,
        assignments,
        grades,
    }))
}

// ---------------------------------------------------------------------------
// Tenant-scoped reads (local, so the public surface never relies on a user_id).
// Each opens a tx with the tenant GUC set so RLS applies under `aulalite_app`,
// mirroring `db::announcements`.
// ---------------------------------------------------------------------------

async fn set_tenant(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: Uuid,
) -> sqlx::Result<()> {
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn list_courses_for_tenant(
    pool: &sqlx::PgPool,
    tenant_id: Uuid,
) -> sqlx::Result<Vec<db::courses::CourseRow>> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    let rows = sqlx::query_as::<_, db::courses::CourseRow>(
        "SELECT id, tenant_id, slug, title, description, status, visibility, \
                cover_asset_id, owner_user_id, syllabus_md, grading_policy_md, \
                created_at, updated_at \
           FROM courses WHERE tenant_id = $1 ORDER BY created_at DESC",
    )
    .bind(tenant_id)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(rows)
}

async fn fetch_course_in_tenant(
    pool: &sqlx::PgPool,
    tenant_id: Uuid,
    id: Uuid,
) -> sqlx::Result<Option<db::courses::CourseRow>> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    let row = sqlx::query_as::<_, db::courses::CourseRow>(
        "SELECT id, tenant_id, slug, title, description, status, visibility, \
                cover_asset_id, owner_user_id, syllabus_md, grading_policy_md, \
                created_at, updated_at \
           FROM courses WHERE id = $1 AND tenant_id = $2",
    )
    .bind(id)
    .bind(tenant_id)
    .fetch_optional(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(row)
}

async fn roster_in_tenant(
    pool: &sqlx::PgPool,
    tenant_id: Uuid,
    course_id: Uuid,
) -> sqlx::Result<Vec<db::course_detail::MemberRow>> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    let rows = db::course_detail::members(&mut *tx, course_id).await?;
    tx.commit().await?;
    Ok(rows)
}

// ---------------------------------------------------------------------------
// Admin: mint / list / revoke API keys (require_auth router, org-admin gated)
// ---------------------------------------------------------------------------

fn require_admin(ctx: &RequestContext) -> Result<Uuid, ApiError> {
    if !ctx.has_capability(core_types::Capability::IntegrationsManage) {
        return Err(ApiError::Forbidden);
    }
    ctx.tenant_id
        .ok_or_else(|| ApiError::BadRequest("no tenant".into()))
}

#[derive(Serialize)]
struct ApiKeyDto {
    id: Uuid,
    name: String,
    prefix: String,
    scopes: Vec<String>,
    created_by: Uuid,
    created_at: chrono::DateTime<chrono::Utc>,
    last_used_at: Option<chrono::DateTime<chrono::Utc>>,
    revoked_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl From<db::api_keys::ApiKeyRow> for ApiKeyDto {
    fn from(r: db::api_keys::ApiKeyRow) -> Self {
        Self {
            id: r.id,
            name: r.name,
            prefix: r.prefix,
            scopes: r.scopes,
            created_by: r.created_by,
            created_at: r.created_at,
            last_used_at: r.last_used_at,
            revoked_at: r.revoked_at,
        }
    }
}

#[derive(Deserialize)]
struct MintKey {
    name: String,
    #[serde(default)]
    scopes: Vec<String>,
}

/// Mint response. `plaintext` is the ONLY time the full token is ever returned.
#[derive(Serialize)]
struct MintedKeyDto {
    #[serde(flatten)]
    key: ApiKeyDto,
    plaintext: String,
}

async fn list_keys(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<Vec<ApiKeyDto>>, ApiError> {
    let tenant = require_admin(&ctx)?;
    let rows = db::api_keys::list(&s.pool, tenant)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(rows.into_iter().map(ApiKeyDto::from).collect()))
}

async fn mint_key(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Json(body): Json<MintKey>,
) -> Result<Json<MintedKeyDto>, ApiError> {
    let tenant = require_admin(&ctx)?;

    let name = body.name.trim();
    if name.is_empty() {
        return Err(ApiError::Validation("name_required".into()));
    }
    if name.chars().count() > MAX_NAME_LEN {
        return Err(ApiError::Validation("name_too_long".into()));
    }

    // Default to a blanket read grant when none specified; otherwise validate
    // each requested scope against the allow-list (dedup, reject unknown).
    let mut scopes: Vec<String> = if body.scopes.is_empty() {
        vec!["read".to_string()]
    } else {
        body.scopes
            .iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    };
    scopes.sort();
    scopes.dedup();
    for sc in &scopes {
        if !ALLOWED_SCOPES.contains(&sc.as_str()) {
            return Err(ApiError::Validation(format!("unknown_scope:{sc}")));
        }
    }

    // Public prefix + high-entropy secret; persist ONLY the hash of the secret.
    let prefix = format!("ak_{}", random_b64(6));
    let secret = random_b64(32);
    let key_hash = db::api_keys::hash_secret(&secret);

    let row = db::api_keys::insert(
        &s.pool,
        tenant,
        name,
        &prefix,
        &key_hash,
        &scopes,
        ctx.user_id,
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;

    // The one-time plaintext the caller must capture now.
    let plaintext = format!("{prefix}_{secret}");
    Ok(Json(MintedKeyDto {
        key: ApiKeyDto::from(row),
        plaintext,
    }))
}

async fn revoke_key(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let tenant = require_admin(&ctx)?;
    let revoked = db::api_keys::revoke(&s.pool, tenant, id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !revoked {
        // Not in this tenant, or already revoked: 404 keeps the surface opaque.
        return Err(ApiError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

/// `n` bytes of CSPRNG entropy as URL-safe base64 (no padding). Mirrors
/// `db::enrollments::generate_invitation_token`.
fn random_b64(n: usize) -> String {
    use base64::Engine;
    use rand::RngCore;
    let mut bytes = vec![0u8; n];
    rand::rng().fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::api_keys::AuthedKey;

    fn key_with(scopes: &[&str]) -> AuthedKey {
        AuthedKey {
            key_id: Uuid::nil(),
            tenant_id: Uuid::nil(),
            scopes: scopes.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn wildcard_and_read_grant_everything() {
        assert!(has_scope(&key_with(&["*"]), "courses:read"));
        assert!(has_scope(&key_with(&["read"]), "grades:read"));
    }

    #[test]
    fn exact_scope_required_otherwise() {
        let k = key_with(&["courses:read"]);
        assert!(has_scope(&k, "courses:read"));
        assert!(!has_scope(&k, "grades:read"));
        assert!(!has_scope(&key_with(&[]), "courses:read"));
    }
}
