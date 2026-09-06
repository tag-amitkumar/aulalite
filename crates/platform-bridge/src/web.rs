#![cfg(target_arch = "wasm32")]

use async_trait::async_trait;
use wasm_bindgen::prelude::*;

use crate::{BridgeError, PlatformBridge};

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["aula", "fb"], js_name = signIn, catch)]
    async fn js_sign_in(email: &str, password: &str) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(js_namespace = ["aula", "fb"], js_name = signUp, catch)]
    async fn js_sign_up(email: &str, password: &str) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(js_namespace = ["aula", "fb"], js_name = signOut, catch)]
    async fn js_sign_out() -> Result<JsValue, JsValue>;

    #[wasm_bindgen(js_namespace = ["aula", "fb"], js_name = forgot, catch)]
    async fn js_forgot(email: &str) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(js_namespace = ["aula", "fb"], js_name = currentIdToken, catch)]
    async fn js_current_id_token() -> Result<JsValue, JsValue>;

    #[wasm_bindgen(js_namespace = ["aula", "fcm"], js_name = requestToken, catch)]
    async fn js_fcm_request_token() -> Result<JsValue, JsValue>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FcmRequestTokenOutcome {
    Token(String),
    MissingVapidKey,
    Unsupported,
    PermissionDenied,
    ServiceWorkerFailed,
    TokenFailed,
}

/// Request an FCM web-push registration token, preserving backwards
/// compatibility for callers that only need token-or-none.
pub async fn fcm_request_token() -> Result<Option<String>, BridgeError> {
    match fcm_request_token_outcome().await? {
        FcmRequestTokenOutcome::Token(token) => Ok(Some(token)),
        _ => Ok(None),
    }
}

/// Request an FCM web-push registration token and classify graceful setup
/// failures so the UI can show specific state.
pub async fn fcm_request_token_outcome() -> Result<FcmRequestTokenOutcome, BridgeError> {
    let value = js_fcm_request_token().await?;
    if value.is_null() || value.is_undefined() {
        return Ok(FcmRequestTokenOutcome::TokenFailed);
    }
    if let Some(token) = value.as_string() {
        persist_fcm_token(&token);
        return Ok(FcmRequestTokenOutcome::Token(token));
    }
    let status = js_sys::Reflect::get(&value, &JsValue::from_str("status"))
        .ok()
        .and_then(|v| v.as_string())
        .unwrap_or_else(|| "token_failed".into());
    let token = js_sys::Reflect::get(&value, &JsValue::from_str("token"))
        .ok()
        .and_then(|v| v.as_string());
    let outcome = match (status.as_str(), token) {
        ("token", Some(token)) => FcmRequestTokenOutcome::Token(token),
        ("missing_vapid_key", _) => FcmRequestTokenOutcome::MissingVapidKey,
        ("unsupported", _) => FcmRequestTokenOutcome::Unsupported,
        ("permission_denied", _) => FcmRequestTokenOutcome::PermissionDenied,
        ("service_worker_failed", _) => FcmRequestTokenOutcome::ServiceWorkerFailed,
        _ => FcmRequestTokenOutcome::TokenFailed,
    };
    if let FcmRequestTokenOutcome::Token(token) = &outcome {
        persist_fcm_token(token);
    }
    Ok(outcome)
}

/// localStorage key mirroring the dev "local-login" bypass token. The Firebase
/// JS SDK persists its own session in IndexedDB, but the local-login bypass has
/// no such persistence — so we stash its token here to survive a full page
/// reload (otherwise every refresh silently logs the dev user out).
const LOCAL_TOKEN_KEY: &str = "aulalite.local_token";
const FCM_TOKEN_KEY: &str = "aulalite.fcm_token";
const TRUSTED_DEVICE_PREFIX: &str = "aulalite.mfa_trusted_device.";

fn local_storage() -> Option<web_sys::Storage> {
    web_sys::window().and_then(|w| w.local_storage().ok().flatten())
}

