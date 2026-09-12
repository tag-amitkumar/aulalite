// crates/features-courses/src/live_room_ice.rs
//! ICE lifecycle helpers shared by the WHIP publisher and the WHEP viewer.
//!
//! Both halves of the media plane have the same two obligations that plain
//! "create offer → POST → set answer" signalling does not discharge:
//!
//!   1. **Ship a COMPLETE offer.** WHIP/WHEP as implemented by MediaMTX is a
//!      single request/response exchange: the SDP we POST is the only chance we
//!      get to tell the server where to send its connectivity checks. There is
//!      no trickle-ICE channel afterwards, so any candidate that has not been
//!      gathered by the time we read `pc.localDescription` is lost for the
//!      lifetime of that session. `RTCPeerConnection.setLocalDescription()`
//!      resolves *before* gathering starts, so reading the SDP straight after
//!      it yields an offer with ZERO candidates — measured, not assumed. Such
//!      an offer only connects when the browser happens to reach one of the
//!      server's own advertised candidates directly and the server learns our
//!      address as a peer-reflexive candidate. The moment a relay is required
//!      (teacher and SFU on different networks) there is no viable pair and
//!      MediaMTX fails the session with
//!      `deadline exceeded while waiting connection`.
//!
//!      `await_ice_gathering` therefore waits for `iceGatheringState` to reach
//!      `complete` — which is exactly when every host, server-reflexive and
//!      **relay** candidate is present in the local description — bounded by a
//!      timeout so a hung STUN/TURN server cannot wedge go-live.
//!
//!   2. **Verify the connection actually came up.** A `201` on the WHIP/WHEP
//!      POST only proves the SDP was accepted. DTLS/ICE can still fail
//!      afterwards, in which case the teacher's UI reported "live" while
//!      nothing was flowing and viewers got `404 no stream is available`.
//!      `await_connected` turns that silent half-failure into a real error the
//!      existing retry/error UI can act on.
//!
//! `await_connected` does this **without an application timeout**: it waits
//! on the peer connection's own state. A negotiation still in progress is not
//! a failure, and the 15s budget that used to bound it reported
//! `timed out after 15000ms waiting for the media connection` for the most
//! common real failure there is -- a relay allocation that lands late -- then
//! tore down the publisher. The browser's ICE agent and MediaMTX's own
//! `webrtcHandshakeTimeout` (10s) are the safeguards that still produce a
//! terminal `failed`, so nothing hangs.
//!
//! The one remaining bound here, `ICE_GATHERING_TIMEOUT_MS`, is sized against
//! the MediaMTX defaults this deployment runs with
//! (`webrtcSTUNGatherTimeout: 5s`), read from `/v3/config/global/get` rather
//! than guessed. It never fails a publish -- it only decides when to POST the
//! offer with the candidates gathered so far.

/// How long to wait for ICE gathering to reach `complete` before POSTing the
/// offer anyway.
///
/// Gathering normally completes in well under a second on a LAN; the slow leg
/// is a TURN allocation, which is why this is seconds rather than milliseconds.
/// Matched to MediaMTX's own `webrtcSTUNGatherTimeout` (5s) — past that point a
/// relay is not going to appear, and blocking go-live longer is worse than
/// publishing with the candidates we do have.
pub const ICE_GATHERING_TIMEOUT_MS: u32 = 5_000;

/// How long a WHEP subscribe keeps retrying while MediaMTX reports that the
/// path is not readable yet.
///
/// MediaMTX only publishes a path once it has gathered the publisher's tracks,
/// which lags the publisher's ICE connection by up to `webrtcTrackGatherTimeout`
/// (2s here) -- and reliably the FULL 2s for a simulcast publisher, whose top
/// layer sends nothing while bandwidth ramps up, so MediaMTX can never finish
/// gathering early. Measured on this deployment: publisher `connected` at
/// +4.97s, path readable at +7.03s. 20s leaves generous room above that gap
/// without leaving a genuinely absent stream spinning forever.
pub const PATH_READY_TIMEOUT_MS: u32 = 20_000;

/// Delay between those readiness retries.
pub const PATH_READY_RETRY_MS: u32 = 500;

