// crates/backend/src/handlers/mfa.rs
//! Self-service TOTP MFA enrollment for the signed-in user. All routes live
//! INSIDE the `require_auth` layer and act ONLY on the caller (`ctx.user_id`).
//!
//! Routes (authed router):
//!   * GET  /v1/me/mfa            — current enrollment status (enabled?)
//!   * POST /v1/me/mfa/enroll     — begin: returns base32 secret + otpauth URI
//!   * POST /v1/me/mfa/verify     — confirm a 6-digit code; enables MFA and
//!                                  returns the one-time recovery codes
//!   * POST /v1/me/mfa/disable    — turn MFA off (delete enrollment)
//!
//! Secrets never leave the server except (a) the base32 secret + otpauth URI at
//! enroll time so the user can add it to an authenticator app, and (b) the
//! plaintext recovery codes exactly ONCE at verify time. The DB stores the raw
//! secret bytes and only SHA-256 hashes of the recovery codes.
use axum::extract::{Extension, State};
use axum::{routing, Json, Router};
use serde::{Deserialize, Serialize};

use crate::context::RequestContext;
use crate::db;
use crate::error::ApiError;
use crate::services::totp;
use crate::AppState;

/// Issuer label shown in authenticator apps. Static product name.
const ISSUER: &str = "AulaLite";
/// How many one-time recovery codes to mint at verify time.
const RECOVERY_CODE_COUNT: usize = 10;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/me/mfa", routing::get(status))
        .route("/v1/me/mfa/enroll", routing::post(enroll))
        .route("/v1/me/mfa/verify", routing::post(verify))
        .route("/v1/me/mfa/disable", routing::post(disable))
        .route("/v1/auth/mfa/challenge", routing::post(challenge))
        .route(
            "/v1/me/mfa/trusted-devices",
            routing::get(list_trusted_devices),
        )
        .route(
            "/v1/me/mfa/trusted-devices/{id}",
            routing::delete(revoke_trusted_device),
        )
}

#[doc(hidden)]
pub fn router_for_tests(pool: sqlx::PgPool) -> Router {
    Router::new()
        .route("/v1/me/mfa", routing::get(status_t))
        .route("/v1/me/mfa/enroll", routing::post(enroll_t))
        .route("/v1/me/mfa/verify", routing::post(verify_t))
        .route("/v1/me/mfa/disable", routing::post(disable_t))
        .route("/v1/auth/mfa/challenge", routing::post(challenge_t))
        .route(
            "/v1/me/mfa/trusted-devices",
            routing::get(list_trusted_devices_t),
        )
        .route(
            "/v1/me/mfa/trusted-devices/{id}",
            routing::delete(revoke_trusted_device_t),
        )
        .with_state(MfaTestState { pool })
}

#[derive(Clone)]
struct MfaTestState {
    pool: sqlx::PgPool,
}

#[derive(Serialize)]
pub struct MfaStatus {
    pub enabled: bool,
    /// True when a secret has been generated but not yet verified.
    pub pending: bool,
}

#[derive(Serialize)]
pub struct EnrollResponse {
    /// Base32 (no padding) secret the user can type into an app manually.
    pub secret_base32: String,
    /// `otpauth://totp/...` URI; render as a QR code or a copyable link.
    pub otpauth_uri: String,
}

#[derive(Deserialize)]
pub struct VerifyRequest {
    pub code: String,
}

#[derive(Serialize)]
pub struct VerifyResponse {
    pub enabled: bool,
    /// One-time recovery codes — shown EXACTLY once. Store them safely.
    pub recovery_codes: Vec<String>,
}

#[derive(Serialize)]
pub struct DisableResponse {
    pub disabled: bool,
}

