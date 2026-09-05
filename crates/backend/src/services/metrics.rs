// crates/backend/src/services/metrics.rs
//! Prometheus metrics: a small, dependency-free, in-process registry plus an
//! axum middleware that times every request and a `GET /metrics` handler that
//! renders the Prometheus text exposition format (v0.0.4).
//!
//! Why hand-rolled (no `metrics` / `metrics-exporter-prometheus`)?
//! --------------------------------------------------------------
//! Adding those two crates pulls in sizeable dependency trees, and this repo's
//! Docker build is OOM-sensitive (see the concurrent-build fix in the recent
//! history). The surface we need is tiny — a labelled request counter, a
//! request-duration histogram, and two process/uptime gauges — so an atomic
//! registry is both simpler and avoids a global recorder install that could
//! race other init. It is fully reversible: delete this file + its wiring.
//!
//! Cardinality
//! -----------
//! The per-route label is the axum **route template** (e.g. `/v1/courses/:slug`)
//! taken from `MatchedPath`, never the concrete path — so `/v1/courses/algebra`
//! and `/v1/courses/biology` collapse to one series. Requests that match no
//! route (404s, pre-routing rejects) are bucketed under the literal
//! `<unmatched>` so a path-scanning client cannot explode the series count.
//! The status is reduced to its **class** (`2xx`, `3xx`, `4xx`, `5xx`, `1xx`),
//! again to bound cardinality. Method is the (small, fixed) set of HTTP verbs.
//!
//! Concurrency
//! -----------
//! The registry is a process-global `OnceLock<MetricsRegistry>`. Counters and
//! histogram buckets are plain `AtomicU64`s incremented with `Relaxed` ordering
//! (metrics need eventual visibility, not happens-before). The label → series
//! map sits behind a `RwLock` taken only briefly to find-or-insert a series;
//! the hot path (incrementing an existing series) takes a read lock.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, RwLock};
use std::time::Instant;

use axum::extract::{MatchedPath, Request};
use axum::http::header::CONTENT_TYPE;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

/// Prometheus text exposition format content type (version 0.0.4).
const PROM_CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

/// Histogram bucket upper bounds, in **seconds**. These are the conventional
/// Prometheus latency buckets; `+Inf` is implied and emitted separately. Tuned
/// for a JSON API: sub-millisecond up to ~10s tail.
const BUCKET_BOUNDS_SECS: &[f64] = &[
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
];

// ── series key ───────────────────────────────────────────────────────────────

/// The label tuple that identifies one request series. Kept small and owned so
/// it can key a `HashMap`. `status_class` is e.g. `"2xx"`, `method` is the HTTP
/// verb, `path` is the route template (or `<unmatched>`).
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct SeriesKey {
    method: String,
    path: String,
    status_class: &'static str,
}

/// Per-series accumulators: a request counter, a running duration sum (seconds)
/// and the per-bucket cumulative counts (NOT yet cumulative — made cumulative
/// at render time). Stored behind an `Arc` so the hot path can clone the handle
/// out of the map under a short read lock and then increment lock-free.
#[derive(Debug)]
struct Series {
    count: AtomicU64,
    /// Sum of observed durations in seconds, stored as micros to stay integral
    /// and avoid float atomics (Prometheus `_sum` is rendered back to seconds).
    sum_micros: AtomicU64,
    /// One counter per finite bucket in `BUCKET_BOUNDS_SECS` (NON-cumulative
    /// here: each counts observations whose value fell in `(prev, bound]`). The
    /// `+Inf` bucket is `count` minus the sum of these.
    buckets: Vec<AtomicU64>,
}

impl Series {
    fn new() -> Self {
        Self {
            count: AtomicU64::new(0),
            sum_micros: AtomicU64::new(0),
            buckets: (0..BUCKET_BOUNDS_SECS.len())
                .map(|_| AtomicU64::new(0))
                .collect(),
        }
    }

    /// Record one observation of `secs` seconds.
    fn observe(&self, secs: f64) {
        self.count.fetch_add(1, Ordering::Relaxed);
        let micros = (secs * 1_000_000.0).round().max(0.0) as u64;
        self.sum_micros.fetch_add(micros, Ordering::Relaxed);
        // Smallest finite bucket whose upper bound >= secs gets the observation.
        for (i, bound) in BUCKET_BOUNDS_SECS.iter().enumerate() {
            if secs <= *bound {
                self.buckets[i].fetch_add(1, Ordering::Relaxed);
                return;
            }
        }
        // Falls past the largest finite bound → only the implicit +Inf bucket,
        // which is derived from `count` at render time, so nothing to add here.
    }
}

