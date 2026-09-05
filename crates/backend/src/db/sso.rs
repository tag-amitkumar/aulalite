// crates/backend/src/db/sso.rs
//! Enterprise SSO data layer: per-tenant OIDC IdP configuration plus the
//! short-lived authorization-request state needed to complete the OIDC
//! authorization-code flow.
//!
//! Two tables:
//!   * `tenant_sso_configs` — one OIDC IdP per tenant (issuer, client id/secret,
//!     discovery/endpoint URLs, enabled flag). Tenant-scoped under RLS exactly
//!     like `announcements` for the org-admin management path; PLUS a
//!     `system_context` SELECT so the UNAUTHENTICATED `/v1/sso/:slug/start`
//!     handler can resolve a tenant's config cross-tenant (it has no
//!     `app.tenant_id` until the tenant is resolved from the slug).
//!   * `sso_login_states` — server-side `state`/`nonce` for one in-flight login,
//!     created at `/start` and consumed (deleted) at `/callback`. Written and
//!     read by the UNAUTHENTICATED handlers, so it is reached exclusively via
//!     the `system_context` policies (`app.system='on'`).
//!
//! Client secrets are authenticated-encrypted before they reach the DB and are
//! NEVER returned to the browser (the admin handler DTO omits the secret).
//!
//! Uses ONLY runtime sqlx (no compile-time macros — there is no DATABASE_URL at
//! build time).
use sqlx::PgPool;
use uuid::Uuid;

use super::{begin_system_context, begin_with_context};

/// Full per-tenant OIDC config with its secret decrypted in memory for the
/// token exchange; handlers strip it before serializing anything to a client.
pub struct SsoConfig {
    pub tenant_id: Uuid,
    pub issuer: String,
    pub client_id: String,
    pub client_secret: String,
    pub authorize_url: String,
    pub token_url: String,
    pub jwks_url: String,
    pub enabled: bool,
}

/// Secret-free view used by the org-admin settings surface. Keeping this as a
/// separate type prevents routine reads and successful writes from decrypting
/// provider credentials merely to render `has_client_secret`.
pub struct SsoConfigMetadata {
    pub issuer: String,
    pub client_id: String,
    pub authorize_url: String,
    pub token_url: String,
    pub jwks_url: String,
    pub enabled: bool,
    pub has_client_secret: bool,
}

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

type ConfigTuple = (Uuid, String, String, String, String, String, String, bool);

pub(crate) fn secret_context(tenant_id: Uuid) -> String {
    format!("tenant-sso:{tenant_id}:client-secret")
}

fn secret_error(error: crate::services::secret_box::SecretBoxError) -> sqlx::Error {
    sqlx::Error::Protocol(format!("tenant SSO secret unavailable: {error}"))
}

fn config_from_tuple(t: ConfigTuple) -> sqlx::Result<SsoConfig> {
    let client_secret =
        crate::services::secret_box::open_text(&t.3, secret_context(t.0).as_bytes())
            .map_err(secret_error)?;
    Ok(SsoConfig {
        tenant_id: t.0,
        issuer: t.1,
        client_id: t.2,
        client_secret,
        authorize_url: t.4,
        token_url: t.5,
        jwks_url: t.6,
        enabled: t.7,
    })
}

const SELECT_COLS: &str =
    "tenant_id, issuer, client_id, client_secret, authorize_url, token_url, jwks_url, enabled";
const SELECT_METADATA_COLS: &str =
    "issuer, client_id, authorize_url, token_url, jwks_url, enabled, client_secret <> ''";
type ConfigMetadataTuple = (String, String, String, String, String, bool, bool);

fn metadata_from_tuple(t: ConfigMetadataTuple) -> SsoConfigMetadata {
    SsoConfigMetadata {
        issuer: t.0,
        client_id: t.1,
        authorize_url: t.2,
        token_url: t.3,
        jwks_url: t.4,
        enabled: t.5,
        has_client_secret: t.6,
    }
}

