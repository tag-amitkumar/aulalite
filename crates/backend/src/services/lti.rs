// crates/backend/src/services/lti.rs
//! LTI 1.3 (Tool side) crypto + claim helpers.
//!
//! We are the LTI **Tool**. The flow has two legs:
//!
//!  1. OIDC third-party-initiated login (`/v1/lti/login`): the platform hits us
//!     with `iss` + `login_hint` (+ optional `target_link_uri`, `client_id`,
//!     `lti_message_hint`). We mint a `state` + `nonce`, remember them, and
//!     302 the browser to the platform's `auth_login_url` with the OIDC params.
//!
//!  2. Launch (`/v1/lti/launch`): the platform POSTs back an `id_token` (a JWT
//!     signed with one of the platform JWKS keys) + the `state` we issued. We
//!     verify the signature against the platform JWKS, check `iss`/`aud`/`exp`,
//!     match the `nonce` + `deployment_id`, then read the LTI claims (roles,
//!     resource link, custom course mapping) to resolve/JIT-provision the user
//!     and land them on a course.
//!
//! Signature verification reuses the same `jsonwebtoken` + RSA-from-JWKS approach
//! as `auth::jwks` / `auth::verify`. We fetch the platform JWKS on demand (no
//! long-lived cache here — launches are infrequent relative to API traffic, and a
//! per-tenant cache would need keying we don't yet have; a TTL cache is a clean
//! follow-up). The login `state`/`nonce` round-trip is persisted in
//! `db::lti::lti_login_states` so multi-replica deployments can complete
//! launches without sticky routing.

use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use serde::Deserialize;

/// LTI 1.3 / IMS message-type + role URN constants.
pub const LTI_VERSION: &str = "1.3.0";
pub const MSG_TYPE_RESOURCE_LINK: &str = "LtiResourceLinkRequest";
const CLAIM_MESSAGE_TYPE: &str = "https://purl.imsglobal.org/spec/lti/claim/message_type";
const CLAIM_VERSION: &str = "https://purl.imsglobal.org/spec/lti/claim/version";
const CLAIM_DEPLOYMENT_ID: &str = "https://purl.imsglobal.org/spec/lti/claim/deployment_id";
const CLAIM_ROLES: &str = "https://purl.imsglobal.org/spec/lti/claim/roles";
const CLAIM_RESOURCE_LINK: &str = "https://purl.imsglobal.org/spec/lti/claim/resource_link";
const CLAIM_CONTEXT: &str = "https://purl.imsglobal.org/spec/lti/claim/context";
const CLAIM_CUSTOM: &str = "https://purl.imsglobal.org/spec/lti/claim/custom";
const CLAIM_TARGET_LINK_URI: &str = "https://purl.imsglobal.org/spec/lti/claim/target_link_uri";

/// Role URN substrings that mark a launching user as course staff. LTI sends
/// fully-qualified URNs (e.g.
/// `http://purl.imsglobal.org/vocab/lis/v2/membership#Instructor`); we match the
/// final role token case-insensitively.
const STAFF_ROLE_TOKENS: &[&str] = &[
    "instructor",
    "teachingassistant",
    "contentdeveloper",
    "administrator",
    "mentor",
];

#[derive(Debug, thiserror::Error)]
pub enum LtiError {
    #[error("http: {0}")]
    Http(String),
    #[error("invalid id_token: {0}")]
    InvalidToken(String),
    #[error("missing kid header")]
    MissingKid,
    #[error("jwks key not found for kid")]
    KidNotFound,
    #[error("claim missing or invalid: {0}")]
    Claim(String),
    #[error("nonce/state invalid or expired")]
    StateInvalid,
    #[error("unsafe or invalid platform endpoint")]
    UnsafeEndpoint,
}

/// The validated, decoded LTI launch we hand to the handler.
#[derive(Debug, Clone)]
pub struct LaunchClaims {
    /// The platform's stable user id (the JWT `sub`).
    pub subject: String,
    pub email: Option<String>,
    pub name: Option<String>,
    pub deployment_id: String,
    pub is_staff: bool,
    /// `resource_link.id` — the platform's id for the launched resource.
    pub resource_link_id: Option<String>,
    /// `context.id` — the platform's course/context id.
    pub context_id: Option<String>,
    /// Custom claim `course_slug` or `course_id` mapping the launch to an
    /// AulaLite course, when the platform was configured to send one.
    pub custom_course: Option<String>,
    /// `target_link_uri` — where the platform wants the user to land.
    pub target_link_uri: Option<String>,
}

