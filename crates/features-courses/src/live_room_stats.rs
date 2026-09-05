// crates/features-courses/src/live_room_stats.rs
//! Network-quality probe + badge for the live room.
//!
//! Given an `&RtcPeerConnection` we call `getStats()` (a `Promise` resolving
//! to an `RTCStatsReport`, which is a JS Map-like keyed by stats-object id),
//! iterate the entries, and reduce the inbound/outbound RTP + candidate-pair
//! samples into a coarse `Good | Fair | Poor` rating from packet loss, jitter,
//! and round-trip-time. The reduction (`classify`) is a pure function so it is
//! unit-tested on host. Browser peers use typed `web_sys`; native peers are
//! sampled inside the WebView by `live_room_native` and reduced here.
//!
//! The main stream owns the polling loop (e.g. a `gloo_timers` interval) and
//! feeds the result into `NetworkQualityBadge`.

use dioxus::prelude::*;

/// Coarse connection quality, ordered worst → best for easy comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum NetQuality {
    /// No samples yet / stats unavailable.
    #[default]
    Unknown,
    Poor,
    Fair,
    Good,
}

impl NetQuality {
    /// CSS modifier suffix for the badge pill.
    pub fn css_modifier(self) -> &'static str {
        match self {
            NetQuality::Unknown => "unknown",
            NetQuality::Poor => "poor",
            NetQuality::Fair => "fair",
            NetQuality::Good => "good",
        }
    }

    /// Short human label.
    pub fn label(self) -> &'static str {
        match self {
            NetQuality::Unknown => "—",
            NetQuality::Poor => "Poor",
            NetQuality::Fair => "Fair",
            NetQuality::Good => "Good",
        }
    }

    /// Number of filled signal bars (0–3) for the glyph.
    pub fn bars(self) -> u8 {
        match self {
            NetQuality::Unknown => 0,
            NetQuality::Poor => 1,
            NetQuality::Fair => 2,
            NetQuality::Good => 3,
        }
    }
}

/// The handful of raw values we extract from an `RTCStatsReport` before
/// classifying. Kept as a plain struct so the reduction logic is pure and
/// testable without any browser types.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct StatsSample {
    /// Fraction of packets lost over the connection's lifetime, 0.0–1.0.
    /// Derived from `packetsLost / (packetsLost + packetsReceived/Sent)`.
    pub loss_fraction: f64,
    /// Jitter in seconds (inbound-rtp `jitter`).
    pub jitter_secs: f64,
    /// Round-trip time in seconds (candidate-pair / remote-inbound-rtp
    /// `roundTripTime`). 0.0 when unknown.
    pub rtt_secs: f64,
    /// True when at least one RTP report contributed, so the caller can
    /// distinguish "measured Good" from "no data".
    pub has_data: bool,
}

/// Reduce a `StatsSample` to a coarse quality rating.
///
/// Thresholds are intentionally lenient so transient blips don't flap the
/// badge:
///   * Poor  — >5% loss, or >150ms jitter, or >400ms RTT
///   * Fair  — >1% loss, or >50ms jitter, or >200ms RTT
///   * Good  — everything below that
pub fn classify(sample: &StatsSample) -> NetQuality {
    if !sample.has_data {
        return NetQuality::Unknown;
    }
    let loss = sample.loss_fraction;
    let jitter_ms = sample.jitter_secs * 1000.0;
    let rtt_ms = sample.rtt_secs * 1000.0;

    if loss > 0.05 || jitter_ms > 150.0 || rtt_ms > 400.0 {
        NetQuality::Poor
    } else if loss > 0.01 || jitter_ms > 50.0 || rtt_ms > 200.0 {
        NetQuality::Fair
    } else {
        NetQuality::Good
    }
}

#[cfg(target_arch = "wasm32")]
mod imp {
    use super::{classify, NetQuality, StatsSample};
    use wasm_bindgen::{JsCast, JsValue};
    use wasm_bindgen_futures::JsFuture;
    use web_sys::RtcPeerConnection;

    /// Read a numeric field off a stats object via `Reflect.get`. Missing /
    /// non-numeric fields yield `None`.
    fn num(obj: &JsValue, key: &str) -> Option<f64> {
        js_sys::Reflect::get(obj, &JsValue::from_str(key))
            .ok()
            .and_then(|v| v.as_f64())
    }

    /// Read the `type` string off a stats object.
    fn stat_type(obj: &JsValue) -> Option<String> {
        js_sys::Reflect::get(obj, &JsValue::from_str("type"))
            .ok()
            .and_then(|v| v.as_string())
    }