/// Org-admin upsert of the tenant's OIDC config. Tenant-scoped.
#[allow(clippy::too_many_arguments)]
pub async fn upsert_config(
    pool: &PgPool,
    tenant_id: Uuid,
    issuer: &str,
    client_id: &str,
    client_secret: Option<&str>,
    authorize_url: &str,
    token_url: &str,
    jwks_url: &str,
    enabled: bool,
) -> sqlx::Result<SsoConfigMetadata> {
    let encrypted_secret = client_secret
        .map(|client_secret| {
            crate::services::secret_box::seal_text(
                client_secret,
                secret_context(tenant_id).as_bytes(),
            )
            .map_err(secret_error)
        })
        .transpose()?;
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    let row: ConfigMetadataTuple = if let Some(encrypted_secret) = encrypted_secret {
        sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "INSERT INTO tenant_sso_configs
                 (tenant_id, issuer, client_id, client_secret, authorize_url, token_url, jwks_url, enabled)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
             ON CONFLICT (tenant_id) DO UPDATE
                 SET issuer = EXCLUDED.issuer,
                     client_id = EXCLUDED.client_id,
                     client_secret = EXCLUDED.client_secret,
                     authorize_url = EXCLUDED.authorize_url,
                     token_url = EXCLUDED.token_url,
                     jwks_url = EXCLUDED.jwks_url,
                     enabled = EXCLUDED.enabled,
                     updated_at = now()
             RETURNING {SELECT_METADATA_COLS}"
        )))
        .bind(tenant_id)
        .bind(issuer)
        .bind(client_id)
        .bind(encrypted_secret)
        .bind(authorize_url)
        .bind(token_url)
        .bind(jwks_url)
        .bind(enabled)
        .fetch_one(&mut *tx)
        .await?
    } else {
        // Preserve the ciphertext in the same atomic UPDATE that changes the
        // public metadata. Fetch-then-upsert in the handler could otherwise
        // restore a stale plaintext secret over a concurrent rotation.
        sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "UPDATE tenant_sso_configs
                SET issuer = $2,
                    client_id = $3,
                    authorize_url = $4,
                    token_url = $5,
                    jwks_url = $6,
                    enabled = $7,
                    updated_at = now()
              WHERE tenant_id = $1
          RETURNING {SELECT_METADATA_COLS}"
        )))
        .bind(tenant_id)
        .bind(issuer)
        .bind(client_id)
        .bind(authorize_url)
        .bind(token_url)
        .bind(jwks_url)
        .bind(enabled)
        .fetch_one(&mut *tx)
        .await?
    };
    tx.commit().await?;
    Ok(metadata_from_tuple(row))
}

/// Fetch secret-free tenant config for the owner/admin settings path.
pub async fn get_config_metadata(
    pool: &PgPool,
    tenant_id: Uuid,
) -> sqlx::Result<Option<SsoConfigMetadata>> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    let row: Option<ConfigMetadataTuple> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {SELECT_METADATA_COLS} FROM tenant_sso_configs WHERE tenant_id = $1"
    )))
    .bind(tenant_id)
    .fetch_optional(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(row.map(metadata_from_tuple))
}

/// Resolve an ENABLED OIDC config by the tenant's slug, cross-tenant. Used by
/// the unauthenticated `/v1/sso/:slug/start` and `/callback` handlers, which run
/// before tenant resolution and therefore elevate to `app.system='on'` (guarded
/// by the `system_context_select` policies on both `tenants` and
/// `tenant_sso_configs`). Returns `None` when the slug is unknown, has no config,
/// or the config is disabled.
pub async fn get_enabled_config_by_slug(
    pool: &PgPool,
    slug: &str,
) -> sqlx::Result<Option<SsoConfig>> {
    let mut tx = begin_system_context(pool).await?;
    let row: Option<ConfigTuple> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {SELECT_COLS}
           FROM tenant_sso_configs c
           JOIN tenants t ON t.id = c.tenant_id
          WHERE t.slug = $1
            AND c.enabled = true
            AND tenant_access_allowed(t.id)"
    )))
    .bind(slug)
    .fetch_optional(&mut *tx)
    .await?;
    tx.commit().await?;
    row.map(config_from_tuple).transpose()
}

