// crates/platform-bridge/src/native.rs
#![cfg(not(target_arch = "wasm32"))]
//! Native (desktop/mobile) auth bridge.
//!
//! Mirrors the semantics of [`crate::web::WebBridge`] but talks directly to the
//! Firebase Identity Toolkit / Secure Token REST APIs using `reqwest`, instead
//! of going through the `aula.fb.*` JS bridge that only exists in the browser.
//!
//! Authentication and trusted-device tokens default to the operating system's
//! protected credential store: Windows Credential Manager, Apple Keychain /
//! protected data, Linux Secret Service, or Android secure storage. A JSON-file
//! implementation remains available only through `AULALITE_TOKEN_STORE=dev-file`
//! for explicit local development and tests.

use async_trait::async_trait;
use base64::Engine;
use serde::{de::DeserializeOwned, Deserialize};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::native_store::{selected_token_store, TokenStore};
pub use crate::native_store::{TokenFile, TokenSource, TrustedDeviceFile};
use crate::{BridgeError, PlatformBridge};

/// Env var holding the publishable Firebase web API key (same key the web
/// bundle injects as `window.__AULALITE_FIREBASE_API_KEY__`).
const API_KEY_ENV: &str = "FIREBASE_WEB_API_KEY";
const API_BASE_URL_ENV: &str = "AULALITE_API_BASE_URL";

/// Firebase auth responses are tiny. Capping both success and error bodies
/// prevents an upstream/proxy response from growing native-client memory
/// without bound, and ensures raw provider payloads never reach the UI.
const MAX_AUTH_RESPONSE_BYTES: usize = 64 * 1024;
const AUTH_REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const AUTH_CONNECT_TIMEOUT: Duration = Duration::from_secs(8);

/// Refresh the id token when it is within this many seconds of expiry.
const EXPIRY_SKEW_SECS: u64 = 60;

impl From<reqwest::Error> for BridgeError {
    fn from(_value: reqwest::Error) -> Self {
        // reqwest errors may include the request URL, and Firebase URLs contain
        // the publishable API key. Keep the user-facing error stable and clean.
        BridgeError::Authentication(
            "The authentication service could not be reached. Check your connection and try again."
                .into(),
        )
    }
}

impl TokenFile {
    /// True when the id token is expired or within [`EXPIRY_SKEW_SECS`] of it,
    /// relative to `now_unix`.
    pub fn is_expired(&self, now_unix: u64) -> bool {
        self.expires_at_unix <= now_unix.saturating_add(EXPIRY_SKEW_SECS)
    }
}

/// Compute the absolute unix expiry from `expires_in` (seconds, as returned by
/// Firebase as a string) relative to `now_unix`. Unparseable values fall back
/// to a conservative 3600s.
pub fn compute_expires_at(now_unix: u64, expires_in: &str) -> u64 {
    let secs: u64 = expires_in.trim().parse().unwrap_or(3600);
    now_unix.saturating_add(secs)
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn configured_value(runtime_name: &str, compiled: Option<&'static str>) -> Option<String> {
    let runtime = cfg!(debug_assertions)
        .then(|| std::env::var(runtime_name).ok())
        .flatten()
        .filter(|value| !value.trim().is_empty());
    runtime
        .or_else(|| {
            compiled
                .map(str::to_string)
                .filter(|value| !value.trim().is_empty())
        })
        .map(|value| value.trim().to_string())
}

/// Resolve the native API origin. Debug builds may use a runtime environment
/// override; release builds accept only the origin embedded at compile time.
/// Debug Android defaults to the emulator host alias and other debug targets
/// use loopback.
pub fn api_base_url() -> String {
    configured_value(API_BASE_URL_ENV, option_env!("AULALITE_API_BASE_URL"))
        .map(|value| value.trim_end_matches('/').to_string())
        .unwrap_or_else(|| {
            if cfg!(target_os = "android") {
                "http://10.0.2.2:8080".to_string()
            } else {
                "http://localhost:8080".to_string()
            }
        })
}

fn compiled_api_key() -> Option<&'static str> {
    option_env!("FIREBASE_WEB_API_KEY")
}

