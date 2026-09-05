// crates/backend/src/handlers/lti.rs
//! LTI 1.3 Tool endpoints (UNAUTHENTICATED — mounted OUTSIDE require_auth, next
//! to the mediamtx/stripe callbacks). We are the Tool an LMS platform launches
//! into.
//!
//! Surface:
//!   * GET|POST /v1/lti/login   — OIDC third-party-init: validate `iss`, resolve
//!     the platform registration, mint `state`+`nonce`, 302 to the platform's
//!     authorization endpoint.
//!   * POST     /v1/lti/launch  — verify the returned `id_token` against the
//!     platform JWKS, check nonce/aud/deployment/roles, JIT-provision/resolve the
//!     user, then 302 the browser to the resolved course (or app home) with a
//!     short-lived handoff the SPA completes.
//!   * GET      /v1/admin/lti/platforms        — list (org-admin, authed router)
//!   * POST     /v1/admin/lti/platforms        — register (org-admin)
//!   * DELETE   /v1/admin/lti/platforms/{id}    — delete (org-admin)
//!
//! Platform resolution during login/launch is cross-tenant (the tenant is
//! unknown until the platform row is found) via `db::lti::find_by_issuer`, which
//! elevates to `app.system='on'` like `db::api_keys::authenticate`.
use axum::extract::{Extension, Form, Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use axum::{routing, Json, Router};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::auth::identity::{EnterpriseProvider, IdentityScope};
use crate::auth::jit_provision::ensure_enterprise_user;
use crate::auth::verify::FirebaseClaims;
use crate::context::RequestContext;
use crate::db;
use crate::error::ApiError;
use crate::services::{lti, oidc};
use crate::AppState;

// ---------------------------------------------------------------------------
// Public (unauthenticated) router: OIDC login + launch
// ---------------------------------------------------------------------------

/// PUBLIC LTI router. Mounted in `lib.rs` WITHOUT the require_auth layer.
pub fn public_routes() -> Router<AppState> {
    Router::new()
        .route("/v1/lti/login", routing::get(login_get).post(login_post))
        .route("/v1/lti/launch", routing::post(launch))
}

/// ADMIN router (org-admin gated) for platform registration. Merged INSIDE the
/// require_auth layer in `lib.rs`.
pub fn admin_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/v1/admin/lti/platforms",
            routing::get(list_platforms).post(register_platform),
        )
        .route(
            "/v1/admin/lti/platforms/{id}",
            routing::delete(delete_platform),
        )
}

// ---------------------------------------------------------------------------
// OIDC third-party-initiated login
// ---------------------------------------------------------------------------

/// OIDC login params. Platforms send these either as query (GET) or form (POST);
/// both legs converge on `login_inner`.
#[derive(Debug, Deserialize)]
pub struct LoginParams {
    pub iss: String,
    pub login_hint: Option<String>,
    pub target_link_uri: Option<String>,
    pub lti_message_hint: Option<String>,
    pub client_id: Option<String>,
}

async fn login_get(
    State(s): State<AppState>,
    Query(p): Query<LoginParams>,
) -> Result<Response, ApiError> {
    login_inner(&s, p).await
}

async fn login_post(
    State(s): State<AppState>,
    Form(p): Form<LoginParams>,
) -> Result<Response, ApiError> {
    login_inner(&s, p).await
}

