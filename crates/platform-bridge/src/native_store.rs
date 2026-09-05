#![cfg(not(target_arch = "wasm32"))]

use crate::BridgeError;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;

pub const TOKEN_STORE_ENV: &str = "AULALITE_TOKEN_STORE";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TokenFile {
    pub id_token: String,
    pub refresh_token: String,
    pub expires_at_unix: u64,
    /// Older secure-store records predate this field and are Firebase-backed.
    #[serde(default)]
    pub source: TokenSource,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenSource {
    #[default]
    Firebase,
    /// The local-development login bypass has no refresh endpoint. Its token is
    /// still kept in the OS credential store so desktop/mobile dev sessions
    /// survive a restart, but it is never sent to Google's refresh endpoint.
    LocalDevelopment,
    /// Short-lived session JWT returned by the backend after an OIDC/LTI
    /// callback. The backend owns its lifetime and provides no refresh token.
    BackendCallback,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TrustedDeviceFile {
    pub tokens_by_email: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativePushRegistration {
    pub token: String,
    pub platform: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_token: Option<String>,
    pub updated_at_unix: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenStoreMode {
    Auto,
    Secure,
    DevFile,
}

impl TokenStoreMode {
    pub fn parse(raw: Option<&str>) -> Result<Self, BridgeError> {
        match raw.map(str::trim).filter(|value| !value.is_empty()) {
            None => Ok(Self::Auto),
            Some("auto") => Ok(Self::Auto),
            Some("secure") => Ok(Self::Secure),
            Some("dev-file") => Ok(Self::DevFile),
            Some(other) => Err(BridgeError::Io(format!(
                "invalid {TOKEN_STORE_ENV} value '{other}'; expected secure, dev-file, or auto"
            ))),
        }
    }

    pub fn resolve(self, cfg_test: bool, debug_build: bool) -> Result<Self, BridgeError> {
        match self {
            Self::Auto if cfg_test => Ok(Self::DevFile),
            Self::Auto => Ok(Self::Secure),
            Self::DevFile if !cfg_test && !debug_build => Err(BridgeError::Configuration(format!(
                "{TOKEN_STORE_ENV}=dev-file is forbidden in release builds"
            ))),
            explicit => Ok(explicit),
        }
    }
}

pub trait TokenStore: Send + Sync {
    fn load_primary(&self) -> Result<Option<TokenFile>, BridgeError>;
    fn save_primary(&self, tokens: &TokenFile) -> Result<(), BridgeError>;
    fn clear_primary(&self) -> Result<(), BridgeError>;
    fn trusted_device_token(&self, email: &str) -> Result<Option<String>, BridgeError>;
    fn persist_trusted_device_token(&self, email: &str, token: &str) -> Result<(), BridgeError>;
    fn clear_trusted_device_token(&self, email: &str) -> Result<(), BridgeError>;
    fn clear_all_trusted_device_tokens(&self) -> Result<(), BridgeError>;
    fn load_push_registration(&self) -> Result<Option<NativePushRegistration>, BridgeError>;
    fn save_push_registration(
        &self,
        registration: &NativePushRegistration,
    ) -> Result<(), BridgeError>;
    fn clear_push_registration(&self) -> Result<(), BridgeError>;
}

#[derive(Debug, Clone)]
pub struct DevFileTokenStore {
    auth_path: PathBuf,
    trusted_devices_path: PathBuf,
    push_registration_path: PathBuf,
}

impl DevFileTokenStore {
    pub fn new(auth_path: PathBuf, trusted_devices_path: PathBuf) -> Self {
        let push_registration_path = auth_path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join("push_registration.json");
        Self {
            auth_path,
            trusted_devices_path,
            push_registration_path,
        }
    }

    pub fn default_paths() -> Result<Self, BridgeError> {
        let settings_dir = crate::native_paths::settings_dir()?;
        Ok(Self::new(
            settings_dir.join("auth.json"),
            settings_dir.join("mfa_trusted_devices.json"),
        ))
    }
}

fn normalize_email(email: &str) -> String {
    email.trim().to_ascii_lowercase()
}

fn load_json<T>(path: &PathBuf, missing: T, corrupt_label: &str) -> Result<T, BridgeError>
where
    T: serde::de::DeserializeOwned,
{
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|e| BridgeError::Io(format!("corrupt {corrupt_label}: {e}"))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(missing),
        Err(e) => Err(BridgeError::Io(e.to_string())),
    }
}

fn save_json<T>(path: &PathBuf, value: &T) -> Result<(), BridgeError>
where
    T: serde::Serialize,
{
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| BridgeError::Io(e.to_string()))?;
    }
    let json = serde_json::to_vec_pretty(value).map_err(|e| BridgeError::Io(e.to_string()))?;
    std::fs::write(path, json).map_err(|e| BridgeError::Io(e.to_string()))
}

fn remove_file_if_present(path: &PathBuf) -> Result<(), BridgeError> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(BridgeError::Io(e.to_string())),
    }
}