fn validate_runtime_values(
    api_origin: &str,
    firebase_api_key: Option<&str>,
    release_build: bool,
) -> Result<(), BridgeError> {
    let parsed = reqwest::Url::parse(api_origin).map_err(|_| {
        BridgeError::Configuration(format!(
            "{API_BASE_URL_ENV} must be an absolute HTTP(S) origin"
        ))
    })?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.path() != "/"
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(BridgeError::Configuration(format!(
            "{API_BASE_URL_ENV} must be a bare HTTP(S) origin without a path, credentials, query, or fragment"
        )));
    }
    if release_build && parsed.scheme() != "https" {
        return Err(BridgeError::Configuration(format!(
            "{API_BASE_URL_ENV} must use HTTPS in a release build"
        )));
    }
    if release_build && firebase_api_key.is_none_or(|key| key.trim().is_empty()) {
        return Err(BridgeError::Configuration(format!(
            "{API_KEY_ENV} must be supplied when creating a release build"
        )));
    }
    Ok(())
}

/// Validate public native runtime configuration before mounting the shared UI.
/// Release bundles fail closed on cleartext API origins and missing Firebase
/// configuration; debug builds keep local/emulator development frictionless.
pub fn validate_runtime_config() -> Result<(), BridgeError> {
    let api_origin = api_base_url();
    let firebase_api_key = configured_value(API_KEY_ENV, compiled_api_key());
    validate_runtime_values(
        &api_origin,
        firebase_api_key.as_deref(),
        !cfg!(debug_assertions),
    )
}