async fn login_inner(s: &AppState, p: LoginParams) -> Result<Response, ApiError> {
    let platform = db::lti::find_by_issuer(&s.pool, &p.iss, p.client_id.as_deref())
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::Unauthorized("unknown lti issuer".into()))?;

    for endpoint in [
        &platform.issuer,
        &platform.auth_login_url,
        &platform.jwks_url,
    ] {
        crate::services::webhook_delivery::validate_target_url(endpoint).map_err(|reason| {
            tracing::warn!(
                platform_id = %platform.id,
                reason,
                "rejected unsafe LTI platform configuration"
            );
            ApiError::Validation("lti_platform_configuration_invalid".into())
        })?;
    }

    // Mint single-use state + nonce and remember them for the launch leg.
    let state = lti::random_token(24);
    let nonce = lti::random_token(24);
    db::lti::insert_login_state(&s.pool, &state, &nonce, p.target_link_uri.as_deref())
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    // Our launch endpoint is the OIDC redirect_uri the platform calls back.
    let redirect_uri = format!("{}/v1/lti/launch", s.api_origin.trim_end_matches('/'));

    // Build the authorization request to the platform (OIDC implicit id_token,
    // form_post response per the LTI 1.3 security profile).
    let mut url = url_with_query(
        &platform.auth_login_url,
        &[
            ("scope", "openid"),
            ("response_type", "id_token"),
            ("response_mode", "form_post"),
            ("prompt", "none"),
            ("client_id", &platform.client_id),
            ("redirect_uri", &redirect_uri),
            ("state", &state),
            ("nonce", &nonce),
        ],
    );
    if let Some(hint) = p.login_hint.as_deref() {
        url = append_query(&url, "login_hint", hint);
    }
    if let Some(hint) = p.lti_message_hint.as_deref() {
        url = append_query(&url, "lti_message_hint", hint);
    }

    Ok(Redirect::to(&url).into_response())
}

// ---------------------------------------------------------------------------
// Launch: verify id_token, provision user, land on the course
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct LaunchForm {
    pub state: String,
    pub id_token: String,
}

async fn launch(State(s): State<AppState>, Form(form): Form<LaunchForm>) -> Response {
    match launch_inner(&s, form).await {
        Ok(resp) => resp,
        Err(e) => e.into_response(),
    }
}

async fn launch_inner(s: &AppState, form: LaunchForm) -> Result<Response, ApiError> {
    // Consume the pending state (single-use) → recover the nonce we issued.
    let pending = db::lti::take_login_state(&s.pool, &form.state)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::Unauthorized("lti state invalid or expired".into()))?;
    let expected_nonce = pending.nonce;
    let target_link_uri = pending.target_link_uri;

    // Read the issuer + client_id from the UNVERIFIED token only to look up the
    // platform; the signature + claims are verified immediately after against
    // that platform's registered JWKS/issuer/client_id.
    let (iss, aud) = peek_iss_aud(&form.id_token)
        .ok_or_else(|| ApiError::Unauthorized("malformed id_token".into()))?;
    let platform = db::lti::find_by_issuer(&s.pool, &iss, aud.as_deref())
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::Unauthorized("unknown lti issuer".into()))?;

    let claims = lti::verify_launch(
        &form.id_token,
        &platform.issuer,
        &platform.client_id,
        &platform.jwks_url,
        &platform.deployment_id,
        &expected_nonce,
    )
    .await
    .map_err(|error| {
        tracing::warn!(
            %error,
            platform_id = %platform.id,
            "LTI launch verification failed"
        );
        ApiError::Unauthorized("lti_launch_rejected".into())
    })?;

    // JIT-provision/resolve the platform user as an AulaLite user. We mint a
    // stable synthetic `firebase_uid` from (issuer, subject) so repeat launches
    // map to the same row — mirroring the local-login `local-login-<email>`
    // convention. An email is required to provision (the users table keys on it).
    let email = claims
        .email
        .clone()
        .ok_or_else(|| ApiError::BadRequest("lti launch missing email claim".into()))?;
    let synthetic = synthetic_claims(
        platform.tenant_id,
        &platform.issuer,
        &claims.subject,
        &email,
        claims.name.clone(),
    );
    let user = ensure_enterprise_user(
        &s.pool,
        &synthetic,
        platform.tenant_id,
        EnterpriseProvider::Lti,
    )
    .await
    .map_err(crate::auth::middleware::map_provision_error)?;

    // Ensure the launched user is an ACTIVE member of the platform's tenant so
    // the SPA's `/v1/me` resolves a membership (mirrors the SSO callback). The
    // LTI registration is tenant-scoped, so `platform.tenant_id` is authoritative.
    let activation = db::sso::ensure_enterprise_tenant_membership(
        &s.pool,
        platform.tenant_id,
        user.user_id,
        &email,
        if claims.is_staff {
            "teacher"
        } else {
            "student"
        },
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    require_enterprise_activation(activation)?;

    // Mint an app session bearer for the launched user by reusing the SSO
    // session minter (iss == SSO_SESSION_ISS) so the auth middleware accepts it
    // exactly like an OIDC-SSO session. The secret comes from SSO_SESSION_SECRET;
    // when unset, LTI sign-in is unavailable (clear error rather than an unsigned
    // session) — same posture as the SSO start handler.
    let session_secret = crate::services::oidc::session_secret_from_env()
        .ok_or_else(|| ApiError::Internal("sso_session_secret_unset".into()))?;
    let token = crate::services::oidc::mint_session_token(
        &session_secret,
        &synthetic.sub,
        &email,
        claims.name.as_deref(),
        IdentityScope::Enterprise {
            tenant_id: platform.tenant_id,
            provider: EnterpriseProvider::Lti,
        },
        false,
    )
    .map_err(|e| ApiError::Internal(e.to_string()))?;

    // Resolve where to land. Preference order: explicit per-launch
    // target_link_uri → custom course mapping → app home. The token rides in the
    // URL fragment (never logged / sent in Referer), exactly like the SSO finish.
    let landing = resolve_landing(s, &platform, &claims, target_link_uri.as_deref());
    let redirect = format!("{landing}#token={}", urlencode(&token));

    Ok(Redirect::to(&redirect).into_response())
}

