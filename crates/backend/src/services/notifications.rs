// crates/backend/src/services/notifications.rs
//! Notification delivery: mockable email + push sender traits (mirroring the
//! `EmailLinkSender` / `StripeClient` pattern), the reqwest-backed production
//! implementations, deterministic test mocks, and a `notify` facade that fans a
//! single event out across the in-app / email / push channels honoring the
//! user's `notification_preferences`.
//!
//! Everything here compiles, tests, and runs WITHOUT provider keys: both traits
//! are object-safe + `Send + Sync` so `MockEmailNotifier` / `MockPushSender`
//! can be injected behind `Arc<dyn …>` in `AppState` (non-prod startup + tests).
//! The `notify` facade is best-effort: a failing channel is logged and skipped,
//! never propagated to the caller.

use async_trait::async_trait;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use uuid::Uuid;

fn provider_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(15))
        .build()
        .expect("static notification provider HTTP client configuration must be valid")
}

/// Escape a string for safe inclusion in HTML *text* content (and, since it
/// escapes quotes too, it is also safe inside double-quoted attributes).
///
/// Replacements are applied in order with `&` FIRST so the entities introduced
/// by the later replacements (e.g. `&lt;`) are not themselves re-escaped:
///   `&` -> `&amp;`, `<` -> `&lt;`, `>` -> `&gt;`, `"` -> `&quot;`, `'` -> `&#x27;`
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

/// Escape a URL for safe inclusion inside a double-quoted `href="…"` attribute.
///
/// This neutralizes the characters that could break out of the attribute or the
/// surrounding tag (`"`, `<`, `>`, `&`) while leaving the rest of the URL intact
/// so it stays usable as a link target. We deliberately do NOT escape `'` here
/// (the attribute is double-quoted) and we keep everything else verbatim. `&`
/// is escaped FIRST for the same reason as in [`html_escape`].
fn html_attr_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Build the escaped (subject, html) pair for a transactional notification
/// email from the user/teacher-controlled `title`/`body`/`link`.
///
/// Pure + side-effect-free so it can be unit-tested without a DB. Every value
/// interpolated into the returned HTML is escaped: `title`/`body` are HTML text
/// content (`html_escape`); `link` is placed inside an `href="…"` attribute
/// (`html_attr_escape`). The returned subject is escaped for defense in depth.
fn build_email_subject_html(
    title: &str,
    body: Option<&str>,
    link: Option<&str>,
) -> (String, String) {
    let esc_title = html_escape(title);
    let esc_body = body.map(html_escape);
    let esc_link = link.map(html_attr_escape);
    // Minimal HTML body; this is transactional, not marketing.
    let html = match (esc_body.as_deref(), esc_link.as_deref()) {
        (Some(b), Some(l)) => format!("<p>{b}</p><p><a href=\"{l}\">View</a></p>"),
        (Some(b), None) => format!("<p>{b}</p>"),
        (None, Some(l)) => format!("<p><a href=\"{l}\">View</a></p>"),
        (None, None) => format!("<p>{esc_title}</p>"),
    };
    (esc_title, html)
}

#[derive(Debug, thiserror::Error)]
pub enum NotifyError {
    #[error("provider api error: {0}")]
    Api(String),
    #[error("transport: {0}")]
    Transport(String),
    #[error("not configured")]
    NotConfigured,
}

// ===========================================================================
// Email
// ===========================================================================

/// One concern: deliver a transactional email. Object-safe + `Send + Sync` so
/// it can live behind `Arc<dyn EmailNotifier>` in `AppState`.
#[async_trait]
pub trait EmailNotifier: Send + Sync {
    async fn send_email(
        &self,
        to: &str,
        subject: &str,
        html: &str,
        text: &str,
    ) -> Result<(), NotifyError>;
}

/// Production `EmailNotifier` backed by the Resend REST API over reqwest.
///
/// POSTs to `https://api.resend.com/emails` with a Bearer `RESEND_API_KEY` and
/// a configured `from` address. The api key is read from env at startup (see
/// `main.rs`); it is never logged.
#[derive(Clone)]
pub struct ResendEmailNotifier {
    api_key: String,
    from: String,
    base_url: String,
    http: reqwest::Client,
}

impl ResendEmailNotifier {
    /// Construct against the live Resend API base (`https://api.resend.com`).
    pub fn new(api_key: impl Into<String>, from: impl Into<String>) -> Self {
        Self::with_base_url(api_key, from, "https://api.resend.com")
    }

