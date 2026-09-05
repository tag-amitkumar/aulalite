// crates/backend/src/handlers/admin.rs
//
// Admin-only endpoints. Currently exposes the audit-event read API for
// org admins and platform admins.
use axum::extract::{Extension, Query, State};
use axum::{routing, Json, Router};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::context::RequestContext;
use crate::db;
use crate::error::ApiError;
use crate::AppState;

#[derive(Deserialize, Default)]
pub struct AuditQuery {
    /// Maximum rows to return. Clamped to [1, 200].
    #[serde(default)]
    pub limit: Option<i64>,
    /// Cursor: return events strictly older than this timestamp (RFC 3339).
    #[serde(default)]
    pub before: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Serialize)]
pub struct AuditEventDto {
    pub id: Uuid,
    pub actor_user_id: Uuid,
    pub actor_email: Option<String>,
    pub actor_display_name: Option<String>,
    pub action: String,
    pub resource_type: String,
    pub resource_id: Uuid,
    pub metadata: Option<serde_json::Value>,
    pub occurred_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Serialize)]
pub struct AuditListResponse {
    pub events: Vec<AuditEventDto>,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/admin/audit", routing::get(list_audit))
        .route(
            "/v1/admin/tenant",
            routing::get(get_tenant).patch(patch_tenant),
        )
        .route(
            "/v1/admin/tenant/memberships",
            routing::get(list_memberships),
        )
        .route(
            "/v1/admin/tenant/memberships/{user_id}",
            routing::patch(patch_membership),
        )
        .route(
            "/v1/admin/tenant/transfer-ownership",
            routing::post(transfer_ownership),
        )
        .route(
            "/v1/admin/tenant/memberships/{user_id}/mfa/recovery-codes",
            routing::post(reset_member_mfa_recovery_codes),
        )
        .route("/v1/admin/file-assets", routing::get(list_file_assets))
        .route(
            "/v1/admin/branding",
            routing::get(get_branding).patch(patch_branding),
        )
        .route("/v1/me/branding", routing::get(get_my_branding))
}

/// Test-only router mirroring [`routes`] for the branding endpoints, with a
/// plain `PgPool` state so integration tests can drive them under StubAuth.
#[doc(hidden)]
pub fn branding_router_for_tests(pool: PgPool) -> Router {
    Router::new()
        .route(
            "/v1/admin/branding",
            routing::get(get_branding_t).patch(patch_branding_t),
        )
        .route("/v1/me/branding", routing::get(get_my_branding_t))
        .with_state(BrandingTestState { pool })
}

#[derive(Clone)]
struct BrandingTestState {
    pool: PgPool,
}

#[doc(hidden)]
pub fn mfa_admin_router_for_tests(pool: PgPool) -> Router {
    Router::new()
        .route(
            "/v1/admin/tenant/memberships/{user_id}/mfa/recovery-codes",
            routing::post(reset_member_mfa_recovery_codes_t),
        )
        .with_state(AdminMfaTestState { pool })
}

/// Test-only router for the membership mutation invariants. Keeping this
/// wrapper state-light lets integration tests exercise the real transaction
/// and locking path without constructing every production service.
#[doc(hidden)]
pub fn membership_router_for_tests(pool: PgPool) -> Router {
    Router::new()
        .route(
            "/v1/admin/tenant/memberships/{user_id}",
            routing::patch(patch_membership_t),
        )
        .route(
            "/v1/admin/tenant/transfer-ownership",
            routing::post(transfer_ownership_t),
        )
        .with_state(MembershipTestState { pool })
}

#[derive(Clone)]
struct AdminMfaTestState {
    pool: PgPool,
}

#[derive(Clone)]
struct MembershipTestState {
    pool: PgPool,
}

// ===========================================================================
// Tenant branding
// ===========================================================================