/// Resolve an ENABLED OIDC config by tenant_id, cross-context. Used at callback
/// time, where the tenant_id is carried in the persisted login state.
pub async fn get_enabled_config_by_tenant(
    pool: &PgPool,
    tenant_id: Uuid,
) -> sqlx::Result<Option<SsoConfig>> {
    let mut tx = begin_system_context(pool).await?;
    let row: Option<ConfigTuple> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {SELECT_COLS}
           FROM tenant_sso_configs c
          WHERE tenant_id = $1
            AND enabled = true
            AND EXISTS (
                SELECT 1 FROM tenants t
                 WHERE t.id = c.tenant_id
                   AND tenant_access_allowed(t.id)
            )"
    )))
    .bind(tenant_id)
    .fetch_optional(&mut *tx)
    .await?;
    tx.commit().await?;
    row.map(config_from_tuple).transpose()
}

// ---------------------------------------------------------------------------
// In-flight login state (state + nonce), system-context only.
// ---------------------------------------------------------------------------

/// A consumed login-state row: the values needed to validate the callback.
#[derive(Debug, Clone)]
pub struct LoginState {
    pub tenant_id: Uuid,
    pub nonce: String,
    pub redirect_uri: String,
    /// PKCE (RFC 7636) `code_verifier` we minted at `/start`; replayed on the
    /// token exchange so the IdP can bind the code to this client.
    pub code_verifier: String,
}