impl TokenStore for DevFileTokenStore {
    fn load_primary(&self) -> Result<Option<TokenFile>, BridgeError> {
        load_json(&self.auth_path, None::<TokenFile>, "auth.json")
    }

    fn save_primary(&self, tokens: &TokenFile) -> Result<(), BridgeError> {
        save_json(&self.auth_path, tokens)
    }

    fn clear_primary(&self) -> Result<(), BridgeError> {
        remove_file_if_present(&self.auth_path)
    }

    fn trusted_device_token(&self, email: &str) -> Result<Option<String>, BridgeError> {
        let file: TrustedDeviceFile = load_json(
            &self.trusted_devices_path,
            TrustedDeviceFile::default(),
            "mfa_trusted_devices.json",
        )?;
        Ok(file
            .tokens_by_email
            .get(&normalize_email(email))
            .filter(|token| !token.trim().is_empty())
            .cloned())
    }

    fn persist_trusted_device_token(&self, email: &str, token: &str) -> Result<(), BridgeError> {
        let mut file: TrustedDeviceFile = load_json(
            &self.trusted_devices_path,
            TrustedDeviceFile::default(),
            "mfa_trusted_devices.json",
        )?;
        file.tokens_by_email
            .insert(normalize_email(email), token.to_string());
        save_json(&self.trusted_devices_path, &file)
    }

    fn clear_trusted_device_token(&self, email: &str) -> Result<(), BridgeError> {
        let mut file: TrustedDeviceFile = load_json(
            &self.trusted_devices_path,
            TrustedDeviceFile::default(),
            "mfa_trusted_devices.json",
        )?;
        file.tokens_by_email.remove(&normalize_email(email));
        save_json(&self.trusted_devices_path, &file)
    }

    fn clear_all_trusted_device_tokens(&self) -> Result<(), BridgeError> {
        remove_file_if_present(&self.trusted_devices_path)
    }

    fn load_push_registration(&self) -> Result<Option<NativePushRegistration>, BridgeError> {
        load_json(
            &self.push_registration_path,
            None::<NativePushRegistration>,
            "push_registration.json",
        )
    }

    fn save_push_registration(
        &self,
        registration: &NativePushRegistration,
    ) -> Result<(), BridgeError> {
        save_json(&self.push_registration_path, registration)
    }

    fn clear_push_registration(&self) -> Result<(), BridgeError> {
        remove_file_if_present(&self.push_registration_path)
    }
}

#[derive(Debug, Clone)]
pub struct SecureTokenStore {
    store: Arc<keyring_core::CredentialStore>,
}

impl SecureTokenStore {
    pub fn new() -> Result<Self, BridgeError> {
        Ok(Self {
            store: secure_credential_store()?,
        })
    }

    fn entry(&self, user: &str) -> Result<keyring_core::Entry, BridgeError> {
        self.store
            .build(KEYRING_SERVICE, user, None)
            .map_err(keyring_error)
    }

    fn trusted_device_entry(&self, email: &str) -> Result<keyring_core::Entry, BridgeError> {
        let user = format!("{TRUSTED_DEVICE_PREFIX}{}", normalize_email(email));
        self.entry(&user)
    }

    fn load_trusted_device_index(&self) -> Result<BTreeSet<String>, BridgeError> {
        match self.entry(TRUSTED_DEVICE_INDEX_ENTRY)?.get_password() {
            Ok(json) => serde_json::from_str(&json)
                .map_err(|e| BridgeError::Io(format!("corrupt secure trusted-device index: {e}"))),
            Err(keyring_core::Error::NoEntry) => Ok(BTreeSet::new()),
            Err(err) => Err(keyring_error(err)),
        }
    }