/// The branding payload returned by all three endpoints. Field shape is a
/// FIXED contract shared with the frontend shell — do not reorder/rename.
#[derive(Serialize)]
pub struct BrandingDto {
    pub logo_url: Option<String>,
    pub primary_color: Option<String>,
    pub accent_color: Option<String>,
}

impl From<db::branding::Branding> for BrandingDto {
    fn from(b: db::branding::Branding) -> Self {
        Self {
            logo_url: b.logo_url,
            primary_color: b.primary_color,
            accent_color: b.accent_color,
        }
    }
}

/// PATCH body. Uses `double_option` so an OMITTED field (`None`) leaves the
/// stored value untouched, while an EXPLICIT `null` (`Some(None)`) clears it.
/// An empty string is normalized to a clear (see [`normalize_clearable`]).
#[derive(Deserialize, Default)]
pub struct PatchBrandingBody {
    #[serde(default, with = "::serde_with::rust::double_option")]
    pub logo_url: Option<Option<String>>,
    #[serde(default, with = "::serde_with::rust::double_option")]
    pub primary_color: Option<Option<String>>,
    #[serde(default, with = "::serde_with::rust::double_option")]
    pub accent_color: Option<Option<String>>,
}

/// Validate a CSS hex color: `#rgb` or `#rrggbb`, case-insensitive.
fn is_valid_hex_color(s: &str) -> bool {
    let Some(hex) = s.strip_prefix('#') else {
        return false;
    };
    (hex.len() == 3 || hex.len() == 6) && hex.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Validate a basic http(s) URL: non-empty and starting with `http://` or
/// `https://` after trimming.
fn is_valid_http_url(s: &str) -> bool {
    let s = s.trim();
    !s.is_empty() && (s.starts_with("http://") || s.starts_with("https://"))
}

/// Normalize an incoming patch field into the value to store:
///   * `None`               -> leave unchanged (returns `None`)
///   * `Some(None)`         -> clear (returns `Some(None)`)
///   * `Some(Some(""))`     -> treat empty/whitespace as clear (`Some(None)`)
///   * `Some(Some(v))`      -> set, after running `validate` on the trimmed value
fn normalize_clearable(
    field: Option<Option<String>>,
    field_name: &str,
    validate: impl Fn(&str) -> bool,
) -> Result<Option<Option<String>>, ApiError> {
    match field {
        None => Ok(None),
        Some(None) => Ok(Some(None)),
        Some(Some(raw)) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Ok(Some(None));
            }
            if !validate(trimmed) {
                return Err(ApiError::Validation(format!("invalid {field_name}")));
            }
            Ok(Some(Some(trimmed.to_string())))
        }
    }
}

fn is_branding_admin(ctx: &RequestContext) -> bool {
    ctx.can_manage_organization()
}

async fn get_branding_inner(
    pool: &PgPool,
    ctx: &RequestContext,
) -> Result<Json<BrandingDto>, ApiError> {
    if !is_branding_admin(ctx) {
        return Err(ApiError::Forbidden);
    }
    let tenant_id = ctx
        .tenant_id
        .ok_or_else(|| ApiError::BadRequest("no tenant".into()))?;
    let mut tx = db::begin_with_context(pool, ctx.user_id, Some(tenant_id))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let branding = db::branding::get(&mut tx, tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::NotFound)?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(branding.into()))
}

