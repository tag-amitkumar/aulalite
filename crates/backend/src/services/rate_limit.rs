// crates/backend/src/services/rate_limit.rs
//! Global, opt-in API rate limiting.
//!
//! A per-client sliding-window limiter backed by Redis (fred), wired as an
//! axum middleware via `axum::middleware::from_fn_with_state`.
//!
//! Design
//! ------
//! * **Keying** — when the request carries an authenticated principal we key on
//!   that (so a user gets a single budget across devices/IPs); otherwise we key
//!   on the client IP. Because the limiter runs as the OUTERMOST layer (before
//!   `require_auth` has populated `RequestContext`), we cannot read the verified
//!   user id here. Instead we derive a stable, opaque principal from the bearer
//!   token / `?access_token=` WS param by hashing it (SHA-256, truncated). The
//!   hash is never logged and never leaves the process meaningfully — two
//!   requests bearing the same credential share a budget, which is exactly the
//!   per-user semantics we want without paying for JWKS verification on the hot
//!   path. Requests with no credential fall back to the client IP.
//!
//! * **Window** — a true sliding window implemented with a Redis sorted set per
//!   key. A small Lua script trims, counts, conditionally inserts, and refreshes
//!   expiry atomically in one Redis round trip. This avoids both the race and
//!   the 3–5 network calls of a client-side command sequence.
//!
//! * **Ceilings** — limits selected by key class:
//!     - authenticated principal: `DEFAULT_AUTH_LIMIT` (~300) / 60s
//!     - anonymous (IP):          `DEFAULT_ANON_LIMIT` (~60)  / 60s
//!     - credential edge (IP):    `DEFAULT_EDGE_LIMIT` (~600) / 60s
//!   Credential-bearing requests must pass both their opaque-token bucket and
//!   the higher edge-IP bucket. This prevents an attacker from rotating fake
//!   bearer strings to manufacture unlimited "authenticated" buckets without
//!   penalising a normal school network as aggressively as the anonymous cap.
//!
//! * **Whitelist** — `/healthz` and the MediaMTX auth callback path are never
//!   limited (health probes and the media server's auth hook must not be
//!   throttled).
//!
//! * **Opt-in** — the layer is only installed when `AULALITE_RATE_LIMIT=on`
//!   (see `RateLimitConfig::from_env`), so dev and the test suite are
//!   unaffected unless the flag is explicitly set.
//!
//! On exceed we return `429 Too Many Requests` with a `Retry-After` header
//! (seconds until the oldest in-window request ages out). On any Redis error we
//! **fail open** — a limiter outage must not take the API down.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Request, State};
use axum::http::header::{AUTHORIZATION, CONNECTION, UPGRADE};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use fred::prelude::{Builder, Client, ClientLike, LuaInterface};
use fred::types::config::Config;
use percent_encoding::percent_decode_str;
use sha2::{Digest, Sha256};

// ── tunables ─────────────────────────────────────────────────────────────────

/// Sliding-window span. Both ceilings are expressed per this window.
pub const DEFAULT_WINDOW: Duration = Duration::from_secs(60);
/// Requests allowed per window for an authenticated principal.
pub const DEFAULT_AUTH_LIMIT: u32 = 300;
/// Requests allowed per window for an anonymous client (keyed by IP).
pub const DEFAULT_ANON_LIMIT: u32 = 60;
/// Aggregate credential-bearing requests allowed per client IP and window.
pub const DEFAULT_EDGE_LIMIT: u32 = 600;
/// MFA verification attempts per credential+IP and window.
pub const DEFAULT_MFA_LIMIT: u32 = 10;

/// Atomic sliding-window decision. Returns
/// `[inserted, count_before_insert, oldest_score_or_minus_one]`.
///
/// Keep all keys in `KEYS` (rather than ARGV) so the script remains compatible
/// with a future Redis Cluster deployment.
const RATE_LIMIT_SCRIPT: &str = r#"
local key = KEYS[1]
local cutoff = tonumber(ARGV[1])
local now = tonumber(ARGV[2])
local member = ARGV[3]
local ttl = tonumber(ARGV[4])
local limit = tonumber(ARGV[5])