    fn save_trusted_device_index(&self, emails: &BTreeSet<String>) -> Result<(), BridgeError> {
        let json = serde_json::to_string(emails).map_err(|e| BridgeError::Io(e.to_string()))?;
        self.entry(TRUSTED_DEVICE_INDEX_ENTRY)?
            .set_password(&json)
            .map_err(keyring_error)
    }
}

const KEYRING_SERVICE: &str = "aulalite";
const PRIMARY_ENTRY: &str = "auth.primary";
const TRUSTED_DEVICE_INDEX_ENTRY: &str = "mfa.trusted-device-index";
const TRUSTED_DEVICE_PREFIX: &str = "mfa.trusted-device.";
const PUSH_REGISTRATION_ENTRY: &str = "push.registration";

pub fn selected_token_store() -> Result<Box<dyn TokenStore>, BridgeError> {
    let raw = std::env::var(TOKEN_STORE_ENV).ok();
    match TokenStoreMode::parse(raw.as_deref())?.resolve(cfg!(test), cfg!(debug_assertions))? {
        TokenStoreMode::DevFile => Ok(Box::new(DevFileTokenStore::default_paths()?)),
        TokenStoreMode::Secure => Ok(Box::new(SecureTokenStore::new()?)),
        TokenStoreMode::Auto => unreachable!("auto is resolved before selecting a store"),
    }
}

#[cfg(windows)]
fn secure_credential_store() -> Result<Arc<keyring_core::CredentialStore>, BridgeError> {
    windows_native_keyring_store::Store::new()
        .map(|store| -> Arc<keyring_core::CredentialStore> { store })
        .map_err(keyring_error)
}

#[cfg(target_os = "macos")]
fn secure_credential_store() -> Result<Arc<keyring_core::CredentialStore>, BridgeError> {
    apple_native_keyring_store::keychain::Store::new()
        .map(|store| -> Arc<keyring_core::CredentialStore> { store })
        .map_err(keyring_error)
}

#[cfg(target_os = "ios")]
fn secure_credential_store() -> Result<Arc<keyring_core::CredentialStore>, BridgeError> {
    apple_native_keyring_store::protected::Store::new()
        .map(|store| -> Arc<keyring_core::CredentialStore> { store })
        .map_err(keyring_error)
}

#[cfg(target_os = "linux")]
fn secure_credential_store() -> Result<Arc<keyring_core::CredentialStore>, BridgeError> {
    zbus_secret_service_keyring_store::Store::new()
        .map(|store| -> Arc<keyring_core::CredentialStore> { store })
        .map_err(keyring_error)
}

#[cfg(target_os = "android")]
fn secure_credential_store() -> Result<Arc<keyring_core::CredentialStore>, BridgeError> {
    android_native_keyring_store::Store::new()
        .map(|store| -> Arc<keyring_core::CredentialStore> { store })
        .map_err(keyring_error)
}

#[cfg(not(any(
    windows,
    target_os = "macos",
    target_os = "ios",
    target_os = "linux",
    target_os = "android"
)))]
fn secure_credential_store() -> Result<Arc<keyring_core::CredentialStore>, BridgeError> {
    Err(BridgeError::Io("secure token store unavailable".into()))
}

fn keyring_error(err: keyring_core::Error) -> BridgeError {
    BridgeError::Io(format!("secure token store unavailable: {err}"))
}

fn ignore_missing_credential(result: keyring_core::Result<()>) -> Result<(), BridgeError> {
    match result {
        Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
        Err(err) => Err(keyring_error(err)),
    }
}

impl TokenStore for SecureTokenStore {
    fn load_primary(&self) -> Result<Option<TokenFile>, BridgeError> {
        match self.entry(PRIMARY_ENTRY)?.get_password() {
            Ok(json) => serde_json::from_str(&json)
                .map(Some)
                .map_err(|e| BridgeError::Io(format!("corrupt secure auth token: {e}"))),
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(err) => Err(keyring_error(err)),
        }
    }