async fn patch_branding_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    b: PatchBrandingBody,
) -> Result<Json<BrandingDto>, ApiError> {
    if !is_branding_admin(ctx) {
        return Err(ApiError::Forbidden);
    }
    let tenant_id = ctx
        .tenant_id
        .ok_or_else(|| ApiError::BadRequest("no tenant".into()))?;

    // Validate + normalize each field BEFORE touching the DB.
    let logo_url = normalize_clearable(b.logo_url, "logo_url", is_valid_http_url)?;
    let primary_color = normalize_clearable(b.primary_color, "primary_color", is_valid_hex_color)?;
    let accent_color = normalize_clearable(b.accent_color, "accent_color", is_valid_hex_color)?;

    let mut tx = db::begin_with_context(pool, ctx.user_id, Some(tenant_id))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    // Read-merge-write: load current branding, apply only the provided fields.
    let mut current = db::branding::get(&mut tx, tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::NotFound)?;
    if let Some(v) = logo_url {
        current.logo_url = v;
    }
    if let Some(v) = primary_color {
        current.primary_color = v;
    }
    if let Some(v) = accent_color {
        current.accent_color = v;
    }
    let stored = db::branding::set(&mut tx, tenant_id, &current)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::NotFound)?;

    db::audit::emit_audit_event(
        &mut tx,
        tenant_id,
        ctx.user_id,
        "tenant.branding.update",
        "tenant",
        tenant_id,
        None,
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(stored.into()))
}