/// Resolve the `Location` header of a WHIP/WHEP `201 Created` against the URL
/// the request was sent to.
///
/// MediaMTX returns a **relative** resource URL (measured: `POST
/// .../whip` → `Location: /aula/<tenant>/<course>/<session>/whip/<uuid>`).
/// Handing that string straight to `fetch()` resolves it against the *page*
/// origin, so the teardown `DELETE` went to the app host instead of the media
/// host and the server-side session was never released — publisher sessions
/// leaked until MediaMTX timed them out or a later publish evicted them with
/// `closing existing publisher`.
///
/// Absolute `Location` values (which a spec-compliant WHIP server may also
/// return) pass through unchanged, because `Url::join` prefers an absolute
/// reference. Kept target-independent so it is unit-testable on the host.
pub fn resolve_resource_url(request_url: &str, location: &str) -> Option<String> {
    let location = location.trim();
    if location.is_empty() {
        return None;
    }
    url::Url::parse(request_url)
        .ok()?
        .join(location)
        .ok()
        .map(|u| u.to_string())
}

#[cfg(target_arch = "wasm32")]
mod imp {
    use std::cell::RefCell;
    use std::rc::Rc;
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen::JsCast;
    use web_sys::{
        RtcIceConnectionState, RtcIceGatheringState, RtcPeerConnection, RtcPeerConnectionState,
    };

    /// A oneshot that either arm — a peer-connection event or a timer — can
    /// complete, whichever happens first. `Rc<RefCell<Option<Sender>>>` is the
    /// wasm-appropriate shape here: everything runs on one thread, and
    /// `Option::take` makes "first writer wins" explicit without a lock.
    type Arm<T> = Rc<RefCell<Option<futures_channel::oneshot::Sender<T>>>>;

    fn arm<T>() -> (Arm<T>, futures_channel::oneshot::Receiver<T>) {
        let (tx, rx) = futures_channel::oneshot::channel::<T>();
        (Rc::new(RefCell::new(Some(tx))), rx)
    }

    fn fire<T>(slot: &Arm<T>, value: T) {
        if let Some(tx) = slot.borrow_mut().take() {
            let _ = tx.send(value);
        }
    }

