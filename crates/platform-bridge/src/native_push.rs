//! Native push-token lifecycle. APNs/FCM SDK glue calls
//! [`set_registration_token_from_host`] whenever the OS rotates a token.

#![cfg(not(target_arch = "wasm32"))]

use crate::native_store::{selected_token_store, NativePushRegistration, TokenStore};
use crate::BridgeError;
use std::time::{SystemTime, UNIX_EPOCH};

const TOKEN_ENV: &str = "AULALITE_NATIVE_PUSH_TOKEN";
const MAX_PUSH_TOKEN_BYTES: usize = 8 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativePushTokenOutcome {
    Token(NativePushRegistration),
    AwaitingHostRegistration,
    Unsupported,
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

pub fn native_push_platform() -> Option<&'static str> {
    if cfg!(target_os = "android") {
        Some("android")
    } else if cfg!(target_os = "ios") {
        Some("ios")
    } else {
        None
    }
}

pub fn default_device_label() -> String {
    if cfg!(target_os = "android") {
        "AulaLite for Android".into()
    } else if cfg!(target_os = "ios") {
        "AulaLite for iOS".into()
    } else if cfg!(windows) {
        "AulaLite for Windows".into()
    } else if cfg!(target_os = "macos") {
        "AulaLite for macOS".into()
    } else {
        "AulaLite desktop app".into()
    }
}

fn validate_token(token: &str) -> Result<&str, BridgeError> {
    let token = token.trim();
    if token.is_empty() || token.len() > MAX_PUSH_TOKEN_BYTES || token.chars().any(char::is_control)
    {
        return Err(BridgeError::Configuration(
            "The native push service returned an invalid token.".into(),
        ));
    }
    Ok(token)
}

fn store_registration(
    store: &dyn TokenStore,
    token: &str,
    platform: &str,
    label: &str,
) -> Result<NativePushRegistration, BridgeError> {
    let token = validate_token(token)?;
    if !matches!(platform, "android" | "ios") {
        return Err(BridgeError::Configuration(
            "Native push is only supported on Android and iOS.".into(),
        ));
    }
    let existing = store.load_push_registration()?;
    let previous_token = existing
        .as_ref()
        .filter(|existing| existing.token != token)
        .map(|existing| existing.token.clone())
        .or_else(|| existing.and_then(|existing| existing.previous_token));
    let registration = NativePushRegistration {
        token: token.to_string(),
        platform: platform.to_string(),
        label: label.trim().chars().take(120).collect(),
        previous_token,
        updated_at_unix: now_unix(),
    };
    store.save_push_registration(&registration)?;
    Ok(registration)
}

/// Public host hook for Android FCM / iOS APNs-through-FCM SDK callbacks.
pub fn set_registration_token_from_host(
    token: &str,
    platform: &str,
    label: Option<&str>,
) -> Result<NativePushRegistration, BridgeError> {
    let store = selected_token_store()?;
    store_registration(
        store.as_ref(),
        token,
        platform,
        label.unwrap_or(&default_device_label()),
    )
}

pub fn registration_token_outcome() -> Result<NativePushTokenOutcome, BridgeError> {
    let Some(platform) = native_push_platform() else {
        return Ok(NativePushTokenOutcome::Unsupported);
    };
    let store = selected_token_store()?;
    if let Some(registration) = store.load_push_registration()? {
        return Ok(NativePushTokenOutcome::Token(registration));
    }
    // Useful for local device harnesses. Store builds should inject tokens via
    // the host callback, never by compiling a token into the application.
    if let Ok(token) = std::env::var(TOKEN_ENV) {
        return store_registration(store.as_ref(), &token, platform, &default_device_label())
            .map(NativePushTokenOutcome::Token);
    }
    Ok(NativePushTokenOutcome::AwaitingHostRegistration)
}

/// Ask the mobile host to show its runtime notification permission prompt and
/// refresh the provider token. Android implements this through MainActivity;
/// the iOS adapter remains an explicit host responsibility.
pub fn request_permission_from_host() -> Result<(), BridgeError> {
    #[cfg(target_os = "android")]
    {
        return call_android_activity("requestPushPermissionFromRust");
    }

    #[cfg(not(target_os = "android"))]
    Err(BridgeError::NotImplemented)
}

#[cfg(target_os = "android")]
fn call_android_activity(method: &str) -> Result<(), BridgeError> {
    use jni::objects::JObject;

    let context = std::panic::catch_unwind(ndk_context::android_context)
        .map_err(|_| BridgeError::Io("Android application context is not initialized".into()))?;
    if context.vm().is_null() || context.context().is_null() {
        return Err(BridgeError::Io(
            "Android application context is unavailable".into(),
        ));
    }
    // SAFETY: Dioxus/Tao owns both pointers for the Activity lifetime.
    let vm = unsafe { jni::JavaVM::from_raw(context.vm().cast()) }
        .map_err(|error| BridgeError::Io(format!("could not access Android JavaVM: {error}")))?;
    let mut env = vm.attach_current_thread().map_err(|error| {
        BridgeError::Io(format!("could not attach to the Android runtime: {error}"))
    })?;
    // SAFETY: ndk-context exposes the global Activity reference and retains
    // ownership; this wrapper is used only for the duration of the call.
    let activity = unsafe { JObject::from_raw(context.context().cast::<jni::sys::_jobject>()) };
    env.call_method(&activity, method, "()V", &[])
        .map_err(|error| BridgeError::Io(format!("Android host call {method} failed: {error}")))?;
    Ok(())
}

/// Called after the new token was accepted and any previous token was revoked
/// from the backend. This collapses rotation state without discarding the OS
/// token that will be reused after the next sign-in.
pub fn mark_registration_synced() -> Result<(), BridgeError> {
    let store = selected_token_store()?;
    if let Some(mut registration) = store.load_push_registration()? {
        registration.previous_token = None;
        store.save_push_registration(&registration)?;
    }
    Ok(())
}

pub fn clear_local_registration() -> Result<(), BridgeError> {
    #[cfg(target_os = "android")]
    let _ = call_android_activity("deletePushTokenFromRust");
    selected_token_store()?.clear_push_registration()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_store::DevFileTokenStore;

    #[test]
    fn rotation_retains_previous_token_until_backend_sync() {
        let dir = tempfile::tempdir().unwrap();
        let store = DevFileTokenStore::new(
            dir.path().join("auth.json"),
            dir.path().join("trusted.json"),
        );
        store_registration(&store, "first", "android", "Pixel").unwrap();
        let rotated = store_registration(&store, "second", "android", "Pixel").unwrap();
        assert_eq!(rotated.previous_token.as_deref(), Some("first"));
        let same = store_registration(&store, "second", "android", "Pixel").unwrap();
        assert_eq!(same.previous_token.as_deref(), Some("first"));
    }

    #[test]
    fn rejects_invalid_platform_and_token() {
        let dir = tempfile::tempdir().unwrap();
        let store = DevFileTokenStore::new(
            dir.path().join("auth.json"),
            dir.path().join("trusted.json"),
        );
        assert!(store_registration(&store, "", "android", "Phone").is_err());
        assert!(store_registration(&store, "token", "windows", "PC").is_err());
    }
}