async fn status_inner(
    pool: &sqlx::PgPool,
    ctx: &RequestContext,
) -> Result<Json<MfaStatus>, ApiError> {
    let enabled = db::mfa::enrollment_status(pool, ctx.user_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let (enabled, pending) = match enabled {
        Some(enabled) => (enabled, !enabled),
        None => (false, false),
    };
    Ok(Json(MfaStatus { enabled, pending }))
}

async fn status(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<MfaStatus>, ApiError> {
    status_inner(&s.pool, &ctx).await
}

async fn status_t(
    State(s): State<MfaTestState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<MfaStatus>, ApiError> {
    status_inner(&s.pool, &ctx).await
}

async fn enroll_inner(
    pool: &sqlx::PgPool,
    ctx: &RequestContext,
) -> Result<Json<EnrollResponse>, ApiError> {
    // Re-enrolling while already enabled would silently rotate the secret and
    // lock the user's existing app out. Require an explicit disable first.
    if db::mfa::enrollment_status(pool, ctx.user_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .unwrap_or(false)
    {
        return Err(ApiError::Conflict("mfa_already_enabled".into()));
    }

    let secret = totp::generate_secret();
    let started = db::mfa::start_enrollment(pool, ctx.user_id, &secret)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !started {
        // The database guard closes the race where another request verifies an
        // enrollment after the optimistic read above.
        return Err(ApiError::Conflict("mfa_already_enabled".into()));
    }

    let secret_base32 = totp::base32_encode(&secret);
    let otpauth_uri = totp::otpauth_uri(&secret_base32, ISSUER, &ctx.email);

    Ok(Json(EnrollResponse {
        secret_base32,
        otpauth_uri,
    }))
}

async fn enroll(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<EnrollResponse>, ApiError> {
    enroll_inner(&s.pool, &ctx).await
}

async fn enroll_t(
    State(s): State<MfaTestState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<EnrollResponse>, ApiError> {
    enroll_inner(&s.pool, &ctx).await
}

pub(crate) fn mint_recovery_codes() -> (Vec<String>, Vec<String>) {
    let plaintext: Vec<String> = (0..RECOVERY_CODE_COUNT).map(|_| recovery_code()).collect();
    let hashes = plaintext
        .iter()
        .map(|c| db::api_keys::hash_secret(c))
        .collect();
    (plaintext, hashes)
}

async fn verify_inner(
    pool: &sqlx::PgPool,
    ctx: &RequestContext,
    body: VerifyRequest,
) -> Result<Json<VerifyResponse>, ApiError> {
    let row = db::mfa::get(pool, ctx.user_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::BadRequest("not_enrolled".into()))?;

    if row.enabled {
        return Err(ApiError::Conflict("mfa_already_enabled".into()));
    }

    if !totp::verify(&row.secret, body.code.trim()) {
        return Err(ApiError::BadRequest("invalid_code".into()));
    }

    // Mint plaintext recovery codes; persist only their hashes.
    let (plaintext, hashes) = mint_recovery_codes();

    let activated = db::mfa::confirm_enrollment(pool, ctx.user_id, &row.secret, &hashes)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !activated {
        // Lost the race to a concurrent verify/disable.
        return Err(ApiError::Conflict("mfa_state_changed".into()));
    }

    Ok(Json(VerifyResponse {
        enabled: true,
        recovery_codes: plaintext,
    }))
}

async fn verify(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Json(body): Json<VerifyRequest>,
) -> Result<Json<VerifyResponse>, ApiError> {
    verify_inner(&s.pool, &ctx, body).await
}

async fn verify_t(
    State(s): State<MfaTestState>,
    Extension(ctx): Extension<RequestContext>,
    Json(body): Json<VerifyRequest>,
) -> Result<Json<VerifyResponse>, ApiError> {
    verify_inner(&s.pool, &ctx, body).await
}

async fn disable_inner(
    pool: &sqlx::PgPool,
    ctx: &RequestContext,
) -> Result<Json<DisableResponse>, ApiError> {
    let disabled = db::mfa::disable(pool, ctx.user_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(DisableResponse { disabled }))
}

async fn disable(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<DisableResponse>, ApiError> {
    disable_inner(&s.pool, &ctx).await
}

async fn disable_t(
    State(s): State<MfaTestState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<DisableResponse>, ApiError> {
    disable_inner(&s.pool, &ctx).await
}

// ---------------------------------------------------------------------------
// POST /v1/auth/mfa/challenge — step-up second factor
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct ChallengeRequest {
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub trusted_device_token: Option<String>,
    #[serde(default)]
    pub remember_device: bool,
    #[serde(default)]
    pub device_label: Option<String>,
}

#[derive(Serialize)]
pub struct ChallengeResponse {
    pub stepped_up: bool,
    pub stepup_token: String,
    pub used_recovery_code: bool,
    pub trusted_device_token: Option<String>,
    pub trusted_device_expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

fn exactly_one_challenge_method(body: &ChallengeRequest) -> bool {
    let has_code = body.code.as_deref().is_some_and(|v| !v.trim().is_empty());
    let has_device = body
        .trusted_device_token
        .as_deref()
        .is_some_and(|v| !v.trim().is_empty());
    has_code ^ has_device
}

#[derive(Serialize)]
pub struct TrustedDeviceDto {
    pub id: uuid::Uuid,
    pub label: String,
    pub user_agent: Option<String>,
    pub last_used_at: Option<chrono::DateTime<chrono::Utc>>,
    pub expires_at: chrono::DateTime<chrono::Utc>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Serialize)]
pub struct TrustedDeviceListResponse {
    pub devices: Vec<TrustedDeviceDto>,
}

impl From<db::mfa::TrustedDevice> for TrustedDeviceDto {
    fn from(value: db::mfa::TrustedDevice) -> Self {
        Self {
            id: value.id,
            label: value.label,
            user_agent: value.user_agent,
            last_used_at: value.last_used_at,
            expires_at: value.expires_at,
            created_at: value.created_at,
        }
    }
}

async fn list_trusted_devices_inner(
    pool: &sqlx::PgPool,
    ctx: &RequestContext,
) -> Result<Json<TrustedDeviceListResponse>, ApiError> {
    let devices = db::mfa::list_trusted_devices(pool, ctx.user_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .into_iter()
        .map(TrustedDeviceDto::from)
        .collect();
    Ok(Json(TrustedDeviceListResponse { devices }))
}

async fn revoke_trusted_device_inner(
    pool: &sqlx::PgPool,
    ctx: &RequestContext,
    device_id: uuid::Uuid,
) -> Result<Json<serde_json::Value>, ApiError> {
    let revoked = db::mfa::revoke_trusted_device(pool, ctx.user_id, device_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !revoked {
        return Err(ApiError::NotFound);
    }
    Ok(Json(serde_json::json!({ "revoked": true })))
}

async fn list_trusted_devices(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<TrustedDeviceListResponse>, ApiError> {
    list_trusted_devices_inner(&s.pool, &ctx).await
}

async fn revoke_trusted_device(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    axum::extract::Path(id): axum::extract::Path<uuid::Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    revoke_trusted_device_inner(&s.pool, &ctx, id).await
}

async fn list_trusted_devices_t(
    State(s): State<MfaTestState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<TrustedDeviceListResponse>, ApiError> {
    list_trusted_devices_inner(&s.pool, &ctx).await
}

async fn revoke_trusted_device_t(
    State(s): State<MfaTestState>,
    Extension(ctx): Extension<RequestContext>,
    axum::extract::Path(id): axum::extract::Path<uuid::Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    revoke_trusted_device_inner(&s.pool, &ctx, id).await
}

/// Step-up MFA: the caller is ALREADY primary-authenticated (this route is inside
/// `require_auth`). A valid TOTP/recovery code can optionally mint a trusted
/// device token, while an existing trusted-device token can satisfy step-up.
async fn challenge_inner(
    pool: &sqlx::PgPool,
    ctx: &RequestContext,
    body: ChallengeRequest,
) -> Result<Json<ChallengeResponse>, ApiError> {
    if !exactly_one_challenge_method(&body) {
        return Err(ApiError::BadRequest("one_challenge_method_required".into()));
    }

    let mut used_recovery_code = false;
    let mut trusted_device_token = None;
    let mut trusted_device_expires_at = None;

    if let Some(device_token) = body
        .trusted_device_token
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        match db::mfa::validate_trusted_device(pool, ctx.user_id, device_token)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?
        {
            db::mfa::TrustedDeviceCheck::Valid(_) => {}
            other => return Err(ApiError::BadRequest(other.error_code().into())),
        }
    } else {
        let presented = body
            .code
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| ApiError::BadRequest("code_required".into()))?;

        let row = db::mfa::get(pool, ctx.user_id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?
            .ok_or_else(|| ApiError::BadRequest("mfa_not_enabled".into()))?;
        if !row.enabled {
            return Err(ApiError::BadRequest("mfa_not_enabled".into()));
        }

        used_recovery_code =
            if let Some(matched_step) = totp::verify_matched(&row.secret, presented) {
                // RFC 6238 §5.2 replay guard: claim the matched time-step atomically.
                // A code already consumed (or from an older step) is rejected even
                // though it would still cryptographically verify within the window.
                let claimed = db::mfa::claim_totp_step(pool, ctx.user_id, matched_step)
                    .await
                    .map_err(|e| ApiError::Internal(e.to_string()))?;
                if !claimed {
                    return Err(ApiError::BadRequest("code_already_used".into()));
                }
                false
            } else {
                let hash = db::api_keys::hash_secret(presented);
                let consumed = db::mfa::consume_recovery_code(pool, ctx.user_id, &hash)
                    .await
                    .map_err(|e| ApiError::Internal(e.to_string()))?;
                if !consumed {
                    return Err(ApiError::BadRequest("invalid_code".into()));
                }
                true
            };

        if body.remember_device {
            let (device, token) =
                db::mfa::create_trusted_device(pool, ctx.user_id, body.device_label.clone(), None)
                    .await
                    .map_err(|e| match e {
                        sqlx::Error::RowNotFound => ApiError::BadRequest("mfa_not_enabled".into()),
                        other => ApiError::Internal(other.to_string()),
                    })?;
            trusted_device_token = Some(token);
            trusted_device_expires_at = Some(device.expires_at);
        }
    }

    // Mint the stepped-up bearer (reuses the SSO session minter so the auth
    // middleware accepts it AND skips the MFA gate, since iss == SSO_SESSION_ISS).
    let secret = crate::services::oidc::session_secret_from_env()
        .ok_or_else(|| ApiError::Internal("sso_session_secret_unset".into()))?;
    let stepup_token = crate::services::oidc::mint_session_token(
        &secret,
        &ctx.firebase_uid,
        &ctx.email,
        ctx.display_name.as_deref(),
        ctx.identity_scope,
        true,
    )
    .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(ChallengeResponse {
        stepped_up: true,
        stepup_token,
        used_recovery_code,
        trusted_device_token,
        trusted_device_expires_at,
    }))
}

async fn challenge(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Json(body): Json<ChallengeRequest>,
) -> Result<Json<ChallengeResponse>, ApiError> {
    challenge_inner(&s.pool, &ctx, body).await
}

async fn challenge_t(
    State(s): State<MfaTestState>,
    Extension(ctx): Extension<RequestContext>,
    Json(body): Json<ChallengeRequest>,
) -> Result<Json<ChallengeResponse>, ApiError> {
    challenge_inner(&s.pool, &ctx, body).await
}

/// A human-readable one-time recovery code: two 5-char crockford-ish groups,
/// e.g. `q7m2x-9pk4d`. ~50 bits of entropy from the CSPRNG.
fn recovery_code() -> String {
    use rand::Rng;
    const ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789"; // no 0/o/1/i/l
    let mut rng = rand::rng();
    let pick = |rng: &mut rand::rngs::ThreadRng| {
        (0..5)
            .map(|_| ALPHABET[rng.random_range(0..ALPHABET.len())] as char)
            .collect::<String>()
    };
    format!("{}-{}", pick(&mut rng), pick(&mut rng))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_code_shape() {
        let c = recovery_code();
        assert_eq!(c.len(), 11); // 5 + '-' + 5
        let parts: Vec<&str> = c.split('-').collect();
        assert_eq!(parts.len(), 2);
        assert!(parts.iter().all(|p| p.len() == 5));
        assert!(c.chars().all(|ch| ch == '-' || ch.is_ascii_alphanumeric()));
    }

    #[test]
    fn recovery_codes_are_distinct() {
        let a = recovery_code();
        let b = recovery_code();
        assert_ne!(a, b);
    }

    #[test]
    fn challenge_requires_exactly_one_non_empty_method() {
        let code_only = ChallengeRequest {
            code: Some("123456".to_string()),
            trusted_device_token: None,
            remember_device: false,
            device_label: None,
        };
        assert!(exactly_one_challenge_method(&code_only));

        let trusted_device_only = ChallengeRequest {
            code: None,
            trusted_device_token: Some("device-token".to_string()),
            remember_device: false,
            device_label: None,
        };
        assert!(exactly_one_challenge_method(&trusted_device_only));

        let both_methods = ChallengeRequest {
            code: Some("123456".to_string()),
            trusted_device_token: Some("device-token".to_string()),
            remember_device: false,
            device_label: None,
        };
        assert!(!exactly_one_challenge_method(&both_methods));

        let neither_method = ChallengeRequest {
            code: Some("   ".to_string()),
            trusted_device_token: None,
            remember_device: false,
            device_label: None,
        };
        assert!(!exactly_one_challenge_method(&neither_method));
    }

    #[tokio::test]
    async fn routers_construct_with_trusted_device_routes() {
        let _ = routes();
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://postgres:postgres@localhost/aulalite_test")
            .expect("valid lazy postgres URL");
        let _ = router_for_tests(pool);
    }
}
