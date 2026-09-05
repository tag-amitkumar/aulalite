# AulaLite Phase 3 Mobile And Native Auth Parity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Share web, desktop, and mobile auth bootstrap behavior while moving production native/mobile credentials behind OS secure storage.

**Architecture:** Split native credential persistence into a `TokenStore` boundary with secure-store and explicit dev-file implementations, then extract the shell auth bootstrap into a reusable `shell-web` hook used by web, desktop, and mobile. Keep route-level behavior stable, add a mobile auth-ready gate, and migrate the shared mobile route signout path through one helper.

**Tech Stack:** Rust 1.94 workspace, Dioxus 0.7, `platform-bridge`, `shell-web`, `shell-mobile`, `features-auth`, `features-courses`, `keyring-core`, Windows Credential Manager, Apple Keychain/Protected Data, Linux Secret Service, and Android native secure storage.

---

## File Structure

- Create: `crates/platform-bridge/src/native_store.rs`
  - Owns `TokenFile`, `TrustedDeviceFile`, `TokenStore`, dev-file storage, secure keyring storage, and store-mode selection.
- Modify: `crates/platform-bridge/src/native.rs`
  - Re-exports token DTOs, routes primary and remembered-device persistence through `TokenStore`, and keeps Firebase REST auth logic unchanged.
- Modify: `crates/platform-bridge/src/lib.rs`
  - Exposes the native store module on non-wasm targets.
- Modify: `crates/platform-bridge/Cargo.toml`
  - Adds `keyring-core` plus target-gated provider crates and `tempfile` as a native dev dependency.
- Create: `crates/shell-web/src/auth_bootstrap.rs`
  - Provides `use_auth_bootstrap`, `AuthBootstrapState`, `AuthSplash`, and pure bootstrap outcome helpers.
- Modify: `crates/shell-web/src/lib.rs`
  - Uses the shared auth bootstrap hook and keeps the existing router/provider stack.
- Modify: `crates/shell-web/src/routes/mod.rs`
  - Adds `use_signout_action` for shared signout cleanup.
- Modify: `crates/shell-web/src/routes/dashboard.rs`
  - Migrates the primary mobile-shared shell signout call to the shared helper.
- Modify: `crates/shell-mobile/src/lib.rs`
  - Moves `MobileApp` into the library target and uses the shared auth bootstrap/provider stack.
- Modify: `crates/shell-mobile/src/main.rs`
  - Launches `shell_mobile::MobileApp`.
- Modify: `crates/shell-mobile/src/route_enum.rs`
  - Keeps the existing mobile route set.
- Modify: `docs/superpowers/specs/2026-06-17-aulalite-phase-3-mobile-native-auth-parity-design.md`
  - No code-phase changes expected; update only if implementation discovers a concrete design correction.

## Task 1: Add Native Token Store Boundary

**Files:**
- Create: `crates/platform-bridge/src/native_store.rs`
- Modify: `crates/platform-bridge/src/lib.rs`
- Modify: `crates/platform-bridge/Cargo.toml`
- Modify: `crates/platform-bridge/src/native.rs`

- [ ] **Step 1: Add failing token-store tests**

Create `crates/platform-bridge/src/native_store.rs` with the public data types, trait, and tests first:

```rust
#![cfg(not(target_arch = "wasm32"))]

use crate::BridgeError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

pub const TOKEN_STORE_ENV: &str = "AULALITE_TOKEN_STORE";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TokenFile {
    pub id_token: String,
    pub refresh_token: String,
    pub expires_at_unix: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TrustedDeviceFile {
    pub tokens_by_email: BTreeMap<String, String>,
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

    pub fn resolve(self, cfg_test: bool) -> Self {
        match self {
            Self::Auto if cfg_test => Self::DevFile,
            Self::Auto => Self::Secure,
            explicit => explicit,
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
}

#[derive(Debug, Clone)]
pub struct DevFileTokenStore {
    auth_path: PathBuf,
    trusted_devices_path: PathBuf,
}

impl DevFileTokenStore {
    pub fn new(auth_path: PathBuf, trusted_devices_path: PathBuf) -> Self {
        Self {
            auth_path,
            trusted_devices_path,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SecureTokenStore;

impl SecureTokenStore {
    pub fn new() -> Result<Self, BridgeError> {
        Err(BridgeError::Io("secure token store unavailable".into()))
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
        }
    }

    #[test]
    fn token_store_mode_parse_accepts_known_values() {
        assert_eq!(TokenStoreMode::parse(None).unwrap(), TokenStoreMode::Auto);
        assert_eq!(TokenStoreMode::parse(Some("")).unwrap(), TokenStoreMode::Auto);
        assert_eq!(TokenStoreMode::parse(Some("auto")).unwrap(), TokenStoreMode::Auto);
        assert_eq!(TokenStoreMode::parse(Some("secure")).unwrap(), TokenStoreMode::Secure);
        assert_eq!(TokenStoreMode::parse(Some("dev-file")).unwrap(), TokenStoreMode::DevFile);
    }

    #[test]
    fn token_store_mode_parse_rejects_unknown_values() {
        let err = TokenStoreMode::parse(Some("plaintext")).unwrap_err().to_string();
        assert!(err.contains("invalid AULALITE_TOKEN_STORE value"), "{err}");
    }

    #[test]
    fn token_store_mode_auto_is_secure_outside_tests() {
        assert_eq!(TokenStoreMode::Auto.resolve(false), TokenStoreMode::Secure);
        assert_eq!(TokenStoreMode::Auto.resolve(true), TokenStoreMode::DevFile);
        assert_eq!(TokenStoreMode::DevFile.resolve(false), TokenStoreMode::DevFile);
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

        assert_eq!(store.trusted_device_token("ADA@EXAMPLE.TEST").unwrap(), None);
        store
            .persist_trusted_device_token("  ADA@EXAMPLE.TEST  ", "device-token")
            .unwrap();
        assert_eq!(
            store.trusted_device_token("ada@example.test").unwrap(),
            Some("device-token".into())
        );
        store.clear_trusted_device_token("ada@example.test").unwrap();
        assert_eq!(store.trusted_device_token("ada@example.test").unwrap(), None);
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
}
```

- [ ] **Step 2: Expose the native store module**

Add this to `crates/platform-bridge/src/lib.rs` below the existing native module declaration:

```rust
#[cfg(not(target_arch = "wasm32"))]
pub mod native_store;
```

- [ ] **Step 3: Add native test dependency**

Add this section to `crates/platform-bridge/Cargo.toml`:

```toml
[target.'cfg(not(target_arch = "wasm32"))'.dev-dependencies]
tempfile = "3"
```

- [ ] **Step 4: Re-export token DTOs from native bridge**

In `crates/platform-bridge/src/native.rs`, remove the local `TokenFile` and `TrustedDeviceFile` definitions and add this import near the top:

```rust
pub use crate::native_store::{TokenFile, TrustedDeviceFile};
```

- [ ] **Step 5: Run failing token-store tests**

Run:

```powershell
$env:CARGO_TARGET_DIR='C:\Users\Chiranjib Chaudhuri\Documents\Chiranjib\Elementors_Aula\target'
cargo test -p platform-bridge --lib native_store -- --nocapture
```

Expected: compile fails because `DevFileTokenStore` does not implement `TokenStore`.

- [ ] **Step 6: Implement dev-file store**

Replace the lower half of `crates/platform-bridge/src/native_store.rs` after `impl DevFileTokenStore` with:

```rust
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
}
```

- [ ] **Step 7: Run token-store tests**

Run:

```powershell
$env:CARGO_TARGET_DIR='C:\Users\Chiranjib Chaudhuri\Documents\Chiranjib\Elementors_Aula\target'
cargo test -p platform-bridge --lib native_store -- --nocapture
```

Expected: tests pass.

- [ ] **Step 8: Commit token-store boundary**

Run:

```powershell
git add crates/platform-bridge/src/native_store.rs crates/platform-bridge/src/lib.rs crates/platform-bridge/src/native.rs crates/platform-bridge/Cargo.toml Cargo.lock
git commit -m "feat(native): add token store boundary"
```

Expected: commit succeeds with the new module, re-export, and native test dependency.

## Task 2: Add Secure Store Providers And Store Selection

**Files:**
- Modify: `crates/platform-bridge/Cargo.toml`
- Modify: `crates/platform-bridge/src/native_store.rs`
- Modify: `crates/platform-bridge/src/native.rs`

- [ ] **Step 1: Add failing selection tests**

