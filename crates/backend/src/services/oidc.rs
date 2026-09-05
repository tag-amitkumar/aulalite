// crates/backend/src/services/oidc.rs
//! OIDC authorization-code helpers for enterprise SSO.
//!
//! Responsibilities:
//!   * Build the IdP authorization-redirect URL (with `state` + `nonce`).
//!   * Exchange the returned `code` for tokens at the IdP token endpoint
//!     (`reqwest`, HTTP Basic client auth, `application/x-www-form-urlencoded`).
//!   * Validate the returned `id_token`: RS256 signature against the IdP JWKS
//!     (reusing `auth::jwks::JwksCache`), issuer + audience (client_id) + expiry,
//!     and the `nonce` we stored server-side.
//!   * Mint + verify a SERVER session token the app's auth middleware accepts.
//!
//! The session token is a self-signed HS256 JWT whose claims deserialize into the
//! existing `auth::verify::FirebaseClaims` shape, so the middleware can treat an
//! SSO session identically to a Firebase / local-login token after one extra
//! verification branch (see the middleware wiring this agent returns). The HS256
//! key comes from `SSO_SESSION_SECRET` (see `session_secret_from_env`).

use std::collections::HashMap;
use std::time::Duration;

use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::auth::identity::IdentityScope;
use crate::auth::jwks::JwksCache;
use crate::auth::verify::FirebaseClaims;

const JWKS_TTL: Duration = Duration::from_secs(3600);
/// SSO session token lifetime. Kept modest; the SPA re-initiates SSO on expiry.
const SESSION_TTL_SECS: i64 = 12 * 60 * 60;
/// Fixed issuer/audience stamped on our self-minted SSO session tokens, used to
/// distinguish them from Firebase/local tokens at verify time.
pub const SSO_SESSION_ISS: &str = "aulalite-sso-session";
pub const SSO_SESSION_AUD: &str = "aulalite-sso-session";