    /// Construct against an arbitrary base URL (used to point at a mock server
    /// in tests; production always uses [`Self::new`]).
    pub fn with_base_url(
        api_key: impl Into<String>,
        from: impl Into<String>,
        base_url: impl Into<String>,
    ) -> Self {
        Self {
            api_key: api_key.into(),
            from: from.into(),
            base_url: base_url.into(),
            http: provider_http_client(),
        }
    }
}

#[async_trait]
impl EmailNotifier for ResendEmailNotifier {
    async fn send_email(
        &self,
        to: &str,
        subject: &str,
        html: &str,
        text: &str,
    ) -> Result<(), NotifyError> {
        let body = serde_json::json!({
            "from": self.from,
            "to": [to],
            "subject": subject,
            "html": html,
            "text": text,
        });
        let resp = self
            .http
            .post(format!("{}/emails", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| NotifyError::Transport(e.to_string()))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = crate::services::bounded_http::text(resp, 64 * 1024)
                .await
                .unwrap_or_else(|error| error.to_string());
            return Err(NotifyError::Api(format!("resend {status}: {text}")));
        }
        Ok(())
    }
}

// ===========================================================================
// Push
// ===========================================================================

const FCM_SCOPE: &str = "https://www.googleapis.com/auth/firebase.messaging";
const DEFAULT_FCM_BASE_URL: &str = "https://fcm.googleapis.com";
const DEFAULT_FCM_TOKEN_URI: &str = "https://oauth2.googleapis.com/token";
const OAUTH_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:jwt-bearer";
const TOKEN_REFRESH_SKEW_SECONDS: i64 = 300;

#[derive(Debug, Clone)]
pub struct FcmProviderConfig {
    pub project_id: String,
    pub client_email: String,
    pub private_key: String,
    pub private_key_id: Option<String>,
    pub token_uri: String,
    pub fcm_base_url: String,
}