redis.call('ZREMRANGEBYSCORE', key, '-inf', cutoff)
local count = redis.call('ZCARD', key)
if count < limit then
    redis.call('ZADD', key, now, member)
    redis.call('EXPIRE', key, ttl)
    return {1, count, -1}
end

local oldest = redis.call('ZRANGE', key, 0, 0, 'WITHSCORES')
local oldest_score = -1
if oldest[2] then
    oldest_score = tonumber(oldest[2])
end
return {0, count, oldest_score}
"#;

/// Paths that are never rate limited.
///
/// `/healthz` — liveness/readiness probes.
/// `/v1/mediamtx/auth/publish` — the MediaMTX → backend auth callback fires on
/// every publish/read and must not be throttled or live streaming breaks.
fn is_whitelisted(path: &str) -> bool {
    matches!(path, "/healthz" | "/readyz") || path == "/v1/mediamtx/auth/publish"
}

// ── config / state ─────────────────────────────────────────────────────────────

/// Limiter configuration resolved from the environment.
#[derive(Clone, Debug)]
pub struct RateLimitConfig {
    pub window: Duration,
    pub auth_limit: u32,
    pub anon_limit: u32,
    pub edge_limit: u32,
    pub mfa_limit: u32,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            window: DEFAULT_WINDOW,
            auth_limit: DEFAULT_AUTH_LIMIT,
            anon_limit: DEFAULT_ANON_LIMIT,
            edge_limit: DEFAULT_EDGE_LIMIT,
            mfa_limit: DEFAULT_MFA_LIMIT,
        }
    }
}

impl RateLimitConfig {
    /// `Some(cfg)` iff `AULALITE_RATE_LIMIT` is truthy (`1|true|yes|on`).
    /// Optional overrides: `AULALITE_RATE_LIMIT_WINDOW_SECS`,
    /// `AULALITE_RATE_LIMIT_AUTH`, `AULALITE_RATE_LIMIT_ANON`, and
    /// `AULALITE_RATE_LIMIT_EDGE`.
    pub fn from_env() -> Option<Self> {
        let enabled = std::env::var("AULALITE_RATE_LIMIT")
            .map(|v| {
                matches!(
                    v.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            })
            .unwrap_or(false);
        if !enabled {
            return None;
        }
        let mut cfg = RateLimitConfig::default();
        if let Some(secs) = env_u64("AULALITE_RATE_LIMIT_WINDOW_SECS").filter(|s| *s > 0) {
            cfg.window = Duration::from_secs(secs);
        }
        if let Some(n) = env_u64("AULALITE_RATE_LIMIT_AUTH").filter(|n| *n > 0) {
            cfg.auth_limit = n as u32;
        }
        if let Some(n) = env_u64("AULALITE_RATE_LIMIT_ANON").filter(|n| *n > 0) {
            cfg.anon_limit = n as u32;
        }
        if let Some(n) = env_u64("AULALITE_RATE_LIMIT_EDGE").filter(|n| *n > 0) {
            cfg.edge_limit = n as u32;
        }
        if let Some(n) = env_u64("AULALITE_RATE_LIMIT_MFA").filter(|n| *n > 0) {
            cfg.mfa_limit = n as u32;
        }
        Some(cfg)
    }
}

fn env_u64(name: &str) -> Option<u64> {
    std::env::var(name).ok().and_then(|s| s.trim().parse().ok())
}

/// Shared limiter state injected into the middleware. Cloneable — the inner
/// `fred::clients::Client` is an Arc-wrapped multiplexed handle (same idiom as
/// `RedisLiveRoomBroker`).
#[derive(Clone)]
pub struct RateLimitState {
    client: Arc<Client>,
    config: RateLimitConfig,
    script_hash: Arc<String>,
}

impl RateLimitState {
    /// Connect to Redis at `url` (e.g. `redis://127.0.0.1:6379`) and build the
    /// limiter state. Mirrors `RedisLiveRoomBroker::connect`.
    pub async fn connect(url: impl Into<String>, config: RateLimitConfig) -> anyhow::Result<Self> {
        let url = url.into();
        let cfg =
            Config::from_url(&url).map_err(|e| anyhow::anyhow!("rate_limit redis url: {e}"))?;
        let client = Builder::from_config(cfg)
            .build()
            .map_err(|e| anyhow::anyhow!("rate_limit redis build: {e}"))?;
        client
            .init()
            .await
            .map_err(|e| anyhow::anyhow!("rate_limit redis connect: {e}"))?;
        let script_hash: String = client
            .script_load(RATE_LIMIT_SCRIPT)
            .await
            .map_err(|e| anyhow::anyhow!("rate_limit redis script load: {e}"))?;
        Ok(Self {
            client: Arc::new(client),
            config,
            script_hash: Arc::new(script_hash),
        })
    }
}

// ── decision logic (pure; unit-tested) ───────────────────────────────────────

/// What class of client a request maps to, which selects the ceiling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientKey {
    /// Authenticated: opaque hash of the presented credential.
    Auth(String),
    /// Anonymous: client IP (or the literal `"unknown"` when undeterminable).
    Anon(String),
    /// Aggregate safety ceiling for all credential-bearing traffic from an IP.
    Edge(String),
    /// MFA challenge attempts keyed by both credential and normalized IP.
    Mfa(String),
}