#[derive(Debug, thiserror::Error)]
pub enum OidcError {
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    #[error("response: {0}")]
    Response(#[from] crate::services::bounded_http::BoundedJsonError),
    #[error("token endpoint returned HTTP {0}")]
    TokenEndpoint(u16),
    #[error("id_token missing from token response")]
    MissingIdToken,
    #[error("id_token invalid: {0}")]
    InvalidIdToken(String),
    #[error("nonce mismatch")]
    NonceMismatch,
    #[error("id_token has no email claim")]
    MissingEmail,
    #[error("jwks: {0}")]
    Jwks(#[from] crate::auth::jwks::JwksError),
    #[error("session token: {0}")]
    Session(String),
    #[error("unsafe or invalid {0}")]
    UnsafeEndpoint(&'static str),
}

/// The subset of OIDC id_token claims we rely on for provisioning.
#[derive(Debug, Clone, Deserialize)]
pub struct IdTokenClaims {
    pub sub: String,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub email_verified: Option<bool>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub nonce: Option<String>,
}

/// The minimal IdP config the flow needs. Mirrors `db::sso::SsoConfig` fields.
#[derive(Debug, Clone)]
pub struct OidcEndpoints {
    pub issuer: String,
    pub client_id: String,
    pub client_secret: String,
    pub authorize_url: String,
    pub token_url: String,
    pub jwks_url: String,
}

/// Validate every tenant-configurable OIDC URL before it is persisted or used.
/// HTTPS and a public host are mandatory. Network calls additionally re-resolve
/// and pin DNS immediately before connecting (see `public_https_client`).
pub fn validate_endpoints(ep: &OidcEndpoints) -> Result<(), OidcError> {
    for (name, raw) in [
        ("issuer", ep.issuer.as_str()),
        ("authorize_url", ep.authorize_url.as_str()),
        ("token_url", ep.token_url.as_str()),
        ("jwks_url", ep.jwks_url.as_str()),
    ] {
        crate::services::webhook_delivery::validate_target_url(raw)
            .map_err(|_| OidcError::UnsafeEndpoint(name))?;
    }
    Ok(())
}

/// Build the authorization-code redirect URL to the IdP. `redirect_uri` is our
/// own `/v1/sso/callback`; `state` + `nonce` are server-generated opaque values
/// we persist and re-check at callback time. `code_challenge` is the S256 PKCE
/// challenge derived from the `code_verifier` we persist with the login state
/// (RFC 7636) — it defends the code exchange against interception even when the
/// IdP does not enforce `client_secret`.
pub fn build_authorize_url(
    ep: &OidcEndpoints,
    redirect_uri: &str,
    state: &str,
    nonce: &str,
    code_challenge: &str,
) -> String {
    let q = form_urlencode(&[
        ("response_type", "code"),
        ("client_id", &ep.client_id),
        ("redirect_uri", redirect_uri),
        ("scope", "openid email profile"),
        ("state", state),
        ("nonce", nonce),
        ("code_challenge", code_challenge),
        ("code_challenge_method", "S256"),
    ]);
    let sep = if ep.authorize_url.contains('?') {
        '&'
    } else {
        '?'
    };
    format!("{}{}{}", ep.authorize_url, sep, q)
}

/// Generate a PKCE (RFC 7636) `code_verifier`: a high-entropy, URL-safe random
/// string. Reuses `random_token` (32 CSPRNG bytes -> 43-char base64url), which
/// satisfies the spec's 43..=128 unreserved-char requirement.
pub fn pkce_code_verifier() -> String {
    random_token()
}

/// Derive the S256 PKCE `code_challenge` from a `code_verifier`:
/// `BASE64URL-NO-PAD(SHA256(ASCII(code_verifier)))` (RFC 7636 §4.2).
pub fn pkce_code_challenge_s256(code_verifier: &str) -> String {
    use base64::Engine;
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(code_verifier.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

#[derive(Deserialize)]
struct TokenResponse {
    #[serde(default)]
    id_token: Option<String>,
}

/// Exchange an authorization `code` for tokens at the IdP token endpoint and
/// return the raw `id_token` JWT string. Uses HTTP Basic client authentication
/// (the most widely supported `client_secret_basic` method) AND sends the PKCE
/// `code_verifier` (RFC 7636) so the IdP can confirm this token request comes
/// from the same client that started the authorization request.
pub async fn exchange_code(
    ep: &OidcEndpoints,
    code: &str,
    redirect_uri: &str,
    code_verifier: &str,
) -> Result<String, OidcError> {
    validate_endpoints(ep)?;
    let token_url = crate::services::webhook_delivery::validate_target_url(&ep.token_url)
        .map_err(|_| OidcError::UnsafeEndpoint("token_url"))?;
    let client =
        crate::services::webhook_delivery::public_https_client(&token_url, Duration::from_secs(15))
            .await
            .map_err(|_| OidcError::UnsafeEndpoint("token_url"))?;
    let form = [
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("code_verifier", code_verifier),
    ];
    let resp = client
        .post(token_url)
        .basic_auth(&ep.client_id, Some(&ep.client_secret))
        .header("accept", "application/json")
        .form(&form)
        .send()
        .await?;

    let status = resp.status();
    if !status.is_success() {
        return Err(OidcError::TokenEndpoint(status.as_u16()));
    }
    let tokens: TokenResponse = crate::services::bounded_http::json(resp, 64 * 1024).await?;
    tokens.id_token.ok_or(OidcError::MissingIdToken)
}

/// Validate an id_token's RS256 signature against the IdP JWKS, check
/// issuer/audience/expiry, and confirm the `nonce` matches what we stored.
/// Returns the decoded claims on success.
pub async fn validate_id_token(
    ep: &OidcEndpoints,
    id_token: &str,
    expected_nonce: &str,
) -> Result<IdTokenClaims, OidcError> {
    validate_endpoints(ep)?;
    let header = jsonwebtoken::decode_header(id_token)
        .map_err(|e| OidcError::InvalidIdToken(e.to_string()))?;
    let kid = header
        .kid
        .ok_or_else(|| OidcError::InvalidIdToken("missing kid header".into()))?;

    let jwks_url = crate::services::webhook_delivery::validate_target_url(&ep.jwks_url)
        .map_err(|_| OidcError::UnsafeEndpoint("jwks_url"))?;
    let http =
        crate::services::webhook_delivery::public_https_client(&jwks_url, Duration::from_secs(10))
            .await
            .map_err(|_| OidcError::UnsafeEndpoint("jwks_url"))?;
    let jwks = JwksCache::with_client(jwks_url.to_string(), JWKS_TTL, http);
    let key = jwks.key_for_kid(&kid).await?;

    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_issuer(&[ep.issuer.as_str()]);
    validation.set_audience(&[ep.client_id.as_str()]);
    let data = jsonwebtoken::decode::<IdTokenClaims>(id_token, &key, &validation)
        .map_err(|e| OidcError::InvalidIdToken(e.to_string()))?;
    let claims = data.claims;

    // Nonce binding: defends against id_token replay / injection.
    match claims.nonce.as_deref() {
        Some(n)
            if crate::auth::local_login::constant_time_eq(
                n.as_bytes(),
                expected_nonce.as_bytes(),
            ) => {}
        _ => return Err(OidcError::NonceMismatch),
    }

    if claims
        .email
        .as_deref()
        .map(str::trim)
        .unwrap_or("")
        .is_empty()
    {
        return Err(OidcError::MissingEmail);
    }
    Ok(claims)
}

// ---------------------------------------------------------------------------
// App session token (HS256) — the credential the SPA sends as a Bearer token.
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
struct SessionClaims {
    sub: String,
    email: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    aud: String,
    iss: String,
    exp: i64,
    iat: i64,
    identity_scope: IdentityScope,
    mfa_verified: bool,
}

#[derive(Debug, Clone)]
pub struct VerifiedSession {
    pub claims: FirebaseClaims,
    pub identity_scope: IdentityScope,
    pub mfa_verified: bool,
}

/// Read the HS256 signing secret for SSO session tokens from `SSO_SESSION_SECRET`.
/// Returns `None` when unset (SSO session minting is then unavailable — the
/// handler surfaces a clear error rather than minting unsigned sessions).
pub fn session_secret_from_env() -> Option<String> {
    std::env::var("SSO_SESSION_SECRET")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Mint a server-signed (HS256) session JWT for an SSO-authenticated user. The
/// `firebase_uid` is the synthetic uid we provision the user under (see
/// `sso_firebase_uid`). Verified by `verify_session_token` in the middleware.
pub fn mint_session_token(
    secret: &str,
    firebase_uid: &str,
    email: &str,
    name: Option<&str>,
    identity_scope: IdentityScope,
    mfa_verified: bool,
) -> Result<String, OidcError> {
    let now = chrono::Utc::now().timestamp();
    let claims = SessionClaims {
        sub: firebase_uid.to_string(),
        email: email.to_string(),
        name: name.map(|s| s.to_string()),
        aud: SSO_SESSION_AUD.to_string(),
        iss: SSO_SESSION_ISS.to_string(),
        exp: now + SESSION_TTL_SECS,
        iat: now,
        identity_scope,
        mfa_verified,
    };
    jsonwebtoken::encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|e| OidcError::Session(e.to_string()))
}

/// Verify a server-signed SSO session token and return `FirebaseClaims` the auth
/// middleware can consume exactly like a Firebase token. Returns `None` on any
/// failure (wrong signature/issuer/audience/expiry) so the middleware can fall
/// through to its other token paths. Called from the auth middleware wiring.
pub fn verify_session_token(secret: &str, token: &str) -> Option<VerifiedSession> {
    let mut validation = Validation::new(Algorithm::HS256);
    validation.set_issuer(&[SSO_SESSION_ISS]);
    validation.set_audience(&[SSO_SESSION_AUD]);
    let data = jsonwebtoken::decode::<SessionClaims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )
    .ok()?;
    let c = data.claims;
    Some(VerifiedSession {
        identity_scope: c.identity_scope,
        mfa_verified: c.mfa_verified,
        claims: FirebaseClaims {
            sub: c.sub,
            email: Some(c.email),
            email_verified: Some(true),
            name: c.name,
            picture: None,
            aud: c.aud,
            iss: c.iss,
            exp: c.exp,
            iat: c.iat,
            auth_time: Some(c.iat),
        },
    })
}

/// Deterministic synthetic `firebase_uid` for an SSO user, namespaced by tenant
/// + issuer + exact IdP subject. A cryptographic digest avoids the collisions
/// caused by lossy punctuation replacement while keeping the DB key compact.
pub fn sso_firebase_uid(tenant_id: Uuid, issuer: &str, sub: &str) -> String {
    format!(
        "enterprise-sso-{}",
        scoped_identity_hash(&[tenant_id.as_bytes(), issuer.as_bytes(), sub.as_bytes()])
    )
}

pub(crate) fn scoped_identity_hash(parts: &[&[u8]]) -> String {
    use sha2::{Digest, Sha256};

    let mut digest = Sha256::new();
    for part in parts {
        digest.update((part.len() as u64).to_be_bytes());
        digest.update(part);
    }
    digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Generate an opaque, URL-safe random token (used for `state` and `nonce`).
pub fn random_token() -> String {
    use base64::Engine;
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// Minimal `application/x-www-form-urlencoded` query builder.
fn form_urlencode(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", percent_encode(k), percent_encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push_str("%20"),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Unused helper kept for symmetry with potential discovery-document parsing.
/// `discovery_url` -> the well-known endpoints. Currently the per-tenant config
/// stores explicit endpoints, so discovery is OPTIONAL (see pending).
#[allow(dead_code)]
async fn _discover(_discovery_url: &str) -> Result<HashMap<String, String>, OidcError> {
    Ok(HashMap::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn endpoints() -> OidcEndpoints {
        OidcEndpoints {
            issuer: "https://idp.example.com".into(),
            client_id: "client-123".into(),
            client_secret: "secret".into(),
            authorize_url: "https://idp.example.com/authorize".into(),
            token_url: "https://idp.example.com/token".into(),
            jwks_url: "https://idp.example.com/jwks".into(),
        }
    }

    #[test]
    fn authorize_url_includes_required_params() {
        let url = build_authorize_url(
            &endpoints(),
            "https://app.example.com/v1/sso/callback",
            "state-abc",
            "nonce-xyz",
            "challenge-123",
        );
        assert!(url.starts_with("https://idp.example.com/authorize?"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("client_id=client-123"));
        assert!(url.contains("redirect_uri=https%3A%2F%2Fapp.example.com%2Fv1%2Fsso%2Fcallback"));
        assert!(url.contains("scope=openid%20email%20profile"));
        assert!(url.contains("state=state-abc"));
        assert!(url.contains("nonce=nonce-xyz"));
        assert!(url.contains("code_challenge=challenge-123"));
        assert!(url.contains("code_challenge_method=S256"));
    }

    #[test]
    fn authorize_url_appends_with_amp_when_query_present() {
        let mut ep = endpoints();
        ep.authorize_url = "https://idp.example.com/authorize?foo=bar".into();
        let url = build_authorize_url(&ep, "https://app/cb", "s", "n", "c");
        assert!(url.contains("/authorize?foo=bar&response_type=code"));
    }

    #[test]
    fn oidc_endpoints_require_public_https_origins() {
        assert!(validate_endpoints(&endpoints()).is_ok());

        let mut insecure = endpoints();
        insecure.token_url = "http://idp.example.com/token".into();
        assert!(matches!(
            validate_endpoints(&insecure),
            Err(OidcError::UnsafeEndpoint("token_url"))
        ));

        let mut private = endpoints();
        private.jwks_url = "https://127.0.0.1/jwks".into();
        assert!(matches!(
            validate_endpoints(&private),
            Err(OidcError::UnsafeEndpoint("jwks_url"))
        ));

        let mut credentialed = endpoints();
        credentialed.authorize_url = "https://user:pass@idp.example.com/authorize".into();
        assert!(matches!(
            validate_endpoints(&credentialed),
            Err(OidcError::UnsafeEndpoint("authorize_url"))
        ));
    }

    #[test]
    fn pkce_s256_challenge_matches_rfc7636_appendix_b() {
        // RFC 7636 Appendix B reference vector.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = pkce_code_challenge_s256(verifier);
        assert_eq!(challenge, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[test]
    fn pkce_verifier_is_unique_and_unreserved() {
        let a = pkce_code_verifier();
        let b = pkce_code_verifier();
        assert_ne!(a, b);
        assert!(a.len() >= 43 && a.len() <= 128);
        assert!(a
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    }

    #[test]
    fn session_token_roundtrips_to_firebase_claims() {
        let secret = "test-session-secret";
        let token = mint_session_token(
            secret,
            "sso-acme-abc",
            "u@acme.test",
            Some("U"),
            IdentityScope::Global,
            true,
        )
        .unwrap();
        let session = verify_session_token(secret, &token).expect("verifies");
        let claims = session.claims;
        assert_eq!(claims.sub, "sso-acme-abc");
        assert_eq!(claims.email.as_deref(), Some("u@acme.test"));
        assert_eq!(claims.iss, SSO_SESSION_ISS);
        assert_eq!(claims.aud, SSO_SESSION_AUD);
    }

    #[test]
    fn session_token_rejects_wrong_secret() {
        let token = mint_session_token(
            "right",
            "uid",
            "e@x.test",
            None,
            IdentityScope::Global,
            false,
        )
        .unwrap();
        assert!(verify_session_token("wrong", &token).is_none());
    }

    #[test]
    fn sso_uid_is_namespaced_and_collision_resistant() {
        let tenant = Uuid::new_v4();
        let uid = sso_firebase_uid(tenant, "https://idp.test", "auth0|abc.123");
        let distinct = sso_firebase_uid(tenant, "https://idp.test", "auth0-abc.123");
        assert!(uid.starts_with("enterprise-sso-"));
        assert!(uid.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
        assert_ne!(uid, distinct);
    }

    #[test]
    fn random_token_is_urlsafe_and_unique() {
        let a = random_token();
        let b = random_token();
        assert_ne!(a, b);
        assert!(a
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    }
}