// ── registry ─────────────────────────────────────────────────────────────────

/// Process-global metrics registry. Construct once via [`registry`].
pub struct MetricsRegistry {
    series: RwLock<HashMap<SeriesKey, Arc<Series>>>,
    /// Process start, for the uptime gauge.
    started: Instant,
    /// Total requests seen (cheap top-level gauge/counter, also the in-flight
    /// denominator if ever needed). Incremented once per request.
    requests_total: AtomicU64,
}

impl MetricsRegistry {
    fn new() -> Self {
        Self {
            series: RwLock::new(HashMap::new()),
            started: Instant::now(),
            requests_total: AtomicU64::new(0),
        }
    }

    /// Record a finished request. `method` and `path` are borrowed; the series
    /// is created on first sight of a given (method, path, status-class) tuple.
    pub fn record(&self, method: &str, path: &str, status: u16, secs: f64) {
        self.requests_total.fetch_add(1, Ordering::Relaxed);
        let key = SeriesKey {
            method: method.to_string(),
            path: path.to_string(),
            status_class: status_class(status),
        };

        // Fast path: series already exists — take a read lock, clone the Arc.
        if let Some(series) = self
            .series
            .read()
            .expect("metrics series lock poisoned")
            .get(&key)
            .cloned()
        {
            series.observe(secs);
            return;
        }

        // Slow path: insert under a write lock (double-checked so a racing
        // writer that won the lock first is reused rather than overwritten).
        let series = {
            let mut map = self.series.write().expect("metrics series lock poisoned");
            map.entry(key)
                .or_insert_with(|| Arc::new(Series::new()))
                .clone()
        };
        series.observe(secs);
    }

    /// Uptime in whole seconds since process start.
    fn uptime_secs(&self) -> u64 {
        self.started.elapsed().as_secs()
    }

    /// Render the full registry as Prometheus text exposition format.
    pub fn render(&self) -> String {
        let mut out = String::with_capacity(4096);

        // ── process / uptime gauges ────────────────────────────────────────
        out.push_str(
            "# HELP aulalite_process_uptime_seconds Seconds since the backend process started.\n",
        );
        out.push_str("# TYPE aulalite_process_uptime_seconds gauge\n");
        out.push_str(&format!(
            "aulalite_process_uptime_seconds {}\n",
            self.uptime_secs()
        ));

        out.push_str("# HELP aulalite_process_start_time_seconds Approximate start time, exposed as a monotonically-derived gauge (uptime-relative).\n");
        out.push_str("# TYPE aulalite_process_start_time_seconds gauge\n");
        // We only have a monotonic Instant (no wall clock captured at start),
        // so expose 0 as a stable anchor; uptime above is the useful signal.
        out.push_str("aulalite_process_start_time_seconds 0\n");

        out.push_str(
            "# HELP aulalite_http_requests_observed_total Total HTTP requests observed by the metrics layer.\n",
        );
        out.push_str("# TYPE aulalite_http_requests_observed_total counter\n");
        out.push_str(&format!(
            "aulalite_http_requests_observed_total {}\n",
            self.requests_total.load(Ordering::Relaxed)
        ));

        // Snapshot the series map so the rest of render holds no lock.
        let snapshot: Vec<(SeriesKey, Arc<Series>)> = {
            let map = self.series.read().expect("metrics series lock poisoned");
            map.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
        };

        // ── per-route request counter ──────────────────────────────────────
        out.push_str(
            "# HELP aulalite_http_requests_total HTTP requests by method, route template and status class.\n",
        );
        out.push_str("# TYPE aulalite_http_requests_total counter\n");
        for (key, series) in &snapshot {
            out.push_str(&format!(
                "aulalite_http_requests_total{{method=\"{}\",path=\"{}\",status=\"{}\"}} {}\n",
                escape_label(&key.method),
                escape_label(&key.path),
                key.status_class,
                series.count.load(Ordering::Relaxed),
            ));
        }

        // ── request-duration histogram ─────────────────────────────────────
        out.push_str(
            "# HELP aulalite_http_request_duration_seconds HTTP request latency by method, route template and status class.\n",
        );
        out.push_str("# TYPE aulalite_http_request_duration_seconds histogram\n");
        for (key, series) in &snapshot {
            let method = escape_label(&key.method);
            let path = escape_label(&key.path);
            let status = key.status_class;
            let total = series.count.load(Ordering::Relaxed);

            // Cumulative bucket counts: each `le` bucket is the running sum of
            // all finite buckets up to and including it.
            let mut cumulative: u64 = 0;
            for (i, bound) in BUCKET_BOUNDS_SECS.iter().enumerate() {
                cumulative += series.buckets[i].load(Ordering::Relaxed);
                out.push_str(&format!(
                    "aulalite_http_request_duration_seconds_bucket{{method=\"{}\",path=\"{}\",status=\"{}\",le=\"{}\"}} {}\n",
                    method, path, status, format_bound(*bound), cumulative,
                ));
            }
            // +Inf bucket == total observations.
            out.push_str(&format!(
                "aulalite_http_request_duration_seconds_bucket{{method=\"{}\",path=\"{}\",status=\"{}\",le=\"+Inf\"}} {}\n",
                method, path, status, total,
            ));
            // _sum (seconds) and _count.
            let sum_secs = series.sum_micros.load(Ordering::Relaxed) as f64 / 1_000_000.0;
            out.push_str(&format!(
                "aulalite_http_request_duration_seconds_sum{{method=\"{}\",path=\"{}\",status=\"{}\"}} {}\n",
                method, path, status, format_float(sum_secs),
            ));
            out.push_str(&format!(
                "aulalite_http_request_duration_seconds_count{{method=\"{}\",path=\"{}\",status=\"{}\"}} {}\n",
                method, path, status, total,
            ));
        }

        out
    }
}

