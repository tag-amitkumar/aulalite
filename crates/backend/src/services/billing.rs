// crates/backend/src/services/billing.rs
//! Stripe billing integration: a mockable client trait (mirroring the
//! `S3Client` / `MediaMtxClient` pattern), the reqwest-backed production
//! implementation, a deterministic test mock, and a pure webhook-signature
//! verifier.
//!
//! Everything here compiles and tests WITHOUT live Stripe keys: the trait is
//! `Send + Sync` and object-safe so `MockStripeClient` can be injected in tests
//! and in non-production startup, and `verify_webhook_signature` is a free
//! function that takes the secret + clock as arguments so it can be unit-tested
//! against a locally-computed HMAC.

use async_trait::async_trait;
use std::time::Duration;

use crate::db::billing::PlanRow;

#[derive(Debug, thiserror::Error)]
pub enum BillingError {
    #[error("stripe api error: {0}")]
    Api(String),
    #[error("transport: {0}")]
    Transport(String),
    /// The `Stripe-Signature` header was malformed (missing `t=`/`v1=`).
    #[error("malformed signature header")]
    MalformedSignature,
    /// No `v1` signature matched the locally-computed HMAC.
    #[error("signature mismatch")]
    SignatureMismatch,
    /// `|now - t|` exceeded the allowed tolerance window (replay protection).
    #[error("timestamp outside tolerance")]
    TimestampOutOfTolerance,
}

/// The result of creating a Stripe Checkout Session: the hosted-page URL the
/// client redirects the buyer to.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct CheckoutSession {
    pub id: String,
    pub url: String,
    pub expires_at: i64,
}

/// The result of creating a Stripe Customer Portal Session. Portal sessions
/// are deliberately short-lived and the URL must be handed directly to the
/// authenticated workspace administrator rather than persisted.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct BillingPortalSession {
    pub url: String,
}

/// Operations the (future) billing handlers need from Stripe. Object-safe and
/// `Send + Sync` so it can live behind `Arc<dyn StripeClient>` in `AppState`.
#[async_trait]
pub trait StripeClient: Send + Sync {
    /// Create a hosted Checkout Session for `tenant_id` to subscribe to `plan`.
    /// `customer_id` reuses an existing Stripe customer when present. Returns
    /// the hosted-page URL.
    async fn create_checkout_session(
        &self,
        tenant_id: uuid::Uuid,
        plan: &PlanRow,
        customer_id: Option<&str>,
        success_url: &str,
        cancel_url: &str,
        idempotency_key: &str,
        expires_at: i64,
    ) -> Result<CheckoutSession, BillingError>;

    /// Best-effort: ensure a Stripe customer exists for `tenant_id`, returning
    /// its id. Implementations may create one keyed by `email` + tenant
    /// metadata. Callers persist the returned id on `tenants.stripe_customer_id`.
    async fn ensure_customer(
        &self,
        tenant_id: uuid::Uuid,
        email: &str,
        idempotency_key: &str,
    ) -> Result<String, BillingError>;

    /// Create a hosted Stripe Customer Portal Session for an existing
    /// customer. `return_url` is supplied by the server from `APP_ORIGIN`; it
    /// must never come from an untrusted browser parameter.
    async fn create_billing_portal_session(
        &self,
        customer_id: &str,
        return_url: &str,
    ) -> Result<BillingPortalSession, BillingError>;
}

/// Production `StripeClient` backed by the Stripe REST API over reqwest.
///
/// Uses form-urlencoded request bodies + Bearer secret-key auth, matching the
/// Stripe HTTP API. The secret key is read from env at startup (see
/// `main.rs`); it is never logged.
#[derive(Clone)]
pub struct HttpStripeClient {
    secret_key: String,
    base_url: String,
    http: reqwest::Client,
}

impl HttpStripeClient {
    /// Construct against the live Stripe API base (`https://api.stripe.com`).
    pub fn new(secret_key: impl Into<String>) -> Self {
        Self::with_base_url(secret_key, "https://api.stripe.com")
    }

    /// Construct against an arbitrary base URL (used to point at a mock server
    /// in tests; production always uses [`Self::new`]).
    pub fn with_base_url(secret_key: impl Into<String>, base_url: impl Into<String>) -> Self {
        Self {
            secret_key: secret_key.into(),
            base_url: base_url.into(),
            http: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(5))
                .timeout(Duration::from_secs(20))
                .build()
                .expect("static Stripe HTTP client configuration must be valid"),
        }
    }
}