    fn save_primary(&self, tokens: &TokenFile) -> Result<(), BridgeError> {
        let json = serde_json::to_string(tokens).map_err(|e| BridgeError::Io(e.to_string()))?;
        self.entry(PRIMARY_ENTRY)?
            .set_password(&json)
            .map_err(keyring_error)
    }

    fn clear_primary(&self) -> Result<(), BridgeError> {
        ignore_missing_credential(self.entry(PRIMARY_ENTRY)?.delete_credential())
    }

    fn trusted_device_token(&self, email: &str) -> Result<Option<String>, BridgeError> {
        match self.trusted_device_entry(email)?.get_password() {
            Ok(token) if token.trim().is_empty() => Ok(None),
            Ok(token) => Ok(Some(token)),
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(err) => Err(keyring_error(err)),
        }
    }

    fn persist_trusted_device_token(&self, email: &str, token: &str) -> Result<(), BridgeError> {
        let normalized = normalize_email(email);
        self.trusted_device_entry(&normalized)?
            .set_password(token)
            .map_err(keyring_error)?;
        let mut index = self.load_trusted_device_index()?;
        index.insert(normalized);
        self.save_trusted_device_index(&index)
    }

    fn clear_trusted_device_token(&self, email: &str) -> Result<(), BridgeError> {
        let normalized = normalize_email(email);
        ignore_missing_credential(self.trusted_device_entry(&normalized)?.delete_credential())?;
        let mut index = self.load_trusted_device_index()?;
        index.remove(&normalized);
        self.save_trusted_device_index(&index)
    }

    fn clear_all_trusted_device_tokens(&self) -> Result<(), BridgeError> {
        for email in self.load_trusted_device_index()? {
            ignore_missing_credential(self.trusted_device_entry(&email)?.delete_credential())?;
        }
        ignore_missing_credential(self.entry(TRUSTED_DEVICE_INDEX_ENTRY)?.delete_credential())
    }

    fn load_push_registration(&self) -> Result<Option<NativePushRegistration>, BridgeError> {
        match self.entry(PUSH_REGISTRATION_ENTRY)?.get_password() {
            Ok(json) => serde_json::from_str(&json)
                .map(Some)
                .map_err(|e| BridgeError::Io(format!("corrupt secure push registration: {e}"))),
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(err) => Err(keyring_error(err)),
        }
    }

    fn save_push_registration(
        &self,
        registration: &NativePushRegistration,
    ) -> Result<(), BridgeError> {
        let json =
            serde_json::to_string(registration).map_err(|e| BridgeError::Io(e.to_string()))?;
        self.entry(PUSH_REGISTRATION_ENTRY)?
            .set_password(&json)
            .map_err(keyring_error)
    }

