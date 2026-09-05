// crates/backend/src/services/webhook_delivery.rs
//! Outbound webhook delivery.
//!
//! Two halves:
//!   * `emit_event` — called from request handlers (or other in-process code)
//!     to ENQUEUE a tenant-scoped event into `webhook_deliveries` for every
//!     active subscription matching the event. Cheap; does no network I/O.
//!   * `run_delivery_worker` — a background loop the main stream spawns. Each
//!     tick it claims a batch of due deliveries (CROSS-TENANT via the system
//!     context), signs each payload with the subscription's HMAC-SHA256 secret,
//!     POSTs it via `reqwest`, and records the result with retry/backoff.
//!
//! SIGNING: the `X-Aula-Signature` header is `t=<unix>,v1=<hex>` where the hex
//! is HMAC-SHA256 over the literal `"{t}.{raw_body}"` keyed by the subscription
//! secret — the same construction `services::billing::verify_webhook_signature`
//! checks for inbound Stripe webhooks, so integrators can reuse that logic. The
//! raw body we sign is the EXACT bytes we POST (serialized once, no re-encode).
//!
//! Uses only existing deps: reqwest (POST), hmac + sha2 (signing), serde_json.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use futures_util::{stream, StreamExt};
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use sqlx::PgPool;
use uuid::Uuid;

use crate::db;

type HmacSha256 = Hmac<Sha256>;

/// Max delivery attempts before a delivery is marked terminally `failed`. Mirror
/// of the backoff ladder length in `db::webhooks::claim_due_deliveries` (1m, 5m,
/// 15m, 1h, 6h) — 6 attempts total.
const MAX_ATTEMPTS: i32 = 6;

/// Per-request timeout for the outbound POST. Kept short so one slow/blackholed
/// endpoint can't stall the worker tick; a timeout is recorded as a failed
/// attempt and retried on the backoff schedule.
const POST_TIMEOUT: Duration = Duration::from_secs(10);

/// How many due deliveries one worker tick claims.
const BATCH: i64 = 20;

/// Bound outbound sockets and DB result writes while avoiding head-of-line
/// blocking when one subscriber consumes the full request timeout.
const MAX_IN_FLIGHT: usize = 5;

/// Parse and validate the non-DNS portion of a webhook target. HTTPS is
/// mandatory and literal/private hosts are rejected. DNS is revalidated and
/// pinned immediately before each request in `delivery_client`.
pub fn validate_target_url(raw: &str) -> Result<reqwest::Url, &'static str> {
    let url = reqwest::Url::parse(raw.trim()).map_err(|_| "url_invalid")?;
    if url.scheme() != "https" {
        return Err("url_https_required");
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("url_credentials_forbidden");
    }
    if url.fragment().is_some() {
        return Err("url_fragment_forbidden");
    }
    let host = url.host_str().ok_or("url_host_required")?;
    // `Url::host_str` retains brackets for IPv6 literals in this reqwest/url
    // version, so normalize them before parsing as an address.
    let ip_literal = host
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(host);
    if let Ok(ip) = ip_literal.parse::<IpAddr>() {
        if !is_public_webhook_ip(ip) {
            return Err("url_host_not_public");
        }
    } else {
        let domain = host.trim_end_matches('.').to_ascii_lowercase();
        if domain == "localhost"
            || domain.ends_with(".localhost")
            || domain.ends_with(".local")
            || domain.ends_with(".internal")
        {
            return Err("url_host_not_public");
        }
    }
    Ok(url)
}

fn is_public_webhook_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_public_v4(ip),
        IpAddr::V6(ip) => {
            if let Some(v4) = ipv4_mapped(ip) {
                return is_public_v4(v4);
            }
            let first = ip.segments()[0];
            !(ip.is_unspecified()
                || ip.is_loopback()
                || ip.is_multicast()
                || (first & 0xfe00) == 0xfc00 // fc00::/7 unique-local
                || (first & 0xffc0) == 0xfe80 // fe80::/10 link-local
                || (first & 0xffc0) == 0xfec0 // fec0::/10 deprecated site-local
                || (ip.segments()[0] == 0x2001 && ip.segments()[1] == 0x0db8))
        }
    }
}