/// Build the same-origin landing URL (without the token fragment) the browser is
/// redirected to post-launch. The SPA `/lti/landing` route stores the token from
/// the fragment, then routes to this destination. We always land on
/// `/lti/landing` with a `?next=` same-origin destination so the SPA performs
/// the credential handoff before navigating.
fn resolve_landing(
    s: &AppState,
    platform: &db::lti::LtiPlatformRow,
    claims: &lti::LaunchClaims,
    target_link_uri: Option<&str>,
) -> String {
    let origin = s.app_origin.trim_end_matches('/');

    // Resolve the in-app destination path (open-redirect-safe: only same-origin
    // paths are ever honored).
    let next_path = resolve_next_path(origin, platform, claims, target_link_uri);
    format!("{origin}/lti/landing?next={}", urlencode(&next_path))
}

/// Resolve the same-origin in-app destination path for a launch.
fn resolve_next_path(
    origin: &str,
    platform: &db::lti::LtiPlatformRow,
    claims: &lti::LaunchClaims,
    target_link_uri: Option<&str>,
) -> String {
    // If the platform handed us a same-origin target_link_uri, honor its path.
    if let Some(t) = target_link_uri {
        if let Some(path) = same_origin_path(origin, t) {
            return path;
        }
    }
    // Custom course mapping → deep link the SPA to the course by slug.
    if let Some(course) = claims.custom_course.as_deref() {
        return format!("/courses/{}", urlencode(course));
    }
    // Otherwise land on the learner's course list. (A platform `default_course_id`
    // is stored on the registration; mapping it to a slug-based deep link is a
    // follow-up — see pending.)
    let _ = platform.default_course_id;
    "/courses".to_string()
}

// ---------------------------------------------------------------------------
// Admin: register / list / delete platforms (require_auth, org-admin gated)
// ---------------------------------------------------------------------------

fn require_admin(ctx: &RequestContext) -> Result<Uuid, ApiError> {
    if !ctx.has_capability(core_types::Capability::IntegrationsManage) {
        return Err(ApiError::Forbidden);
    }
    ctx.tenant_id
        .ok_or_else(|| ApiError::BadRequest("no tenant".into()))
}