/// Cryptographically-random URL-safe token (state / nonce). Reuses the same
/// CSPRNG + base64-url approach as `handlers::public_api::random_b64`.
pub fn random_token(n: usize) -> String {
    use base64::Engine;
    use rand::RngCore;
    let mut bytes = vec![0u8; n];
    rand::rng().fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// Fetch a platform JWKS and resolve the RSA decoding key for `kid`. No cache —
/// see module docs. Mirrors `auth::jwks::refresh`.
pub async fn jwks_key_for_kid(jwks_url: &str, kid: &str) -> Result<DecodingKey, LtiError> {
    #[derive(Deserialize)]
    struct JwkSet {
        keys: Vec<Jwk>,
    }
    #[derive(Deserialize)]
    struct Jwk {
        kid: String,
        kty: String,
        n: Option<String>,
        e: Option<String>,
    }

    let target = crate::services::webhook_delivery::validate_target_url(jwks_url)
        .map_err(|_| LtiError::UnsafeEndpoint)?;
    let http = crate::services::webhook_delivery::public_https_client(
        &target,
        std::time::Duration::from_secs(10),
    )
    .await
    .map_err(|_| LtiError::UnsafeEndpoint)?;
    let response = http
        .get(target)
        .send()
        .await
        .map_err(|e| LtiError::Http(e.to_string()))?
        .error_for_status()
        .map_err(|e| LtiError::Http(e.to_string()))?;
    let set: JwkSet = crate::services::bounded_http::json(response, 1024 * 1024)
        .await
        .map_err(|e| LtiError::Http(e.to_string()))?;

    for jwk in set.keys {
        if jwk.kty != "RSA" || jwk.kid != kid {
            continue;
        }
        let n = jwk
            .n
            .ok_or_else(|| LtiError::Claim("jwk missing n".into()))?;
        let e = jwk
            .e
            .ok_or_else(|| LtiError::Claim("jwk missing e".into()))?;
        return DecodingKey::from_rsa_components(&n, &e)
            .map_err(|err| LtiError::InvalidToken(err.to_string()));
    }
    Err(LtiError::KidNotFound)
}

/// Verify the launch `id_token` against the platform JWKS, enforce
/// `iss`/`aud`/`exp`/`nonce`/`deployment_id`/`message_type`, and project the LTI
/// claims into a `LaunchClaims`. `expected_nonce` is the value we issued during
/// the OIDC login leg.
pub async fn verify_launch(
    id_token: &str,
    issuer: &str,
    client_id: &str,
    jwks_url: &str,
    expected_deployment_id: &str,
    expected_nonce: &str,
) -> Result<LaunchClaims, LtiError> {
    let header = decode_header(id_token).map_err(|e| LtiError::InvalidToken(e.to_string()))?;
    let kid = header.kid.ok_or(LtiError::MissingKid)?;
    let key = jwks_key_for_kid(jwks_url, &kid).await?;

    let mut validation = Validation::new(Algorithm::RS256);
    validation.leeway = 30; // small clock skew allowance
    validation.set_issuer(&[issuer]);
    validation.set_audience(&[client_id]);
    // We read claims out of the raw JSON below; require only the standard set.
    validation.set_required_spec_claims(&["iss", "aud", "exp", "sub"]);

    let data = decode::<serde_json::Value>(id_token, &key, &validation)
        .map_err(|e| LtiError::InvalidToken(e.to_string()))?;
    let claims = data.claims;

    project_claims(&claims, expected_deployment_id, expected_nonce)
}

/// Pure projection of a verified JWT claim set into `LaunchClaims`, enforcing the
/// LTI-specific (non-signature) checks. Separated so it is unit-testable without
/// real keys.
fn project_claims(
    claims: &serde_json::Value,
    expected_deployment_id: &str,
    expected_nonce: &str,
) -> Result<LaunchClaims, LtiError> {
    // nonce binds this launch to our login leg (replay protection).
    let nonce = claims
        .get("nonce")
        .and_then(|v| v.as_str())
        .ok_or_else(|| LtiError::Claim("nonce".into()))?;
    if nonce != expected_nonce {
        return Err(LtiError::StateInvalid);
    }

    // message_type + version must mark this as an LTI 1.3 resource-link launch.
    let msg_type = claims
        .get(CLAIM_MESSAGE_TYPE)
        .and_then(|v| v.as_str())
        .ok_or_else(|| LtiError::Claim("message_type".into()))?;
    if msg_type != MSG_TYPE_RESOURCE_LINK {
        return Err(LtiError::Claim(format!(
            "unsupported message_type: {msg_type}"
        )));
    }
    if let Some(ver) = claims.get(CLAIM_VERSION).and_then(|v| v.as_str()) {
        if !ver.starts_with("1.3") {
            return Err(LtiError::Claim(format!("unsupported lti version: {ver}")));
        }
    }

    // deployment_id must match the registered one.
    let deployment_id = claims
        .get(CLAIM_DEPLOYMENT_ID)
        .and_then(|v| v.as_str())
        .ok_or_else(|| LtiError::Claim("deployment_id".into()))?;
    if deployment_id != expected_deployment_id {
        return Err(LtiError::Claim("deployment_id mismatch".into()));
    }

    let subject = claims
        .get("sub")
        .and_then(|v| v.as_str())
        .ok_or_else(|| LtiError::Claim("sub".into()))?
        .to_string();

    let email = claims
        .get("email")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let name = claims
        .get("name")
        .and_then(|v| v.as_str())
        .map(str::to_string);

    let is_staff = claims
        .get(CLAIM_ROLES)
        .and_then(|v| v.as_array())
        .map(|roles| roles.iter().any(role_is_staff))
        .unwrap_or(false);

    let resource_link_id = claims
        .get(CLAIM_RESOURCE_LINK)
        .and_then(|rl| rl.get("id"))
        .and_then(|v| v.as_str())
        .map(str::to_string);

    let context_id = claims
        .get(CLAIM_CONTEXT)
        .and_then(|c| c.get("id"))
        .and_then(|v| v.as_str())
        .map(str::to_string);

    // Custom claim: the platform may be configured to send `course_slug` or
    // `course_id` so a launch deep-links straight to an AulaLite course.
    let custom_course = claims.get(CLAIM_CUSTOM).and_then(|c| {
        c.get("course_slug")
            .or_else(|| c.get("course_id"))
            .and_then(|v| v.as_str())
            .map(str::to_string)
    });

    let target_link_uri = claims
        .get(CLAIM_TARGET_LINK_URI)
        .and_then(|v| v.as_str())
        .map(str::to_string);

    Ok(LaunchClaims {
        subject,
        email,
        name,
        deployment_id: deployment_id.to_string(),
        is_staff,
        resource_link_id,
        context_id,
        custom_course,
        target_link_uri,
    })
}

/// True if a role URN's final token denotes course staff.
fn role_is_staff(role: &serde_json::Value) -> bool {
    let Some(s) = role.as_str() else {
        return false;
    };
    let token = s
        .rsplit(['#', '/'])
        .next()
        .unwrap_or(s)
        .to_ascii_lowercase();
    STAFF_ROLE_TOKENS.iter().any(|t| token == *t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn base_claims(nonce: &str, deployment: &str) -> serde_json::Value {
        json!({
            "sub": "platform-user-1",
            "email": "learner@example.edu",
            "name": "Learner One",
            "nonce": nonce,
            CLAIM_MESSAGE_TYPE: MSG_TYPE_RESOURCE_LINK,
            CLAIM_VERSION: "1.3.0",
            CLAIM_DEPLOYMENT_ID: deployment,
            CLAIM_ROLES: ["http://purl.imsglobal.org/vocab/lis/v2/membership#Learner"],
            CLAIM_RESOURCE_LINK: { "id": "rl-123" },
            CLAIM_CONTEXT: { "id": "ctx-9" },
        })
    }

    #[test]
    fn projects_a_valid_learner_launch() {
        let claims = base_claims("nonce-abc", "dep-1");
        let out = project_claims(&claims, "dep-1", "nonce-abc").unwrap();
        assert_eq!(out.subject, "platform-user-1");
        assert_eq!(out.email.as_deref(), Some("learner@example.edu"));
        assert!(!out.is_staff);
        assert_eq!(out.resource_link_id.as_deref(), Some("rl-123"));
        assert_eq!(out.context_id.as_deref(), Some("ctx-9"));
    }

    #[test]
    fn detects_instructor_as_staff() {
        let mut claims = base_claims("n", "d");
        claims[CLAIM_ROLES] =
            json!(["http://purl.imsglobal.org/vocab/lis/v2/membership#Instructor"]);
        let out = project_claims(&claims, "d", "n").unwrap();
        assert!(out.is_staff);
    }

    #[test]
    fn rejects_nonce_mismatch() {
        let claims = base_claims("real", "d");
        let err = project_claims(&claims, "d", "different").unwrap_err();
        assert!(matches!(err, LtiError::StateInvalid));
    }

    #[test]
    fn rejects_deployment_mismatch() {
        let claims = base_claims("n", "dep-A");
        let err = project_claims(&claims, "dep-B", "n").unwrap_err();
        assert!(matches!(err, LtiError::Claim(_)));
    }

    #[test]
    fn rejects_wrong_message_type() {
        let mut claims = base_claims("n", "d");
        claims[CLAIM_MESSAGE_TYPE] = json!("LtiDeepLinkingRequest");
        let err = project_claims(&claims, "d", "n").unwrap_err();
        assert!(matches!(err, LtiError::Claim(_)));
    }

    #[test]
    fn reads_custom_course_mapping() {
        let mut claims = base_claims("n", "d");
        claims[CLAIM_CUSTOM] = json!({ "course_slug": "intro-bio" });
        let out = project_claims(&claims, "d", "n").unwrap();
        assert_eq!(out.custom_course.as_deref(), Some("intro-bio"));
    }
}