    fn clear_push_registration(&self) -> Result<(), BridgeError> {
        ignore_missing_credential(self.entry(PUSH_REGISTRATION_ENTRY)?.delete_credential())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_tokens() -> TokenFile {
        TokenFile {
            id_token: "id-token".into(),
            refresh_token: "refresh-token".into(),
            expires_at_unix: 1_800_000_000,
            source: TokenSource::Firebase,
        }
    }

    #[test]
    fn token_store_mode_parse_accepts_known_values() {
        assert_eq!(TokenStoreMode::parse(None).unwrap(), TokenStoreMode::Auto);
        assert_eq!(
            TokenStoreMode::parse(Some("")).unwrap(),
            TokenStoreMode::Auto
        );
        assert_eq!(
            TokenStoreMode::parse(Some("auto")).unwrap(),
            TokenStoreMode::Auto
        );
        assert_eq!(
            TokenStoreMode::parse(Some("secure")).unwrap(),
            TokenStoreMode::Secure
        );
        assert_eq!(
            TokenStoreMode::parse(Some("dev-file")).unwrap(),
            TokenStoreMode::DevFile
        );
    }

    #[test]
    fn token_store_mode_parse_rejects_unknown_values() {
        let err = TokenStoreMode::parse(Some("plaintext"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("invalid AULALITE_TOKEN_STORE value"), "{err}");
    }

    #[test]
    fn token_store_mode_auto_is_secure_outside_tests() {
        assert_eq!(
            TokenStoreMode::Auto.resolve(false, true).unwrap(),
            TokenStoreMode::Secure
        );
        assert_eq!(
            TokenStoreMode::Auto.resolve(true, false).unwrap(),
            TokenStoreMode::DevFile
        );
        assert_eq!(
            TokenStoreMode::DevFile.resolve(false, true).unwrap(),
            TokenStoreMode::DevFile
        );
    }

    #[test]
    fn dev_file_store_is_forbidden_in_release_builds() {
        let error = TokenStoreMode::DevFile
            .resolve(false, false)
            .unwrap_err()
            .to_string();
        assert!(error.contains("forbidden in release builds"), "{error}");
        assert_eq!(
            TokenStoreMode::Secure.resolve(false, false).unwrap(),
            TokenStoreMode::Secure
        );
    }

    #[test]
    fn dev_file_store_roundtrips_primary_tokens() {
        let dir = tempfile::tempdir().unwrap();
        let store = DevFileTokenStore::new(
            dir.path().join("auth.json"),
            dir.path().join("mfa_trusted_devices.json"),
        );

        assert_eq!(store.load_primary().unwrap(), None);
        store.save_primary(&sample_tokens()).unwrap();
        assert_eq!(store.load_primary().unwrap(), Some(sample_tokens()));
        store.clear_primary().unwrap();
        assert_eq!(store.load_primary().unwrap(), None);
    }

    #[test]
    fn dev_file_store_roundtrips_trusted_device_tokens() {
        let dir = tempfile::tempdir().unwrap();
        let store = DevFileTokenStore::new(
            dir.path().join("auth.json"),
            dir.path().join("mfa_trusted_devices.json"),
        );

        assert_eq!(
            store.trusted_device_token("ADA@EXAMPLE.TEST").unwrap(),
            None
        );
        store
            .persist_trusted_device_token("  ADA@EXAMPLE.TEST  ", "device-token")
            .unwrap();
        assert_eq!(
            store.trusted_device_token("ada@example.test").unwrap(),
            Some("device-token".into())
        );
        store
            .clear_trusted_device_token("ada@example.test")
            .unwrap();
        assert_eq!(
            store.trusted_device_token("ada@example.test").unwrap(),
            None
        );
    }

    #[test]
    fn dev_file_store_roundtrips_push_registration() {
        let dir = tempfile::tempdir().unwrap();
        let store = DevFileTokenStore::new(
            dir.path().join("auth.json"),
            dir.path().join("mfa_trusted_devices.json"),
        );
        let registration = NativePushRegistration {
            token: "push-token".into(),
            platform: "android".into(),
            label: "Pixel".into(),
            previous_token: None,
            updated_at_unix: 1_800_000_000,
        };
        assert_eq!(store.load_push_registration().unwrap(), None);
        store.save_push_registration(&registration).unwrap();
        assert_eq!(store.load_push_registration().unwrap(), Some(registration));
        store.clear_push_registration().unwrap();
        assert_eq!(store.load_push_registration().unwrap(), None);
    }

    #[test]
    fn secure_store_unavailable_error_is_stable_on_unsupported_test_target() {
        #[cfg(not(any(
            windows,
            target_os = "macos",
            target_os = "ios",
            target_os = "linux",
            target_os = "android"
        )))]
        {
            let err = SecureTokenStore::new().unwrap_err().to_string();
            assert!(err.contains("secure token store unavailable"), "{err}");
        }
    }

    #[test]
    fn dev_file_store_paths_match_existing_native_paths() {
        let store = DevFileTokenStore::default_paths().unwrap();
        assert!(
            store.auth_path.ends_with("auth.json"),
            "{:?}",
            store.auth_path
        );
        assert!(
            store
                .trusted_devices_path
                .ends_with("mfa_trusted_devices.json"),
            "{:?}",
            store.trusted_devices_path
        );
        assert!(
            store
                .push_registration_path
                .ends_with("push_registration.json"),
            "{:?}",
            store.push_registration_path
        );
    }

    #[test]
    fn selected_store_uses_dev_file_in_cfg_test_auto_mode() {
        let mode = TokenStoreMode::Auto.resolve(true, false).unwrap();
        assert_eq!(mode, TokenStoreMode::DevFile);
    }
}