Append these tests inside the existing `#[cfg(test)] mod tests` in `crates/platform-bridge/src/native_store.rs`:

```rust
#[test]
fn dev_file_store_paths_match_existing_native_paths() {
    let store = DevFileTokenStore::default_paths().unwrap();
    assert!(store.auth_path.ends_with("auth.json"), "{:?}", store.auth_path);
    assert!(
        store.trusted_devices_path.ends_with("mfa_trusted_devices.json"),
        "{:?}",
        store.trusted_devices_path
    );
}

#[test]
fn selected_store_uses_dev_file_in_cfg_test_auto_mode() {
    let mode = TokenStoreMode::Auto.resolve(true);
    assert_eq!(mode, TokenStoreMode::DevFile);
}
```

- [ ] **Step 2: Run failing selection tests**

Run:

```powershell
$env:CARGO_TARGET_DIR='C:\Users\Chiranjib Chaudhuri\Documents\Chiranjib\Elementors_Aula\target'
cargo test -p platform-bridge --lib native_store::tests::dev_file_store_paths_match_existing_native_paths native_store::tests::selected_store_uses_dev_file_in_cfg_test_auto_mode
```

Expected: compile fails because `DevFileTokenStore::default_paths` does not exist.

- [ ] **Step 3: Add secure-store dependencies**

Update `crates/platform-bridge/Cargo.toml` with these target-gated dependencies:

```toml
[target.'cfg(not(target_arch = "wasm32"))'.dependencies]
reqwest = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
directories = "6"
keyring-core = "1.0.0"

[target.'cfg(windows)'.dependencies]
windows-native-keyring-store = "1.1.0"

[target.'cfg(any(target_os = "macos", target_os = "ios"))'.dependencies]
apple-native-keyring-store = { version = "1.0.0", default-features = false, features = ["keychain", "protected"] }

[target.'cfg(target_os = "linux")'.dependencies]
zbus-secret-service-keyring-store = { version = "1.0.0", features = ["rt-tokio-crypto-rust"] }

[target.'cfg(target_os = "android")'.dependencies]
android-native-keyring-store = "1.0.0"
```

- [ ] **Step 4: Implement default paths and store selection**

Add this code below `impl DevFileTokenStore` in `crates/platform-bridge/src/native_store.rs`:

```rust
impl DevFileTokenStore {
    pub fn default_paths() -> Result<Self, BridgeError> {
        let dirs = directories::ProjectDirs::from("com", "aulalite", "aulalite")
            .ok_or_else(|| BridgeError::Io("could not resolve OS config dir".into()))?;
        Ok(Self::new(
            dirs.config_dir().join("auth.json"),
            dirs.config_dir().join("mfa_trusted_devices.json"),
        ))
    }
}

pub fn selected_token_store() -> Result<Box<dyn TokenStore>, BridgeError> {
    let raw = std::env::var(TOKEN_STORE_ENV).ok();
    match TokenStoreMode::parse(raw.as_deref())?.resolve(cfg!(test)) {
        TokenStoreMode::DevFile => Ok(Box::new(DevFileTokenStore::default_paths()?)),
        TokenStoreMode::Secure => Ok(Box::new(SecureTokenStore::new()?)),
        TokenStoreMode::Auto => unreachable!("auto is resolved before selecting a store"),
    }
}
```

- [ ] **Step 5: Implement keyring secure store**

Replace `SecureTokenStore` in `crates/platform-bridge/src/native_store.rs` with:

```rust
const KEYRING_SERVICE: &str = "aulalite";
const PRIMARY_ENTRY: &str = "auth.primary";
const TRUSTED_DEVICE_PREFIX: &str = "mfa.trusted-device.";

#[derive(Clone)]
pub struct SecureTokenStore {
    store: std::sync::Arc<dyn keyring_core::CredentialStore>,
}

impl std::fmt::Debug for SecureTokenStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecureTokenStore").finish_non_exhaustive()
    }
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
}

#[cfg(windows)]
fn secure_credential_store() -> Result<std::sync::Arc<dyn keyring_core::CredentialStore>, BridgeError>
{
    windows_native_keyring_store::Store::new()
        .map(|store| store as std::sync::Arc<dyn keyring_core::CredentialStore>)
        .map_err(keyring_error)
}

#[cfg(all(target_os = "macos", not(target_os = "ios")))]
fn secure_credential_store() -> Result<std::sync::Arc<dyn keyring_core::CredentialStore>, BridgeError>
{
    apple_native_keyring_store::keychain::Store::new()
        .map(|store| store as std::sync::Arc<dyn keyring_core::CredentialStore>)
        .map_err(keyring_error)
}

#[cfg(target_os = "ios")]
fn secure_credential_store() -> Result<std::sync::Arc<dyn keyring_core::CredentialStore>, BridgeError>
{
    apple_native_keyring_store::protected::Store::new()
        .map(|store| store as std::sync::Arc<dyn keyring_core::CredentialStore>)
        .map_err(keyring_error)
}

#[cfg(target_os = "linux")]
fn secure_credential_store() -> Result<std::sync::Arc<dyn keyring_core::CredentialStore>, BridgeError>
{
    zbus_secret_service_keyring_store::Store::new()
        .map(|store| store as std::sync::Arc<dyn keyring_core::CredentialStore>)
        .map_err(keyring_error)
}

#[cfg(target_os = "android")]
fn secure_credential_store() -> Result<std::sync::Arc<dyn keyring_core::CredentialStore>, BridgeError>
{
    android_native_keyring_store::Store::new()
        .map(|store| store as std::sync::Arc<dyn keyring_core::CredentialStore>)
        .map_err(keyring_error)
}

#[cfg(not(any(
    windows,
    target_os = "macos",
    target_os = "ios",
    target_os = "linux",
    target_os = "android"
)))]
fn secure_credential_store() -> Result<std::sync::Arc<dyn keyring_core::CredentialStore>, BridgeError>
{
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
        self.trusted_device_entry(email)?
            .set_password(token)
            .map_err(keyring_error)
    }

    fn clear_trusted_device_token(&self, email: &str) -> Result<(), BridgeError> {
        ignore_missing_credential(self.trusted_device_entry(email)?.delete_credential())
    }

    fn clear_all_trusted_device_tokens(&self) -> Result<(), BridgeError> {
        let mut spec = std::collections::HashMap::new();
        spec.insert("service", KEYRING_SERVICE);
        for entry in self.store.search(&spec).map_err(keyring_error)? {
            if entry
                .get_specifiers()
                .map(|(_, user)| user.starts_with(TRUSTED_DEVICE_PREFIX))
                .unwrap_or(false)
            {
                ignore_missing_credential(entry.delete_credential())?;
            }
        }
        Ok(())
    }
}
```

- [ ] **Step 6: Wire `NativeBridge` through selected store**

In `crates/platform-bridge/src/native.rs`, replace direct file helpers with selected-store calls:

```rust
use crate::native_store::selected_token_store;

impl NativeBridge {
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

    fn clear_all_trusted_device_tokens() -> Result<(), BridgeError> {
        selected_token_store()?.clear_all_trusted_device_tokens()
    }
}
```

Keep `password_flow`, `refresh`, `current_id_token`, and Firebase REST calls unchanged except that they call these store-backed methods.

- [ ] **Step 7: Run platform bridge verification**

Run:

```powershell
$env:CARGO_TARGET_DIR='C:\Users\Chiranjib Chaudhuri\Documents\Chiranjib\Elementors_Aula\target'
cargo test -p platform-bridge --lib -- --nocapture
```

Expected: tests pass on the host target.

- [ ] **Step 8: Commit secure token store selection**

Run:

```powershell
git add crates/platform-bridge/Cargo.toml crates/platform-bridge/src/native_store.rs crates/platform-bridge/src/native.rs Cargo.lock
git commit -m "feat(native): use secure token store by default"
```

Expected: commit succeeds with provider dependencies and `NativeBridge` using `selected_token_store`.

## Task 3: Extract Shared Auth Bootstrap

**Files:**
- Create: `crates/shell-web/src/auth_bootstrap.rs`
- Modify: `crates/shell-web/src/lib.rs`

- [ ] **Step 1: Add shared bootstrap helper tests**

Create `crates/shell-web/src/auth_bootstrap.rs` with pure helpers and tests:

```rust
use crate::contexts::{UserContext, UserContextSignal};
use dioxus::prelude::*;
use features_courses::api::{self, ApiContext};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BootstrapDecision {
    SignedIn,
    Anonymous,
    StaleToken,
}

pub fn decide_bootstrap_outcome(had_token: bool, me_ok: bool) -> BootstrapDecision {
    match (had_token, me_ok) {
        (true, true) => BootstrapDecision::SignedIn,
        (true, false) => BootstrapDecision::StaleToken,
        (false, _) => BootstrapDecision::Anonymous,
    }
}

pub struct AuthBootstrapState {
    pub api_ctx_signal: Signal<ApiContext>,
    pub user_ctx_signal: UserContextSignal,
    pub auth_ready: Signal<bool>,
}

#[component]
pub fn AuthSplash() -> Element {
    rsx! {
        div { class: "app-auth-splash", "aria-busy": "true",
            span { class: "app-auth-splash-mark", "Aula", span { class: "gold", "Lite" } }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bootstrap_decision_distinguishes_signed_in_stale_and_anonymous() {
        assert_eq!(decide_bootstrap_outcome(true, true), BootstrapDecision::SignedIn);
        assert_eq!(decide_bootstrap_outcome(true, false), BootstrapDecision::StaleToken);
        assert_eq!(decide_bootstrap_outcome(false, true), BootstrapDecision::Anonymous);
        assert_eq!(decide_bootstrap_outcome(false, false), BootstrapDecision::Anonymous);
    }

    #[test]
    fn auth_splash_renders_stable_busy_markup() {
        let mut vdom = VirtualDom::new(AuthSplash);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("app-auth-splash"), "html: {html}");
        assert!(html.contains("aria-busy=\"true\""), "html: {html}");
        assert!(html.contains("Aula"), "html: {html}");
    }
}
```

- [ ] **Step 2: Run failing helper tests**

Run:

```powershell
$env:CARGO_TARGET_DIR='C:\Users\Chiranjib Chaudhuri\Documents\Chiranjib\Elementors_Aula\target'
cargo test -p shell-web --lib auth_bootstrap -- --nocapture
```

Expected: compile fails because `auth_bootstrap` is not exported.

- [ ] **Step 3: Export auth bootstrap module**

Add this near the top of `crates/shell-web/src/lib.rs`:

```rust
pub mod auth_bootstrap;
```

- [ ] **Step 4: Implement shared hook**

Add this function to `crates/shell-web/src/auth_bootstrap.rs` below `AuthSplash`:

```rust
pub fn use_auth_bootstrap() -> AuthBootstrapState {
    #[cfg(target_arch = "wasm32")]
    let initial_base_url = String::new();
    #[cfg(not(target_arch = "wasm32"))]
    let initial_base_url = api::native_api_base_url();

    let mut api_ctx_signal = use_signal(|| ApiContext {
        base_url: initial_base_url,
        id_token: String::new(),
    });
    let mut user_ctx_signal: UserContextSignal = use_signal(|| None);
    let mut bootstrapped = use_signal(|| false);
    let mut auth_ready = use_signal(|| false);

    use_hook(|| {
        #[cfg(target_arch = "wasm32")]
        {
            use platform_bridge::PlatformBridge;
            let refresh: api::RefreshFn = Arc::new(|| {
                Box::pin(async move {
                    let bridge = platform_bridge::web::WebBridge;
                    bridge.current_id_token().await.map_err(|e| format!("{e}"))
                })
            });
            api::set_refresh_fn(refresh);
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            use platform_bridge::PlatformBridge;
            let refresh: api::RefreshFn = Arc::new(|| {
                Box::pin(async move {
                    let bridge = platform_bridge::native::NativeBridge;
                    bridge.current_id_token().await.map_err(|e| format!("{e}"))
                })
            });
            api::set_refresh_fn(refresh);
        }
    });

    use_future(move || async move {
        if *bootstrapped.read() {
            return;
        }
        bootstrapped.set(true);

        #[cfg(target_arch = "wasm32")]
        {
            use platform_bridge::PlatformBridge;
            let bridge = platform_bridge::web::WebBridge;

            if let Some(token) = platform_bridge::web::local_token() {
                let bootstrap_ctx = ApiContext {
                    base_url: String::new(),
                    id_token: token.clone(),
                };
                match api::get_me(&bootstrap_ctx).await {
                    Ok(dto) => {
                        api_ctx_signal.set(bootstrap_ctx);
                        user_ctx_signal.set(Some(UserContext::from_dto(dto)));
                        auth_ready.set(true);
                        return;
                    }
                    Err(_) => {
                        platform_bridge::web::clear_local_token();
                    }
                }
            }

            match bridge.current_id_token().await {
                Ok(token) => {
                    if !api_ctx_signal.read().id_token.is_empty() {
                        auth_ready.set(true);
                        return;
                    }
                    let bootstrap_ctx = ApiContext {
                        base_url: String::new(),
                        id_token: token.clone(),
                    };
                    api_ctx_signal.set(bootstrap_ctx.clone());
                    match api::get_me(&bootstrap_ctx).await {
                        Ok(dto) => {
                            if api_ctx_signal.read().id_token == token {
                                user_ctx_signal.set(Some(UserContext::from_dto(dto)));
                            }
                        }
                        Err(_) => {
                            if api_ctx_signal.read().id_token == token {
                                user_ctx_signal.set(None);
                            }
                        }
                    }
                }
                Err(_) => {
                    if api_ctx_signal.read().id_token.is_empty() {
                        user_ctx_signal.set(None);
                    }
                }
            }
            auth_ready.set(true);
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            use platform_bridge::PlatformBridge;
            let bridge = platform_bridge::native::NativeBridge;
            match bridge.current_id_token().await {
                Ok(token) => {
                    if !api_ctx_signal.read().id_token.is_empty() {
                        auth_ready.set(true);
                        return;
                    }
                    let bootstrap_ctx = ApiContext {
                        base_url: api::native_api_base_url(),
                        id_token: token.clone(),
                    };
                    api_ctx_signal.set(bootstrap_ctx.clone());
                    match api::get_me(&bootstrap_ctx).await {
                        Ok(dto) => {
                            if api_ctx_signal.read().id_token == token {
                                user_ctx_signal.set(Some(UserContext::from_dto(dto)));
                            }
                        }
                        Err(_) => {
                            if api_ctx_signal.read().id_token == token {
                                user_ctx_signal.set(None);
                            }
                        }
                    }
                }
                Err(_) => {
                    if api_ctx_signal.read().id_token.is_empty() {
                        user_ctx_signal.set(None);
                    }
                }
            }
            auth_ready.set(true);
        }
    });

    use_context_provider::<Signal<ApiContext>>(|| api_ctx_signal);
    use_context_provider::<UserContextSignal>(|| user_ctx_signal);

    AuthBootstrapState {
        api_ctx_signal,
        user_ctx_signal,
        auth_ready,
    }
}
```

- [ ] **Step 5: Replace duplicated web bootstrap**

Replace the state, refresh, and bootstrap blocks in `crates/shell-web/src/lib.rs` with:

```rust
let auth = auth_bootstrap::use_auth_bootstrap();
```

Keep the existing provider stack and change the router gate to:

```rust
if *auth.auth_ready.read() {
    Router::<route_enum::Route> {}
} else {
    auth_bootstrap::AuthSplash {}
}
```

- [ ] **Step 6: Run shell-web tests**

Run:

```powershell
$env:CARGO_TARGET_DIR='C:\Users\Chiranjib Chaudhuri\Documents\Chiranjib\Elementors_Aula\target'
cargo test -p shell-web --lib auth_bootstrap -- --nocapture
```

Expected: auth bootstrap helper tests pass.

- [ ] **Step 7: Commit shared bootstrap extraction**

Run:

```powershell
git add crates/shell-web/src/auth_bootstrap.rs crates/shell-web/src/lib.rs
git commit -m "feat(shell): share auth bootstrap hook"
```

Expected: commit succeeds with no mobile changes yet.

## Task 4: Move Mobile App Into Library And Use Shared Bootstrap

**Files:**
- Modify: `crates/shell-mobile/src/lib.rs`
- Modify: `crates/shell-mobile/src/main.rs`
- Modify: `crates/shell-mobile/src/route_enum.rs`

- [ ] **Step 1: Add mobile SSR gate test**

Replace `crates/shell-mobile/src/lib.rs` with:

```rust
mod route_enum;

use design_system::ToastProvider;
use dioxus::prelude::*;
use dioxus_router::Router;

#[component]
pub fn MobileApp() -> Element {
    let auth = shell_web::auth_bootstrap::use_auth_bootstrap();

    rsx! {
        design_system::KineticsStyles {}
        design_system::LocaleProvider {
            design_system::kinetics_ui::ThemeProvider {
                ToastProvider {
                    if *auth.auth_ready.read() {
                        Router::<route_enum::MobileRoute> {}
                    } else {
                        shell_web::auth_bootstrap::AuthSplash {}
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mobile_app_renders_auth_splash_before_bootstrap_ready() {
        let mut dom = VirtualDom::new(MobileApp);
        dom.rebuild_in_place();
        let html = dioxus_ssr::render(&dom);
        assert!(html.contains("app-auth-splash"), "html: {html}");
        assert!(html.contains("Aula"), "html: {html}");
    }
}
```

- [ ] **Step 2: Simplify mobile main**

Replace `crates/shell-mobile/src/main.rs` with:

```rust
fn main() {
    dioxus::launch(shell_mobile::MobileApp);
}
```

- [ ] **Step 3: Run mobile tests**

Run:

```powershell
$env:CARGO_TARGET_DIR='C:\Users\Chiranjib Chaudhuri\Documents\Chiranjib\Elementors_Aula\target'
cargo test -p shell-mobile --lib -- --nocapture
```

Expected: mobile SSR test passes and renders the shared auth splash before route components mount.

- [ ] **Step 4: Commit mobile shared bootstrap**

Run:

```powershell
git add crates/shell-mobile/src/lib.rs crates/shell-mobile/src/main.rs crates/shell-mobile/src/route_enum.rs
git commit -m "feat(mobile): use shared auth bootstrap"
```

Expected: commit succeeds with mobile now using the same auth gate/provider stack as web.

## Task 5: Centralize Signout For Shared Routes

**Files:**
- Modify: `crates/shell-web/src/routes/mod.rs`
- Modify: `crates/shell-web/src/routes/dashboard.rs`

- [ ] **Step 1: Add signout action helper**

Add this to `crates/shell-web/src/routes/mod.rs` below `use_api`:

```rust
pub fn use_signout_action() -> impl Fn() + Copy + 'static {
    let nav = dioxus_router::use_navigator();
    let mut api_signal = use_context::<Signal<ApiContext>>();
    let mut user_signal = use_context::<UserContextSignal>();

    move || {
        #[cfg(target_arch = "wasm32")]
        {
            use platform_bridge::PlatformBridge;
            spawn(async move {
                platform_bridge::web::clear_local_token();
                let _ = platform_bridge::web::WebBridge.sign_out().await;
            });
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            use platform_bridge::PlatformBridge;
            spawn(async move {
                let _ = platform_bridge::native::NativeBridge.sign_out().await;
            });
        }

        #[cfg(target_arch = "wasm32")]
        let base_url = String::new();
        #[cfg(not(target_arch = "wasm32"))]
        let base_url = ApiContext {
            base_url: features_courses::api::native_api_base_url(),
            id_token: String::new(),
        }
        .base_url;

        api_signal.set(ApiContext {
            base_url,
            id_token: String::new(),
        });
        user_signal.set(None);
        nav.push(crate::route_enum::Route::Login {});
    }
}
```

- [ ] **Step 2: Migrate dashboard signout**

In `crates/shell-web/src/routes/dashboard.rs`, replace:

```rust
let nav = use_navigator();
```

with:

```rust
let nav = use_navigator();
let signout = crate::routes::use_signout_action();
```

Then replace the `on_signout` body in the `AppShell` with:

```rust
on_signout: move |_| signout(),
```

- [ ] **Step 3: Run dashboard compile coverage**

Run:

```powershell
$env:CARGO_TARGET_DIR='C:\Users\Chiranjib Chaudhuri\Documents\Chiranjib\Elementors_Aula\target'
cargo test -p shell-web --lib dashboard -- --nocapture
```

Expected: shell-web lib tests compile and dashboard tests pass.

- [ ] **Step 4: Commit shared signout helper**

Run:

```powershell
git add crates/shell-web/src/routes/mod.rs crates/shell-web/src/routes/dashboard.rs
git commit -m "feat(shell): centralize signout cleanup"
```

Expected: commit succeeds with the primary mobile-shared dashboard signout path centralized.

## Task 6: Final Verification And Integration

**Files:**
- Review: `Cargo.lock`
- Review: `crates/platform-bridge/src/native_store.rs`
- Review: `crates/shell-web/src/auth_bootstrap.rs`
- Review: `crates/shell-mobile/src/lib.rs`

- [ ] **Step 1: Run platform bridge tests**

