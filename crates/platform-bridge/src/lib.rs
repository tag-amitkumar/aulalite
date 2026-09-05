// crates/platform-bridge/src/lib.rs
use async_trait::async_trait;
use thiserror::Error;

pub mod navigation;

#[derive(Debug, Error)]
pub enum BridgeError {
    #[error("permission denied")]
    PermissionDenied,
    #[error("not implemented on this platform")]
    NotImplemented,
    #[error("native application configuration error: {0}")]
    Configuration(String),
    #[error("{0}")]
    Authentication(String),
    #[error("io: {0}")]
    Io(String),
}

#[cfg(not(target_arch = "wasm32"))]
#[async_trait]
pub trait PlatformBridge: Send + Sync {
    async fn current_id_token(&self) -> Result<String, BridgeError>;
    async fn sign_in_email_password(
        &self,
        email: &str,
        password: &str,
    ) -> Result<String, BridgeError>;
    async fn sign_up_email_password(
        &self,
        email: &str,
        password: &str,
    ) -> Result<String, BridgeError>;
    async fn sign_out(&self) -> Result<(), BridgeError>;
    async fn send_password_reset(&self, email: &str) -> Result<(), BridgeError>;
}

#[cfg(target_arch = "wasm32")]
#[async_trait(?Send)]
pub trait PlatformBridge {
    async fn current_id_token(&self) -> Result<String, BridgeError>;
    async fn sign_in_email_password(
        &self,
        email: &str,
        password: &str,
    ) -> Result<String, BridgeError>;
    async fn sign_up_email_password(
        &self,
        email: &str,
        password: &str,
    ) -> Result<String, BridgeError>;
    async fn sign_out(&self) -> Result<(), BridgeError>;
    async fn send_password_reset(&self, email: &str) -> Result<(), BridgeError>;
}

#[cfg(target_arch = "wasm32")]
pub mod web;

#[cfg(not(target_arch = "wasm32"))]
pub mod native;

#[cfg(not(target_arch = "wasm32"))]
pub mod native_store;

#[cfg(not(target_arch = "wasm32"))]
pub mod native_paths;

#[cfg(not(target_arch = "wasm32"))]
pub mod native_preferences;

#[cfg(not(target_arch = "wasm32"))]
pub mod deep_links;

#[cfg(not(target_arch = "wasm32"))]
pub mod external;

#[cfg(not(target_arch = "wasm32"))]
pub mod native_files;

#[cfg(not(target_arch = "wasm32"))]
pub mod native_push;

#[cfg(not(target_arch = "wasm32"))]
pub mod offline;

#[cfg(all(test, target_arch = "wasm32"))]
mod tests {
    use crate::PlatformBridge;

    fn assert_platform_bridge<T: PlatformBridge>(_: T) {}

    #[test]
    fn web_bridge_implements_platform_bridge() {
        assert_platform_bridge(crate::web::WebBridge);
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod native_trait_tests {
    use crate::PlatformBridge;

    fn assert_platform_bridge<T: PlatformBridge>(_: T) {}

    #[test]
    fn native_bridge_implements_platform_bridge() {
        // Also exercises the Send + Sync bounds on the non-wasm trait.
        assert_platform_bridge(crate::native::NativeBridge);
    }
}