/// Persist one in-flight authorization request keyed by the random `state`.
/// Cross-context (system) write — the `/start` handler is unauthenticated.
pub async fn insert_login_state(
    pool: &PgPool,
    state: &str,
    tenant_id: Uuid,
    nonce: &str,
    redirect_uri: &str,
    code_verifier: &str,
) -> sqlx::Result<()> {
    let mut tx = begin_system_context(pool).await?;
    // A user may abandon the IdP flow and never hit the callback, so callback-
    // only cleanup is insufficient. Opportunistic GC on insert keeps the
    // unauthenticated state table bounded without a separate maintenance loop.
    sqlx::query("DELETE FROM sso_login_states WHERE created_at <= now() - interval '15 minutes'")
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO sso_login_states (state, tenant_id, nonce, redirect_uri, code_verifier)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(state)
    .bind(tenant_id)
    .bind(nonce)
    .bind(redirect_uri)
    .bind(code_verifier)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Atomically consume (delete + return) a login state by its `state` value, but
/// only if it has not expired (15-minute TTL). Returns `None` when the state is
/// unknown or expired — both treated as an invalid callback. Cross-context.
pub async fn take_login_state(pool: &PgPool, state: &str) -> sqlx::Result<Option<LoginState>> {
    let mut tx = begin_system_context(pool).await?;
    // `code_verifier` may be NULL on rows written before the PKCE migration; we
    // coalesce to '' so the (transitional) row still resolves and the token
    // exchange simply omits a verifier the IdP never received a challenge for.
    let row: Option<(Uuid, String, String, String)> = sqlx::query_as(
        "DELETE FROM sso_login_states
          WHERE state = $1 AND created_at > now() - interval '15 minutes'
         RETURNING tenant_id, nonce, redirect_uri, COALESCE(code_verifier, '')",
    )
    .bind(state)
    .fetch_optional(&mut *tx)
    .await?;
    // Opportunistic GC of any other expired rows in the same tx.
    sqlx::query("DELETE FROM sso_login_states WHERE created_at <= now() - interval '15 minutes'")
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(row.map(
        |(tenant_id, nonce, redirect_uri, code_verifier)| LoginState {
            tenant_id,
            nonce,
            redirect_uri,
            code_verifier,
        },
    ))
}

/// JIT-provision an SSO-authenticated user into `tenant_id` as a member: ensure
/// a `tenant_memberships` row exists (default role `student`) and is `active`.
/// The callback runs before normal request tenant resolution, but already knows
/// the concrete SSO/LTI tenant and therefore opens a tenant-scoped RLS
/// transaction. Idempotent: re-login never downgrades an existing role. A
/// missing or inactive membership is activated only when plan capacity allows.
pub async fn ensure_tenant_membership(
    pool: &PgPool,
    tenant_id: Uuid,
    user_id: Uuid,
    default_role: &str,
) -> sqlx::Result<crate::db::seats::MembershipActivationOutcome> {
    // The callback has resolved one concrete tenant, so use its normal RLS
    // context instead of a cross-tenant system transaction.
    let mut tx = begin_with_context(pool, user_id, Some(tenant_id)).await?;
    let outcome = crate::db::seats::ensure_active_tenant_membership(
        &mut tx,
        tenant_id,
        user_id,
        default_role,
    )
    .await?;
    tx.commit().await?;
    Ok(outcome)
}

/// Activate a tenant-scoped enterprise identity, consuming only an invitation
/// issued by this same tenant. Unlike platform-global JIT provisioning, this
/// never searches or mutates invitations in another workspace.
pub async fn ensure_enterprise_tenant_membership(
    pool: &PgPool,
    tenant_id: Uuid,
    user_id: Uuid,
    asserted_email: &str,
    default_role: &str,
) -> sqlx::Result<crate::db::seats::MembershipActivationOutcome> {
    use crate::db::seats::MembershipActivationOutcome;

    let mut tx = begin_with_context(pool, user_id, Some(tenant_id)).await?;
    crate::db::seats::lock_tenant_for_seat_mutation(&mut tx, tenant_id).await?;

    let invitation: Option<(Uuid, String)> = sqlx::query_as(
        "SELECT id, role
           FROM tenant_invitations
          WHERE tenant_id = $1
            AND status = 'pending'
            AND lower(email::text) = lower($2)
          ORDER BY created_at, id
          LIMIT 1
          FOR UPDATE",
    )
    .bind(tenant_id)
    .bind(asserted_email)
    .fetch_optional(&mut *tx)
    .await?;

    let role = invitation
        .as_ref()
        .map(|(_, role)| role.as_str())
        .unwrap_or(default_role);

    if let Some((invitation_id, invitation_role)) = invitation.as_ref() {
        if matches!(invitation_role.as_str(), "org_owner" | "org_admin") {
            let result: String = sqlx::query_scalar(
                "SELECT accept_privileged_enterprise_invitation($1, $2, $3, $4)",
            )
            .bind(invitation_id)
            .bind(tenant_id)
            .bind(user_id)
            .bind(asserted_email)
            .fetch_one(&mut *tx)
            .await?;
            tx.commit().await?;
            return Ok(match result.as_str() {
                "activated" => MembershipActivationOutcome::Activated,
                "suspended" => MembershipActivationOutcome::Suspended,
                "preserved" => MembershipActivationOutcome::AlreadyActive,
                _ => return Err(sqlx::Error::Protocol("unknown activation outcome".into())),
            });
        }
    }

    if let Some((invitation_id, _)) = invitation.as_ref() {
        let status: Option<String> = sqlx::query_scalar(
            "SELECT status FROM tenant_memberships
              WHERE tenant_id = $1 AND user_id = $2
              FOR UPDATE",
        )
        .bind(tenant_id)
        .bind(user_id)
        .fetch_optional(&mut *tx)
        .await?;
        if status.as_deref() == Some("suspended") {
            sqlx::query("UPDATE tenant_invitations SET status = 'revoked' WHERE id = $1")
                .bind(invitation_id)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
            return Ok(MembershipActivationOutcome::Suspended);
        }

        // Remove the already-reserved pending seat before activation. Both
        // changes are in this transaction, so total usage cannot transiently
        // exceed the cap and rollback restores the reservation on any error.
        sqlx::query(
            "UPDATE tenant_invitations
                SET status = 'accepted', accepted_at = now()
              WHERE id = $1",
        )
        .bind(invitation_id)
        .execute(&mut *tx)
        .await?;
    }

    let outcome =
        crate::db::seats::ensure_active_tenant_membership(&mut tx, tenant_id, user_id, role)
            .await?;

    if invitation.is_some() && outcome == MembershipActivationOutcome::Activated {
        sqlx::query(
            "UPDATE tenant_memberships SET role = $3, updated_at = now()
              WHERE tenant_id = $1 AND user_id = $2",
        )
        .bind(tenant_id)
        .bind(user_id)
        .bind(role)
        .execute(&mut *tx)
        .await?;
    }

    if outcome == MembershipActivationOutcome::SeatLimitReached && invitation.is_some() {
        tx.rollback().await?;
        return Ok(outcome);
    }
    tx.commit().await?;
    Ok(outcome)
}
