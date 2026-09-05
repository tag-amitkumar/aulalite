use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::header::{AUTHORIZATION, CONNECTION, UPGRADE};
use axum::http::{HeaderMap, Method};
use axum::middleware::Next;
use axum::response::Response;
use percent_encoding::percent_decode_str;
use sqlx::PgPool;

use crate::auth::{
    identity::IdentityScope,
    jit_provision::{ensure_enterprise_user, ensure_user_with_self_service, ProvisionError},
    local_login::LocalLoginConfig,
    super_admin::SuperAdminConfig,
    verify::Verifier,
};
use crate::context::RequestContext;
use crate::error::ApiError;

/// Selects the tenant membership used to build [`RequestContext`]. Clients
/// persist the UUID locally and send it on every authenticated HTTP request.
/// Browser WebSockets cannot set custom headers, so upgrade requests may send
/// the same value as the `workspace_id` query parameter.
pub const TENANT_HEADER: &str = "x-aulalite-tenant";

#[derive(Clone)]
pub struct AuthState {
    pub pool: PgPool,
    pub verifier: Arc<Verifier>,
    pub local_login: LocalLoginConfig,
    pub super_admins: SuperAdminConfig,
}

pub async fn require_auth(
    State(state): State<AuthState>,
    mut req: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let token =
        resolve_token(&req).ok_or_else(|| ApiError::Unauthorized("missing bearer token".into()))?;

    let (claims, identity_scope, is_firebase_identity, mfa_verified) =
        match state.verifier.verify(&token).await {
            Ok(claims) => (claims, IdentityScope::Global, true, false),
            Err(verify_err) => {
                // 1) local-login bypass token (dev only)
                if let Some(c) = state.local_login.claims_for_token(&token) {
                    (c, IdentityScope::Global, false, false)
                } else if let Some(secret) = crate::services::oidc::session_secret_from_env() {
                    // 2) self-signed enterprise-SSO session token (HS256)
                    let session = crate::services::oidc::verify_session_token(&secret, &token)
                        .ok_or_else(|| {
                            tracing::debug!(error = %verify_err, "bearer token rejected");
                            ApiError::Unauthorized("invalid bearer token".into())
                        })?;
                    (
                        session.claims,
                        session.identity_scope,
                        false,
                        session.mfa_verified,
                    )
                } else {
                    tracing::debug!(error = %verify_err, "bearer token rejected");
                    return Err(ApiError::Unauthorized("invalid bearer token".into()));
                }
            }
        };

    let user = match identity_scope {
        IdentityScope::Global => {
            ensure_user_with_self_service(
                &state.pool,
                &claims,
                is_firebase_identity && self_service_signup_enabled(),
            )
            .await
        }
        IdentityScope::Enterprise {
            tenant_id,
            provider,
        } => ensure_enterprise_user(&state.pool, &claims, tenant_id, provider).await,
    }
    .map_err(map_provision_error)?;

    // Environment-configured super admins are promoted only for platform-global
    // identities. Tenant-controlled SSO/LTI assertions can never gain platform
    // authority even when they reuse a configured email address.
    if matches!(identity_scope, IdentityScope::Global)
        && state
            .super_admins
            .promote_user_if_configured(&state.pool, user.user_id, &user.email)
            .await
            .map_err(|err| ApiError::Internal(format!("super-admin bootstrap failed: {err}")))?
    {
        tracing::info!(user_id = %user.user_id, "environment-configured super admin promoted");
    }

    let requested_tenant_id = requested_tenant_id(&req)?;
    let selected_tenant_id = match identity_scope.enterprise_tenant_id() {
        Some(identity_tenant_id) => match requested_tenant_id {
            Some(requested) if requested != identity_tenant_id => {
                return Err(ApiError::Forbidden);
            }
            _ => Some(identity_tenant_id),
        },
        None => requested_tenant_id,
    };

    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(|err| ApiError::Internal(format!("transaction begin failed: {err}")))?;

    // NOTE: `set_config(name, value, true)` is transaction-LOCAL. The GUC is
    // discarded when this tx commits below. It is set here only so the
    // `tenant_memberships` lookup that follows can satisfy its RLS policy
    // (`user_id = current_setting('app.user_id', true)::uuid`). Handlers
    // that need RLS context (`app.tenant_id`, `app.user_id`) on subsequent
    // queries MUST re-issue `set_config` at the start of their own
    // transactions — use `crate::db::set_request_guc(tx, user_id, tenant_id)`
    // for the standard call. This is unconditional once `aulalite_app`
    // (migration `20260517000020_app_role.sql`) is the connecting role;
    // until then the existing superuser connection bypasses RLS anyway and
    // the call is defence-in-depth.
    sqlx::query("SELECT set_config('app.user_id', $1, true)")
        .bind(user.user_id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(|err| ApiError::Internal(err.to_string()))?;

    let membership: Option<(uuid::Uuid, String)> =
        sqlx::query_as("SELECT tenant_id, role FROM resolve_active_workspace($1, $2)")
            .bind(user.user_id)
            .bind(selected_tenant_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|err| ApiError::Internal(format!("membership lookup failed: {err}")))?;

    // A platform owner is not implicitly a tenant member. Cross-tenant owner
    // tools use their dedicated SECURITY DEFINER functions; this selection
    // header never grants ordinary tenant permissions.
    if selected_tenant_id.is_some() && membership.is_none() {
        return Err(ApiError::Forbidden);
    }

    tx.commit()
        .await
        .map_err(|err| ApiError::Internal(format!("transaction commit failed: {err}")))?;

    // Single-row lookup: platform-admin flag + the token-revocation cutoff.
    let (is_platform_admin, tokens_valid_after, deleted_at): (
        bool,
        Option<chrono::DateTime<chrono::Utc>>,
        Option<chrono::DateTime<chrono::Utc>>,
    ) = sqlx::query_as(
        "SELECT is_platform_admin, tokens_valid_after, deleted_at FROM users WHERE id = $1",
    )
    .bind(user.user_id)
    .fetch_one(&state.pool)
    .await
    .map_err(|err| ApiError::Internal(err.to_string()))?;

    if deleted_at.is_some() {
        return Err(ApiError::Unauthorized("account_disabled".into()));
    }

    // Token revocation ("sign out everywhere" / account deletion): reject any
    // token issued before the user's cutoff. Fail-CLOSED only when the column
    // is non-null; a null cutoff (the common case) never rejects. `iat` is in
    // whole seconds, so compare against the cutoff second to avoid racing a
    // token minted in the same second as the revocation.
    if let Some(cutoff) = tokens_valid_after {
        if claims.iat <= cutoff.timestamp() {
            return Err(ApiError::Unauthorized("token revoked".into()));
        }
    }

    // MFA step-up gate (opt-in via AULALITE_MFA_ENFORCE=on; default OFF => fail
    // open). When a user has TOTP enabled, only tokens that carry a verified
    // second factor may proceed. The SSO session token (iss == SSO_SESSION_ISS)
    // and MFA challenge route are exempt for now. `is_enabled` FAILS CLOSED:
    // any DB/table error aborts the request with 500 rather than skipping the
    // gate — keep it that way.
    let mfa_enforced = mfa_enforcement_enabled();
    let challenge_request = is_mfa_challenge_request(&req);
    if mfa_enforced {
        let user_mfa_enabled = crate::db::mfa::is_enabled(&state.pool, user.user_id)
            .await
            .map_err(|error| {
                tracing::error!(user_id = %user.user_id, %error, "MFA enforcement lookup failed");
                ApiError::Internal("mfa_state_unavailable".into())
            })?;
        if should_require_mfa_step_up(
            mfa_enforced,
            user_mfa_enabled,
            mfa_verified,
            challenge_request,
        ) {
            return Err(ApiError::Unauthorized("mfa_required".into()));
        }
    }

    let (tenant_id, tenant_role) = match membership {
        Some((tenant_id, role)) => (Some(tenant_id), Some(parse_role(&role)?)),
        None => (None, None),
    };

    let context = RequestContext {
        user_id: user.user_id,
        firebase_uid: user.firebase_uid,
        email: user.email,
        display_name: user.display_name,
        tenant_id,
        tenant_role,
        // Tenant-controlled SSO/LTI identities can never become platform
        // authority, even if a legacy/manual row was mistakenly flagged.
        is_platform_admin: is_platform_admin && matches!(identity_scope, IdentityScope::Global),
        identity_scope,
    };

    req.extensions_mut().insert(context);
    Ok(next.run(req).await)
}

pub(crate) fn map_provision_error(err: ProvisionError) -> ApiError {
    match err {
        ProvisionError::MissingEmail => ApiError::Unauthorized("email_required".into()),
        ProvisionError::EmailNotVerified => {
            ApiError::Unauthorized("email_verification_required".into())
        }
        ProvisionError::SeatLimitReached => {
            ApiError::Conflict(crate::db::seats::SEAT_LIMIT_REACHED.into())
        }
        ProvisionError::AccountDisabled => ApiError::Unauthorized("account_disabled".into()),
        ProvisionError::IdentityScopeConflict => {
            ApiError::Unauthorized("identity_scope_conflict".into())
        }
        err @ ProvisionError::Db(_) => {
            ApiError::Internal(format!("user provisioning failed: {err}"))
        }
    }
}

fn resolve_token(req: &Request) -> Option<String> {
    if let Some(token) = bearer_header(req.headers()) {
        return Some(token.to_string());
    }
    if is_websocket_upgrade(req.headers()) {
        if let Some(query) = req.uri().query() {
            return query_access_token(query);
        }
    }
    None
}

fn requested_tenant_id(req: &Request) -> Result<Option<uuid::Uuid>, ApiError> {
    if let Some(raw) = req.headers().get(TENANT_HEADER) {
        let value = raw
            .to_str()
            .map_err(|_| ApiError::BadRequest("invalid workspace selection".into()))?;
        return parse_tenant_id(value).map(Some);
    }

    if is_websocket_upgrade(req.headers()) {
        if let Some(query) = req.uri().query() {
            if let Some(value) = query_parameter(query, "workspace_id") {
                return parse_tenant_id(&value).map(Some);
            }
        }
    }

    Ok(None)
}

fn parse_tenant_id(value: &str) -> Result<uuid::Uuid, ApiError> {
    uuid::Uuid::parse_str(value.trim())
        .map_err(|_| ApiError::BadRequest("invalid workspace selection".into()))
}

fn mfa_enforcement_enabled() -> bool {
    std::env::var("AULALITE_MFA_ENFORCE")
        .map(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

/// Self-service defaults on only for the explicit local/dev/test allowlist.
/// Production and unknown environments fail closed unless an operator sets an
/// affirmative value. This keeps the public signup policy visible in deploy
/// configuration instead of silently changing it through a missing variable.
fn self_service_signup_enabled() -> bool {
    let app_env = std::env::var("APP_ENV").unwrap_or_else(|_| "production".into());
    resolve_self_service_signup(
        &app_env,
        std::env::var("SELF_SERVICE_SIGNUP_ENABLED").ok().as_deref(),
    )
}

fn resolve_self_service_signup(app_env: &str, configured: Option<&str>) -> bool {
    match configured.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => matches!(
            value.to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        None => !crate::auth::local_login::is_production(app_env),
    }
}

fn is_mfa_challenge_request(req: &Request) -> bool {
    req.method() == Method::POST && req.uri().path() == "/v1/auth/mfa/challenge"
}

fn should_require_mfa_step_up(
    enforcement_enabled: bool,
    user_mfa_enabled: bool,
    mfa_verified: bool,
    challenge_request: bool,
) -> bool {
    enforcement_enabled && user_mfa_enabled && !mfa_verified && !challenge_request
}

fn bearer_header(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(AUTHORIZATION)
        .and_then(|header| header.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
}

fn is_websocket_upgrade(headers: &HeaderMap) -> bool {
    let upgrade_is_websocket = headers
        .get(UPGRADE)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.eq_ignore_ascii_case("websocket"))
        .unwrap_or(false);
    let connection_has_upgrade = headers
        .get(CONNECTION)
        .and_then(|value| value.to_str().ok())
        .map(|value| {
            value
                .split(',')
                .any(|part| part.trim().eq_ignore_ascii_case("upgrade"))
        })
        .unwrap_or(false);
    upgrade_is_websocket && connection_has_upgrade
}

fn query_access_token(query: &str) -> Option<String> {
    query_parameter(query, "access_token").filter(|value| !value.is_empty())
}

fn query_parameter(query: &str, expected_key: &str) -> Option<String> {
    query.split('&').find_map(|part| {
        let (key, value) = part.split_once('=')?;
        if key != expected_key || value.is_empty() {
            return None;
        }
        let decoded = percent_decode_str(value).decode_utf8().ok()?;
        let decoded = decoded.into_owned();
        if decoded.is_empty() {
            None
        } else {
            Some(decoded)
        }
    })
}

fn parse_role(role: &str) -> Result<core_types::TenantRole, ApiError> {
    role.parse()
        .map_err(|_| ApiError::Internal(format!("unknown role: {role}")))
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::sync::Mutex;

    use axum::body::Body;
    use axum::http::{Method, Request};

    use super::{
        is_mfa_challenge_request, map_provision_error, mfa_enforcement_enabled, parse_role,
        requested_tenant_id, resolve_self_service_signup, should_require_mfa_step_up,
        TENANT_HEADER,
    };
    use crate::auth::jit_provision::ProvisionError;
    use crate::error::ApiError;

    static MFA_ENV_LOCK: Mutex<()> = Mutex::new(());

    struct EnvVarGuard {
        key: &'static str,
        original: Option<OsString>,
    }

    impl EnvVarGuard {
        fn capture(key: &'static str) -> Self {
            Self {
                key,
                original: std::env::var_os(key),
            }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            match self.original.as_ref() {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }

    #[test]
    fn parse_role_maps_database_values() {
        assert_eq!(
            parse_role("org_owner").unwrap(),
            core_types::TenantRole::OrgOwner
        );
        assert_eq!(
            parse_role("org_admin").unwrap(),
            core_types::TenantRole::OrgAdmin
        );
        assert_eq!(
            parse_role("teacher").unwrap(),
            core_types::TenantRole::Teacher
        );
        assert_eq!(parse_role("ta").unwrap(), core_types::TenantRole::Ta);
        assert_eq!(
            parse_role("student").unwrap(),
            core_types::TenantRole::Student
        );
        assert_eq!(
            parse_role("parent").unwrap(),
            core_types::TenantRole::Parent
        );
    }

    #[test]
    fn explicit_workspace_header_is_parsed() {
        let tenant_id = uuid::Uuid::new_v4();
        let req = Request::builder()
            .uri("/v1/me")
            .header(TENANT_HEADER, tenant_id.to_string())
            .body(Body::empty())
            .unwrap();

        assert_eq!(requested_tenant_id(&req).unwrap(), Some(tenant_id));
    }

    #[test]
    fn invalid_workspace_header_is_rejected() {
        let req = Request::builder()
            .uri("/v1/me")
            .header(TENANT_HEADER, "not-a-uuid")
            .body(Body::empty())
            .unwrap();

        assert!(matches!(
            requested_tenant_id(&req),
            Err(ApiError::BadRequest(message)) if message == "invalid workspace selection"
        ));
    }

    #[test]
    fn websocket_workspace_query_is_supported_without_weakening_http() {
        let tenant_id = uuid::Uuid::new_v4();
        let uri = format!("/v1/sessions/abc/socket?workspace_id={tenant_id}");
        let ws_req = Request::builder()
            .uri(uri)
            .header("connection", "keep-alive, Upgrade")
            .header("upgrade", "websocket")
            .body(Body::empty())
            .unwrap();
        assert_eq!(requested_tenant_id(&ws_req).unwrap(), Some(tenant_id));

        let http_req = Request::builder()
            .uri(format!("/v1/me?workspace_id={tenant_id}"))
            .body(Body::empty())
            .unwrap();
        assert_eq!(requested_tenant_id(&http_req).unwrap(), None);
    }

    #[test]
    fn unverified_email_maps_to_stable_authentication_error() {
        assert!(matches!(
            map_provision_error(ProvisionError::EmailNotVerified),
            ApiError::Unauthorized(message) if message == "email_verification_required"
        ));
    }

    #[test]
    fn mfa_challenge_route_identifies_post_challenge() {
        let req = Request::builder()
            .method(Method::POST)
            .uri("/v1/auth/mfa/challenge")
            .body(Body::empty())
            .unwrap();

        assert!(is_mfa_challenge_request(&req));
    }

    #[test]
    fn mfa_challenge_route_rejects_other_methods_and_paths() {
        let get_challenge = Request::builder()
            .method(Method::GET)
            .uri("/v1/auth/mfa/challenge")
            .body(Body::empty())
            .unwrap();
        let disable_mfa = Request::builder()
            .method(Method::POST)
            .uri("/v1/me/mfa/disable")
            .body(Body::empty())
            .unwrap();

        assert!(!is_mfa_challenge_request(&get_challenge));
        assert!(!is_mfa_challenge_request(&disable_mfa));
    }

    #[test]
    fn mfa_step_up_required_only_for_enforced_mfa_user_non_sso_non_challenge() {
        assert!(should_require_mfa_step_up(true, true, false, false));

        for (enforcement_enabled, user_mfa_enabled, sso_session_token, challenge_request) in [
            (false, true, false, false),
            (true, false, false, false),
            (true, true, true, false),
            (true, true, false, true),
        ] {
            assert!(!should_require_mfa_step_up(
                enforcement_enabled,
                user_mfa_enabled,
                sso_session_token,
                challenge_request,
            ));
        }
    }

    #[test]
    fn mfa_enforcement_enabled_reads_truthy_values() {
        let _guard = MFA_ENV_LOCK.lock().unwrap();
        let _env = EnvVarGuard::capture("AULALITE_MFA_ENFORCE");

        for value in ["1", "true", "yes", "on", " TRUE "] {
            std::env::set_var("AULALITE_MFA_ENFORCE", value);
            assert!(
                mfa_enforcement_enabled(),
                "{value:?} should enable MFA enforcement"
            );
        }

        for value in ["0", "false", "no", "off", ""] {
            std::env::set_var("AULALITE_MFA_ENFORCE", value);
            assert!(
                !mfa_enforcement_enabled(),
                "{value:?} should not enable MFA enforcement"
            );
        }

        std::env::remove_var("AULALITE_MFA_ENFORCE");
        assert!(!mfa_enforcement_enabled());
    }

    #[test]
    fn self_service_defaults_on_locally_and_requires_production_opt_in() {
        assert!(resolve_self_service_signup("local", None));
        assert!(resolve_self_service_signup("test", None));
        assert!(!resolve_self_service_signup("production", None));
        assert!(!resolve_self_service_signup("staging", None));
        assert!(resolve_self_service_signup("production", Some("true")));
        assert!(!resolve_self_service_signup("local", Some("false")));
    }
}