    /// Arm a timer that completes `slot` with `value` after `timeout_ms`.
    ///
    /// Detached rather than raced with `select`, so this needs no `futures-util`
    /// dependency: whichever arm fires first takes the sender and the other's
    /// `send` becomes a no-op.
    fn arm_timeout<T: 'static>(slot: &Arm<T>, timeout_ms: u32, value: T) {
        let slot = slot.clone();
        wasm_bindgen_futures::spawn_local(async move {
            gloo_timers::future::TimeoutFuture::new(timeout_ms).await;
            fire(&slot, value);
        });
    }

    /// Keeps an event `Closure` alive for exactly as long as it is installed on
    /// the peer connection, and uninstalls it on drop.
    ///
    /// Installing a handler and only clearing it after `rx.await` returns is
    /// wrong: if the enclosing future is CANCELLED at that await -- which is
    /// routine, the route can unmount mid-handshake -- the `Closure` is dropped
    /// while the peer connection still holds a pointer to it, and the next
    /// state change calls freed wasm memory. `Drop` runs before the fields are
    /// dropped, so the handler is always uninstalled first.
    struct InstalledHandler {
        pc: RtcPeerConnection,
        clear: fn(&RtcPeerConnection),
        _cb: Closure<dyn FnMut()>,
    }

    impl Drop for InstalledHandler {
        fn drop(&mut self) {
            (self.clear)(&self.pc);
        }
    }

    /// Closes a peer connection if the function that created it bails out.
    ///
    /// Every `?` between `RtcPeerConnection::new_with_configuration` and the
    /// point where the connection is handed to a `WhipPublisher`/`WhepViewer`
    /// used to abandon an open connection: nothing owned it yet, so nothing
    /// closed it, and the ICE agent and any gathered ports stayed alive until
    /// the page went away. Disarm once ownership has been transferred.
    pub struct PcCloseGuard {
        pc: Option<RtcPeerConnection>,
    }

    impl PcCloseGuard {
        pub fn new(pc: RtcPeerConnection) -> Self {
            Self { pc: Some(pc) }
        }

        /// Ownership has moved to a type whose own `close`/`Drop` handles it.
        pub fn disarm(mut self) {
            let _ = self.pc.take();
        }
    }

    impl Drop for PcCloseGuard {
        fn drop(&mut self) {
            if let Some(pc) = self.pc.take() {
                pc.close();
            }
        }
    }

    /// Wait until `pc` has gathered every ICE candidate it is going to gather,
    /// or `timeout_ms` elapses.
    ///
    /// Returns `true` when gathering genuinely completed, `false` on timeout —
    /// the caller publishes the partial offer either way (a candidate-light
    /// offer still connects on a LAN), but the distinction is worth logging
    /// because a timeout is what a hung TURN server looks like.
    ///
    /// Must be called AFTER `set_local_description`, which is what starts
    /// gathering.
    pub async fn await_ice_gathering(pc: &RtcPeerConnection, timeout_ms: u32) -> bool {
        if pc.ice_gathering_state() == RtcIceGatheringState::Complete {
            return true;
        }

        let (slot, rx) = arm::<bool>();
        arm_timeout(&slot, timeout_ms, false);

        let cb = {
            let slot = slot.clone();
            let pc = pc.clone();
            Closure::<dyn FnMut()>::new(move || {
                if pc.ice_gathering_state() == RtcIceGatheringState::Complete {
                    fire(&slot, true);
                }
            })
        };
        pc.set_onicegatheringstatechange(Some(cb.as_ref().unchecked_ref()));
        // Uninstalls on drop, so cancelling this future cannot leave the peer
        // connection pointing at a freed closure.
        let _installed = InstalledHandler {
            pc: pc.clone(),
            clear: |pc| pc.set_onicegatheringstatechange(None),
            _cb: cb,
        };

        // Re-check after attaching: `complete` can land between the early
        // return above and the handler being installed, and that transition
        // would otherwise be missed entirely (waiting the full timeout).
        if pc.ice_gathering_state() == RtcIceGatheringState::Complete {
            fire(&slot, true);
        }

        rx.await.unwrap_or(false)
    }

    /// Wait until the peer connection actually reaches `connected`, or fail
    /// with the real reason it will never get there.
    ///
    /// **There is deliberately no application timeout here.** A publish that is
    /// still negotiating is not a failed publish. The previous 15s budget
    /// converted the single most common real-world failure -- a TURN allocation
    /// that lands late, or a slow relay handshake -- into
    /// `timed out after 15000ms waiting for the media connection`, and the
    /// caller then tore down the peer connection, released the camera and mic,
    /// DELETEd the WHIP session and parked the room in a terminal error with no
    /// retry. A clock is the wrong authority for that decision.
    ///
    /// Termination is driven by state instead, which is what makes a timer
    /// unnecessary rather than merely absent:
    ///
    /// * `connected` -> success, media can flow.
    /// * `failed` -> terminal, reported as the actual ICE failure it is.
    /// * `closed` -> terminal (the caller closed it, or the page went away).
    /// * `disconnected` -> transient. ICE recovers from it on its own, and when
    ///   it cannot, the browser's own consent-freshness machinery (RFC 7675)
    ///   drives it on to `failed`. That is the low-level safeguard this relies
    ///   on, and it is why waiting indefinitely does not hang: MediaMTX's
    ///   handshake timeout closes the server side, which produces `failed`
    ///   here.
    ///
    /// Both `connectionState` and `iceConnectionState` are watched. Success is
    /// taken only from `connectionState` (ICE `connected` can precede DTLS, so
    /// media is not yet flowing), but failure is taken from either, because a
    /// failed ICE agent is authoritative even where `connectionState` lags.
    ///
    /// Cancellation remains safe and leak-free: both handlers uninstall on
    /// drop, and the caller's `close()`/`Drop` releases the peer connection and
    /// the server-side WHIP session.
    pub async fn await_connected(pc: &RtcPeerConnection) -> Result<(), String> {
        fn classify(
            conn: RtcPeerConnectionState,
            ice: RtcIceConnectionState,
        ) -> Option<Result<(), String>> {
            if conn == RtcPeerConnectionState::Connected {
                return Some(Ok(()));
            }
            if conn == RtcPeerConnectionState::Failed || ice == RtcIceConnectionState::Failed {
                return Some(Err(
                    "ICE failed: no usable network path to the media server. \
                     A TURN relay is required when publisher and server are on \
                     different networks."
                        .to_string(),
                ));
            }
            if conn == RtcPeerConnectionState::Closed || ice == RtcIceConnectionState::Closed {
                return Some(Err(
                    "peer connection closed before it connected".to_string()
                ));
            }
            None
        }

        let snapshot = |pc: &RtcPeerConnection| classify(pc.connection_state(), pc.ice_connection_state());

        if let Some(done) = snapshot(pc) {
            return done;
        }

        let (slot, rx) = arm::<Result<(), String>>();

        // One closure body, installed on both state surfaces: whichever fires
        // first and reaches a terminal verdict takes the sender.
        let make_cb = || {
            let slot = slot.clone();
            let pc = pc.clone();
            Closure::<dyn FnMut()>::new(move || {
                if let Some(done) = classify(pc.connection_state(), pc.ice_connection_state()) {
                    fire(&slot, done);
                }
            })
        };

        let conn_cb = make_cb();
        pc.set_onconnectionstatechange(Some(conn_cb.as_ref().unchecked_ref()));
        let _conn_installed = InstalledHandler {
            pc: pc.clone(),
            clear: |pc| pc.set_onconnectionstatechange(None),
            _cb: conn_cb,
        };

        let ice_cb = make_cb();
        pc.set_oniceconnectionstatechange(Some(ice_cb.as_ref().unchecked_ref()));
        let _ice_installed = InstalledHandler {
            pc: pc.clone(),
            clear: |pc| pc.set_oniceconnectionstatechange(None),
            _cb: ice_cb,
        };

        // The state can advance while we are attaching, and that transition
        // would otherwise be missed entirely.
        if let Some(done) = snapshot(pc) {
            fire(&slot, done);
        }

        // The sender is held by the installed handlers, which outlive this
        // await; a cancelled receiver simply drops them. `Err` on the channel
        // therefore means the peer connection went away underneath us.
        rx.await
            .unwrap_or_else(|_| Err("peer connection was dropped while connecting".to_string()))
    }

    /// Duplicate a `MediaStream` HANDLE -- the same underlying JS object --
    /// rather than its media.
    ///
    /// **This exists because `stream.clone()` does NOT do what it looks like.**
    /// `web_sys::MediaStream` exposes the DOM's own `MediaStream.clone()` as an
    /// INHERENT method, and Rust resolves inherent methods BEFORE trait methods.
    /// So `stream.clone()` calls the DOM method, which returns a BRAND NEW
    /// `MediaStream` containing CLONES of the tracks -- not the `Clone` impl
    /// that merely copies the JS handle.
    ///
    /// That silently broke the student's video: `live_room_whep::view` handed
    /// its `ontrack` closure a `stream.clone()`, so incoming tracks were added
    /// to a DETACHED copy while the `MediaStream` stored on the viewer -- the
    /// one wired onto `<video id="live-room-main-video">` -- stayed empty
    /// forever. Measured: `ontrack` added `video`+`audio` to stream
    /// `bbe452b9` while the element held the empty stream `4641e1ca`, with the
    /// peer connection decoding 654 frames the whole time.
    ///
    /// UFCS forces the trait impl, which is the handle copy we actually want.
    pub fn same_stream(stream: &web_sys::MediaStream) -> web_sys::MediaStream {
        Clone::clone(stream)
    }

    /// Count of `a=candidate` lines in an SDP. Used only for diagnostics — an
    /// offer that leaves here with zero candidates is the signature of the bug
    /// this module exists to prevent, so it is worth a console line.
    pub fn candidate_count(sdp: &str) -> usize {
        sdp.lines()
            .filter(|l| l.starts_with("a=candidate:"))
            .count()
    }
}

