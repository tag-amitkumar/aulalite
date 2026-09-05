// crates/backend/src/handlers/sso.rs
//! Enterprise SSO (OIDC authorization-code flow) + per-tenant IdP config admin.
//!
//! UNAUTHENTICATED browser-facing endpoints (mounted OUTSIDE `require_auth`, like
//! the mediamtx/stripe callbacks + dev_login), reached via `public_routes`:
//!   * GET /v1/sso/{tenant_slug}/start  — 302 to the tenant's IdP authorize URL,
//!     after persisting a server-side `state`+`nonce` for this login.
//!   * GET /v1/sso/callback            — exchange `code` for tokens, validate the
//!     id_token (sig+nonce), JIT-provision the user into the tenant, mint an app
//!     session token, and 302 back to the SPA carrying the token in the fragment.
//!
//! AUTHED org-admin config management (mounted INSIDE `require_auth`), via
//! `admin_routes`:
//!   * GET  /v1/admin/sso  — current config (client_secret NEVER returned)
//!   * PUT  /v1/admin/sso  — upsert the tenant's OIDC config
//!
//! The session token the callback mints is a self-signed HS256 JWT
//! (`services::oidc::mint_session_token`) the auth middleware accepts via the SSO
//! verification branch (see the middleware wiring this agent returns). It is
//! handed to the SPA in the URL fragment of the success redirect so it never
//! reaches the server logs or `Referer` header.
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use axum::{Extension, Json, Router};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::auth::identity::{EnterpriseProvider, IdentityScope};
use crate::auth::jit_provision::ensure_enterprise_user;
use crate::auth::verify::FirebaseClaims;
use crate::context::RequestContext;
use crate::db;
use crate::error::ApiError;
use crate::services::oidc;
use crate::AppState;

/// State for the UNAUTHENTICATED SSO router. Mirrors `DevLoginState`: it carries
/// only what the public start/callback handlers need.
#[derive(Clone)]
pub struct SsoState {
    pub pool: PgPool,
    /// Public application origin used for the post-login SPA redirect.
    pub app_origin: String,
    /// Public API origin used to build the OIDC callback URL.
    pub api_origin: String,
    /// HS256 secret for minting app session tokens. `None` disables SSO login
    /// (the start handler returns a clear error rather than minting unsigned
    /// sessions).
    pub session_secret: Option<String>,
}

/// PUBLIC SSO router (no `require_auth`). Merged in `lib.rs` alongside the other
/// public callbacks.
pub fn public_routes(state: SsoState) -> Router {
    Router::new()
        .route("/v1/sso/{tenant_slug}/start", get(start))
        .route("/v1/sso/callback", get(callback))
        .with_state(state)
}

/// AUTHED org-admin config router. Merged INSIDE the `require_auth` layer.
pub fn admin_routes() -> Router<AppState> {
    Router::new().route("/v1/admin/sso", get(get_config).put(put_config))
}

/// Our own callback URL the IdP redirects back to.
fn redirect_uri(app_origin: &str) -> String {
    format!("{}/v1/sso/callback", app_origin.trim_end_matches('/'))
}

fn endpoints_from_config(c: &db::sso::SsoConfig) -> oidc::OidcEndpoints {
    oidc::OidcEndpoints {
        issuer: c.issuer.clone(),
        client_id: c.client_id.clone(),
        client_secret: c.client_secret.clone(),
        authorize_url: c.authorize_url.clone(),
        token_url: c.token_url.clone(),
        jwks_url: c.jwks_url.clone(),
    }
}

// ---------------------------------------------------------------------------
// GET /v1/sso/{tenant_slug}/start
// ---------------------------------------------------------------------------

async fn start(
    State(state): State<SsoState>,
    Path(tenant_slug): Path<String>,
) -> Result<Response, ApiError> {
    if state.session_secret.is_none() {
        return Err(ApiError::Internal("sso_session_secret_unset".into()));
    }

    let cfg = db::sso::get_enabled_config_by_slug(&state.pool, &tenant_slug)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::NotFound)?;

    let ep = endpoints_from_config(&cfg);
    oidc::validate_endpoints(&ep).map_err(|error| {
        tracing::warn!(%error, tenant_id = %cfg.tenant_id, "rejected unsafe OIDC configuration");
        ApiError::Validation("sso_configuration_invalid".into())
    })?;
    let csrf_state = oidc::random_token();
    let nonce = oidc::random_token();
    let our_redirect = redirect_uri(&state.api_origin);

    // PKCE (RFC 7636): persist the verifier with the login state; send only the
    // S256 challenge to the IdP. The verifier is replayed at token exchange.
    let code_verifier = oidc::pkce_code_verifier();
    let code_challenge = oidc::pkce_code_challenge_s256(&code_verifier);

    db::sso::insert_login_state(
        &state.pool,
        &csrf_state,
        cfg.tenant_id,
        &nonce,
        &our_redirect,
        &code_verifier,
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;

    let url = oidc::build_authorize_url(&ep, &our_redirect, &csrf_state, &nonce, &code_challenge);
    Ok(Redirect::to(&url).into_response())
}