/// Persist the local-login bypass token (dev only). No-op if storage is
/// unavailable (private mode / disabled).
pub fn persist_local_token(token: &str) {
    if let Some(storage) = local_storage() {
        let _ = storage.set_item(LOCAL_TOKEN_KEY, token);
    }
}

/// Read a previously-persisted local-login token, if any (non-empty).
pub fn local_token() -> Option<String> {
    local_storage()
        .and_then(|s| s.get_item(LOCAL_TOKEN_KEY).ok().flatten())
        .filter(|t| !t.trim().is_empty())
}

/// Remove the persisted local-login token (on sign-out, or when the bootstrap
/// finds it stale/rejected).
pub fn clear_local_token() {
    if let Some(storage) = local_storage() {
        let _ = storage.remove_item(LOCAL_TOKEN_KEY);
    }
}

fn persist_fcm_token(token: &str) {
    if let Some(storage) = local_storage() {
        let _ = storage.set_item(FCM_TOKEN_KEY, token);
    }
}

pub fn fcm_token() -> Option<String> {
    local_storage()
        .and_then(|storage| storage.get_item(FCM_TOKEN_KEY).ok().flatten())
        .filter(|token| !token.trim().is_empty())
}

pub fn clear_fcm_token() {
    if let Some(storage) = local_storage() {
        let _ = storage.remove_item(FCM_TOKEN_KEY);
    }
}

fn trusted_device_key(email: &str) -> String {
    format!(
        "{TRUSTED_DEVICE_PREFIX}{}",
        email.trim().to_ascii_lowercase()
    )
}

/// sessionStorage, not localStorage: the trusted-device token is a 30-day
/// second-factor bypass, so it must not sit in persistent storage where any
/// XSS payload could exfiltrate it at leisure. Tab-scoped storage keeps the
/// "remember this browser" convenience for the active session while shrinking
/// the theft window to a single tab's lifetime.
fn session_storage() -> Option<web_sys::Storage> {
    web_sys::window().and_then(|w| w.session_storage().ok().flatten())
}

pub fn persist_trusted_device_token(email: &str, token: &str) {
    // Purge any pre-hardening copy that may still sit in localStorage.
    if let Some(persistent) = local_storage() {
        let _ = persistent.remove_item(&trusted_device_key(email));
    }
    if let Some(storage) = session_storage() {
        let _ = storage.set_item(&trusted_device_key(email), token);
    }
}

pub fn trusted_device_token(email: &str) -> Option<String> {
    session_storage()
        .and_then(|s| s.get_item(&trusted_device_key(email)).ok().flatten())
        .filter(|t| !t.trim().is_empty())
}

pub fn clear_trusted_device_token(email: &str) {
    if let Some(storage) = session_storage() {
        let _ = storage.remove_item(&trusted_device_key(email));
    }
}

pub fn clear_all_trusted_device_tokens() {
    // Also purge legacy localStorage copies from pre-hardening deployments.
    if let Some(persistent) = local_storage() {
        let mut legacy = Vec::new();
        for i in 0..persistent.length().unwrap_or(0) {
            if let Ok(Some(key)) = persistent.key(i) {
                if key.starts_with(TRUSTED_DEVICE_PREFIX) {
                    legacy.push(key);
                }
            }
        }
        for key in legacy {
            let _ = persistent.remove_item(&key);
        }
    }
    let Some(storage) = session_storage() else {
        return;
    };
    let mut keys = Vec::new();
    for i in 0..storage.length().unwrap_or(0) {
        if let Ok(Some(key)) = storage.key(i) {
            if key.starts_with(TRUSTED_DEVICE_PREFIX) {
                keys.push(key);
            }
        }
    }
    for key in keys {
        let _ = storage.remove_item(&key);
    }
}

pub struct WebBridge;

#[async_trait(?Send)]
impl PlatformBridge for WebBridge {
    async fn current_id_token(&self) -> Result<String, BridgeError> {
        token_from_js(js_current_id_token().await?)
    }