fn auth_client() -> Result<reqwest::Client, BridgeError> {
    reqwest::Client::builder()
        .connect_timeout(AUTH_CONNECT_TIMEOUT)
        .timeout(AUTH_REQUEST_TIMEOUT)
        .user_agent(concat!("AulaLite/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(BridgeError::from)
}

async fn read_bounded_body(mut response: reqwest::Response) -> Result<Vec<u8>, BridgeError> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_AUTH_RESPONSE_BYTES as u64)
    {
        return Err(BridgeError::Authentication(
            "The authentication service returned an invalid response.".into(),
        ));
    }

    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(BridgeError::from)? {
        if body.len().saturating_add(chunk.len()) > MAX_AUTH_RESPONSE_BYTES {
            return Err(BridgeError::Authentication(
                "The authentication service returned an invalid response.".into(),
            ));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

async fn read_auth_json<T: DeserializeOwned>(
    response: reqwest::Response,
) -> Result<T, BridgeError> {
    let body = read_bounded_body(response).await?;
    serde_json::from_slice(&body).map_err(|_| {
        BridgeError::Authentication(
            "The authentication service returned an invalid response.".into(),
        )
    })
}

#[derive(Debug, Deserialize)]
struct FirebaseErrorEnvelope {
    error: FirebaseErrorBody,
}

#[derive(Debug, Deserialize)]
struct FirebaseErrorBody {
    message: String,
}

fn friendly_firebase_error(code: &str) -> &'static str {
    let code = code.split(':').next().unwrap_or(code).trim();
    match code {
        "EMAIL_EXISTS" => "An account already exists for this email address.",
        "INVALID_EMAIL" => "Enter a valid email address.",
        "INVALID_PASSWORD" | "EMAIL_NOT_FOUND" | "INVALID_LOGIN_CREDENTIALS" => {
            "The email or password is incorrect."
        }
        "USER_DISABLED" => {
            "This account has been disabled. Contact your organization administrator."
        }
        "WEAK_PASSWORD" => "Choose a stronger password with at least six characters.",
        "TOO_MANY_ATTEMPTS_TRY_LATER" => "Too many attempts. Wait a moment before trying again.",
        "TOKEN_EXPIRED" | "USER_NOT_FOUND" | "INVALID_REFRESH_TOKEN" => {
            "Your session has expired. Sign in again."
        }
        _ => "Authentication could not be completed. Try again.",
    }
}

async fn rejected_auth_response(response: reqwest::Response) -> BridgeError {
    let body = match read_bounded_body(response).await {
        Ok(body) => body,
        Err(_) => {
            return BridgeError::Authentication(
                "Authentication could not be completed. Try again.".into(),
            )
        }
    };
    let message = serde_json::from_slice::<FirebaseErrorEnvelope>(&body)
        .ok()
        .map(|envelope| friendly_firebase_error(&envelope.error.message))
        .unwrap_or("Authentication could not be completed. Try again.");
    BridgeError::Authentication(message.into())
}

/// Native Firebase auth bridge. Stateless — all auth state lives in the token
/// store, so the bridge can be constructed freely (`NativeBridge`).
pub struct NativeBridge;

fn clear_session_store(store: &dyn TokenStore) -> Result<(), BridgeError> {
    // Always attempt both erasures. A damaged primary entry must not leave an
    // MFA trusted-device token behind (or vice versa).
    let primary = store.clear_primary();
    let trusted_devices = store.clear_all_trusted_device_tokens();
    primary.and(trusted_devices)
}

impl NativeBridge {
    fn api_key() -> Result<String, BridgeError> {
        configured_value(API_KEY_ENV, compiled_api_key()).ok_or_else(|| {
            BridgeError::Configuration(format!(
                "{API_KEY_ENV} is not set; native Firebase authentication is unavailable"
            ))
        })
    }

    fn load_tokens() -> Result<Option<TokenFile>, BridgeError> {
        selected_token_store()?.load_primary()
    }

    fn save_tokens(tf: &TokenFile) -> Result<(), BridgeError> {
        selected_token_store()?.save_primary(tf)
    }

    fn clear_tokens() -> Result<(), BridgeError> {
        selected_token_store()?.clear_primary()
    }

    pub fn persist_trusted_device_token(email: &str, token: &str) -> Result<(), BridgeError> {
        selected_token_store()?.persist_trusted_device_token(email, token)
    }

    pub fn trusted_device_token(email: &str) -> Result<Option<String>, BridgeError> {
        selected_token_store()?.trusted_device_token(email)
    }

    pub fn clear_trusted_device_token(email: &str) -> Result<(), BridgeError> {
        selected_token_store()?.clear_trusted_device_token(email)
    }

    fn clear_session() -> Result<(), BridgeError> {
        let store = selected_token_store()?;
        clear_session_store(store.as_ref())
    }

    /// Persist a token produced by the backend's explicitly local-only login
    /// bypass. It has no provider refresh token; the backend owns its lifetime.
    pub fn persist_local_development_token(token: &str) -> Result<(), BridgeError> {
        if token.trim().is_empty() {
            return Err(BridgeError::Authentication(
                "Authentication did not return a session token.".into(),
            ));
        }
        Self::save_tokens(&TokenFile {
            id_token: token.to_string(),
            refresh_token: String::new(),
            expires_at_unix: u64::MAX,
            source: TokenSource::LocalDevelopment,
        })
    }

    /// Persist the short-lived session JWT returned by the backend's OIDC/LTI
    /// callback. Its unverified `exp` claim is used only for local refresh
    /// scheduling; the backend still verifies the token on every request.
    pub fn persist_backend_callback_token(token: &str) -> Result<(), BridgeError> {
        let token = token.trim();
        if token.is_empty() || token.len() > 12 * 1024 || token.chars().any(char::is_control) {
            return Err(BridgeError::Authentication(
                "Authentication did not return a valid session token.".into(),
            ));
        }
        let expires_at_unix = token
            .split('.')
            .nth(1)
            .and_then(|payload| {
                base64::engine::general_purpose::URL_SAFE_NO_PAD
                    .decode(payload)
                    .ok()
            })
            .and_then(|payload| serde_json::from_slice::<serde_json::Value>(&payload).ok())
            .and_then(|claims| claims.get("exp")?.as_u64())
            .filter(|expiry| *expiry > now_unix())
            .unwrap_or_else(|| now_unix().saturating_add(60 * 60));
        Self::save_tokens(&TokenFile {
            id_token: token.to_string(),
            refresh_token: String::new(),
            expires_at_unix,
            source: TokenSource::BackendCallback,
        })
    }

    /// Shared body for accounts:signInWithPassword / accounts:signUp. Persists
    /// the returned tokens and returns the id token.
    async fn password_flow(
        endpoint: &str,
        email: &str,
        password: &str,
    ) -> Result<String, BridgeError> {
        let api_key = Self::api_key()?;
        let url =
            format!("https://identitytoolkit.googleapis.com/v1/accounts:{endpoint}?key={api_key}");
        let body = serde_json::json!({
            "email": email,
            "password": password,
            "returnSecureToken": true,
        });
        let resp = auth_client()?.post(&url).json(&body).send().await?;
        if !resp.status().is_success() {
            return Err(rejected_auth_response(resp).await);
        }

        #[derive(Deserialize)]
        struct SignInResponse {
            #[serde(rename = "idToken")]
            id_token: String,
            #[serde(rename = "refreshToken")]
            refresh_token: String,
            #[serde(rename = "expiresIn")]
            expires_in: String,
        }
        let parsed: SignInResponse = read_auth_json(resp).await?;

        let tf = TokenFile {
            id_token: parsed.id_token.clone(),
            refresh_token: parsed.refresh_token,
            expires_at_unix: compute_expires_at(now_unix(), &parsed.expires_in),
            source: TokenSource::Firebase,
        };
        Self::save_tokens(&tf)?;
        Ok(parsed.id_token)
    }

    /// Exchange the persisted refresh token for a fresh id token via the Secure
    /// Token API, persist it, and return the new id token.
    async fn refresh(existing: &TokenFile) -> Result<String, BridgeError> {
        let api_key = Self::api_key()?;
        let url = format!("https://securetoken.googleapis.com/v1/token?key={api_key}");
        let resp = auth_client()?
            .post(&url)
            .form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", existing.refresh_token.as_str()),
            ])
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(rejected_auth_response(resp).await);
        }

        #[derive(Deserialize)]
        struct RefreshResponse {
            id_token: String,
            refresh_token: String,
            expires_in: String,
        }
        let parsed: RefreshResponse = read_auth_json(resp).await?;

        let tf = TokenFile {
            id_token: parsed.id_token.clone(),
            refresh_token: parsed.refresh_token,
            expires_at_unix: compute_expires_at(now_unix(), &parsed.expires_in),
            source: TokenSource::Firebase,
        };
        Self::save_tokens(&tf)?;
        Ok(parsed.id_token)
    }
}