fn ipv4_mapped(ip: Ipv6Addr) -> Option<Ipv4Addr> {
    let s = ip.segments();
    if s[0..5] == [0, 0, 0, 0, 0] && s[5] == 0xffff {
        Some(Ipv4Addr::new(
            (s[6] >> 8) as u8,
            s[6] as u8,
            (s[7] >> 8) as u8,
            s[7] as u8,
        ))
    } else {
        None
    }
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(a == 0
        || a == 10
        || a == 127
        || (a == 100 && (64..=127).contains(&b))
        || (a == 169 && b == 254)
        || (a == 172 && (16..=31).contains(&b))
        || (a == 192 && b == 0 && c == 0)
        || (a == 192 && b == 0 && c == 2)
        || (a == 192 && b == 168)
        || (a == 198 && (b == 18 || b == 19))
        || (a == 198 && b == 51 && c == 100)
        || (a == 203 && b == 0 && c == 113)
        || a >= 224)
}

/// Build a redirect-free HTTP client pinned to the public address that was
/// resolved and checked for `url`. Reusable by other tenant-configurable
/// outbound integrations (for example OIDC token/JWKS endpoints) so they share
/// the same DNS-rebinding and private-network protections as webhooks.
pub async fn public_https_client(
    url: &reqwest::Url,
    request_timeout: Duration,
) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(request_timeout);

    if let Some(domain) = url.domain() {
        let port = url.port_or_known_default().unwrap_or(443);
        let resolved = tokio::time::timeout(
            Duration::from_secs(3),
            tokio::net::lookup_host((domain, port)),
        )
        .await
        .map_err(|_| "webhook DNS lookup timed out".to_string())?
        .map_err(|err| format!("webhook DNS lookup failed: {err}"))?;
        let addresses: Vec<SocketAddr> = resolved.collect();
        if addresses.is_empty()
            || addresses
                .iter()
                .any(|addr| !is_public_webhook_ip(addr.ip()))
        {
            return Err("webhook target resolved to a non-public address".into());
        }
        // Pin the validated address for this request. This closes the gap
        // between validation and connect where DNS rebinding otherwise occurs.
        builder = builder.resolve(domain, addresses[0]);
    }

    builder
        .build()
        .map_err(|err| format!("webhook client build failed: {err}"))
}

/// Enqueue `event` (with `payload`) for every active subscription in `tenant_id`
/// subscribed to it. Best-effort by contract for the CALLER: most call sites
/// invoke this after their primary action has committed and must not fail on a
/// webhook-enqueue error, so they should log-and-ignore the `Err`. Returns the
/// number of deliveries enqueued.
pub async fn emit_event(
    pool: &PgPool,
    tenant_id: Uuid,
    event: &str,
    payload: serde_json::Value,
) -> sqlx::Result<u64> {
    db::webhooks::enqueue_for_event(pool, tenant_id, event, &payload).await
}

/// Compute the `X-Aula-Signature` header value for a raw body at unix time `t`.
/// Format: `t=<t>,v1=<hex HMAC-SHA256 of "{t}.{body}">`. Pure + unit-tested.
pub fn sign_payload(secret: &str, raw_body: &[u8], t: i64) -> String {
    // `new_from_slice` accepts any key length for HMAC, so this never errors in
    // practice; on the impossible error we fall back to an all-zero sig, which
    // simply fails verification at the receiver.
    let sig_hex = match HmacSha256::new_from_slice(secret.as_bytes()) {
        Ok(mut mac) => {
            mac.update(t.to_string().as_bytes());
            mac.update(b".");
            mac.update(raw_body);
            let bytes = mac.finalize().into_bytes();
            let mut out = String::with_capacity(bytes.len() * 2);
            for b in bytes {
                out.push_str(&format!("{b:02x}"));
            }
            out
        }
        Err(_) => String::new(),
    };
    format!("t={t},v1={sig_hex}")
}

/// One worker tick: claim due deliveries and attempt each. Returns how many were
/// attempted (0 when the queue is idle). Separated from the loop so it is easy
/// to drive once from a test/manual trigger.
pub async fn run_once(pool: &PgPool) -> anyhow::Result<usize> {
    let due = db::webhooks::claim_due_deliveries(pool, BATCH).await?;
    let n = due.len();
    stream::iter(due)
        .for_each_concurrent(MAX_IN_FLIGHT, |delivery| attempt_delivery(pool, delivery))
        .await;
    Ok(n)
}