impl ClientKey {
    /// The per-window ceiling that applies to this key class.
    fn limit(&self, cfg: &RateLimitConfig) -> u32 {
        match self {
            ClientKey::Auth(_) => cfg.auth_limit,
            ClientKey::Anon(_) => cfg.anon_limit,
            ClientKey::Edge(_) => cfg.edge_limit,
            ClientKey::Mfa(_) => cfg.mfa_limit,
        }
    }

    /// The Redis key for this client's sliding-window sorted set.
    fn redis_key(&self) -> String {
        match self {
            ClientKey::Auth(h) => format!("aulalite:rl:u:{h}"),
            ClientKey::Anon(ip) => format!("aulalite:rl:ip:{ip}"),
            ClientKey::Edge(ip) => format!("aulalite:rl:edge:{ip}"),
            ClientKey::Mfa(hash) => format!("aulalite:rl:mfa:{hash}"),
        }
    }
}

/// Outcome of a window check — pure, so it can be unit-tested without Redis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowDecision {
    /// Whether this request is allowed (i.e. count-after-insert <= limit).
    pub allowed: bool,
    /// Seconds the client should wait before retrying (only meaningful when
    /// `!allowed`). Always >= 1 so a `Retry-After: 0` is never emitted.
    pub retry_after_secs: u64,
}

/// Core sliding-window threshold logic, factored out of any I/O.
///
/// `count_in_window` is the number of requests already recorded in the window
/// that are still fresh (NOT counting the current request). The current request
/// is admitted iff including it keeps the total at or under `limit`.
///
/// When rejected, `retry_after` is the time until the oldest in-window entry
/// ages out: `window - (now - oldest)`, clamped to `[1, window]`.
///
/// All times are milliseconds. `oldest_ts_ms` is `None` when the window is
/// empty (in which case a rejection cannot occur, but we still return a safe
/// full-window backoff defensively).
pub fn evaluate_window(
    count_in_window: u32,
    limit: u32,
    now_ms: i64,
    oldest_ts_ms: Option<i64>,
    window: Duration,
) -> WindowDecision {
    let window_ms = window.as_millis() as i64;
    // Admitting this request makes the total `count_in_window + 1`.
    let allowed = count_in_window < limit;
    if allowed {
        return WindowDecision {
            allowed: true,
            retry_after_secs: 0,
        };
    }
    // Rejected: compute when the oldest entry leaves the window.
    let remaining_ms = match oldest_ts_ms {
        Some(oldest) => {
            let age = now_ms.saturating_sub(oldest);
            (window_ms - age).clamp(0, window_ms)
        }
        None => window_ms,
    };
    // Ceil to whole seconds, floor at 1 so callers never see Retry-After: 0.
    let retry_after_secs = ((remaining_ms + 999) / 1000).max(1) as u64;
    WindowDecision {
        allowed: false,
        retry_after_secs,
    }
}