Run:

```powershell
$env:CARGO_TARGET_DIR='C:\Users\Chiranjib Chaudhuri\Documents\Chiranjib\Elementors_Aula\target'
cargo test -p platform-bridge --lib -- --nocapture
```

Expected: all platform bridge tests pass.

- [ ] **Step 2: Run shell-web tests**

Run:

```powershell
$env:CARGO_TARGET_DIR='C:\Users\Chiranjib Chaudhuri\Documents\Chiranjib\Elementors_Aula\target'
cargo test -p shell-web --lib -- --nocapture
```

Expected: shell-web lib tests pass.

- [ ] **Step 3: Run shell-mobile tests**

Run:

```powershell
$env:CARGO_TARGET_DIR='C:\Users\Chiranjib Chaudhuri\Documents\Chiranjib\Elementors_Aula\target'
cargo test -p shell-mobile --lib -- --nocapture
```

Expected: shell-mobile lib tests pass.

- [ ] **Step 4: Run MFA login regression tests**

Run:

```powershell
$env:CARGO_TARGET_DIR='C:\Users\Chiranjib Chaudhuri\Documents\Chiranjib\Elementors_Aula\target'
cargo test -p features-auth --test login -- --nocapture
```

Expected: login and MFA regression tests pass.

- [ ] **Step 5: Run native check**

Run:

```powershell
$env:CARGO_TARGET_DIR='C:\Users\Chiranjib Chaudhuri\Documents\Chiranjib\Elementors_Aula\target'
cargo check -p shell-web --all-targets
```

Expected: native shell-web targets compile with the selected host secure-store provider.

- [ ] **Step 6: Run wasm check**

Run:

```powershell
$env:CARGO_TARGET_DIR='C:\Users\Chiranjib Chaudhuri\Documents\Chiranjib\Elementors_Aula\target'
cargo check -p shell-web --target wasm32-unknown-unknown
```

Expected: wasm shell-web compile succeeds and excludes native keyring providers.

- [ ] **Step 7: Inspect lockfile and dependency graph**

Run:

```powershell
git diff -- Cargo.lock
cargo tree -p platform-bridge
```

Expected: `Cargo.lock` contains keyring/provider crates only through `platform-bridge`; wasm-only dependencies are unchanged.

- [ ] **Step 8: Commit final verification adjustments**

Run:

```powershell
git status --short
git diff --check
git add Cargo.lock crates/platform-bridge crates/shell-web crates/shell-mobile
git commit -m "chore: verify phase 3 auth parity"
```

Expected: commit is created only if verification produced formatting, lockfile, or small compile-fix changes not already committed. If there are no changes, record the clean status in the final response instead of creating an empty commit.

## Self-Review

Spec coverage:
- Shared auth bootstrap: Task 3 extracts the hook; Task 4 uses it in mobile.
- Mobile auth-ready gate: Task 4 gates `Router::<MobileRoute>` behind `auth.auth_ready`.
- Provider stack parity: Task 4 adds locale, theme, toast, and shared auth/user contexts before mobile routes render.
- Token store abstraction: Task 1 creates `TokenStore`; Task 2 wires `NativeBridge`.
- Secure production default: Task 2 resolves `auto` to `secure` outside `cfg(test)` and adds target-gated OS providers.
- Dev-file fallback: Task 1 keeps JSON storage behind `DevFileTokenStore`; Task 2 selects it only through explicit mode or `cfg(test)`.
- Remembered-device preservation: Task 1 and Task 2 store remembered-device tokens through the same store policy.
- Signout consistency: Task 5 clears platform storage, live API token, user context, and route state through one helper for the mobile-shared dashboard path.
- Web storage separation: Task 3 keeps the existing web local-token/Firebase path.

Completion-language scan:
- The plan contains concrete file paths, code snippets, commands, and expected outcomes.
- No deferred work markers are intentionally left in the task list.

Type consistency:
- `TokenFile` and `TrustedDeviceFile` move from `native.rs` to `native_store.rs` and are re-exported from `native.rs`.
- `TokenStoreMode`, `DevFileTokenStore`, `SecureTokenStore`, and `selected_token_store` are introduced before `NativeBridge` uses them.
- `AuthBootstrapState`, `use_auth_bootstrap`, and `AuthSplash` are introduced before web and mobile shells use them.