/// Sign + POST a single claimed delivery and record its outcome. Never returns
/// an error — every failure path is folded into a recorded attempt (status →
/// `retrying`/`failed`) so the worker loop keeps draining the queue.
async fn attempt_delivery(pool: &PgPool, d: db::webhooks::PendingDelivery) {
    // Serialize the envelope ONCE; the exact bytes we POST are the bytes we sign.
    let envelope = serde_json::json!({
        "event": d.event,
        "delivery_id": d.id,
        "tenant_id": d.tenant_id,
        "data": d.payload_json,
    });
    let raw_body = match serde_json::to_vec(&envelope) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(?e, delivery_id = %d.id, "webhook payload serialize failed");
            // Treat as a (likely permanent) failure attempt.
            record(pool, d.id, false, None).await;
            return;
        }
    };

    let t = chrono::Utc::now().timestamp();
    let signature = sign_payload(&d.secret, &raw_body, t);

    let target = match validate_target_url(&d.url) {
        Ok(url) => url,
        Err(reason) => {
            tracing::warn!(delivery_id = %d.id, url = %d.url, reason, "webhook target rejected");
            record(pool, d.id, false, None).await;
            return;
        }
    };
    let http = match public_https_client(&target, POST_TIMEOUT).await {
        Ok(client) => client,
        Err(error) => {
            tracing::warn!(delivery_id = %d.id, url = %d.url, %error, "webhook target resolution rejected");
            record(pool, d.id, false, None).await;
            return;
        }
    };

    let resp = http
        .post(target)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header("X-Aula-Signature", signature)
        .header("X-Aula-Event", d.event.clone())
        .header("X-Aula-Delivery", d.id.to_string())
        .timeout(POST_TIMEOUT)
        .body(raw_body)
        .send()
        .await;

    match resp {
        Ok(r) => {
            let code = r.status().as_u16() as i32;
            // 2xx is success; anything else is a retryable failure.
            let success = r.status().is_success();
            if !success {
                tracing::warn!(delivery_id = %d.id, url = %d.url, code, "webhook non-2xx");
            }
            record(pool, d.id, success, Some(code)).await;
        }
        Err(e) => {
            tracing::warn!(?e, delivery_id = %d.id, url = %d.url, "webhook POST failed");
            record(pool, d.id, false, None).await;
        }
    }
}

/// Record an attempt result; log (never propagate) a DB error.
async fn record(pool: &PgPool, delivery_id: Uuid, success: bool, code: Option<i32>) {
    if let Err(e) = db::webhooks::mark_result(pool, delivery_id, success, code, MAX_ATTEMPTS).await
    {
        tracing::warn!(?e, %delivery_id, "webhook mark_result failed");
    }
}

/// Background delivery loop. The main stream spawns this once at startup. Ticks
/// every `interval`; on each tick it drains up to `BATCH` due deliveries with a
/// bounded number of requests in flight.
pub async fn run_delivery_worker(pool: PgPool, interval: Duration) {
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        ticker.tick().await;
        match run_once(&pool).await {
            Ok(n) if n > 0 => tracing::debug!(attempted = n, "webhook delivery tick"),
            Ok(_) => {}
            Err(e) => tracing::warn!(?e, "webhook delivery tick failed"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_payload_is_deterministic_and_verifiable() {
        let secret = "whsec_test_secret";
        let body = br#"{"event":"course.created"}"#;
        let t = 1_700_000_000_i64;
        let header = sign_payload(secret, body, t);
        assert!(header.starts_with(&format!("t={t},v1=")));

        // The signature must verify under the SAME construction the inbound
        // billing verifier uses, so integrators can reuse it.
        let sig_part = header
            .split(',')
            .find_map(|p| p.strip_prefix("v1="))
            .unwrap();
        assert_eq!(sig_part.len(), 64, "hex sha256");

        // Recompute independently.
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(t.to_string().as_bytes());
        mac.update(b".");
        mac.update(body);
        let expected = mac.finalize().into_bytes();
        let expected_hex: String = expected.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(sig_part, expected_hex);
    }

    #[test]
    fn target_validation_rejects_ssrf_and_insecure_urls() {
        assert!(validate_target_url("https://hooks.example.com/aula").is_ok());
        for url in [
            "http://hooks.example.com/aula",
            "https://localhost/hook",
            "https://127.0.0.1/hook",
            "https://10.0.0.1/hook",
            "https://169.254.169.254/latest/meta-data",
            "https://[::1]/hook",
            "https://user:pass@example.com/hook",
            "https://example.com/hook#fragment",
        ] {
            assert!(validate_target_url(url).is_err(), "accepted {url}");
        }
    }

    const _: () = {
        assert!(MAX_IN_FLIGHT > 1);
        assert!(MAX_IN_FLIGHT < BATCH as usize);
    };
}