// ---------------------------------------------------------------------------
// GET /v1/sso/callback?code=...&state=...
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct CallbackQuery {
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

async fn callback(
    State(state): State<SsoState>,
    Query(q): Query<CallbackQuery>,
) -> Result<Response, ApiError> {
    // IdP-reported error (user denied consent, etc.).
    if let Some(err) = q.error.as_deref() {
        return Ok(sso_error_redirect(&state.app_origin, err));
    }
    let session_secret = state
        .session_secret
        .as_deref()
        .ok_or_else(|| ApiError::Internal("sso_session_secret_unset".into()))?;
    let code = q
        .code
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ApiError::BadRequest("missing code".into()))?;
    let csrf_state = q
        .state
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ApiError::BadRequest("missing state".into()))?;

    // Consume the server-side login state (validates CSRF `state` + gives nonce).
    let login = db::sso::take_login_state(&state.pool, csrf_state)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::Unauthorized("invalid or expired state".into()))?;

    let cfg = db::sso::get_enabled_config_by_tenant(&state.pool, login.tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::Unauthorized("sso disabled".into()))?;
    let ep = endpoints_from_config(&cfg);

    // Exchange code -> id_token at the IdP token endpoint, replaying the PKCE
    // verifier we persisted at `/start`.
    let id_token =
        match oidc::exchange_code(&ep, code, &login.redirect_uri, &login.code_verifier).await {
            Ok(token) => token,
            Err(error) => {
                // Keep provider diagnostics in structured server logs, not in the
                // browser-facing auth response. OidcError itself redacts bodies.
                tracing::warn!(
                    %error,
                    tenant_id = %login.tenant_id,
                    "OIDC authorization-code exchange failed"
                );
                return Ok(sso_error_redirect(
                    &state.app_origin,
                    "sso_authentication_failed",
                ));
            }
        };

    // Validate signature + issuer/audience/expiry + nonce.
    let claims = match oidc::validate_id_token(&ep, &id_token, &login.nonce).await {
        Ok(claims) => claims,
        Err(error) => {
            tracing::warn!(
                %error,
                tenant_id = %login.tenant_id,
                "OIDC identity token validation failed"
            );
            return Ok(sso_error_redirect(
                &state.app_origin,
                "sso_authentication_failed",
            ));
        }
    };

    let email = claims
        .email
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let Some(email) = email else {
        return Ok(sso_error_redirect(
            &state.app_origin,
            "sso_verified_email_required",
        ));
    };

    let firebase_uid = oidc::sso_firebase_uid(login.tenant_id, &cfg.issuer, &claims.sub);

    // JIT-provision the global user row (mirrors local-login/dev-login).
    let provision_claims = FirebaseClaims {
        sub: firebase_uid.clone(),
        email: Some(email.to_string()),
        // Preserve the provider's assertion exactly. JIT provisioning requires
        // an explicit `true`; treating an omitted claim as verified would let an
        // unverified address claim email-based tenant/course invitations.
        email_verified: claims.email_verified,
        name: claims.name.clone(),
        picture: None,
        aud: oidc::SSO_SESSION_AUD.into(),
        iss: oidc::SSO_SESSION_ISS.into(),
        exp: chrono::Utc::now().timestamp() + 60,
        iat: chrono::Utc::now().timestamp(),
        auth_time: Some(chrono::Utc::now().timestamp()),
    };
    let user = match ensure_enterprise_user(
        &state.pool,
        &provision_claims,
        login.tenant_id,
        EnterpriseProvider::Sso,
    )
    .await
    {
        Ok(user) => user,
        Err(crate::auth::jit_provision::ProvisionError::MissingEmail)
        | Err(crate::auth::jit_provision::ProvisionError::EmailNotVerified) => {
            return Ok(sso_error_redirect(
                &state.app_origin,
                "sso_verified_email_required",
            ));
        }
        Err(crate::auth::jit_provision::ProvisionError::SeatLimitReached) => {
            return Ok(sso_error_redirect(
                &state.app_origin,
                db::seats::SEAT_LIMIT_REACHED,
            ));
        }
        Err(error) => {
            return Err(crate::auth::middleware::map_provision_error(error));
        }
    };

    // Ensure the user is an ACTIVE member of the tenant (default role student).
    let activation = db::sso::ensure_enterprise_tenant_membership(
        &state.pool,
        login.tenant_id,
        user.user_id,
        email,
        "student",
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    if let Some(response) = sso_activation_error(&state.app_origin, activation) {
        return Ok(response);
    }

    // Mint the app session token and hand it to the SPA in the URL fragment.
    let token = oidc::mint_session_token(
        session_secret,
        &firebase_uid,
        email,
        claims.name.as_deref(),
        IdentityScope::Enterprise {
            tenant_id: login.tenant_id,
            provider: EnterpriseProvider::Sso,
        },
        false,
    )
    .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(sso_success_redirect(&state.app_origin, &token))
}

