// crates/backend/src/db/lti.rs
//! LTI 1.3 platform registrations (Tool side).
//!
//! We are the LTI **Tool**: an external LMS **platform** (Canvas/Moodle/Blackboard)
//! launches its users into AulaLite. Each tenant registers one or more platforms
//! identified by `(issuer, client_id)`. A registration stores the platform's OIDC
//! third-party-init `auth_login_url`, its `jwks_url` (so we can verify the launch
//! `id_token`), and the `deployment_id` we accept.
//!
//! TENANT-SCOPED under RLS exactly like `db::announcements` for the admin
//! register/list/delete path (policy keys solely on `app.tenant_id`). The login +
//! launch flows are UNAUTHENTICATED — at that point we have only the `iss`
//! (+ optional `client_id`) from the platform and no resolved tenant — so the
//! lookup runs cross-tenant under `app.system='on'`, guarded by the
//! `system_context_select` policy (mirrors `db::api_keys::authenticate` +
//! migration 20260614000047/055). Uses ONLY runtime sqlx (no compile-time macros).
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use super::begin_system_context;

/// One registered LTI platform. The `client_id` is what the platform issued to
/// AulaLite; it is also the `aud` we require on inbound launch tokens.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct LtiPlatformRow {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub name: String,
    pub issuer: String,
    pub client_id: String,
    pub auth_login_url: String,
    pub jwks_url: String,
    pub deployment_id: String,
    /// Course every launch from this platform lands on when the resource-link
    /// claim doesn't otherwise resolve. Optional.
    pub default_course_id: Option<Uuid>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginState {
    pub nonce: String,
    pub target_link_uri: Option<String>,
}

const SELECT_COLS: &str = "id, tenant_id, name, issuer, client_id, auth_login_url, \
     jwks_url, deployment_id, default_course_id, created_at, updated_at";

/// Set the tenant GUC the RLS policy depends on, inside `tx`.
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

/// Register a new platform for `tenant_id`. Tenant-scoped under RLS.
#[allow(clippy::too_many_arguments)]
pub async fn insert(
    pool: &PgPool,
    tenant_id: Uuid,
    name: &str,
    issuer: &str,
    client_id: &str,
    auth_login_url: &str,
    jwks_url: &str,
    deployment_id: &str,
    default_course_id: Option<Uuid>,
) -> sqlx::Result<LtiPlatformRow> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    let row = sqlx::query_as::<_, LtiPlatformRow>(sqlx::AssertSqlSafe(format!(
        "INSERT INTO lti_platforms
             (tenant_id, name, issuer, client_id, auth_login_url, jwks_url,
              deployment_id, default_course_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         RETURNING {SELECT_COLS}"
    )))
    .bind(tenant_id)
    .bind(name)
    .bind(issuer)
    .bind(client_id)
    .bind(auth_login_url)
    .bind(jwks_url)
    .bind(deployment_id)
    .bind(default_course_id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(row)
}

/// List a tenant's registered platforms, newest first. Tenant-scoped.
pub async fn list(pool: &PgPool, tenant_id: Uuid) -> sqlx::Result<Vec<LtiPlatformRow>> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    let rows = sqlx::query_as::<_, LtiPlatformRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {SELECT_COLS} FROM lti_platforms ORDER BY created_at DESC, id DESC"
    )))
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(rows)
}

/// Delete a platform by id within `tenant_id`. Returns true if a row was removed.
/// Tenant-scoped.
pub async fn delete(pool: &PgPool, tenant_id: Uuid, id: Uuid) -> sqlx::Result<bool> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    let res = sqlx::query("DELETE FROM lti_platforms WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(res.rows_affected() > 0)
}

/// Resolve a platform by `issuer` (and, when supplied, `client_id`) for the
/// UNAUTHENTICATED OIDC login + launch flows. Runs WITHOUT a tenant GUC (the
/// tenant is unknown until the platform is resolved), so it elevates to
/// `app.system='on'` for the cross-tenant lookup — guarded by the
/// `system_context_select` policy (migration after 056). Mirrors
/// `db::api_keys::authenticate`.
///
/// When the platform sends a `client_id` (always present in the launch, optional
/// in the third-party-init request) we match on it so two registrations sharing
/// an issuer disambiguate; otherwise we fall back to the single issuer match.
pub async fn find_by_issuer(
    pool: &PgPool,
    issuer: &str,
    client_id: Option<&str>,
) -> sqlx::Result<Option<LtiPlatformRow>> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.system', 'on', true)")
        .execute(&mut *tx)
        .await?;

    let row = match client_id {
        Some(cid) => {
            sqlx::query_as::<_, LtiPlatformRow>(sqlx::AssertSqlSafe(format!(
                "SELECT {SELECT_COLS} FROM lti_platforms lp
                  WHERE issuer = $1 AND client_id = $2
                    AND EXISTS (
                        SELECT 1 FROM tenants t
                         WHERE t.id = lp.tenant_id
                           AND tenant_access_allowed(t.id)
                    )
                  ORDER BY created_at DESC LIMIT 1"
            )))
            .bind(issuer)
            .bind(cid)
            .fetch_optional(&mut *tx)
            .await?
        }
        None => {
            sqlx::query_as::<_, LtiPlatformRow>(sqlx::AssertSqlSafe(format!(
                "SELECT {SELECT_COLS} FROM lti_platforms lp
                  WHERE issuer = $1
                    AND EXISTS (
                        SELECT 1 FROM tenants t
                         WHERE t.id = lp.tenant_id
                           AND tenant_access_allowed(t.id)
                    )
                  ORDER BY created_at DESC LIMIT 1"
            )))
            .bind(issuer)
            .fetch_optional(&mut *tx)
            .await?
        }
    };
    tx.commit().await?;
    Ok(row)
}

/// Persist one in-flight LTI OIDC login keyed by the random `state`.
/// Cross-context (system) write because `/v1/lti/login` is unauthenticated and
/// runs before a tenant is resolved.
pub async fn insert_login_state(
    pool: &PgPool,
    state: &str,
    nonce: &str,
    target_link_uri: Option<&str>,
) -> sqlx::Result<()> {
    let mut tx = begin_system_context(pool).await?;
    // Abandoned browser redirects never reach `take_login_state`; clean their
    // expired rows on every new flow so this public endpoint cannot grow the
    // state table without bound.
    sqlx::query("DELETE FROM lti_login_states WHERE created_at <= now() - interval '15 minutes'")
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO lti_login_states (state, nonce, target_link_uri)
         VALUES ($1, $2, $3)",
    )
    .bind(state)
    .bind(nonce)
    .bind(target_link_uri)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Atomically consume a login state by `state`, but only if it is still within
/// the 15-minute TTL. Returns `None` for unknown, already-consumed, or expired
/// state values. Cross-context because `/v1/lti/launch` is unauthenticated.
pub async fn take_login_state(pool: &PgPool, state: &str) -> sqlx::Result<Option<LoginState>> {
    let mut tx = begin_system_context(pool).await?;
    let row: Option<(String, Option<String>)> = sqlx::query_as(
        "DELETE FROM lti_login_states
          WHERE state = $1 AND created_at > now() - interval '15 minutes'
         RETURNING nonce, target_link_uri",
    )
    .bind(state)
    .fetch_optional(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM lti_login_states WHERE created_at <= now() - interval '15 minutes'")
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(row.map(|(nonce, target_link_uri)| LoginState {
        nonce,
        target_link_uri,
    }))
}