#[async_trait]
impl PlatformBridge for NativeBridge {
    async fn current_id_token(&self) -> Result<String, BridgeError> {
        let tf = match Self::load_tokens()? {
            Some(tf) => tf,
            None => return Err(BridgeError::Authentication("Not signed in.".into())),
        };
        if tf.is_expired(now_unix()) {
            if tf.source != TokenSource::Firebase {
                Self::clear_tokens()?;
                Err(BridgeError::Authentication(
                    "Your session has expired. Sign in again.".into(),
                ))
            } else {
                Self::refresh(&tf).await
            }
        } else {
            Ok(tf.id_token)
        }
    }

    async fn sign_in_email_password(
        &self,
        email: &str,
        password: &str,
    ) -> Result<String, BridgeError> {
        Self::password_flow("signInWithPassword", email, password).await
    }

    async fn sign_up_email_password(
        &self,
        email: &str,
        password: &str,
    ) -> Result<String, BridgeError> {
        Self::password_flow("signUp", email, password).await
    }

    async fn sign_out(&self) -> Result<(), BridgeError> {
        let _ = crate::offline::clear_current_principal();
        Self::clear_session()
    }

    async fn send_password_reset(&self, email: &str) -> Result<(), BridgeError> {
        let api_key = Self::api_key()?;
        let url =
            format!("https://identitytoolkit.googleapis.com/v1/accounts:sendOobCode?key={api_key}");
        let body = serde_json::json!({
            "requestType": "PASSWORD_RESET",
            "email": email,
        });
        let resp = auth_client()?.post(&url).json(&body).send().await?;
        if !resp.status().is_success() {
            // Password reset is deliberately non-enumerating: Firebase may
            // return EMAIL_NOT_FOUND for a valid-looking address, but callers
            // always show the same "check your inbox" result.
            let body = read_bounded_body(resp).await?;
            let code = serde_json::from_slice::<FirebaseErrorEnvelope>(&body)
                .ok()
                .map(|envelope| envelope.error.message)
                .unwrap_or_default();
            if !matches!(
                code.split(':').next(),
                Some("EMAIL_NOT_FOUND" | "USER_NOT_FOUND")
            ) {
                return Err(BridgeError::Authentication(
                    friendly_firebase_error(&code).into(),
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_store::{DevFileTokenStore, TokenStore};

    #[test]
    fn compute_expires_at_adds_seconds() {
        assert_eq!(compute_expires_at(1000, "3600"), 4600);
        // Whitespace tolerated.
        assert_eq!(compute_expires_at(1000, " 60 "), 1060);
    }

    #[test]
    fn compute_expires_at_defaults_on_garbage() {
        assert_eq!(compute_expires_at(0, "not-a-number"), 3600);
        assert_eq!(compute_expires_at(0, ""), 3600);
    }

    #[test]
    fn is_expired_respects_skew() {
        let tf = TokenFile {
            id_token: "id".into(),
            refresh_token: "rt".into(),
            expires_at_unix: 1000,
            source: TokenSource::Firebase,
        };
        // Well before expiry → fresh.
        assert!(!tf.is_expired(500));
        // Within the 60s skew window → treated as expired.
        assert!(tf.is_expired(950));
        // Exactly at expiry → expired.
        assert!(tf.is_expired(1000));
        // Past expiry → expired.
        assert!(tf.is_expired(2000));
    }

    #[test]
    fn token_file_roundtrips_json() {
        let tf = TokenFile {
            id_token: "the-id-token".into(),
            refresh_token: "the-refresh-token".into(),
            expires_at_unix: 1_700_000_000,
            source: TokenSource::Firebase,
        };
        let json = serde_json::to_string(&tf).unwrap();
        let back: TokenFile = serde_json::from_str(&json).unwrap();
        assert_eq!(tf, back);
        // Field names are stable (snake_case) so the file format is portable.
        assert!(json.contains("\"id_token\""));
        assert!(json.contains("\"refresh_token\""));
        assert!(json.contains("\"expires_at_unix\""));
        assert!(json.contains("\"source\":\"firebase\""));
    }

    #[test]
    fn old_token_records_default_to_firebase_source() {
        let json = r#"{
            "id_token":"legacy-id",
            "refresh_token":"legacy-refresh",
            "expires_at_unix":1700000000
        }"#;
        let token: TokenFile = serde_json::from_str(json).unwrap();
        assert_eq!(token.source, TokenSource::Firebase);
    }

    #[test]
    fn callback_source_roundtrips_and_does_not_refresh_with_firebase() {
        let token = TokenFile {
            id_token: "header.payload.signature".into(),
            refresh_token: String::new(),
            expires_at_unix: 1_800_000_000,
            source: TokenSource::BackendCallback,
        };
        let json = serde_json::to_string(&token).unwrap();
        let decoded: TokenFile = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.source, TokenSource::BackendCallback);
        assert!(json.contains("backend_callback"));
    }

    #[test]
    fn signout_erases_primary_and_all_trusted_device_material() {
        let dir = tempfile::tempdir().unwrap();
        let store = DevFileTokenStore::new(
            dir.path().join("auth.json"),
            dir.path().join("trusted-devices.json"),
        );
        store
            .save_primary(&TokenFile {
                id_token: "id-token".into(),
                refresh_token: "refresh-token".into(),
                expires_at_unix: u64::MAX,
                source: TokenSource::Firebase,
            })
            .unwrap();
        store
            .persist_trusted_device_token("owner@example.test", "trusted-token")
            .unwrap();

        clear_session_store(&store).unwrap();

        assert_eq!(store.load_primary().unwrap(), None);
        assert_eq!(
            store.trusted_device_token("owner@example.test").unwrap(),
            None
        );
    }

    #[test]
    fn release_runtime_config_requires_https_and_firebase() {
        let cleartext =
            validate_runtime_values("http://api.example.test", Some("publishable-key"), true)
                .unwrap_err()
                .to_string();
        assert!(cleartext.contains("must use HTTPS"), "{cleartext}");

        let missing_firebase = validate_runtime_values("https://api.example.test", None, true)
            .unwrap_err()
            .to_string();
        assert!(missing_firebase.contains(API_KEY_ENV), "{missing_firebase}");

        validate_runtime_values("https://api.example.test", Some("publishable-key"), true).unwrap();
        validate_runtime_values("http://10.0.2.2:8080", None, false).unwrap();
    }

    #[test]
    fn runtime_config_rejects_credentials_query_and_non_http_schemes() {
        for value in [
            "ftp://api.example.test",
            "https://user:secret@api.example.test",
            "https://api.example.test?token=secret",
        ] {
            assert!(validate_runtime_values(value, Some("key"), false).is_err());
        }
    }

    #[test]
    fn firebase_errors_are_stable_and_do_not_echo_provider_payloads() {
        assert_eq!(
            friendly_firebase_error("INVALID_LOGIN_CREDENTIALS"),
            "The email or password is incorrect."
        );
        assert_eq!(
            friendly_firebase_error("SOMETHING_NEW: provider details"),
            "Authentication could not be completed. Try again."
        );
    }

    #[test]
    fn trusted_device_file_roundtrips_json() {
        let mut file = TrustedDeviceFile::default();
        file.tokens_by_email
            .insert("ada@example.test".into(), "device-token".into());

        let json = serde_json::to_string(&file).unwrap();
        let back: TrustedDeviceFile = serde_json::from_str(&json).unwrap();
        assert_eq!(file, back);
        assert!(json.contains("\"tokens_by_email\""));
        assert!(json.contains("\"ada@example.test\""));
    }
}