/// How long to wait for a relay candidate before calling the relay dead.
///
/// A working TURN answers in well under a second; a dead one surfaces as an
/// `icecandidateerror` almost immediately (measured: code 701 on openrelay).
/// This is only the backstop for a server that accepts the connection and then
/// never replies, so it can be short -- the probe is informational and must not
/// keep the strip in "Checking" for the first minute of a lesson.
pub const RELAY_PROBE_TIMEOUT_MS: u32 = 6_000;

#[cfg(target_arch = "wasm32")]
pub use imp::*;

#[cfg(target_arch = "wasm32")]
mod relay_probe {
    use super::RELAY_PROBE_TIMEOUT_MS;
    use crate::live_room_health::RelayStatus;
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen::{JsCast, JsValue};
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{RtcConfiguration, RtcIceServer, RtcIceTransportPolicy, RtcPeerConnection};

    /// Can this browser actually allocate a TURN relay right now?
    ///
    /// Gathers with `iceTransportPolicy: "relay"`, which suppresses host and
    /// server-reflexive candidates entirely -- so anything that arrives came
    /// from a TURN allocation, and arriving with nothing is proof the relay
    /// cannot be used. Cheap: a data-channel offer, no media, no signalling,
    /// nothing published.
    ///
    /// Uses the session's OWN ICE servers (`current_ice_servers`), not a
    /// hard-coded list, so the row reports on the relay the room would really
    /// use rather than on some other server that happens to work.
    pub async fn probe_relay() -> RelayStatus {
        let servers = crate::live_room_whip::current_ice_servers();
        let turn: Vec<_> = servers
            .iter()
            .filter(|s| {
                s.urls
                    .iter()
                    .any(|u| u.starts_with("turn:") || u.starts_with("turns:"))
            })
            .collect();
        if turn.is_empty() {
            return RelayStatus::NotConfigured;
        }

        let cfg = RtcConfiguration::new();
        let arr = js_sys::Array::new();
        for entry in &turn {
            let urls = js_sys::Array::new();
            for u in &entry.urls {
                urls.push(&JsValue::from_str(u));
            }
            let s = RtcIceServer::new();
            s.set_urls(&urls);
            if let Some(u) = &entry.username {
                s.set_username(u);
            }
            if let Some(c) = &entry.credential {
                s.set_credential(c);
            }
            arr.push(&s);
        }
        cfg.set_ice_servers(&arr);
        cfg.set_ice_transport_policy(RtcIceTransportPolicy::Relay);

        let Ok(pc) = RtcPeerConnection::new_with_configuration(&cfg) else {
            return RelayStatus::Unavailable;
        };
        // Closes the connection however we leave this function, including the
        // early returns below -- an abandoned probe would hold its ICE agent
        // and any allocation open for the life of the page.
        let guard = super::imp::PcCloseGuard::new(pc.clone());

        let found = std::rc::Rc::new(std::cell::Cell::new(false));
        let cb = {
            let found = found.clone();
            Closure::<dyn FnMut(web_sys::RtcPeerConnectionIceEvent)>::new(
                move |ev: web_sys::RtcPeerConnectionIceEvent| {
                    if let Some(c) = ev.candidate() {
                        if c.candidate().contains(" typ relay") {
                            found.set(true);
                        }
                    }
                },
            )
        };
        pc.set_onicecandidate(Some(cb.as_ref().unchecked_ref()));

        // A candidate-less offer gathers nothing, so give it a data channel.
        let _dc = pc.create_data_channel("relay-probe");
        let ok = async {
            let offer = JsFuture::from(pc.create_offer()).await.ok()?;
            let offer: web_sys::RtcSessionDescriptionInit = offer.unchecked_into();
            JsFuture::from(pc.set_local_description(&offer)).await.ok()
        }
        .await;
        if ok.is_none() {
            pc.set_onicecandidate(None);
            drop(guard);
            return RelayStatus::Unavailable;
        }

        // Reuse the gathering wait already used by the publisher, then judge on
        // what actually arrived rather than on whether gathering "completed":
        // with a dead relay gathering completes promptly and empty.
        let _ = super::imp::await_ice_gathering(&pc, RELAY_PROBE_TIMEOUT_MS).await;
        let verdict = if found.get() {
            RelayStatus::Available
        } else {
            RelayStatus::Unavailable
        };
        pc.set_onicecandidate(None);
        drop(guard);
        verdict
    }
}