/// The process-global registry, lazily initialised on first access.
pub fn registry() -> &'static MetricsRegistry {
    static REGISTRY: OnceLock<MetricsRegistry> = OnceLock::new();
    REGISTRY.get_or_init(MetricsRegistry::new)
}

// ── helpers (pure; unit-tested) ────────────────────────────────────────────────

/// Reduce a status code to its class label, bounding cardinality.
fn status_class(status: u16) -> &'static str {
    match status / 100 {
        1 => "1xx",
        2 => "2xx",
        3 => "3xx",
        4 => "4xx",
        5 => "5xx",
        _ => "other",
    }
}

/// Escape a label value per the Prometheus text format: backslash, double-quote
/// and newline are escaped. Route templates and HTTP methods never contain
/// these in practice, but we escape defensively so a crafted request can't
/// break the exposition syntax.
fn escape_label(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            _ => out.push(ch),
        }
    }
    out
}

/// Format a histogram bucket bound for the `le` label without a trailing
/// `.0` where the value is integral-looking but still a float; Prometheus
/// accepts either, but we keep it compact and conventional.
fn format_bound(bound: f64) -> String {
    format_float(bound)
}

/// Compact float formatting: drop a trailing `.0`, otherwise use the shortest
/// round-trippable representation `{}` gives us.
fn format_float(v: f64) -> String {
    if v.fract() == 0.0 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

// ── axum middleware + handler ──────────────────────────────────────────────────

/// Per-request timing middleware. Wire with
/// `axum::middleware::from_fn(crate::services::metrics::track_metrics)`.
///
/// IMPORTANT: install this so it sees `MatchedPath`. In axum 0.7, `MatchedPath`
/// is inserted into the request extensions during routing of the router the
/// layer is attached to, so applying this with `Router::layer(...)` on the
/// fully-merged app router (the same place the trace layer lives) lets us read
/// the matched template here. Requests that match no route fall back to
/// `<unmatched>`.
pub async fn track_metrics(req: Request, next: Next) -> Response {
    let method = req.method().as_str().to_owned();
    // Route template (low cardinality) rather than the concrete URI path.
    let path = req
        .extensions()
        .get::<MatchedPath>()
        .map(|mp| mp.as_str().to_owned())
        .unwrap_or_else(|| "<unmatched>".to_owned());

    let start = Instant::now();
    let response = next.run(req).await;
    let elapsed = start.elapsed().as_secs_f64();

    registry().record(&method, &path, response.status().as_u16(), elapsed);
    response
}

/// `GET /metrics` handler. UNAUTHENTICATED by design — mount it OUTSIDE the
/// `require_auth` layer (alongside `/healthz`). Returns the registry render
/// with the Prometheus text content type.
pub async fn metrics_handler() -> Response {
    let body = registry().render();
    ([(CONTENT_TYPE, PROM_CONTENT_TYPE)], body).into_response()
}

// ── tests (pure; no HTTP) ──────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_class_buckets_by_hundreds() {
        assert_eq!(status_class(100), "1xx");
        assert_eq!(status_class(200), "2xx");
        assert_eq!(status_class(204), "2xx");
        assert_eq!(status_class(301), "3xx");
        assert_eq!(status_class(404), "4xx");
        assert_eq!(status_class(429), "4xx");
        assert_eq!(status_class(500), "5xx");
        assert_eq!(status_class(503), "5xx");
        // Defensive: out-of-range codes don't panic.
        assert_eq!(status_class(0), "other");
        assert_eq!(status_class(700), "other");
    }

    #[test]
    fn escape_label_handles_quote_backslash_newline() {
        assert_eq!(escape_label("/v1/courses/:slug"), "/v1/courses/:slug");
        assert_eq!(escape_label("a\"b"), "a\\\"b");
        assert_eq!(escape_label("a\\b"), "a\\\\b");
        assert_eq!(escape_label("a\nb"), "a\\nb");
    }

    #[test]
    fn format_float_drops_integral_point() {
        assert_eq!(format_float(1.0), "1");
        assert_eq!(format_float(10.0), "10");
        assert_eq!(format_float(0.005), "0.005");
        assert_eq!(format_float(0.25), "0.25");
        assert_eq!(format_bound(5.0), "5");
        assert_eq!(format_bound(0.05), "0.05");
    }

    #[test]
    fn series_observe_increments_count_and_correct_bucket() {
        let s = Series::new();
        // 7ms → falls in the 0.01s bucket (index 1), since 0.007 <= 0.01.
        s.observe(0.007);
        assert_eq!(s.count.load(Ordering::Relaxed), 1);
        // bucket[0] is le=0.005 (0.007 > 0.005 → 0), bucket[1] is le=0.01 → 1.
        assert_eq!(s.buckets[0].load(Ordering::Relaxed), 0);
        assert_eq!(s.buckets[1].load(Ordering::Relaxed), 1);
        // sum recorded in micros: 7000.
        assert_eq!(s.sum_micros.load(Ordering::Relaxed), 7000);
    }

    #[test]
    fn series_observe_past_largest_bound_only_counts_in_inf() {
        let s = Series::new();
        s.observe(42.0); // beyond the 10s top bound
        assert_eq!(s.count.load(Ordering::Relaxed), 1);
        // No finite bucket incremented; +Inf is derived from count at render.
        for b in &s.buckets {
            assert_eq!(b.load(Ordering::Relaxed), 0);
        }
    }

    #[test]
    fn render_emits_expected_metric_names_and_labels() {
        // Use a fresh local registry rather than the global to keep the test
        // independent of other tests touching the process-global one.
        let reg = MetricsRegistry::new();
        reg.record("GET", "/v1/courses/:slug", 200, 0.012);
        reg.record("GET", "/v1/courses/:slug", 200, 0.30);
        reg.record("POST", "/v1/courses", 500, 1.2);

        let text = reg.render();

        // Counter series present with route-template path + status class.
        assert!(text.contains(
            "aulalite_http_requests_total{method=\"GET\",path=\"/v1/courses/:slug\",status=\"2xx\"} 2"
        ));
        assert!(text.contains(
            "aulalite_http_requests_total{method=\"POST\",path=\"/v1/courses\",status=\"5xx\"} 1"
        ));

        // Histogram type + a cumulative bucket + count for the GET series.
        assert!(text.contains("# TYPE aulalite_http_request_duration_seconds histogram"));
        assert!(text.contains(
            "aulalite_http_request_duration_seconds_count{method=\"GET\",path=\"/v1/courses/:slug\",status=\"2xx\"} 2"
        ));
        assert!(text.contains(
            "aulalite_http_request_duration_seconds_bucket{method=\"GET\",path=\"/v1/courses/:slug\",status=\"2xx\",le=\"+Inf\"} 2"
        ));

        // Process/uptime gauges present.
        assert!(text.contains("# TYPE aulalite_process_uptime_seconds gauge"));
        assert!(text.contains("aulalite_http_requests_observed_total 3"));
    }

    #[test]
    fn render_buckets_are_cumulative_and_nondecreasing() {
        let reg = MetricsRegistry::new();
        // One fast (5ms) and one slow (2s) request on the same series.
        reg.record("GET", "/x", 200, 0.005);
        reg.record("GET", "/x", 200, 2.0);

        let text = reg.render();
        // le=0.005 should be 1 (the fast one), le=2.5 should be 2 (both),
        // +Inf should be 2.
        assert!(text.contains(
            "aulalite_http_request_duration_seconds_bucket{method=\"GET\",path=\"/x\",status=\"2xx\",le=\"0.005\"} 1"
        ));
        assert!(text.contains(
            "aulalite_http_request_duration_seconds_bucket{method=\"GET\",path=\"/x\",status=\"2xx\",le=\"2.5\"} 2"
        ));
        assert!(text.contains(
            "aulalite_http_request_duration_seconds_bucket{method=\"GET\",path=\"/x\",status=\"2xx\",le=\"+Inf\"} 2"
        ));
    }
}