/// Resolve a tenant's public slug for the admin-facing SSO login URL. This is
/// called only after an authenticated tenant capability check.
async fn tenant_slug_for(pool: &PgPool, tenant_id: uuid::Uuid) -> sqlx::Result<Option<String>> {
    let mut tx = db::begin_with_context(pool, uuid::Uuid::nil(), Some(tenant_id)).await?;
    let slug = sqlx::query_scalar("SELECT slug FROM tenants WHERE id = $1")
        .bind(tenant_id)
        .fetch_optional(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(slug)
}

/// 302 to the SPA's SSO-finish route with the session token in the fragment.
/// The SPA reads `#token=...`, stores it as the bearer credential, and routes in.
fn sso_success_redirect(app_origin: &str, token: &str) -> Response {
    let url = format!(
        "{}/sso/finish#token={}",
        app_origin.trim_end_matches('/'),
        token
    );
    Redirect::to(&url).into_response()
}

/// 302 to the SPA's SSO-finish route with an error code in the query.
fn sso_error_redirect(app_origin: &str, reason: &str) -> Response {
    let url = format!(
        "{}/sso/finish?error={}",
        app_origin.trim_end_matches('/'),
        urlencode(reason)
    );
    Redirect::to(&url).into_response()
}

fn sso_activation_error(
    app_origin: &str,
    outcome: db::seats::MembershipActivationOutcome,
) -> Option<Response> {
    match outcome {
        db::seats::MembershipActivationOutcome::SeatLimitReached => Some(sso_error_redirect(
            app_origin,
            db::seats::SEAT_LIMIT_REACHED,
        )),
        db::seats::MembershipActivationOutcome::Suspended => Some(sso_error_redirect(
            app_origin,
            db::seats::MEMBERSHIP_SUSPENDED,
        )),
        db::seats::MembershipActivationOutcome::AlreadyActive
        | db::seats::MembershipActivationOutcome::Activated => None,
    }
}

// ---------------------------------------------------------------------------
// Authed org-admin config CRUD
// ---------------------------------------------------------------------------

/// Admin-facing config view — the `client_secret` is INTENTIONALLY omitted; we
/// only return whether one is set.
#[derive(Serialize)]
pub struct SsoConfigDto {
    pub issuer: String,
    pub client_id: String,
    pub authorize_url: String,
    pub token_url: String,
    pub jwks_url: String,
    pub enabled: bool,
    pub has_client_secret: bool,
    /// The org-admin-shareable login URL for end users.
    pub login_url: String,
}

#[derive(Deserialize)]
pub struct UpsertSsoConfig {
    pub issuer: String,
    pub client_id: String,
    /// Optional on update: when omitted/empty AND a secret already exists, keep
    /// the existing one (so editing other fields doesn't require re-entering it).
    #[serde(default)]
    pub client_secret: Option<String>,
    pub authorize_url: String,
    pub token_url: String,
    pub jwks_url: String,
    pub enabled: bool,
}

const MAX_CLIENT_SECRET_BYTES: usize = 8 * 1024;

fn require_admin(ctx: &RequestContext) -> Result<uuid::Uuid, ApiError> {
    if !ctx.has_capability(core_types::Capability::IntegrationsManage) {
        return Err(ApiError::Forbidden);
    }
    ctx.tenant_id
        .ok_or_else(|| ApiError::BadRequest("no tenant".into()))
}

async fn config_dto(
    pool: &PgPool,
    app_origin: &str,
    tenant_id: uuid::Uuid,
    cfg: &db::sso::SsoConfigMetadata,
) -> Result<SsoConfigDto, ApiError> {
    let slug = tenant_slug_for(pool, tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .unwrap_or_default();
    Ok(SsoConfigDto {
        issuer: cfg.issuer.clone(),
        client_id: cfg.client_id.clone(),
        authorize_url: cfg.authorize_url.clone(),
        token_url: cfg.token_url.clone(),
        jwks_url: cfg.jwks_url.clone(),
        enabled: cfg.enabled,
        has_client_secret: cfg.has_client_secret,
        login_url: format!("{}/v1/sso/{}/start", app_origin.trim_end_matches('/'), slug),
    })
}

async fn get_config(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<Option<SsoConfigDto>>, ApiError> {
    let tenant = require_admin(&ctx)?;
    let cfg = db::sso::get_config_metadata(&s.pool, tenant)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    match cfg {
        Some(c) => {
            let dto = config_dto(&s.pool, &s.api_origin, tenant, &c).await?;
            Ok(Json(Some(dto)))
        }
        None => Ok(Json(None)),
    }
}

async fn put_config(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Json(body): Json<UpsertSsoConfig>,
) -> Result<Json<SsoConfigDto>, ApiError> {
    let tenant = require_admin(&ctx)?;

    if body.client_id.trim().is_empty() {
        return Err(ApiError::Validation("client_id_required".into()));
    }

    let proposed_endpoints = oidc::OidcEndpoints {
        issuer: body.issuer.trim().to_string(),
        client_id: body.client_id.trim().to_string(),
        client_secret: String::new(),
        authorize_url: body.authorize_url.trim().to_string(),
        token_url: body.token_url.trim().to_string(),
        jwks_url: body.jwks_url.trim().to_string(),
    };
    oidc::validate_endpoints(&proposed_endpoints).map_err(|error| {
        tracing::warn!(%error, %tenant, "rejected unsafe OIDC configuration update");
        ApiError::Validation("sso_endpoints_must_be_public_https".into())
    })?;

    // An omitted/blank value preserves the encrypted DB value atomically. Do
    // not fetch and replay the decrypted value here: a concurrent secret
    // rotation could otherwise be lost. Nonblank provider secrets are opaque,
    // so preserve their bytes exactly rather than trimming meaningful spaces.
    let secret = body
        .client_secret
        .as_deref()
        .filter(|secret| !secret.trim().is_empty());
    if secret.is_some_and(|secret| secret.len() > MAX_CLIENT_SECRET_BYTES) {
        return Err(ApiError::Validation("client_secret_too_long".into()));
    }

    let cfg = db::sso::upsert_config(
        &s.pool,
        tenant,
        body.issuer.trim(),
        body.client_id.trim(),
        secret,
        body.authorize_url.trim(),
        body.token_url.trim(),
        body.jwks_url.trim(),
        body.enabled,
    )
    .await
    .map_err(|e| {
        if matches!(e, sqlx::Error::RowNotFound) && secret.is_none() {
            ApiError::Validation("client_secret_required".into())
        } else {
            ApiError::Internal(e.to_string())
        }
    })?;

    let dto = config_dto(&s.pool, &s.api_origin, tenant, &cfg).await?;
    Ok(Json(dto))
}

/// Minimal percent-encoding for the error-redirect query value.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

// Avoid an unused-import warning if StatusCode is only referenced conditionally.
#[allow(dead_code)]
const _: StatusCode = StatusCode::OK;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redirect_uri_appends_path_and_trims_slash() {
        assert_eq!(
            redirect_uri("https://app.example.com/"),
            "https://app.example.com/v1/sso/callback"
        );
        assert_eq!(
            redirect_uri("https://app.example.com"),
            "https://app.example.com/v1/sso/callback"
        );
    }

    #[test]
    fn success_redirect_puts_token_in_fragment() {
        let resp = sso_success_redirect("https://app.example.com", "abc.def.ghi");
        let loc = resp
            .headers()
            .get(axum::http::header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap();
        assert_eq!(loc, "https://app.example.com/sso/finish#token=abc.def.ghi");
    }

    #[test]
    fn error_redirect_encodes_reason_in_query() {
        let resp = sso_error_redirect("https://app.example.com", "access denied");
        let loc = resp
            .headers()
            .get(axum::http::header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap();
        assert_eq!(
            loc,
            "https://app.example.com/sso/finish?error=access%20denied"
        );
    }

    #[test]
    fn suspended_membership_maps_to_stable_sso_error() {
        let resp = sso_activation_error(
            "https://app.example.com",
            db::seats::MembershipActivationOutcome::Suspended,
        )
        .unwrap();
        assert_eq!(
            resp.headers()
                .get(axum::http::header::LOCATION)
                .unwrap()
                .to_str()
                .unwrap(),
            "https://app.example.com/sso/finish?error=membership_suspended"
        );
    }
}