#[derive(Serialize)]
pub struct LtiPlatformDto {
    pub id: Uuid,
    pub name: String,
    pub issuer: String,
    pub client_id: String,
    pub auth_login_url: String,
    pub jwks_url: String,
    pub deployment_id: String,
    pub default_course_id: Option<Uuid>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl From<db::lti::LtiPlatformRow> for LtiPlatformDto {
    fn from(r: db::lti::LtiPlatformRow) -> Self {
        Self {
            id: r.id,
            name: r.name,
            issuer: r.issuer,
            client_id: r.client_id,
            auth_login_url: r.auth_login_url,
            jwks_url: r.jwks_url,
            deployment_id: r.deployment_id,
            default_course_id: r.default_course_id,
            created_at: r.created_at,
        }
    }
}

#[derive(Deserialize)]
pub struct RegisterPlatform {
    pub name: String,
    pub issuer: String,
    pub client_id: String,
    pub auth_login_url: String,
    pub jwks_url: String,
    pub deployment_id: String,
    #[serde(default)]
    pub default_course_id: Option<Uuid>,
}

async fn list_platforms(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<Vec<LtiPlatformDto>>, ApiError> {
    let tenant = require_admin(&ctx)?;
    let rows = db::lti::list(&s.pool, tenant)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(rows.into_iter().map(LtiPlatformDto::from).collect()))
}

async fn register_platform(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Json(body): Json<RegisterPlatform>,
) -> Result<Json<LtiPlatformDto>, ApiError> {
    let tenant = require_admin(&ctx)?;

    let name = body.name.trim();
    let issuer = body.issuer.trim();
    let client_id = body.client_id.trim();
    let auth_login_url = body.auth_login_url.trim();
    let jwks_url = body.jwks_url.trim();
    let deployment_id = body.deployment_id.trim();
    if name.is_empty() {
        return Err(ApiError::Validation("name_required".into()));
    }
    for (name, endpoint) in [
        ("issuer", issuer),
        ("auth_login_url", auth_login_url),
        ("jwks_url", jwks_url),
    ] {
        if crate::services::webhook_delivery::validate_target_url(endpoint).is_err() {
            return Err(ApiError::Validation(format!("{name}_must_be_public_https")));
        }
    }
    if client_id.is_empty() {
        return Err(ApiError::Validation("client_id_required".into()));
    }
    if deployment_id.is_empty() {
        return Err(ApiError::Validation("deployment_id_required".into()));
    }

    let row = db::lti::insert(
        &s.pool,
        tenant,
        name,
        issuer,
        client_id,
        auth_login_url,
        jwks_url,
        deployment_id,
        body.default_course_id,
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(LtiPlatformDto::from(row)))
}

async fn delete_platform(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let tenant = require_admin(&ctx)?;
    let deleted = db::lti::delete(&s.pool, tenant, id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !deleted {
        return Err(ApiError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

/// Synthetic Firebase-shaped claims for a JIT-provisioned LTI user. The
/// `firebase_uid` (=`sub`) is stable across launches for the same platform user.
fn synthetic_claims(
    tenant_id: Uuid,
    issuer: &str,
    subject: &str,
    email: &str,
    name: Option<String>,
) -> FirebaseClaims {
    let now = chrono::Utc::now().timestamp();
    let uid = format!(
        "enterprise-lti-{}",
        oidc::scoped_identity_hash(&[tenant_id.as_bytes(), issuer.as_bytes(), subject.as_bytes(),])
    );
    FirebaseClaims {
        sub: uid,
        email: Some(email.to_string()),
        email_verified: Some(true),
        name,
        picture: None,
        aud: "lti-launch".into(),
        iss: "lti-launch".into(),
        exp: now + 24 * 60 * 60,
        iat: now,
        auth_time: Some(now),
    }
}

/// Pull `iss` + `aud` from an UNVERIFIED JWT payload (base64url middle segment).
/// Used only to look up which platform to verify against; the token is fully
/// verified afterwards. `aud` may be a string or a single-element array.
fn peek_iss_aud(token: &str) -> Option<(String, Option<String>)> {
    use base64::Engine;
    let payload_b64 = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload_b64)
        .ok()?;
    let json: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let iss = json.get("iss")?.as_str()?.to_string();
    let aud = match json.get("aud") {
        Some(serde_json::Value::String(s)) => Some(s.clone()),
        Some(serde_json::Value::Array(a)) => a.first().and_then(|v| v.as_str()).map(str::to_string),
        _ => None,
    };
    Some((iss, aud))
}

/// If `candidate` is an absolute URL under `origin`, return its path+query;
/// otherwise None. Avoids open-redirects: we only ever honor same-origin targets.
fn same_origin_path(origin: &str, candidate: &str) -> Option<String> {
    let expected = reqwest::Url::parse(origin).ok()?;
    let candidate = reqwest::Url::parse(candidate).ok()?;
    if expected.scheme() != candidate.scheme()
        || expected.host_str() != candidate.host_str()
        || expected.port_or_known_default() != candidate.port_or_known_default()
        || !candidate.username().is_empty()
        || candidate.password().is_some()
    {
        return None;
    }

    let path = candidate.path();
    if !safe_in_app_path(path) {
        return None;
    }
    let mut destination = path.to_string();
    if let Some(query) = candidate.query() {
        destination.push('?');
        destination.push_str(query);
    }
    Some(destination)
}

fn safe_in_app_path(path: &str) -> bool {
    path.starts_with('/')
        && !path.starts_with("//")
        && !path.contains('\\')
        && !path.chars().any(char::is_control)
}

fn url_with_query(base: &str, params: &[(&str, &str)]) -> String {
    let mut out = base.to_string();
    let mut first = !base.contains('?');
    for (k, v) in params {
        out.push(if first { '?' } else { '&' });
        first = false;
        out.push_str(k);
        out.push('=');
        out.push_str(&urlencode(v));
    }
    out
}

fn append_query(url: &str, key: &str, value: &str) -> String {
    let sep = if url.contains('?') { '&' } else { '?' };
    format!("{url}{sep}{key}={}", urlencode(value))
}

fn require_enterprise_activation(
    outcome: db::seats::MembershipActivationOutcome,
) -> Result<(), ApiError> {
    match outcome {
        db::seats::MembershipActivationOutcome::SeatLimitReached => {
            Err(ApiError::Conflict(db::seats::SEAT_LIMIT_REACHED.into()))
        }
        db::seats::MembershipActivationOutcome::Suspended => Err(ApiError::Forbidden),
        db::seats::MembershipActivationOutcome::AlreadyActive
        | db::seats::MembershipActivationOutcome::Activated => Ok(()),
    }
}

/// Percent-encode for a query value. Mirrors `handlers`/`api.rs` urlencode.
fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthetic_uid_is_stable_and_safe() {
        let tenant = Uuid::new_v4();
        let a = synthetic_claims(tenant, "https://canvas.test", "user|42", "u@e.edu", None);
        let b = synthetic_claims(tenant, "https://canvas.test", "user|42", "u@e.edu", None);
        assert_eq!(a.sub, b.sub);
        assert!(a.sub.starts_with("enterprise-lti-"));
        assert!(!a.sub.contains('|'));
    }

    #[test]
    fn suspended_membership_is_forbidden_on_lti_launch() {
        assert!(matches!(
            require_enterprise_activation(db::seats::MembershipActivationOutcome::Suspended),
            Err(ApiError::Forbidden)
        ));
    }

    #[test]
    fn different_issuers_yield_different_uids() {
        let tenant = Uuid::new_v4();
        let a = synthetic_claims(tenant, "https://canvas.test", "u", "u@e.edu", None);
        let b = synthetic_claims(tenant, "https://moodle.test", "u", "u@e.edu", None);
        assert_ne!(a.sub, b.sub);
    }

    #[test]
    fn lossy_subject_spellings_do_not_collide() {
        let tenant = Uuid::new_v4();
        let a = synthetic_claims(tenant, "https://canvas.test", "user|42", "u@e.edu", None);
        let b = synthetic_claims(tenant, "https://canvas.test", "user-42", "u@e.edu", None);
        assert_ne!(a.sub, b.sub);
    }

    fn launch_claims(custom_course: Option<&str>, target: Option<&str>) -> lti::LaunchClaims {
        lti::LaunchClaims {
            subject: "s".into(),
            email: Some("u@e.edu".into()),
            name: None,
            deployment_id: "d".into(),
            is_staff: false,
            resource_link_id: None,
            context_id: None,
            custom_course: custom_course.map(str::to_string),
            target_link_uri: target.map(str::to_string),
        }
    }

    fn platform_row(default_course: Option<Uuid>) -> db::lti::LtiPlatformRow {
        let now = chrono::Utc::now();
        db::lti::LtiPlatformRow {
            id: Uuid::nil(),
            tenant_id: Uuid::nil(),
            name: "P".into(),
            issuer: "https://p.test".into(),
            client_id: "c".into(),
            auth_login_url: "https://p.test/auth".into(),
            jwks_url: "https://p.test/jwks".into(),
            deployment_id: "d".into(),
            default_course_id: default_course,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn resolve_next_path_prefers_same_origin_target() {
        let origin = "https://app.test";
        let p = platform_row(None);
        // Same-origin target wins.
        let c = launch_claims(
            Some("intro-bio"),
            Some("https://app.test/courses/x/lessons/1"),
        );
        assert_eq!(
            resolve_next_path(origin, &p, &c, c.target_link_uri.as_deref()),
            "/courses/x/lessons/1"
        );
    }

    #[test]
    fn resolve_next_path_falls_back_to_custom_course_then_list() {
        let origin = "https://app.test";
        let p = platform_row(None);
        // Foreign target ignored; custom course used.
        let c = launch_claims(Some("intro bio"), Some("https://evil.test/x"));
        assert_eq!(
            resolve_next_path(origin, &p, &c, c.target_link_uri.as_deref()),
            "/courses/intro%20bio"
        );
        // No target, no custom course → course list.
        let c2 = launch_claims(None, None);
        assert_eq!(resolve_next_path(origin, &p, &c2, None), "/courses");
    }

    #[test]
    fn resolve_landing_routes_through_lti_landing_with_next() {
        let origin = "https://app.test";
        let p = platform_row(None);
        let c = launch_claims(None, None);
        let path = resolve_next_path(origin, &p, &c, None);
        let landing = format!("{origin}/lti/landing?next={}", urlencode(&path));
        assert_eq!(landing, "https://app.test/lti/landing?next=%2Fcourses");
    }

    #[test]
    fn same_origin_path_rejects_foreign() {
        let origin = "https://app.aulalite.test";
        assert_eq!(
            same_origin_path(origin, "https://app.aulalite.test/courses/x"),
            Some("/courses/x".to_string())
        );
        assert_eq!(same_origin_path(origin, "https://evil.test/x"), None);
        assert_eq!(same_origin_path(origin, origin), Some("/".to_string()));
        assert_eq!(
            same_origin_path(origin, "https://app.aulalite.test//evil.example/path"),
            None
        );
        assert_eq!(
            same_origin_path(origin, "https://app.aulalite.test/\\evil.example/path"),
            None
        );
    }

    #[test]
    fn url_with_query_encodes_and_separates() {
        let u = url_with_query("https://p.test/auth", &[("a", "1 2"), ("b", "x/y")]);
        assert_eq!(u, "https://p.test/auth?a=1%202&b=x%2Fy");
        let u2 = url_with_query("https://p.test/auth?z=0", &[("a", "1")]);
        assert_eq!(u2, "https://p.test/auth?z=0&a=1");
    }

    #[test]
    fn peek_iss_aud_reads_unverified_payload() {
        use base64::Engine;
        let payload = serde_json::json!({ "iss": "https://x.test", "aud": "client-1" });
        let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&payload).unwrap());
        let token = format!("h.{b64}.s");
        let (iss, aud) = peek_iss_aud(&token).unwrap();
        assert_eq!(iss, "https://x.test");
        assert_eq!(aud.as_deref(), Some("client-1"));
    }

    #[test]
    fn peek_iss_aud_handles_array_aud() {
        use base64::Engine;
        let payload = serde_json::json!({ "iss": "i", "aud": ["c1", "c2"] });
        let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&payload).unwrap());
        let (_, aud) = peek_iss_aud(&format!("h.{b64}.s")).unwrap();
        assert_eq!(aud.as_deref(), Some("c1"));
    }

    #[test]
    fn platform_endpoint_validation_rejects_unsafe_hosts() {
        use crate::services::webhook_delivery::validate_target_url;

        assert!(validate_target_url("https://x.test/jwks").is_ok());
        assert!(validate_target_url("http://x.test").is_err());
        assert!(validate_target_url("https://127.0.0.1/jwks").is_err());
        assert!(validate_target_url("https://").is_err());
    }
}