// ── request → client key (pure; unit-tested) ─────────────────────────────────

/// Hash a presented credential into a short, stable, opaque key fragment.
/// SHA-256 hex, truncated to 32 chars (128 bits) — collision-resistant enough
/// for bucketing and small enough for a Redis key.
fn hash_credential(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    let digest = hasher.finalize();
    let mut out = String::with_capacity(32);
    for b in digest.iter().take(16) {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// Extract the bearer token (Authorization header) or, for WS upgrades, the
/// `?access_token=` query param — mirrors `auth::middleware::resolve_token` so
/// the limiter keys the same principal the auth layer will.
fn resolve_credential(headers: &HeaderMap, query: Option<&str>) -> Option<String> {
    if let Some(token) = headers
        .get(AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
    {
        return Some(token.to_string());
    }
    if is_websocket_upgrade(headers) {
        if let Some(q) = query {
            return query_access_token(q);
        }
    }
    None
}

fn is_websocket_upgrade(headers: &HeaderMap) -> bool {
    let upgrade_ws = headers
        .get(UPGRADE)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.eq_ignore_ascii_case("websocket"))
        .unwrap_or(false);
    let connection_upgrade = headers
        .get(CONNECTION)
        .and_then(|v| v.to_str().ok())
        .map(|v| {
            v.split(',')
                .any(|p| p.trim().eq_ignore_ascii_case("upgrade"))
        })
        .unwrap_or(false);
    upgrade_ws && connection_upgrade
}

fn query_access_token(query: &str) -> Option<String> {
    query.split('&').find_map(|part| {
        let (key, value) = part.split_once('=')?;
        if key != "access_token" || value.is_empty() {
            return None;
        }
        let decoded = percent_decode_str(value).decode_utf8().ok()?.into_owned();
        if decoded.is_empty() {
            None
        } else {
            Some(decoded)
        }
    })
}

/// Best-effort client IP from proxy forwarding headers. The API runs behind a
/// TLS-terminating reverse proxy (see lib.rs HSTS note), so `X-Forwarded-For`
/// (first hop) / `X-Real-IP` carry the real client. Returns `"unknown"` when
/// neither is present — all such requests then share one anonymous bucket,
/// which is the conservative (safer) behaviour.
fn client_ip(headers: &HeaderMap) -> String {
    if let Some(xff) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
        if let Some(first) = xff.split(',').next() {
            let ip = first.trim();
            if !ip.is_empty() {
                return ip.to_string();
            }
        }
    }
    if let Some(real) = headers.get("x-real-ip").and_then(|v| v.to_str().ok()) {
        let ip = real.trim();
        if !ip.is_empty() {
            return ip.to_string();
        }
    }
    "unknown".to_string()
}

/// Classify a request into its limiter key (pure over headers + query).
pub fn classify(headers: &HeaderMap, query: Option<&str>) -> ClientKey {
    match resolve_credential(headers, query) {
        Some(token) => ClientKey::Auth(hash_credential(&token)),
        None => ClientKey::Anon(client_ip(headers)),
    }
}

// ── Redis sliding-window check ───────────────────────────────────────────────

/// Record + evaluate the current request against the client's sliding window.
/// Returns the decision. Any Redis failure yields a fail-open `allowed` result.
async fn check_and_record(state: &RateLimitState, key: &ClientKey) -> WindowDecision {
    let cfg = &state.config;
    let limit = key.limit(cfg);
    let redis_key = key.redis_key();
    let window_ms = cfg.window.as_millis() as i64;
    let now_ms = chrono::Utc::now().timestamp_millis();
    let cutoff = now_ms - window_ms;

    let member = format!("{now_ms}-{}", uuid::Uuid::new_v4().simple());
    let args = vec![
        cutoff.to_string(),
        now_ms.to_string(),
        member,
        cfg.window.as_secs().max(1).to_string(),
        limit.to_string(),
    ];
    let keys = vec![redis_key];

    // Prefer EVALSHA so every API request sends only the digest. Redis flushes
    // its script cache on restart, so transparently reload and retry once on
    // NOSCRIPT while preserving the limiter's fail-open contract.
    let mut result: Result<(i64, i64, i64), _> = state
        .client
        .evalsha(state.script_hash.as_str(), keys.clone(), args.clone())
        .await;
    if result
        .as_ref()
        .is_err_and(|e| e.details().starts_with("NOSCRIPT"))
    {
        let reload: Result<String, _> = state.client.script_load(RATE_LIMIT_SCRIPT).await;
        result = match reload {
            Ok(_) => {
                state
                    .client
                    .evalsha(state.script_hash.as_str(), keys, args)
                    .await
            }
            Err(error) => Err(error),
        };
    }

    let (inserted, count, oldest_score) = match result {
        Ok(result) => result,
        Err(error) => {
            tracing::warn!(
                ?error,
                "rate_limit: atomic window check failed; failing open"
            );
            return WindowDecision {
                allowed: true,
                retry_after_secs: 0,
            };
        }
    };

    if inserted == 1 {
        WindowDecision {
            allowed: true,
            retry_after_secs: 0,
        }
    } else {
        evaluate_window(
            count.max(0) as u32,
            limit,
            now_ms,
            (oldest_score >= 0).then_some(oldest_score),
            cfg.window,
        )
    }
}

// ── axum middleware ──────────────────────────────────────────────────────────

/// Per-client sliding-window rate-limit middleware.
///
/// Wire with `axum::middleware::from_fn_with_state(rate_limit_state, rate_limit)`
/// as the OUTERMOST `.layer(...)` on the router (so it guards every route,
/// including the unauthenticated ones). Whitelisted paths short-circuit.
pub async fn rate_limit(State(state): State<RateLimitState>, req: Request, next: Next) -> Response {
    let path = req.uri().path();
    if is_whitelisted(path) {
        return next.run(req).await;
    }

    let key = if path == "/v1/auth/mfa/challenge" {
        match resolve_credential(req.headers(), req.uri().query()) {
            Some(token) => ClientKey::Mfa(hash_credential(&format!(
                "{token}:{}",
                client_ip(req.headers())
            ))),
            None => classify(req.headers(), req.uri().query()),
        }
    } else {
        classify(req.headers(), req.uri().query())
    };
    let decision = match &key {
        ClientKey::Auth(_) | ClientKey::Mfa(_) => {
            // The raw token is intentionally only an opaque fairness key; it
            // has not been verified yet. Pair it with an aggregate IP ceiling
            // so rotating made-up tokens cannot create unbounded allowance.
            let edge_key = ClientKey::Edge(client_ip(req.headers()));
            let (principal, edge) = tokio::join!(
                check_and_record(&state, &key),
                check_and_record(&state, &edge_key)
            );
            WindowDecision {
                allowed: principal.allowed && edge.allowed,
                retry_after_secs: principal.retry_after_secs.max(edge.retry_after_secs),
            }
        }
        ClientKey::Anon(_) => check_and_record(&state, &key).await,
        ClientKey::Edge(_) => unreachable!("edge keys are secondary-only"),
    };

    if decision.allowed {
        return next.run(req).await;
    }

    too_many_requests(decision.retry_after_secs)
}

/// Build the `429 Too Many Requests` response with a `Retry-After` header,
/// matching the JSON error envelope used by `ApiError` (`{"error": ...}`).
fn too_many_requests(retry_after_secs: u64) -> Response {
    let body = axum::Json(serde_json::json!({ "error": "rate limited" }));
    let mut resp = (StatusCode::TOO_MANY_REQUESTS, body).into_response();
    if let Ok(val) = HeaderValue::from_str(&retry_after_secs.to_string()) {
        resp.headers_mut()
            .insert(axum::http::header::RETRY_AFTER, val);
    }
    resp
}

// ── tests (pure; no Redis) ────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    const WINDOW: Duration = Duration::from_secs(60);

    // ── evaluate_window ──────────────────────────────────────────────────────

    #[test]
    fn under_limit_is_allowed() {
        // 0 already in window, limit 60 → admitting makes 1, allowed.
        let d = evaluate_window(0, 60, 1_000, None, WINDOW);
        assert!(d.allowed);
        assert_eq!(d.retry_after_secs, 0);
    }

    #[test]
    fn exactly_at_limit_admits_the_last_one() {
        // 59 in window, limit 60 → admitting makes 60 (== limit), allowed.
        let d = evaluate_window(59, 60, 1_000, None, WINDOW);
        assert!(d.allowed);
    }

    #[test]
    fn over_limit_is_rejected() {
        // 60 already in window, limit 60 → admitting would make 61, rejected.
        let now = 100_000;
        let oldest = now - 10_000; // oldest entry 10s into the 60s window
        let d = evaluate_window(60, 60, now, Some(oldest), WINDOW);
        assert!(!d.allowed);
        // 60s window - 10s elapsed = 50s remaining.
        assert_eq!(d.retry_after_secs, 50);
    }

    #[test]
    fn retry_after_is_never_zero_when_rejected() {
        let now = 100_000;
        // Oldest is exactly at the window edge → 0ms remaining, but we floor to 1.
        let oldest = now - WINDOW.as_millis() as i64;
        let d = evaluate_window(60, 60, now, Some(oldest), WINDOW);
        assert!(!d.allowed);
        assert_eq!(d.retry_after_secs, 1);
    }

    #[test]
    fn retry_after_rounds_up_to_whole_seconds() {
        let now = 100_000;
        let oldest = now - 100; // 100ms elapsed → 59_900ms remaining → 60s (ceil).
        let d = evaluate_window(60, 60, now, Some(oldest), WINDOW);
        assert!(!d.allowed);
        assert_eq!(d.retry_after_secs, 60);
    }

    #[test]
    fn rejected_with_empty_window_defaults_to_full_window() {
        // Pathological: count says over-limit but no oldest entry recorded.
        let d = evaluate_window(60, 60, 100_000, None, WINDOW);
        assert!(!d.allowed);
        assert_eq!(d.retry_after_secs, 60);
    }

    #[test]
    fn limit_of_one_admits_first_rejects_second() {
        let first = evaluate_window(0, 1, 1_000, None, WINDOW);
        assert!(first.allowed);
        let second = evaluate_window(1, 1, 1_000, Some(900), WINDOW);
        assert!(!second.allowed);
    }

    #[test]
    fn retry_after_clamps_when_oldest_is_in_the_future() {
        // Clock skew: oldest timestamp ahead of now → age negative → remaining
        // clamps to the full window, never overflows past it.
        let now = 100_000;
        let oldest = now + 5_000;
        let d = evaluate_window(60, 60, now, Some(oldest), WINDOW);
        assert!(!d.allowed);
        assert_eq!(d.retry_after_secs, 60);
    }

    // ── key class selects the ceiling ────────────────────────────────────────

    #[test]
    fn auth_and_anon_keys_pick_their_limits() {
        let cfg = RateLimitConfig::default();
        assert_eq!(
            ClientKey::Auth("abc".into()).limit(&cfg),
            DEFAULT_AUTH_LIMIT
        );
        assert_eq!(
            ClientKey::Anon("1.2.3.4".into()).limit(&cfg),
            DEFAULT_ANON_LIMIT
        );
        assert_eq!(
            ClientKey::Edge("1.2.3.4".into()).limit(&cfg),
            DEFAULT_EDGE_LIMIT
        );
        assert_eq!(ClientKey::Mfa("abc".into()).limit(&cfg), DEFAULT_MFA_LIMIT);
        assert!(cfg.auth_limit > cfg.anon_limit);
        assert!(cfg.edge_limit > cfg.auth_limit);
    }

    #[test]
    fn redis_keys_are_namespaced_by_class() {
        assert_eq!(
            ClientKey::Auth("deadbeef".into()).redis_key(),
            "aulalite:rl:u:deadbeef"
        );
        assert_eq!(
            ClientKey::Anon("1.2.3.4".into()).redis_key(),
            "aulalite:rl:ip:1.2.3.4"
        );
        assert_eq!(
            ClientKey::Edge("1.2.3.4".into()).redis_key(),
            "aulalite:rl:edge:1.2.3.4"
        );
        assert_eq!(
            ClientKey::Mfa("deadbeef".into()).redis_key(),
            "aulalite:rl:mfa:deadbeef"
        );
    }

    // ── whitelist ─────────────────────────────────────────────────────────────

    #[test]
    fn whitelist_covers_healthz_and_mediamtx_auth() {
        assert!(is_whitelisted("/healthz"));
        assert!(is_whitelisted("/readyz"));
        assert!(is_whitelisted("/v1/mediamtx/auth/publish"));
        assert!(!is_whitelisted("/v1/me"));
        assert!(!is_whitelisted("/healthz/extra"));
        assert!(!is_whitelisted("/v1/mediamtx/jwks"));
    }

    // ── credential hashing ────────────────────────────────────────────────────

    #[test]
    fn hash_is_stable_and_distinct() {
        let a = hash_credential("token-A");
        let a2 = hash_credential("token-A");
        let b = hash_credential("token-B");
        assert_eq!(a, a2, "same token hashes identically");
        assert_ne!(a, b, "different tokens hash differently");
        assert_eq!(a.len(), 32, "128-bit hex fragment");
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }

    // ── classify ──────────────────────────────────────────────────────────────

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(
                axum::http::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                HeaderValue::from_str(v).unwrap(),
            );
        }
        h
    }

    #[test]
    fn bearer_token_classifies_as_auth() {
        let h = headers(&[("authorization", "Bearer my-secret-token")]);
        match classify(&h, None) {
            ClientKey::Auth(hash) => assert_eq!(hash, hash_credential("my-secret-token")),
            other => panic!("expected Auth, got {other:?}"),
        }
    }

    #[test]
    fn no_credential_classifies_as_anon_by_xff() {
        let h = headers(&[("x-forwarded-for", "203.0.113.7, 10.0.0.1")]);
        assert_eq!(classify(&h, None), ClientKey::Anon("203.0.113.7".into()));
    }

    #[test]
    fn x_real_ip_used_when_no_xff() {
        let h = headers(&[("x-real-ip", "198.51.100.42")]);
        assert_eq!(classify(&h, None), ClientKey::Anon("198.51.100.42".into()));
    }

    #[test]
    fn missing_ip_falls_back_to_unknown_bucket() {
        let h = headers(&[]);
        assert_eq!(classify(&h, None), ClientKey::Anon("unknown".into()));
    }

    #[test]
    fn ws_upgrade_uses_access_token_query() {
        let h = headers(&[("upgrade", "websocket"), ("connection", "Upgrade")]);
        match classify(&h, Some("foo=bar&access_token=ws-tok&x=1")) {
            ClientKey::Auth(hash) => assert_eq!(hash, hash_credential("ws-tok")),
            other => panic!("expected Auth from WS query, got {other:?}"),
        }
    }

    #[test]
    fn access_token_query_ignored_without_ws_upgrade() {
        // Without the Upgrade headers we must NOT trust the query param.
        let h = headers(&[("x-real-ip", "192.0.2.5")]);
        assert_eq!(
            classify(&h, Some("access_token=should-be-ignored")),
            ClientKey::Anon("192.0.2.5".into())
        );
    }

    #[test]
    fn bearer_takes_precedence_over_ip() {
        let h = headers(&[
            ("authorization", "Bearer tok"),
            ("x-forwarded-for", "203.0.113.9"),
        ]);
        assert_eq!(classify(&h, None), ClientKey::Auth(hash_credential("tok")));
    }

    // ── config / env flag ─────────────────────────────────────────────────────

    #[test]
    fn config_default_matches_spec_ceilings() {
        let cfg = RateLimitConfig::default();
        assert_eq!(cfg.auth_limit, 300);
        assert_eq!(cfg.anon_limit, 60);
        assert_eq!(cfg.edge_limit, 600);
        assert_eq!(cfg.mfa_limit, 10);
        assert_eq!(cfg.window, Duration::from_secs(60));
    }
}
