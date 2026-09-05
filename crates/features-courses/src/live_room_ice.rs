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
//! Timeouts are sized against the MediaMTX defaults this deployment runs with
//! (`webrtcHandshakeTimeout: 10s`, `webrtcSTUNGatherTimeout: 5s`), read from
//! `/v3/config/global/get` rather than guessed.

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

/// How long to wait, after the SDP exchange, for the peer connection to reach
/// `connected`.
///
/// Deliberately longer than MediaMTX's `webrtcHandshakeTimeout` (10s) so that
/// when the server gives up first we observe the resulting failure state and
/// report *that*, instead of racing it and reporting a less specific timeout.
pub const CONNECT_TIMEOUT_MS: u32 = 15_000;

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
    use web_sys::{RtcIceGatheringState, RtcPeerConnection, RtcPeerConnectionState};

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

        // Re-check after attaching: `complete` can land between the early
        // return above and the handler being installed, and that transition
        // would otherwise be missed entirely (waiting the full timeout).
        if pc.ice_gathering_state() == RtcIceGatheringState::Complete {
            fire(&slot, true);
        }

        let completed = rx.await.unwrap_or(false);
        pc.set_onicegatheringstatechange(None);
        drop(cb);
        completed
    }

    /// Wait until `pc` reports `connectionState == "connected"`, or fail with a
    /// description of what actually happened.
    ///
    /// `failed`/`closed` are terminal and reported immediately. `disconnected`
    /// is NOT: it is a transient state ICE recovers from on its own, so it is
    /// left to either recover or run into the timeout.
    pub async fn await_connected(pc: &RtcPeerConnection, timeout_ms: u32) -> Result<(), String> {
        fn classify(state: RtcPeerConnectionState) -> Option<Result<(), String>> {
            match state {
                RtcPeerConnectionState::Connected => Some(Ok(())),
                RtcPeerConnectionState::Failed => Some(Err(
                    "ICE failed: no usable network path to the media server. \
                     A TURN relay is required when publisher and server are on \
                     different networks."
                        .to_string(),
                )),
                RtcPeerConnectionState::Closed => {
                    Some(Err("peer connection closed before it connected".to_string()))
                }
                _ => None,
            }
        }

        if let Some(done) = classify(pc.connection_state()) {
            return done;
        }

        let (slot, rx) = arm::<Option<Result<(), String>>>();
        arm_timeout(&slot, timeout_ms, None);

        let cb = {
            let slot = slot.clone();
            let pc = pc.clone();
            Closure::<dyn FnMut()>::new(move || {
                if let Some(done) = classify(pc.connection_state()) {
                    fire(&slot, Some(done));
                }
            })
        };
        pc.set_onconnectionstatechange(Some(cb.as_ref().unchecked_ref()));

        // Same race as above: the state can advance while we are attaching.
        if let Some(done) = classify(pc.connection_state()) {
            fire(&slot, Some(done));
        }

        let outcome = rx.await.unwrap_or(None);
        pc.set_onconnectionstatechange(None);
        drop(cb);

        outcome.unwrap_or_else(|| {
            Err(format!(
                "timed out after {}ms waiting for the media connection \
                 (last state: {:?}, ICE: {:?})",
                timeout_ms,
                pc.connection_state(),
                pc.ice_connection_state(),
            ))
        })
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

#[cfg(target_arch = "wasm32")]
pub use imp::*;

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
        assert!(PATH_READY_TIMEOUT_MS >= 10_000);
        assert!(PATH_READY_RETRY_MS > 0);
        // At least a handful of attempts inside the budget.
        assert!(PATH_READY_TIMEOUT_MS / PATH_READY_RETRY_MS >= 8);
    }

    #[test]
    fn timeouts_are_ordered_against_the_mediamtx_handshake_window() {
        // MediaMTX's webrtcHandshakeTimeout is 10s in this deployment. We must
        // outlast it so its failure is observed rather than raced.
        assert!(CONNECT_TIMEOUT_MS > 10_000);
        // Gathering happens before the POST, so it must not eat the handshake
        // window; keep it comfortably shorter than the connect budget.
        assert!(ICE_GATHERING_TIMEOUT_MS < CONNECT_TIMEOUT_MS);
    }
}