    async fn sign_in_email_password(
        &self,
        email: &str,
        password: &str,
    ) -> Result<String, BridgeError> {
        token_from_js(js_sign_in(email, password).await?)
    }

    async fn sign_up_email_password(
        &self,
        email: &str,
        password: &str,
    ) -> Result<String, BridgeError> {
        token_from_js(js_sign_up(email, password).await?)
    }

    async fn sign_out(&self) -> Result<(), BridgeError> {
        // Drop any persisted local-login bypass token so a dev sign-out is not
        // silently undone by the bootstrap fallback on the next page load.
        clear_local_token();
        clear_all_trusted_device_tokens();
        js_sign_out().await?;
        Ok(())
    }

    async fn send_password_reset(&self, email: &str) -> Result<(), BridgeError> {
        js_forgot(email).await?;
        Ok(())
    }
}

/// Turn a Firebase `auth/*` error code into something worth showing a person.
///
/// Returns `None` for codes we have no better words for, so the caller falls
/// back to a generic message rather than inventing one.
///
/// Wrong-password, no-such-user and malformed-email deliberately collapse into
/// ONE message. Distinguishing them tells an attacker which addresses have
/// accounts, which is a free account-enumeration oracle on a public login form;
/// the person who genuinely mistyped is helped just as much by the combined
/// wording.
fn firebase_auth_message(code: &str) -> Option<&'static str> {
    Some(match code {
        "auth/invalid-credential"
        | "auth/invalid-login-credentials"
        | "auth/wrong-password"
        | "auth/user-not-found"
        | "auth/invalid-email" => "the email or password is incorrect",
        "auth/user-disabled" => "this account has been disabled",
        "auth/too-many-requests" => {
            "too many attempts from this device. Wait a few minutes and try again"
        }
        "auth/network-request-failed" => "the network request failed. Check your connection",
        "auth/email-already-in-use" => "an account already exists for this email",
        "auth/weak-password" => "that password is too short",
        "auth/requires-recent-login" => "please sign in again to continue",
        "auth/popup-closed-by-user" | "auth/cancelled-popup-request" => "the sign-in was cancelled",
        "auth/operation-not-allowed" => {
            "this sign-in method is not enabled for this workspace"
        }
        _ => return None,
    })
}

impl From<JsValue> for BridgeError {
    /// Firebase rejections arrive as a JS error object. Formatting it with
    /// `{:?}` put the RAW Rust debug of that object in front of the user --
    /// measured on the live login form, a wrong password rendered as
    /// `Sign in failed: io: JsValue(FirebaseError: Firebase: Error
    /// (auth/invalid-credential))`. That is noise to a person and detail to an
    /// attacker, and `Io` even prefixes it with "io: " because these are not
    /// I/O failures at all.
    ///
    /// Read the structured `code` instead and map it. Anything unrecognised
    /// becomes a neutral sentence -- never the debug dump.
    fn from(value: JsValue) -> Self {
        let code = js_sys::Reflect::get(&value, &JsValue::from_str("code"))
            .ok()
            .and_then(|c| c.as_string())
            .unwrap_or_default();

        if code.starts_with("auth/") {
            return BridgeError::Authentication(
                firebase_auth_message(&code)
                    .unwrap_or("sign-in could not be completed. Please try again")
                    .to_string(),
            );
        }

        // Not an auth error: keep the message, drop the debug wrapper. The raw
        // object still reaches the console for diagnosis.
        web_sys::console::warn_1(&value);
        let message = js_sys::Reflect::get(&value, &JsValue::from_str("message"))
            .ok()
            .and_then(|m| m.as_string())
            .filter(|m| !m.is_empty())
            .unwrap_or_else(|| "an unexpected error occurred".to_string());
        BridgeError::Io(message)
    }
}

fn token_from_js(value: JsValue) -> Result<String, BridgeError> {
    value
        .as_string()
        .ok_or_else(|| BridgeError::Io("no token".into()))
}
