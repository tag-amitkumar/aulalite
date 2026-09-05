// crates/backend/src/services/invitations.rs
//! Email-link sender abstraction. Production impl talks to Firebase Identity
//! Toolkit REST. Tests inject `MockEmailLinkSender`.

use async_trait::async_trait;
use serde::Deserialize;
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum SendError {
    #[error("firebase reported error: {0}")]
    Firebase(String),
    #[error("transport error: {0}")]
    Transport(String),
    #[error("unexpected response shape: {0}")]
    BadResponse(String),
}

/// One concern: send a Firebase email-link sign-in to `email` with the
/// caller's `continue_url` baked into the OOB action.
#[async_trait]
pub trait EmailLinkSender: Send + Sync {
    async fn send_invite(&self, email: &str, continue_url: &str) -> Result<(), SendError>;
}

// ============================================================
// Production Firebase Identity Toolkit REST impl
// ============================================================

#[derive(Clone)]
pub struct FirebaseEmailLinkSender {
    pub api_key: String,
    pub http: reqwest::Client,
}

impl FirebaseEmailLinkSender {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            http: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(5))
                .timeout(Duration::from_secs(15))
                .build()
                .expect("static Firebase email HTTP client configuration must be valid"),
        }
    }
}

#[async_trait]
impl EmailLinkSender for FirebaseEmailLinkSender {
    async fn send_invite(&self, email: &str, continue_url: &str) -> Result<(), SendError> {
        let url = format!(
            "https://identitytoolkit.googleapis.com/v1/accounts:sendOobCode?key={}",
            self.api_key
        );
        let body = serde_json::json!({
            "requestType": "EMAIL_SIGNIN",
            "email": email,
            "continueUrl": continue_url,
            "canHandleCodeInApp": true,
        });
        let resp = self
            .http
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| SendError::Transport(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = crate::services::bounded_http::text(resp, 64 * 1024)
                .await
                .unwrap_or_else(|error| error.to_string());
            return Err(SendError::Firebase(format!("{status}: {text}")));
        }

        // Identity Toolkit returns { "kind": "...", "email": "..." }
        #[derive(Deserialize)]
        #[allow(dead_code)]
        struct Ok200 {
            email: String,
        }
        let _: Ok200 = crate::services::bounded_http::json(resp, 64 * 1024)
            .await
            .map_err(|e| SendError::BadResponse(e.to_string()))?;
        Ok(())
    }
}

// ============================================================
// Test mock
// ============================================================

pub mod mock {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    pub struct MockEmailLinkSender {
        pub calls: Arc<Mutex<Vec<(String, String)>>>,
    }

    impl MockEmailLinkSender {
        pub fn new() -> Self {
            Self::default()
        }
        pub fn calls(&self) -> Vec<(String, String)> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl EmailLinkSender for MockEmailLinkSender {
        async fn send_invite(
            &self,
            email: &str,
            continue_url: &str,
        ) -> Result<(), super::SendError> {
            self.calls
                .lock()
                .unwrap()
                .push((email.to_string(), continue_url.to_string()));
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::mock::MockEmailLinkSender;
    use super::*;

    #[tokio::test]
    async fn mock_records_calls() {
        let sender = MockEmailLinkSender::new();
        sender
            .send_invite("a@example.test", "https://app.example/accept-invite/tok1")
            .await
            .unwrap();
        sender
            .send_invite("b@example.test", "https://app.example/accept-invite/tok2")
            .await
            .unwrap();
        let calls = sender.calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].0, "a@example.test");
        assert_eq!(calls[1].1, "https://app.example/accept-invite/tok2");
    }
}