#[cfg(target_arch = "wasm32")]
pub use relay_probe::probe_relay;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_mediamtx_relative_location_against_the_media_host() {
        // The exact shape MediaMTX returns, captured from a live 201.
        let resolved = resolve_resource_url(
            "https://media.elementors.guru/aula/tenant/course/session/whip",
            "/aula/tenant/course/session/whip/6b571a87-0036-464c-9763-26a868a8ab50",
        );
        assert_eq!(
            resolved.as_deref(),
            Some(
                "https://media.elementors.guru/aula/tenant/course/session/whip/6b571a87-0036-464c-9763-26a868a8ab50"
            )
        );
    }

    #[test]
    fn keeps_the_media_origin_not_the_page_origin() {
        // The regression this guards: a relative Location must never resolve
        // against the app origin.
        let resolved = resolve_resource_url(
            "https://media.elementors.guru/aula/t/c/s/whip",
            "/aula/t/c/s/whip/abc",
        )
        .unwrap();
        assert!(resolved.starts_with("https://media.elementors.guru/"));
        assert!(!resolved.contains("aula.elementors.guru"));
    }

    #[test]
    fn passes_absolute_location_through_unchanged() {
        let resolved = resolve_resource_url(
            "https://media.elementors.guru/aula/t/c/s/whip",
            "https://other.example/resource/1",
        );
        assert_eq!(resolved.as_deref(), Some("https://other.example/resource/1"));
    }

    #[test]
    fn resolves_relative_without_leading_slash() {
        let resolved = resolve_resource_url("https://media.example/aula/t/c/s/whip", "whip/xyz");
        assert_eq!(
            resolved.as_deref(),
            Some("https://media.example/aula/t/c/s/whip/xyz")
        );
    }

    #[test]
    fn rejects_empty_or_whitespace_location() {
        assert_eq!(
            resolve_resource_url("https://media.example/aula/t/c/s/whip", ""),
            None
        );
        assert_eq!(
            resolve_resource_url("https://media.example/aula/t/c/s/whip", "   "),
            None
        );
    }

    #[test]
    fn rejects_unparseable_request_url() {
        assert_eq!(resolve_resource_url("not a url", "/resource/1"), None);
    }

    #[test]
    fn path_ready_timeout_clears_the_mediamtx_track_gather_window() {
        // MediaMTX's webrtcTrackGatherTimeout is 2s in this deployment, and the
        // measured publisher-connected -> path-readable gap was ~2.1s. The
        // retry budget must clear that by a wide margin or viewers keep losing
        // the race they lost before.
        // `const` blocks: every operand is a compile-time constant, so this
        // belongs at compile time rather than posing as a runtime check.
        const { assert!(PATH_READY_TIMEOUT_MS >= 10_000) };
        const { assert!(PATH_READY_RETRY_MS > 0) };
        // At least a handful of attempts inside the budget.
        const { assert!(PATH_READY_TIMEOUT_MS / PATH_READY_RETRY_MS >= 8) };
    }

    #[test]
    fn there_is_no_publish_connect_timeout_constant() {
        // Guards the fix for "timed out after 15000ms waiting for the media
        // connection". `await_connected` is state driven: it succeeds on
        // `connected`, fails on `failed`/`closed`, and treats `disconnected`
        // as transient. Re-introducing a connect budget here would restore the
        // bug, so this file must not regain such a constant.
        let src = include_str!("live_room_ice.rs");
        // Built from fragments on purpose. Spelled as one literal, the needle
        // occurs in THIS line, so `src.contains` matched the guard itself and
        // the assertion could never pass -- it reported the bug it was written
        // to catch, whether or not the constant existed.
        let needle = concat!("CONNECT", "_TIMEOUT");
        assert!(
            !src.contains(needle),
            "a publish/connect timeout constant is back in live_room_ice.rs"
        );
        // The gathering bound is NOT a connect timeout and must stay: it only
        // decides when to POST the offer with the candidates already gathered,
        // and never fails the publish.
        assert_eq!(ICE_GATHERING_TIMEOUT_MS, 5_000);
    }
}