#[async_trait]
impl StripeClient for HttpStripeClient {
    async fn create_checkout_session(
        &self,
        tenant_id: uuid::Uuid,
        plan: &PlanRow,
        customer_id: Option<&str>,
        success_url: &str,
        cancel_url: &str,
        idempotency_key: &str,
        expires_at: i64,
    ) -> Result<CheckoutSession, BillingError> {
        let price_id = plan.stripe_price_id.as_deref().ok_or_else(|| {
            BillingError::Api(format!("plan '{}' has no stripe_price_id", plan.id))
        })?;
        let tenant_str = tenant_id.to_string();
        let plan_id = plan.id.clone();
        // Stripe expects form-urlencoded, with array/object keys flattened.
        let mut form: Vec<(String, String)> = vec![
            ("mode".into(), "subscription".into()),
            ("success_url".into(), success_url.into()),
            ("cancel_url".into(), cancel_url.into()),
            ("expires_at".into(), expires_at.to_string()),
            ("line_items[0][price]".into(), price_id.into()),
            ("line_items[0][quantity]".into(), "1".into()),
            ("client_reference_id".into(), tenant_str.clone()),
            (
                "subscription_data[metadata][tenant_id]".into(),
                tenant_str.clone(),
            ),
            (
                "subscription_data[metadata][plan_id]".into(),
                plan_id.clone(),
            ),
            ("metadata[tenant_id]".into(), tenant_str),
            ("metadata[plan_id]".into(), plan_id),
        ];
        if let Some(cid) = customer_id {
            form.push(("customer".into(), cid.into()));
        }

        let resp = self
            .http
            .post(format!("{}/v1/checkout/sessions", self.base_url))
            .bearer_auth(&self.secret_key)
            .header("Idempotency-Key", idempotency_key)
            .form(&form)
            .send()
            .await
            .map_err(|e| BillingError::Transport(e.to_string()))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = crate::services::bounded_http::text(resp, 64 * 1024)
                .await
                .unwrap_or_else(|error| error.to_string());
            return Err(BillingError::Api(format!(
                "checkout session create {status}: {body}"
            )));
        }
        let body: serde_json::Value = crate::services::bounded_http::json(resp, 64 * 1024)
            .await
            .map_err(|e| BillingError::Api(e.to_string()))?;
        let id = body
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| BillingError::Api("checkout session response missing id".into()))?
            .to_string();
        let url = body
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| BillingError::Api("checkout session response missing url".into()))?
            .to_string();
        let expires_at = body
            .get("expires_at")
            .and_then(|value| value.as_i64())
            .ok_or_else(|| {
                BillingError::Api("checkout session response missing expires_at".into())
            })?;
        Ok(CheckoutSession {
            id,
            url,
            expires_at,
        })
    }

    async fn ensure_customer(
        &self,
        tenant_id: uuid::Uuid,
        email: &str,
        idempotency_key: &str,
    ) -> Result<String, BillingError> {
        let form: Vec<(String, String)> = vec![
            ("email".into(), email.into()),
            ("metadata[tenant_id]".into(), tenant_id.to_string()),
        ];
        let resp = self
            .http
            .post(format!("{}/v1/customers", self.base_url))
            .bearer_auth(&self.secret_key)
            .header("Idempotency-Key", idempotency_key)
            .form(&form)
            .send()
            .await
            .map_err(|e| BillingError::Transport(e.to_string()))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = crate::services::bounded_http::text(resp, 64 * 1024)
                .await
                .unwrap_or_else(|error| error.to_string());
            return Err(BillingError::Api(format!(
                "customer create {status}: {body}"
            )));
        }
        let body: serde_json::Value = crate::services::bounded_http::json(resp, 64 * 1024)
            .await
            .map_err(|e| BillingError::Api(e.to_string()))?;
        let id = body
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| BillingError::Api("customer response missing id".into()))?
            .to_string();
        Ok(id)
    }

    async fn create_billing_portal_session(
        &self,
        customer_id: &str,
        return_url: &str,
    ) -> Result<BillingPortalSession, BillingError> {
        let form = [("customer", customer_id), ("return_url", return_url)];
        let resp = self
            .http
            .post(format!("{}/v1/billing_portal/sessions", self.base_url))
            .bearer_auth(&self.secret_key)
            .form(&form)
            .send()
            .await
            .map_err(|e| BillingError::Transport(e.to_string()))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = crate::services::bounded_http::text(resp, 64 * 1024)
                .await
                .unwrap_or_else(|error| error.to_string());
            return Err(BillingError::Api(format!(
                "billing portal session create {status}: {body}"
            )));
        }
        let body: serde_json::Value = crate::services::bounded_http::json(resp, 64 * 1024)
            .await
            .map_err(|e| BillingError::Api(e.to_string()))?;
        let url = body
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| BillingError::Api("billing portal session response missing url".into()))?
            .to_string();
        Ok(BillingPortalSession { url })
    }
}