    /// Poll `pc.getStats()` once and reduce it to a `StatsSample`.
    pub async fn sample_stats(pc: &RtcPeerConnection) -> StatsSample {
        let mut sample = StatsSample::default();
        let report_value = match JsFuture::from(pc.get_stats()).await {
            Ok(v) => v,
            Err(_) => return sample,
        };
        // RTCStatsReport is a JS Map; `entries()` yields `[key, value]` pairs.
        let report: web_sys::RtcStatsReport = report_value.unchecked_into();
        let entries = report.entries();

        let mut packets_lost = 0.0_f64;
        let mut packets_total = 0.0_f64;

        loop {
            let next = match entries.next() {
                Ok(n) => n,
                Err(_) => break,
            };
            if next.done() {
                break;
            }
            // Each entry is a two-element array [id, statsObject].
            let pair: js_sys::Array = next.value().unchecked_into();
            let stat = pair.get(1);
            let Some(kind) = stat_type(&stat) else {
                continue;
            };
            match kind.as_str() {
                "inbound-rtp" | "outbound-rtp" => {
                    let lost = num(&stat, "packetsLost").unwrap_or(0.0).max(0.0);
                    // inbound reports `packetsReceived`; outbound `packetsSent`.
                    let moved = num(&stat, "packetsReceived")
                        .or_else(|| num(&stat, "packetsSent"))
                        .unwrap_or(0.0)
                        .max(0.0);
                    packets_lost += lost;
                    packets_total += lost + moved;
                    if let Some(j) = num(&stat, "jitter") {
                        // Track the worst (largest) jitter across RTP streams.
                        if j > sample.jitter_secs {
                            sample.jitter_secs = j;
                        }
                    }
                    sample.has_data = true;
                }
                "remote-inbound-rtp" => {
                    // Sender-side view of loss + RTT reported by the remote.
                    if let Some(rtt) = num(&stat, "roundTripTime") {
                        if rtt > sample.rtt_secs {
                            sample.rtt_secs = rtt;
                        }
                    }
                    if let Some(j) = num(&stat, "jitter") {
                        if j > sample.jitter_secs {
                            sample.jitter_secs = j;
                        }
                    }
                    sample.has_data = true;
                }
                "candidate-pair" => {
                    // Prefer the nominated/succeeded pair's RTT when present.
                    let nominated = js_sys::Reflect::get(&stat, &JsValue::from_str("nominated"))
                        .ok()
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    if nominated {
                        if let Some(rtt) = num(&stat, "currentRoundTripTime") {
                            sample.rtt_secs = rtt;
                        }
                    }
                }
                _ => {}
            }
        }

        if packets_total > 0.0 {
            sample.loss_fraction = (packets_lost / packets_total).clamp(0.0, 1.0);
        }
        sample
    }

    /// Convenience: poll once and classify in a single call.
    pub async fn probe(pc: &RtcPeerConnection) -> NetQuality {
        classify(&sample_stats(pc).await)
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod imp {
    use super::{NetQuality, StatsSample};

    /// Compatibility shim for callers carrying a typed peer handle. Native
    /// publisher quality is sampled by `live_room_native::publisher_quality`,
    /// because the actual peer connection is owned inside the WebView.
    pub async fn sample_stats(_pc: &()) -> StatsSample {
        StatsSample::default()
    }

    pub async fn probe(_pc: &()) -> NetQuality {
        NetQuality::Unknown
    }
}

pub use imp::*;

// ---------------------------------------------------------------------------
// NetworkQualityBadge — colored pill with signal bars
// ---------------------------------------------------------------------------

#[derive(Props, Clone, PartialEq)]
pub struct NetworkQualityBadgeProps {
    pub quality: NetQuality,
    /// When true, show the text label next to the bars. Defaults to true.
    #[props(default = true)]
    pub show_label: bool,
}

#[component]
pub fn NetworkQualityBadge(props: NetworkQualityBadgeProps) -> Element {
    let q = props.quality;
    let modifier = q.css_modifier();
    let filled = q.bars();
    let class = format!("net-quality net-quality--{modifier}");
    let title = format!("Connection quality: {}", q.label());

    rsx! {
        span {
            class: "{class}",
            title: "{title}",
            role: "status",
            span { class: "net-quality-bars", "aria-hidden": "true",
                for i in 0u8..3u8 {
                    span {
                        class: if i < filled { "net-quality-bar net-quality-bar--on" } else { "net-quality-bar" },
                    }
                }
            }
            if props.show_label {
                span { class: "net-quality-label", "{q.label()}" }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_data_is_unknown() {
        let s = StatsSample::default();
        assert_eq!(classify(&s), NetQuality::Unknown);
    }

    #[test]
    fn clean_link_is_good() {
        let s = StatsSample {
            loss_fraction: 0.0,
            jitter_secs: 0.005,
            rtt_secs: 0.03,
            has_data: true,
        };
        assert_eq!(classify(&s), NetQuality::Good);
    }

    #[test]
    fn mild_loss_is_fair() {
        let s = StatsSample {
            loss_fraction: 0.02,
            jitter_secs: 0.01,
            rtt_secs: 0.05,
            has_data: true,
        };
        assert_eq!(classify(&s), NetQuality::Fair);
    }

    #[test]
    fn heavy_loss_is_poor() {
        let s = StatsSample {
            loss_fraction: 0.08,
            jitter_secs: 0.01,
            rtt_secs: 0.05,
            has_data: true,
        };
        assert_eq!(classify(&s), NetQuality::Poor);
    }

    #[test]
    fn high_rtt_alone_downgrades() {
        let s = StatsSample {
            loss_fraction: 0.0,
            jitter_secs: 0.0,
            rtt_secs: 0.45,
            has_data: true,
        };
        assert_eq!(classify(&s), NetQuality::Poor);
    }

    #[test]
    fn quality_orders_worst_to_best() {
        assert!(NetQuality::Poor < NetQuality::Fair);
        assert!(NetQuality::Fair < NetQuality::Good);
        assert!(NetQuality::Unknown < NetQuality::Poor);
    }

    #[test]
    fn badge_renders_modifier_and_bars() {
        fn app() -> Element {
            rsx! { NetworkQualityBadge { quality: NetQuality::Fair } }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("net-quality--fair"),
            "modifier missing: {html}"
        );
        assert!(
            html.contains("net-quality-bar--on"),
            "filled bar missing: {html}"
        );
        assert!(html.contains("Fair"), "label missing: {html}");
    }

    #[test]
    fn badge_can_hide_label() {
        fn app() -> Element {
            rsx! { NetworkQualityBadge { quality: NetQuality::Good, show_label: false } }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            !html.contains("net-quality-label"),
            "label should be hidden: {html}"
        );
    }
}