#[derive(Debug, Deserialize)]
struct ServiceAccountJson {
    project_id: Option<String>,
    private_key_id: Option<String>,
    private_key: Option<String>,
    client_email: Option<String>,
    token_uri: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum FcmConfigError {
    #[error("invalid service account json: {0}")]
    InvalidJson(String),
    #[error("service account is missing {0}")]
    MissingField(&'static str),
}

impl FcmProviderConfig {
    pub fn from_env() -> Result<Option<Self>, FcmConfigError> {
        let project_id = std::env::var("FCM_PROJECT_ID").ok();
        let inline_json = std::env::var("FCM_SERVICE_ACCOUNT_JSON").ok();
        let json_path = std::env::var("FCM_SERVICE_ACCOUNT_JSON_PATH").ok();
        let token_uri = std::env::var("FCM_TOKEN_URI").ok();
        let fcm_base_url = std::env::var("FCM_BASE_URL").ok();
        Self::from_parts(project_id, inline_json, json_path, token_uri, fcm_base_url)
    }

    pub fn from_parts(
        project_id: Option<String>,
        inline_json: Option<String>,
        json_path: Option<String>,
        token_uri: Option<String>,
        fcm_base_url: Option<String>,
    ) -> Result<Option<Self>, FcmConfigError> {
        let raw_json = match inline_json
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
        {
            Some(value) => value,
            None => match json_path
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
            {
                Some(path) => std::fs::read_to_string(path)
                    .map_err(|e| FcmConfigError::InvalidJson(e.to_string()))?,
                None => return Ok(None),
            },
        };

        let parsed: ServiceAccountJson = serde_json::from_str(&raw_json)
            .map_err(|e| FcmConfigError::InvalidJson(e.to_string()))?;
        let project_id = project_id
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .or_else(|| parsed.project_id.clone().and_then(trim_non_empty))
            .ok_or(FcmConfigError::MissingField("project_id"))?;
        let client_email = required(parsed.client_email, "client_email")?;
        let private_key = required(parsed.private_key, "private_key")?;
        Ok(Some(Self {
            project_id,
            client_email,
            private_key,
            private_key_id: parsed.private_key_id.and_then(trim_non_empty),
            token_uri: token_uri
                .and_then(trim_non_empty)
                .or_else(|| parsed.token_uri.and_then(trim_non_empty))
                .unwrap_or_else(|| DEFAULT_FCM_TOKEN_URI.to_string()),
            fcm_base_url: fcm_base_url
                .and_then(trim_non_empty)
                .unwrap_or_else(|| DEFAULT_FCM_BASE_URL.to_string()),
        }))
    }
}

fn trim_non_empty(value: String) -> Option<String> {
    let value = value.trim().to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn required(value: Option<String>, name: &'static str) -> Result<String, FcmConfigError> {
    value
        .and_then(trim_non_empty)
        .ok_or(FcmConfigError::MissingField(name))
}

fn safe_push_link(link: Option<&str>) -> Option<String> {
    let link = link.map(str::trim).filter(|value| !value.is_empty())?;
    if link.starts_with("/app/") || link == "/" {
        Some(link.to_string())
    } else {
        None
    }
}

fn build_fcm_message(
    token: &str,
    title: &str,
    body: &str,
    link: Option<&str>,
) -> serde_json::Value {
    let safe_link = safe_push_link(link);
    let mut data = serde_json::Map::new();
    if let Some(link) = safe_link.as_deref() {
        data.insert("link".into(), serde_json::Value::String(link.to_string()));
    }
    let mut message = serde_json::json!({
        "token": token,
        "notification": { "title": title, "body": body },
        "data": data,
    });
    if let Some(link) = safe_link {
        message["webpush"] = serde_json::json!({
            "fcm_options": { "link": link }
        });
    }
    serde_json::json!({ "message": message })
}

#[derive(Debug, Serialize)]
struct FcmJwtClaims {
    iss: String,
    scope: String,
    aud: String,
    exp: i64,
    iat: i64,
}

impl FcmJwtClaims {
    fn new(client_email: &str, token_uri: &str, now_unix: i64) -> Self {
        Self {
            iss: client_email.to_string(),
            scope: FCM_SCOPE.to_string(),
            aud: token_uri.to_string(),
            iat: now_unix,
            exp: now_unix + 3600,
        }
    }
}

#[derive(Debug, Clone)]
pub struct FcmAccessToken {
    pub token: String,
    pub expires_at_unix: i64,
}

#[async_trait]
pub trait FcmAccessTokenSource: Send + Sync {
    async fn mint_access_token(&self) -> Result<FcmAccessToken, NotifyError>;
}

pub trait Clock: Send + Sync {
    fn now_unix(&self) -> i64;
}

#[derive(Clone)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_unix(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64
    }
}

#[derive(Clone)]
pub struct GoogleServiceAccountTokenSource {
    cfg: FcmProviderConfig,
    http: reqwest::Client,
    clock: SystemClock,
}

#[derive(Debug, Deserialize)]
struct OAuthTokenResponse {
    access_token: String,
    expires_in: Option<i64>,
}

#[async_trait]
impl FcmAccessTokenSource for GoogleServiceAccountTokenSource {
    async fn mint_access_token(&self) -> Result<FcmAccessToken, NotifyError> {
        let now = self.clock.now_unix();
        let mut header = Header::new(Algorithm::RS256);
        header.typ = Some("JWT".into());
        header.kid = self.cfg.private_key_id.clone();
        let claims = FcmJwtClaims::new(&self.cfg.client_email, &self.cfg.token_uri, now);
        let key = EncodingKey::from_rsa_pem(self.cfg.private_key.as_bytes())
            .map_err(|e| NotifyError::Api(format!("fcm service account key: {e}")))?;
        let assertion = jsonwebtoken::encode(&header, &claims, &key)
            .map_err(|e| NotifyError::Api(format!("fcm jwt sign: {e}")))?;
        let resp = self
            .http
            .post(&self.cfg.token_uri)
            .form(&[
                ("grant_type", OAUTH_GRANT_TYPE),
                ("assertion", assertion.as_str()),
            ])
            .send()
            .await
            .map_err(|e| NotifyError::Transport(e.to_string()))?;
        let status = resp.status();
        let text = crate::services::bounded_http::text(resp, 64 * 1024)
            .await
            .map_err(|error| NotifyError::Transport(error.to_string()))?;
        if !status.is_success() {
            return Err(NotifyError::Api(format!(
                "fcm oauth {status}: {}",
                sanitize_provider_text(&text)
            )));
        }
        let parsed: OAuthTokenResponse = serde_json::from_str(&text)
            .map_err(|e| NotifyError::Api(format!("fcm oauth decode: {e}")))?;
        Ok(FcmAccessToken {
            token: parsed.access_token,
            expires_at_unix: now + parsed.expires_in.unwrap_or(3600),
        })
    }
}

pub struct CachedFcmAccessTokenProvider<S, C> {
    source: S,
    clock: C,
    cached: Mutex<Option<FcmAccessToken>>,
}

impl<S, C> CachedFcmAccessTokenProvider<S, C> {
    fn new(source: S, clock: C) -> Self {
        Self {
            source,
            clock,
            cached: Mutex::new(None),
        }
    }
}

impl<S, C> CachedFcmAccessTokenProvider<S, C>
where
    S: FcmAccessTokenSource,
    C: Clock,
{
    async fn access_token(&self) -> Result<String, NotifyError> {
        if let Some(token) = self.cached.lock().unwrap().clone() {
            if token.expires_at_unix - TOKEN_REFRESH_SKEW_SECONDS > self.clock.now_unix() {
                return Ok(token.token);
            }
        }
        let token = self.source.mint_access_token().await?;
        let value = token.token.clone();
        *self.cached.lock().unwrap() = Some(token);
        Ok(value)
    }
}

#[async_trait]
trait FcmTokenProvider: Send + Sync {
    async fn access_token(&self) -> Result<String, NotifyError>;
}

#[async_trait]
impl<S, C> FcmTokenProvider for CachedFcmAccessTokenProvider<S, C>
where
    S: FcmAccessTokenSource,
    C: Clock,
{
    async fn access_token(&self) -> Result<String, NotifyError> {
        CachedFcmAccessTokenProvider::access_token(self).await
    }
}

fn sanitize_provider_text(text: &str) -> String {
    text.replace('\n', " ").chars().take(500).collect()
}

/// One concern: deliver a push notification to a set of device tokens.
/// Object-safe + `Send + Sync` so it can live behind `Arc<dyn PushSender>`.
#[async_trait]
pub trait PushSender: Send + Sync {
    async fn send_push(
        &self,
        tokens: &[String],
        title: &str,
        body: &str,
        link: Option<&str>,
    ) -> Result<(), NotifyError>;
}

/// Production `PushSender` for Firebase Cloud Messaging HTTP v1.
pub struct FcmPushSender<P> {
    project_id: String,
    base_url: String,
    token_provider: P,
    http: reqwest::Client,
}

impl FcmPushSender<CachedFcmAccessTokenProvider<GoogleServiceAccountTokenSource, SystemClock>> {
    pub fn from_config(cfg: FcmProviderConfig) -> Self {
        let http = provider_http_client();
        let source = GoogleServiceAccountTokenSource {
            cfg: cfg.clone(),
            http: http.clone(),
            clock: SystemClock,
        };
        let provider = CachedFcmAccessTokenProvider::new(source, SystemClock);
        Self {
            project_id: cfg.project_id,
            base_url: cfg.fcm_base_url,
            token_provider: provider,
            http,
        }
    }
}

impl<P> FcmPushSender<P> {
    #[cfg(test)]
    fn with_token_provider(
        project_id: impl Into<String>,
        base_url: impl Into<String>,
        token_provider: P,
        http: reqwest::Client,
    ) -> Self {
        Self {
            project_id: project_id.into(),
            base_url: base_url.into(),
            token_provider,
            http,
        }
    }
}

#[async_trait]
impl<P> PushSender for FcmPushSender<P>
where
    P: FcmTokenProvider,
{
    async fn send_push(
        &self,
        tokens: &[String],
        title: &str,
        body: &str,
        link: Option<&str>,
    ) -> Result<(), NotifyError> {
        let access_token = self.token_provider.access_token().await?;
        let url = format!(
            "{}/v1/projects/{}/messages:send",
            self.base_url.trim_end_matches('/'),
            self.project_id
        );
        let mut first_err: Option<NotifyError> = None;
        for token in tokens {
            let message = build_fcm_message(token, title, body, link);
            let resp = self
                .http
                .post(&url)
                .bearer_auth(&access_token)
                .json(&message)
                .send()
                .await;
            match resp {
                Ok(r) if r.status().is_success() => {}
                Ok(r) => {
                    let status = r.status();
                    let text = crate::services::bounded_http::text(r, 64 * 1024)
                        .await
                        .unwrap_or_else(|error| error.to_string());
                    let e = NotifyError::Api(format!("fcm {status}: {text}"));
                    tracing::warn!(error = %e, "fcm send_push per-token failure");
                    first_err.get_or_insert(e);
                }
                Err(e) => {
                    let e = NotifyError::Transport(e.to_string());
                    tracing::warn!(error = %e, "fcm send_push transport failure");
                    first_err.get_or_insert(e);
                }
            }
        }
        match first_err {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}

#[derive(Clone, Default)]
pub struct DisabledPushSender;

#[async_trait]
impl PushSender for DisabledPushSender {
    async fn send_push(
        &self,
        tokens: &[String],
        title: &str,
        _body: &str,
        _link: Option<&str>,
    ) -> Result<(), NotifyError> {
        tracing::debug!(token_count = tokens.len(), %title, "push disabled; skipping send");
        Err(NotifyError::NotConfigured)
    }
}

// ===========================================================================
// Mocks
// ===========================================================================

pub mod mock {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// Test-only `EmailNotifier`. Records calls; always returns Ok. `pub` so
    /// integration tests + non-prod startup can inject it.
    #[doc(hidden)]
    #[derive(Clone, Default)]
    pub struct MockEmailNotifier {
        /// (to, subject, html, text) for each call.
        pub calls: Arc<Mutex<Vec<(String, String, String, String)>>>,
    }

    impl MockEmailNotifier {
        pub fn new() -> Self {
            Self::default()
        }
        pub fn calls(&self) -> Vec<(String, String, String, String)> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl EmailNotifier for MockEmailNotifier {
        async fn send_email(
            &self,
            to: &str,
            subject: &str,
            html: &str,
            text: &str,
        ) -> Result<(), NotifyError> {
            tracing::debug!(%to, %subject, "MockEmailNotifier.send_email");
            self.calls.lock().unwrap().push((
                to.to_string(),
                subject.to_string(),
                html.to_string(),
                text.to_string(),
            ));
            Ok(())
        }
    }

    #[derive(Clone)]
    pub struct MockAccessTokenSource {
        results: Arc<Mutex<Vec<Result<FcmAccessToken, String>>>>,
        calls: Arc<Mutex<usize>>,
    }

    impl MockAccessTokenSource {
        pub fn new(results: Vec<Result<FcmAccessToken, String>>) -> Self {
            Self {
                results: Arc::new(Mutex::new(results)),
                calls: Arc::new(Mutex::new(0)),
            }
        }

        pub fn calls(&self) -> usize {
            *self.calls.lock().unwrap()
        }
    }

    #[async_trait]
    impl FcmAccessTokenSource for MockAccessTokenSource {
        async fn mint_access_token(&self) -> Result<FcmAccessToken, NotifyError> {
            *self.calls.lock().unwrap() += 1;
            let next = self.results.lock().unwrap().remove(0);
            next.map_err(NotifyError::Api)
        }
    }

    #[derive(Clone)]
    pub struct TestClock {
        now: i64,
    }

    impl TestClock {
        pub fn new(now: i64) -> Self {
            Self { now }
        }
    }

    impl Clock for TestClock {
        fn now_unix(&self) -> i64 {
            self.now
        }
    }

    /// Test-only `PushSender`. Records calls; always returns Ok.
    #[doc(hidden)]
    #[derive(Clone, Default)]
    pub struct MockPushSender {
        /// (tokens, title, body, link) for each call.
        pub calls: Arc<Mutex<Vec<(Vec<String>, String, String, Option<String>)>>>,
    }

    impl MockPushSender {
        pub fn new() -> Self {
            Self::default()
        }
        pub fn calls(&self) -> Vec<(Vec<String>, String, String, Option<String>)> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl PushSender for MockPushSender {
        async fn send_push(
            &self,
            tokens: &[String],
            title: &str,
            body: &str,
            link: Option<&str>,
        ) -> Result<(), NotifyError> {
            tracing::debug!(token_count = tokens.len(), %title, "MockPushSender.send_push");
            self.calls.lock().unwrap().push((
                tokens.to_vec(),
                title.to_string(),
                body.to_string(),
                link.map(|s| s.to_string()),
            ));
            Ok(())
        }
    }
}

// ===========================================================================
// notify facade
// ===========================================================================

/// Fan a single notification event out across the in-app / email / push
/// channels for one user, honoring their `notification_preferences`.
///
/// Behavior:
///   1. read the user's preferences (defaults to all-enabled if no row);
///   2. if `in_app_enabled` -> persist a `notifications` row (tenant-scoped);
///   3. if `email_enabled`  -> look up the user's email + `send_email`;
///   4. if `push_enabled`   -> load the user's device tokens + `send_push`.
///
/// BEST-EFFORT: every channel is independent and a failure is logged + skipped.
/// This function never returns an error — the caller (e.g. the grade-release
/// path) must never have its primary operation fail because a notification
/// could not be delivered.
#[allow(clippy::too_many_arguments)]
pub async fn notify(
    pool: &PgPool,
    email: &dyn EmailNotifier,
    push: &dyn PushSender,
    tenant_id: Uuid,
    user_id: Uuid,
    kind: &str,
    title: &str,
    body: Option<&str>,
    link: Option<&str>,
) {
    use crate::db::notifications as ndb;

    // (1) Preferences — default all-true when absent.
    let prefs = match ndb::get_preferences(pool, user_id).await {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(error = %e, %user_id, "notify: get_preferences failed; assuming defaults");
            ndb::PrefRow::default_for(user_id)
        }
    };

    // (2) In-app row.
    let mut notification_id: Option<Uuid> = None;
    if prefs.in_app_enabled {
        match ndb::create_notification(pool, tenant_id, user_id, kind, title, body, link).await {
            Ok(row) => notification_id = Some(row.id),
            Err(e) => {
                tracing::warn!(error = %e, %user_id, %kind, "notify: create_notification failed");
            }
        }
    }

    // (3) Email.
    if prefs.email_enabled {
        match ndb::lookup_user_email(pool, user_id).await {
            Ok(Some(to)) => {
                // The plain-text part stays RAW: plain text needs no HTML escaping.
                let text = body.unwrap_or("");
                // The HTML part + subject interpolate user/teacher-controlled
                // content (notification title + body, and a link inside an href
                // attr), so they MUST be escaped before interpolation to close
                // the HTML-injection seam (e.g. a `grade_released` event carrying
                // a crafted assignment title). See `build_email_subject_html`.
                let (esc_subject, html) = build_email_subject_html(title, body, link);
                let (status, error_message) =
                    match email.send_email(&to, &esc_subject, &html, text).await {
                        Ok(()) => ("sent", None),
                        Err(e) => {
                            tracing::warn!(error = %e, %user_id, "notify: send_email failed");
                            ("failed", Some(e.to_string()))
                        }
                    };
                let target_hash = ndb::target_hash("email", &to);
                if let Err(e) = ndb::record_delivery(
                    pool,
                    ndb::NewDelivery {
                        tenant_id,
                        user_id,
                        notification_id,
                        channel: "email",
                        provider: "resend",
                        target_hash: &target_hash,
                        target_label: Some("email"),
                        device_token_id: None,
                        kind,
                        status,
                        provider_message_id: None,
                        provider_status: None,
                        error_code: None,
                        error_message: error_message.as_deref(),
                    },
                )
                .await
                {
                    tracing::warn!(error = %e, %user_id, "notify: record email delivery failed");
                }
            }
            Ok(None) => {
                tracing::debug!(%user_id, "notify: no email on file; skipping email channel");
                let target_hash = ndb::target_hash("email", "missing-email");
                if let Err(e) = ndb::record_delivery(
                    pool,
                    ndb::NewDelivery {
                        tenant_id,
                        user_id,
                        notification_id,
                        channel: "email",
                        provider: "resend",
                        target_hash: &target_hash,
                        target_label: Some("email missing"),
                        device_token_id: None,
                        kind,
                        status: "skipped",
                        provider_message_id: None,
                        provider_status: None,
                        error_code: Some("email_missing"),
                        error_message: None,
                    },
                )
                .await
                {
                    tracing::warn!(error = %e, %user_id, "notify: record email skipped delivery failed");
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, %user_id, "notify: lookup_user_email failed");
            }
        }
    }

    // (4) Push.
    if prefs.push_enabled {
        match ndb::list_active_device_tokens(pool, tenant_id, user_id).await {
            Ok(devices) if !devices.is_empty() => {
                let push_body = body.unwrap_or(title);
                for (device_id, token, platform, label) in devices {
                    let target_hash = ndb::target_hash("push", &token);
                    let target_label = ndb::device_target_label(&platform, label.as_deref());
                    let (status, error_code, error_message) = match push
                        .send_push(std::slice::from_ref(&token), title, push_body, link)
                        .await
                    {
                        Ok(()) => ("sent", None, None),
                        Err(NotifyError::NotConfigured) => {
                            ("skipped", Some("push_not_configured"), None)
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, %user_id, "notify: send_push failed");
                            ("failed", None, Some(e.to_string()))
                        }
                    };
                    if let Err(e) = ndb::record_delivery(
                        pool,
                        ndb::NewDelivery {
                            tenant_id,
                            user_id,
                            notification_id,
                            channel: "push",
                            provider: "fcm",
                            target_hash: &target_hash,
                            target_label: Some(&target_label),
                            device_token_id: Some(device_id),
                            kind,
                            status,
                            provider_message_id: None,
                            provider_status: None,
                            error_code,
                            error_message: error_message.as_deref(),
                        },
                    )
                    .await
                    {
                        tracing::warn!(error = %e, %user_id, "notify: record push delivery failed");
                    }
                }
            }
            Ok(_) => {
                tracing::trace!(%user_id, "notify: no device tokens; skipping push channel");
            }
            Err(e) => {
                tracing::warn!(error = %e, %user_id, "notify: list_device_tokens failed");
            }
        }
    } else {
        let target_hash = ndb::target_hash("push", "preference-disabled");
        if let Err(e) = ndb::record_delivery(
            pool,
            ndb::NewDelivery {
                tenant_id,
                user_id,
                notification_id,
                channel: "push",
                provider: "fcm",
                target_hash: &target_hash,
                target_label: Some("push preference disabled"),
                device_token_id: None,
                kind,
                status: "skipped",
                provider_message_id: None,
                provider_status: None,
                error_code: Some("push_disabled"),
                error_message: None,
            },
        )
        .await
        {
            tracing::warn!(error = %e, %user_id, "notify: record push skipped delivery failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::mock::{MockEmailNotifier, MockPushSender};
    use super::*;

    #[tokio::test]
    async fn mock_email_records_calls() {
        let m = MockEmailNotifier::new();
        m.send_email("a@example.test", "Subject", "<p>hi</p>", "hi")
            .await
            .unwrap();
        let calls = m.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "a@example.test");
        assert_eq!(calls[0].1, "Subject");
    }

    #[tokio::test]
    async fn mock_push_records_calls() {
        let m = MockPushSender::new();
        m.send_push(
            &["tok1".to_string(), "tok2".to_string()],
            "Title",
            "Body",
            Some("https://app/x"),
        )
        .await
        .unwrap();
        let calls = m.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0.len(), 2);
        assert_eq!(calls[0].3.as_deref(), Some("https://app/x"));
    }

    #[test]
    fn fcm_config_parses_inline_service_account_json() {
        let json = serde_json::json!({
            "project_id": "project-from-json",
            "private_key_id": "kid-123",
            "private_key": "-----BEGIN PRIVATE KEY-----\nabc\n-----END PRIVATE KEY-----\n",
            "client_email": "svc@example.iam.gserviceaccount.com",
            "token_uri": "https://oauth2.googleapis.com/token"
        })
        .to_string();

        let cfg = FcmProviderConfig::from_parts(
            Some("project-from-env".into()),
            Some(json),
            None,
            None,
            None,
        )
        .unwrap()
        .expect("configured");

        assert_eq!(cfg.project_id, "project-from-env");
        assert_eq!(cfg.client_email, "svc@example.iam.gserviceaccount.com");
        assert_eq!(cfg.private_key_id.as_deref(), Some("kid-123"));
        assert_eq!(cfg.token_uri, "https://oauth2.googleapis.com/token");
    }

    #[test]
    fn fcm_config_disabled_without_project_id() {
        let cfg = FcmProviderConfig::from_parts(None, None, None, None, None).unwrap();
        assert!(cfg.is_none());
    }

    #[test]
    fn safe_push_link_keeps_relative_paths_and_rejects_script_urls() {
        assert_eq!(
            safe_push_link(Some("/app/courses")),
            Some("/app/courses".into())
        );
        assert_eq!(safe_push_link(Some("https://app.example.com/x")), None);
        assert_eq!(safe_push_link(Some("javascript:alert(1)")), None);
        assert_eq!(safe_push_link(Some("   ")), None);
    }

    #[test]
    fn fcm_message_payload_omits_unsafe_link() {
        let value = build_fcm_message("tok", "Title", "Body", Some("javascript:alert(1)"));
        assert_eq!(value["message"]["token"], "tok");
        assert_eq!(value["message"]["notification"]["title"], "Title");
        assert!(value["message"]["data"].as_object().unwrap().is_empty());
        assert!(value["message"].get("webpush").is_none());
    }

    #[test]
    fn jwt_claims_use_fcm_scope_and_bounded_expiration() {
        let claims = FcmJwtClaims::new(
            "svc@example.iam.gserviceaccount.com",
            "https://oauth2.googleapis.com/token",
            1_800_000_000,
        );
        assert_eq!(claims.iss, "svc@example.iam.gserviceaccount.com");
        assert_eq!(claims.scope, FCM_SCOPE);
        assert_eq!(claims.aud, "https://oauth2.googleapis.com/token");
        assert_eq!(claims.iat, 1_800_000_000);
        assert_eq!(claims.exp, 1_800_003_600);
    }

    #[tokio::test]
    async fn cached_token_provider_reuses_fresh_token() {
        let source = mock::MockAccessTokenSource::new(vec![Ok(FcmAccessToken {
            token: "access-1".into(),
            expires_at_unix: 1_800_003_600,
        })]);
        let provider =
            CachedFcmAccessTokenProvider::new(source.clone(), mock::TestClock::new(1_800_000_000));

        assert_eq!(provider.access_token().await.unwrap(), "access-1");
        assert_eq!(provider.access_token().await.unwrap(), "access-1");
        assert_eq!(source.calls(), 1);
    }

    #[tokio::test]
    async fn fcm_sender_uses_cached_bearer_and_http_v1_endpoint() {
        let source = mock::MockAccessTokenSource::new(vec![Ok(FcmAccessToken {
            token: "access-1".into(),
            expires_at_unix: 1_800_003_600,
        })]);
        let provider =
            CachedFcmAccessTokenProvider::new(source, mock::TestClock::new(1_800_000_000));
        let sender = FcmPushSender::with_token_provider(
            "project-123",
            "http://127.0.0.1:9",
            provider,
            reqwest::Client::new(),
        );
        let err = sender
            .send_push(&["tok".to_string()], "Title", "Body", Some("/app/x"))
            .await
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("transport") || err.contains("provider api error"),
            "{err}"
        );
    }

    // --- HTML-injection hardening ------------------------------------------

    #[test]
    fn html_escape_neutralizes_script_tag() {
        assert_eq!(
            html_escape("<script>alert(1)</script>"),
            "&lt;script&gt;alert(1)&lt;/script&gt;"
        );
    }

    #[test]
    fn html_escape_handles_quote_and_ampersand() {
        // Double-quote -> &quot;
        assert_eq!(html_escape("say \"hi\""), "say &quot;hi&quot;");
        // Bare ampersand -> &amp; (escaped exactly once, not double-escaped).
        assert_eq!(html_escape("a & b"), "a &amp; b");
        // Single-quote -> &#x27;
        assert_eq!(html_escape("it's"), "it&#x27;s");
        // Ampersand is escaped FIRST: a literal `&lt;` stays a single entity,
        // it is NOT turned into `&amp;lt;`.
        assert_eq!(html_escape("&lt;"), "&amp;lt;");
        assert_eq!(html_escape("&"), "&amp;");
    }

    #[test]
    fn email_html_escapes_tag_in_title() {
        // A notification whose title carries markup (e.g. a teacher-controlled
        // assignment title via `grade_released`) must NOT appear raw in the HTML.
        let (subject, html) = build_email_subject_html("<b>x</b>", None, None);
        assert!(
            !html.contains("<b>x</b>"),
            "raw markup leaked into html: {html}"
        );
        assert!(
            html.contains("&lt;b&gt;x&lt;/b&gt;"),
            "title not escaped: {html}"
        );
        // Subject is escaped too.
        assert!(
            !subject.contains("<b>x</b>"),
            "raw markup leaked into subject: {subject}"
        );
        assert_eq!(subject, "&lt;b&gt;x&lt;/b&gt;");
    }

    #[test]
    fn email_html_escapes_double_quote_in_title() {
        let (subject, html) = build_email_subject_html("a \"quoted\" title", None, None);
        assert!(!html.contains('"') || html.contains("&quot;"));
        assert!(
            html.contains("&quot;quoted&quot;"),
            "quote not escaped in html: {html}"
        );
        assert!(
            subject.contains("&quot;quoted&quot;"),
            "quote not escaped in subject: {subject}"
        );
    }

    #[test]
    fn email_html_escapes_body_and_link_attribute() {
        // Body markup is escaped; a crafted link cannot break out of href="…".
        let (_subject, html) = build_email_subject_html(
            "Grade released",
            Some("<img src=x onerror=alert(1)>"),
            Some("https://app/x\"><script>alert(1)</script>"),
        );
        // Body markup neutralized.
        assert!(!html.contains("<img"), "raw body markup leaked: {html}");
        assert!(html.contains("&lt;img"), "body not escaped: {html}");
        // Link cannot break out of the attribute: the closing quote is escaped
        // and the injected <script> is neutralized.
        assert!(
            !html.contains("\"><script>"),
            "link broke out of href: {html}"
        );
        assert!(
            html.contains("&quot;&gt;&lt;script&gt;"),
            "link attr not escaped: {html}"
        );
        // The href attribute is still well-formed and points at the URL.
        assert!(
            html.contains("href=\"https://app/x"),
            "link no longer usable: {html}"
        );
    }
}