/// GET the caller's ACTIVE tenant branding. ANY authenticated user; the shell
/// uses this to theme itself. Resolves the active tenant the same way the rest
/// of the API does (first active membership), falling back to `ctx.tenant_id`.
/// Returns an empty [`BrandingDto`] if branding is unset.
async fn get_my_branding_inner(
    pool: &PgPool,
    ctx: &RequestContext,
) -> Result<Json<BrandingDto>, ApiError> {
    let empty = || {
        Json(BrandingDto {
            logo_url: None,
            primary_color: None,
            accent_color: None,
        })
    };

    // Resolve the active tenant. `app.user_id` is already set on this tx, so
    // the membership lookup is visible under RLS even before any tenant GUC.
    let mut tx = db::begin_with_context(pool, ctx.user_id, ctx.tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let tenant_id = match ctx.tenant_id {
        Some(t) => Some(t),
        None => {
            let resolved = db::branding::active_tenant_for_user(&mut tx, ctx.user_id)
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?;
            // The tenant GUC was not set by begin_with_context (ctx had no
            // tenant); set it now so the `tenants_self_access` RLS policy lets
            // us read the resolved tenant's branding row.
            if let Some(t) = resolved {
                db::set_request_guc(&mut tx, ctx.user_id, Some(t))
                    .await
                    .map_err(|e| ApiError::Internal(e.to_string()))?;
            }
            resolved
        }
    };
    let Some(tenant_id) = tenant_id else {
        tx.commit()
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        return Ok(empty());
    };

    let branding = db::branding::get(&mut tx, tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .unwrap_or_default();
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(branding.into()))
}

async fn get_branding(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<BrandingDto>, ApiError> {
    get_branding_inner(&state.pool, &ctx).await
}

async fn patch_branding(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Json(b): Json<PatchBrandingBody>,
) -> Result<Json<BrandingDto>, ApiError> {
    patch_branding_inner(&state.pool, &ctx, b).await
}

async fn get_my_branding(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<BrandingDto>, ApiError> {
    get_my_branding_inner(&state.pool, &ctx).await
}

async fn get_branding_t(
    State(state): State<BrandingTestState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<BrandingDto>, ApiError> {
    get_branding_inner(&state.pool, &ctx).await
}

async fn patch_branding_t(
    State(state): State<BrandingTestState>,
    Extension(ctx): Extension<RequestContext>,
    Json(b): Json<PatchBrandingBody>,
) -> Result<Json<BrandingDto>, ApiError> {
    patch_branding_inner(&state.pool, &ctx, b).await
}

async fn get_my_branding_t(
    State(state): State<BrandingTestState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<BrandingDto>, ApiError> {
    get_my_branding_inner(&state.pool, &ctx).await
}

#[derive(Deserialize, Default)]
pub struct PatchTenantBody {
    pub name: Option<String>,
    pub recording_default: Option<bool>,
    pub recording_retention_days: Option<i32>,
}

async fn patch_tenant(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Json(b): Json<PatchTenantBody>,
) -> Result<Json<TenantDto>, ApiError> {
    if !ctx.can_manage_organization() {
        return Err(ApiError::Forbidden);
    }
    let tenant_id = ctx
        .tenant_id
        .ok_or_else(|| ApiError::BadRequest("no tenant".into()))?;
    if let Some(days) = b.recording_retention_days {
        if !(1..=3650).contains(&days) {
            return Err(ApiError::BadRequest(
                "recording_retention_days must be between 1 and 3650".into(),
            ));
        }
    }

    let mut tx = db::begin_with_context(&state.pool, ctx.user_id, Some(tenant_id))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let row: Option<(
        Uuid,
        String,
        String,
        String,
        bool,
        i32,
        chrono::DateTime<chrono::Utc>,
    )> = sqlx::query_as(
        "UPDATE tenants
            SET name = COALESCE($2, name),
                recording_default = COALESCE($3, recording_default),
                recording_retention_days = COALESCE($4, recording_retention_days),
                updated_at = now()
          WHERE id = $1
        RETURNING id, slug, name, status, recording_default,
                  recording_retention_days, created_at",
    )
    .bind(tenant_id)
    .bind(b.name.as_deref())
    .bind(b.recording_default)
    .bind(b.recording_retention_days)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    let (id, slug, name, status, recording_default, recording_retention_days, created_at) =
        row.ok_or(ApiError::NotFound)?;
    db::audit::emit_audit_event(
        &mut tx,
        tenant_id,
        ctx.user_id,
        "tenant.patch",
        "tenant",
        tenant_id,
        None,
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(TenantDto {
        id,
        slug,
        name,
        status,
        recording_default,
        recording_retention_days,
        created_at,
    }))
}

#[derive(Deserialize, Default)]
pub struct PatchMembershipBody {
    pub role: Option<String>,
    pub status: Option<String>,
}

const OWNERSHIP_TRANSFER_REQUIRED: &str = "ownership_transfer_required";
const TARGET_MUST_BE_ACTIVE_ORG_ADMIN: &str = "target_must_be_active_org_admin";
const TRANSFER_CONFIRMATION: &str = "TRANSFER OWNERSHIP";

/// Membership rows use `invited` only while initial access is pending. Once a
/// member has become active or suspended, moving them back to invited would
/// create a misleading half-invitation without a real invitation token.
fn valid_membership_status_transition(from: &str, to: &str) -> bool {
    matches!(
        (from, to),
        ("active", "active")
            | ("active", "suspended")
            | ("suspended", "suspended")
            | ("suspended", "active")
            | ("invited", "invited")
            | ("invited", "active")
            | ("invited", "suspended")
    )
}

#[derive(Serialize)]
pub struct ResetMfaRecoveryCodesResponse {
    pub recovery_codes: Vec<String>,
}

async fn reset_member_mfa_recovery_codes_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    target_user_id: Uuid,
) -> Result<Json<ResetMfaRecoveryCodesResponse>, ApiError> {
    // MFA belongs to the platform-global identity. A school administrator must
    // never receive recovery credentials that work in the user's other
    // workspaces; only the platform owner/support authority may perform this
    // audited emergency operation.
    if !ctx.can_manage_platform() {
        return Err(ApiError::Forbidden);
    }
    let tenant_id = ctx
        .tenant_id
        .ok_or_else(|| ApiError::BadRequest("no tenant".into()))?;

    let mut tx = db::begin_with_context(pool, ctx.user_id, Some(tenant_id))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let target_in_tenant: Option<i32> = sqlx::query_scalar(
        "SELECT 1
           FROM tenant_memberships
          WHERE tenant_id = $1 AND user_id = $2 AND status = 'active'",
    )
    .bind(tenant_id)
    .bind(target_user_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    if target_in_tenant.is_none() {
        return Err(ApiError::NotFound);
    }
    let (plaintext, hashes) = crate::handlers::mfa::mint_recovery_codes();
    db::set_request_guc(&mut tx, target_user_id, Some(tenant_id))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let res = sqlx::query(
        "UPDATE user_mfa
            SET recovery_codes = $2,
                updated_at = now()
          WHERE user_id = $1 AND enabled = true",
    )
    .bind(target_user_id)
    .bind(&hashes)
    .execute(&mut *tx)
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    if res.rows_affected() == 0 {
        return Err(ApiError::BadRequest("mfa_not_enabled".into()));
    }

    db::audit::emit_audit_event(
        &mut tx,
        tenant_id,
        ctx.user_id,
        "user_mfa.recovery_codes.reset",
        "user",
        target_user_id,
        None,
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(ResetMfaRecoveryCodesResponse {
        recovery_codes: plaintext,
    }))
}

async fn reset_member_mfa_recovery_codes(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    axum::extract::Path(user_id): axum::extract::Path<Uuid>,
) -> Result<Json<ResetMfaRecoveryCodesResponse>, ApiError> {
    reset_member_mfa_recovery_codes_inner(&state.pool, &ctx, user_id).await
}

async fn reset_member_mfa_recovery_codes_t(
    State(state): State<AdminMfaTestState>,
    Extension(ctx): Extension<RequestContext>,
    axum::extract::Path(user_id): axum::extract::Path<Uuid>,
) -> Result<Json<ResetMfaRecoveryCodesResponse>, ApiError> {
    reset_member_mfa_recovery_codes_inner(&state.pool, &ctx, user_id).await
}

async fn patch_membership(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    axum::extract::Path(user_id): axum::extract::Path<Uuid>,
    Json(b): Json<PatchMembershipBody>,
) -> Result<Json<MembershipDto>, ApiError> {
    patch_membership_inner(&state.pool, &ctx, user_id, b).await
}

async fn patch_membership_t(
    State(state): State<MembershipTestState>,
    Extension(ctx): Extension<RequestContext>,
    axum::extract::Path(user_id): axum::extract::Path<Uuid>,
    Json(b): Json<PatchMembershipBody>,
) -> Result<Json<MembershipDto>, ApiError> {
    patch_membership_inner(&state.pool, &ctx, user_id, b).await
}

async fn patch_membership_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    user_id: Uuid,
    b: PatchMembershipBody,
) -> Result<Json<MembershipDto>, ApiError> {
    if !ctx.can_manage_members() {
        return Err(ApiError::Forbidden);
    }
    let tenant_id = ctx
        .tenant_id
        .ok_or_else(|| ApiError::BadRequest("no tenant".into()))?;

    if b.role.is_none() && b.status.is_none() {
        return Err(ApiError::Validation("no_membership_changes".into()));
    }

    // Validate enums against the CHECK constraints on tenant_memberships
    let requested_role = b
        .role
        .as_deref()
        .map(str::parse::<core_types::TenantRole>)
        .transpose()
        .map_err(|_| ApiError::BadRequest("invalid role".into()))?;
    if requested_role == Some(core_types::TenantRole::OrgOwner) {
        return Err(ApiError::Conflict(OWNERSHIP_TRANSFER_REQUIRED.into()));
    }
    if let Some(status) = b.status.as_deref() {
        if !matches!(status, "active" | "invited" | "suspended") {
            return Err(ApiError::BadRequest(format!("invalid status: {status}")));
        }
    }

    let mut tx = db::begin_with_context(pool, ctx.user_id, Some(tenant_id))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    // Serialize every role/status mutation for this tenant. This makes seat
    // checks and role-hierarchy changes correct across backend replicas.
    let tenant_exists: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM tenants WHERE id = $1 FOR UPDATE")
            .bind(tenant_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
    if tenant_exists.is_none() {
        return Err(ApiError::NotFound);
    }

    let current: Option<(String, String)> = sqlx::query_as(
        "SELECT role, status
           FROM tenant_memberships
          WHERE tenant_id = $1 AND user_id = $2
          FOR UPDATE",
    )
    .bind(tenant_id)
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    let (current_role, current_status) = current.ok_or(ApiError::NotFound)?;
    if current_role == "org_owner" {
        // Ownership can only move through the dedicated, audited two-row
        // transaction. This also prevents another admin (or a stale support
        // context) from suspending the owner.
        return Err(ApiError::Conflict(OWNERSHIP_TRANSFER_REQUIRED.into()));
    }
    let next_role = requested_role
        .map(core_types::TenantRole::as_str)
        .unwrap_or(current_role.as_str());
    let next_status = b.status.as_deref().unwrap_or(current_status.as_str());

    // Administrators may manage staff and learners, but only the owner may
    // appoint, demote, suspend, or reactivate another administrator.
    if (current_role == "org_admin" || next_role == "org_admin") && !ctx.owns_organization() {
        return Err(ApiError::Forbidden);
    }

    if !valid_membership_status_transition(&current_status, next_status) {
        return Err(ApiError::Conflict(
            "invalid_membership_status_transition".into(),
        ));
    }

    // Reactivation consumes a seat. Check it inside the same tenant-locked
    // transaction as the update so concurrent membership changes cannot both
    // observe the last available seat.
    if current_status != "active" && next_status == "active" {
        let usage = db::seats::seat_usage_in_tx(&mut tx, tenant_id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        if usage.would_block() {
            return Err(ApiError::Conflict(
                crate::handlers::member_invitations::SEAT_LIMIT_REACHED.into(),
            ));
        }
    }

    let row: Option<(
        Uuid,
        Option<String>,
        Option<String>,
        String,
        String,
        chrono::DateTime<chrono::Utc>,
    )> = sqlx::query_as(
        "WITH updated AS (
            UPDATE tenant_memberships
                SET role = COALESCE($3, role),
                    status = COALESCE($4, status),
                    updated_at = now()
              WHERE tenant_id = $1 AND user_id = $2
            RETURNING tenant_id, user_id, role, status, joined_at
         )
         SELECT u.user_id, usr.email, usr.display_name, u.role, u.status, u.joined_at
           FROM updated u
           JOIN users usr ON usr.id = u.user_id",
    )
    .bind(tenant_id)
    .bind(user_id)
    .bind(b.role.as_deref())
    .bind(b.status.as_deref())
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    let (user_id, email, display_name, role, status, joined_at) = row.ok_or(ApiError::NotFound)?;
    db::audit::emit_audit_event(
        &mut tx,
        tenant_id,
        ctx.user_id,
        "tenant_membership.patch",
        "user",
        user_id,
        Some(serde_json::json!({
            "previous_role": current_role,
            "previous_status": current_status,
            "role": role,
            "status": status,
        })),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(MembershipDto {
        user_id,
        email,
        display_name,
        role,
        status,
        joined_at,
    }))
}

#[derive(Deserialize)]
pub struct TransferOwnershipBody {
    pub new_owner_user_id: Uuid,
    pub confirmation: String,
}

#[derive(Serialize)]
pub struct OwnershipTransferResponse {
    pub previous_owner_user_id: Uuid,
    pub new_owner_user_id: Uuid,
    pub transferred_at: chrono::DateTime<chrono::Utc>,
}

async fn transfer_ownership(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Json(body): Json<TransferOwnershipBody>,
) -> Result<Json<OwnershipTransferResponse>, ApiError> {
    transfer_ownership_inner(&state.pool, &ctx, body).await
}

async fn transfer_ownership_t(
    State(state): State<MembershipTestState>,
    Extension(ctx): Extension<RequestContext>,
    Json(body): Json<TransferOwnershipBody>,
) -> Result<Json<OwnershipTransferResponse>, ApiError> {
    transfer_ownership_inner(&state.pool, &ctx, body).await
}

async fn transfer_ownership_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    body: TransferOwnershipBody,
) -> Result<Json<OwnershipTransferResponse>, ApiError> {
    // Platform support uses the separate break-glass recovery route. Requiring
    // the exact tenant role here prevents an implicit support override from
    // becoming an ordinary ownership transfer.
    if !ctx.owns_organization() {
        return Err(ApiError::Forbidden);
    }
    if body.confirmation.trim() != TRANSFER_CONFIRMATION {
        return Err(ApiError::Validation(
            "ownership_confirmation_required".into(),
        ));
    }
    if body.new_owner_user_id == ctx.user_id {
        return Err(ApiError::Validation(
            "new_owner_must_be_another_admin".into(),
        ));
    }
    let tenant_id = ctx
        .tenant_id
        .ok_or_else(|| ApiError::BadRequest("no tenant".into()))?;

    let mut tx = db::begin_with_context(pool, ctx.user_id, Some(tenant_id))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let transferred =
        db::organization::transfer_ownership(&mut tx, tenant_id, body.new_owner_user_id)
            .await
            .map_err(map_ownership_db_error)?;
    tx.commit().await.map_err(map_ownership_db_error)?;

    Ok(Json(OwnershipTransferResponse {
        previous_owner_user_id: transferred.previous_owner_user_id,
        new_owner_user_id: transferred.new_owner_user_id,
        transferred_at: transferred.transferred_at,
    }))
}

fn map_ownership_db_error(error: sqlx::Error) -> ApiError {
    if let sqlx::Error::Database(database_error) = &error {
        if database_error.message().contains("tenant_not_found") {
            return ApiError::NotFound;
        }
        if database_error
            .message()
            .contains(TARGET_MUST_BE_ACTIVE_ORG_ADMIN)
        {
            return ApiError::Conflict(TARGET_MUST_BE_ACTIVE_ORG_ADMIN.into());
        }
        if database_error
            .message()
            .contains("organization_owner_required")
        {
            return ApiError::Forbidden;
        }
        if database_error.constraint() == Some("tenant_memberships_active_org_owner_required") {
            return ApiError::Conflict(OWNERSHIP_TRANSFER_REQUIRED.into());
        }
    }
    ApiError::Internal(error.to_string())
}

#[derive(Serialize)]
pub struct FileAssetSummaryDto {
    pub asset_id: Uuid,
    pub owner_user_id: Uuid,
    pub content_type: String,
    pub size_bytes: i64,
    pub linked_entity_type: Option<String>,
    pub linked_entity_id: Option<Uuid>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Serialize)]
pub struct FileAssetListResponse {
    pub assets: Vec<FileAssetSummaryDto>,
}

async fn list_file_assets(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<FileAssetListResponse>, ApiError> {
    if !ctx.can_manage_organization() {
        return Err(ApiError::Forbidden);
    }
    let tenant_id = ctx
        .tenant_id
        .ok_or_else(|| ApiError::BadRequest("no tenant".into()))?;

    let mut tx = db::begin_with_context(&state.pool, ctx.user_id, Some(tenant_id))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let rows = crate::db::file_assets::list_for_tenant(&mut *tx, tenant_id, 200)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let assets = rows
        .into_iter()
        .map(|r| FileAssetSummaryDto {
            asset_id: r.id,
            owner_user_id: r.owner_user_id,
            content_type: r.content_type,
            size_bytes: r.size_bytes,
            linked_entity_type: r.linked_entity_type,
            linked_entity_id: r.linked_entity_id,
            created_at: r.created_at,
        })
        .collect();
    Ok(Json(FileAssetListResponse { assets }))
}

#[derive(Serialize)]
pub struct TenantDto {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub status: String,
    pub recording_default: bool,
    pub recording_retention_days: i32,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

async fn get_tenant(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<TenantDto>, ApiError> {
    if !ctx.can_manage_organization() {
        return Err(ApiError::Forbidden);
    }
    let tenant_id = ctx
        .tenant_id
        .ok_or_else(|| ApiError::BadRequest("no tenant".into()))?;

    let mut tx = db::begin_with_context(&state.pool, ctx.user_id, Some(tenant_id))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let row: Option<(
        Uuid,
        String,
        String,
        String,
        bool,
        i32,
        chrono::DateTime<chrono::Utc>,
    )> = sqlx::query_as(
        "SELECT id, slug, name, status, recording_default, recording_retention_days, created_at
           FROM tenants WHERE id = $1",
    )
    .bind(tenant_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let (id, slug, name, status, recording_default, recording_retention_days, created_at) =
        row.ok_or(ApiError::NotFound)?;
    Ok(Json(TenantDto {
        id,
        slug,
        name,
        status,
        recording_default,
        recording_retention_days,
        created_at,
    }))
}

#[derive(Serialize)]
pub struct MembershipDto {
    pub user_id: Uuid,
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub role: String,
    pub status: String,
    pub joined_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Serialize)]
pub struct MembershipListResponse {
    pub memberships: Vec<MembershipDto>,
}

async fn list_memberships(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<MembershipListResponse>, ApiError> {
    if !ctx.can_manage_members() {
        return Err(ApiError::Forbidden);
    }
    let tenant_id = ctx
        .tenant_id
        .ok_or_else(|| ApiError::BadRequest("no tenant".into()))?;

    let mut tx = db::begin_with_context(&state.pool, ctx.user_id, Some(tenant_id))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let rows: Vec<(
        Uuid,
        Option<String>,
        Option<String>,
        String,
        String,
        chrono::DateTime<chrono::Utc>,
    )> = sqlx::query_as(
        "SELECT tm.user_id, u.email, u.display_name, tm.role, tm.status, tm.joined_at
           FROM tenant_memberships tm
           JOIN users u ON u.id = tm.user_id
          WHERE tm.tenant_id = $1
          ORDER BY tm.joined_at ASC",
    )
    .bind(tenant_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let memberships = rows
        .into_iter()
        .map(
            |(user_id, email, display_name, role, status, joined_at)| MembershipDto {
                user_id,
                email,
                display_name,
                role,
                status,
                joined_at,
            },
        )
        .collect();
    Ok(Json(MembershipListResponse { memberships }))
}

async fn list_audit(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Query(q): Query<AuditQuery>,
) -> Result<Json<AuditListResponse>, ApiError> {
    if !ctx.can_manage_organization() {
        return Err(ApiError::Forbidden);
    }
    let tenant_id = ctx
        .tenant_id
        .ok_or_else(|| ApiError::BadRequest("no tenant".into()))?;
    let limit = q.limit.unwrap_or(50).clamp(1, 200);

    let mut tx = db::begin_with_context(&state.pool, ctx.user_id, Some(tenant_id))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let rows = db::audit::list_for_tenant(&mut *tx, tenant_id, limit, q.before)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let events = rows
        .into_iter()
        .map(|r| AuditEventDto {
            id: r.id,
            actor_user_id: r.actor_user_id,
            actor_email: r.actor_email,
            actor_display_name: r.actor_display_name,
            action: r.action,
            resource_type: r.resource_type,
            resource_id: r.resource_id,
            metadata: r.metadata,
            occurred_at: r.occurred_at,
        })
        .collect();
    Ok(Json(AuditListResponse { events }))
}

#[cfg(test)]
mod tests {
    use super::valid_membership_status_transition;

    #[test]
    fn membership_status_transitions_are_explicit() {
        for (from, to) in [
            ("active", "active"),
            ("active", "suspended"),
            ("suspended", "suspended"),
            ("suspended", "active"),
            ("invited", "invited"),
            ("invited", "active"),
            ("invited", "suspended"),
        ] {
            assert!(valid_membership_status_transition(from, to));
        }
        assert!(!valid_membership_status_transition("active", "invited"));
        assert!(!valid_membership_status_transition("suspended", "invited"));
        assert!(!valid_membership_status_transition("removed", "active"));
    }
}