/// Test-only stub for `StripeClient`. `pub` so integration tests outside this
/// crate can construct it, but `#[doc(hidden)]` to keep it out of rustdoc and
/// signal that production must never inject it. Returns deterministic fake
/// urls/ids derived from the inputs so assertions are stable.
#[doc(hidden)]
#[derive(Clone, Default)]
pub struct MockStripeClient;

impl MockStripeClient {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl StripeClient for MockStripeClient {
    async fn create_checkout_session(
        &self,
        tenant_id: uuid::Uuid,
        plan: &PlanRow,
        _customer_id: Option<&str>,
        _success_url: &str,
        _cancel_url: &str,
        _idempotency_key: &str,
        expires_at: i64,
    ) -> Result<CheckoutSession, BillingError> {
        Ok(CheckoutSession {
            id: format!("cs_mock_{}_{}", tenant_id.simple(), plan.id),
            url: format!(
                "https://mock.stripe/checkout/{}/{}",
                tenant_id.simple(),
                plan.id
            ),
            expires_at,
        })
    }

    async fn ensure_customer(
        &self,
        tenant_id: uuid::Uuid,
        _email: &str,
        _idempotency_key: &str,
    ) -> Result<String, BillingError> {
        Ok(format!("cus_mock_{}", tenant_id.simple()))
    }

    async fn create_billing_portal_session(
        &self,
        customer_id: &str,
        _return_url: &str,
    ) -> Result<BillingPortalSession, BillingError> {
        Ok(BillingPortalSession {
            url: format!("https://mock.stripe/portal/{customer_id}"),
        })
    }
}

use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Verify a Stripe webhook signature header against `payload`.
///
/// Implements Stripe's scheme: the `Stripe-Signature` header is a
/// comma-separated list of `key=value` pairs containing a timestamp `t=` and
/// one or more `v1=` HMAC-SHA256 signatures. The signed payload is the literal
/// string `"{t}.{payload}"`; the expected signature is the hex-encoded
/// HMAC-SHA256 of that string keyed by the endpoint `secret`. The comparison is
/// constant-time (via the `hmac` crate's `verify_slice`), and the request is
/// rejected if `|now_unix - t| > tolerance_secs` (replay protection).
pub fn verify_webhook_signature(
    payload: &[u8],
    sig_header: &str,
    secret: &str,
    tolerance_secs: i64,
    now_unix: i64,
) -> Result<(), BillingError> {
    // Parse `t=...` and all `v1=...` from the comma-separated header.
    let mut timestamp: Option<i64> = None;
    let mut v1_sigs: Vec<&str> = Vec::new();
    for part in sig_header.split(',') {
        let part = part.trim();
        let Some((k, v)) = part.split_once('=') else {
            continue;
        };
        match k {
            "t" => timestamp = v.trim().parse::<i64>().ok(),
            "v1" => v1_sigs.push(v.trim()),
            _ => {}
        }
    }
    let timestamp = timestamp.ok_or(BillingError::MalformedSignature)?;
    if v1_sigs.is_empty() {
        return Err(BillingError::MalformedSignature);
    }

    // Replay protection: reject if outside the tolerance window.
    if (now_unix - timestamp).abs() > tolerance_secs {
        return Err(BillingError::TimestampOutOfTolerance);
    }

    // Compute HMAC-SHA256 over "{t}.{payload}".
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .map_err(|e| BillingError::Api(format!("hmac key: {e}")))?;
    mac.update(timestamp.to_string().as_bytes());
    mac.update(b".");
    mac.update(payload);
    let expected = mac.finalize().into_bytes();

    // Constant-time compare against each provided v1 signature. A single match
    // is sufficient (Stripe may send multiple during secret rotation).
    for sig_hex in v1_sigs {
        let Some(sig_bytes) = hex_decode(sig_hex) else {
            continue;
        };
        if sig_bytes.len() == expected.len() && bool::from(ct_eq(&sig_bytes, &expected)) {
            return Ok(());
        }
    }
    Err(BillingError::SignatureMismatch)
}

/// Decode a lowercase/uppercase hex string to bytes; `None` on any non-hex
/// nibble or odd length.
fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(s.len() / 2);
    let nibble = |b: u8| -> Option<u8> {
        match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            b'A'..=b'F' => Some(b - b'A' + 10),
            _ => None,
        }
    };
    for pair in bytes.chunks(2) {
        let hi = nibble(pair[0])?;
        let lo = nibble(pair[1])?;
        out.push((hi << 4) | lo);
    }
    Some(out)
}

/// Constant-time equality for equal-length byte slices. Returns a value that is
/// `true` iff every byte matches, with no early exit on mismatch.
fn ct_eq(a: &[u8], b: &[u8]) -> subtle_bool::Choice {
    debug_assert_eq!(a.len(), b.len());
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    subtle_bool::Choice(diff == 0)
}

/// Minimal local constant-time `Choice` so we don't pull a new dependency just
/// for a boolean wrapper. `bool::from(Choice)` reads the inner flag; the
/// constant-time property comes from `ct_eq` never branching on the data.
mod subtle_bool {
    #[derive(Clone, Copy)]
    pub struct Choice(pub bool);
    impl From<Choice> for bool {
        #[inline(never)]
        fn from(c: Choice) -> bool {
            c.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderMap;
    use axum::{routing::post, Json, Router};
    use std::sync::{Arc, Mutex};
    use uuid::Uuid;

    fn tenant() -> Uuid {
        Uuid::parse_str("9c2f4a8e-7b13-4f7c-91d2-b6a8e5c0d3e1").unwrap()
    }

    fn plan() -> PlanRow {
        PlanRow {
            id: "pro".into(),
            name: "Pro".into(),
            monthly_price_cents: 14900,
            included_seats: 250,
            included_class_minutes: 60000,
            included_recording_gb: 250,
            stripe_price_id: Some("price_123".into()),
        }
    }

    /// Helper: compute the hex `v1` signature Stripe would send for a payload.
    fn sign(secret: &str, t: i64, payload: &[u8]) -> String {
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(t.to_string().as_bytes());
        mac.update(b".");
        mac.update(payload);
        let bytes = mac.finalize().into_bytes();
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn verify_accepts_known_good_signature() {
        let secret = "whsec_test_secret";
        let payload = br#"{"id":"evt_1","type":"checkout.session.completed"}"#;
        let t = 1_700_000_000_i64;
        let sig = sign(secret, t, payload);
        let header = format!("t={t},v1={sig}");
        let res = verify_webhook_signature(payload, &header, secret, 300, t + 5);
        assert!(res.is_ok(), "expected ok, got {res:?}");
    }

    #[test]
    fn verify_rejects_tampered_payload() {
        let secret = "whsec_test_secret";
        let payload = br#"{"id":"evt_1","type":"checkout.session.completed"}"#;
        let t = 1_700_000_000_i64;
        let sig = sign(secret, t, payload);
        let header = format!("t={t},v1={sig}");
        // Tamper: verify against a different payload than was signed.
        let tampered = br#"{"id":"evt_1","type":"customer.subscription.deleted"}"#;
        let res = verify_webhook_signature(tampered, &header, secret, 300, t + 5);
        assert!(
            matches!(res, Err(BillingError::SignatureMismatch)),
            "got {res:?}"
        );
    }

    #[test]
    fn verify_rejects_tampered_signature() {
        let secret = "whsec_test_secret";
        let payload = br#"{"id":"evt_1"}"#;
        let t = 1_700_000_000_i64;
        let mut sig = sign(secret, t, payload);
        // Flip the last hex nibble.
        let last = sig.pop().unwrap();
        let flipped = if last == '0' { '1' } else { '0' };
        sig.push(flipped);
        let header = format!("t={t},v1={sig}");
        let res = verify_webhook_signature(payload, &header, secret, 300, t + 5);
        assert!(
            matches!(res, Err(BillingError::SignatureMismatch)),
            "got {res:?}"
        );
    }

    #[test]
    fn verify_rejects_stale_timestamp() {
        let secret = "whsec_test_secret";
        let payload = br#"{"id":"evt_1"}"#;
        let t = 1_700_000_000_i64;
        let sig = sign(secret, t, payload);
        let header = format!("t={t},v1={sig}");
        // now is 10 minutes past t, tolerance only 300s.
        let res = verify_webhook_signature(payload, &header, secret, 300, t + 600);
        assert!(
            matches!(res, Err(BillingError::TimestampOutOfTolerance)),
            "got {res:?}"
        );
    }

    #[test]
    fn verify_rejects_malformed_header() {
        let res = verify_webhook_signature(b"{}", "garbage-no-equals", "s", 300, 0);
        assert!(
            matches!(res, Err(BillingError::MalformedSignature)),
            "got {res:?}"
        );
        let res = verify_webhook_signature(b"{}", "t=123", "s", 300, 123);
        assert!(
            matches!(res, Err(BillingError::MalformedSignature)),
            "got {res:?}"
        );
    }

    #[tokio::test]
    async fn mock_returns_deterministic_checkout_url() {
        let client = MockStripeClient::new();
        let session = client
            .create_checkout_session(
                tenant(),
                &plan(),
                None,
                "https://ok",
                "https://no",
                "checkout-key",
                1_700_003_600,
            )
            .await
            .unwrap();
        assert_eq!(
            session.url,
            "https://mock.stripe/checkout/9c2f4a8e7b134f7c91d2b6a8e5c0d3e1/pro"
        );
    }

    #[tokio::test]
    async fn mock_returns_deterministic_customer_id() {
        let client = MockStripeClient::new();
        let id = client
            .ensure_customer(tenant(), "owner@example.test", "customer-key")
            .await
            .unwrap();
        assert_eq!(id, "cus_mock_9c2f4a8e7b134f7c91d2b6a8e5c0d3e1");
    }

    #[tokio::test]
    async fn mock_returns_deterministic_portal_url() {
        let client = MockStripeClient::new();
        let session = client
            .create_billing_portal_session("cus_123", "https://app.test/admin/billing")
            .await
            .unwrap();
        assert_eq!(session.url, "https://mock.stripe/portal/cus_123");
    }

    #[tokio::test]
    async fn http_client_sends_idempotency_keys_for_customer_and_checkout() {
        let customer_key = Arc::new(Mutex::new(None::<String>));
        let checkout_key = Arc::new(Mutex::new(None::<String>));
        let customer_seen = Arc::clone(&customer_key);
        let checkout_seen = Arc::clone(&checkout_key);
        let app = Router::new()
            .route(
                "/v1/customers",
                post(move |headers: HeaderMap| {
                    let seen = Arc::clone(&customer_seen);
                    async move {
                        *seen.lock().unwrap() = headers
                            .get("idempotency-key")
                            .and_then(|value| value.to_str().ok())
                            .map(str::to_string);
                        Json(serde_json::json!({ "id": "cus_test" }))
                    }
                }),
            )
            .route(
                "/v1/checkout/sessions",
                post(move |headers: HeaderMap| {
                    let seen = Arc::clone(&checkout_seen);
                    async move {
                        *seen.lock().unwrap() = headers
                            .get("idempotency-key")
                            .and_then(|value| value.to_str().ok())
                            .map(str::to_string);
                        Json(serde_json::json!({
                            "id": "cs_test",
                            "url": "https://checkout.stripe.test/cs_test",
                            "expires_at": 1_900_000_000_i64
                        }))
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = HttpStripeClient::with_base_url("sk_test", format!("http://{addr}"));

        client
            .ensure_customer(tenant(), "owner@example.test", "customer-idem")
            .await
            .unwrap();
        client
            .create_checkout_session(
                tenant(),
                &plan(),
                Some("cus_test"),
                "https://app.test/success",
                "https://app.test/cancel",
                "checkout-idem",
                1_900_000_000,
            )
            .await
            .unwrap();

        assert_eq!(
            customer_key.lock().unwrap().as_deref(),
            Some("customer-idem")
        );
        assert_eq!(
            checkout_key.lock().unwrap().as_deref(),
            Some("checkout-idem")
        );
        server.abort();
    }
}
