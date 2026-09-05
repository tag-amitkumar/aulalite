// crates/features-courses/src/live_room_whip.rs
//! WHIP (WebRTC HTTP Ingest) client. Posts an SDP offer to MediaMTX,
//! receives an SDP answer, and streams local MediaStream tracks.
//!
//! Out-of-runtime errors return `Result<_, String>` for easy display.

/// One ICE server in the W3C RTCIceServer shape. Deserialized from the backend
/// go-live/join `ice_servers` field. `urls` is a list; creds are optional (TURN
/// long-term credentials).
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct IceServerConfig {
    pub urls: Vec<String>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub credential: Option<String>,
}

#[cfg(target_arch = "wasm32")]
thread_local! {
    static ICE_SERVERS: std::cell::RefCell<Vec<IceServerConfig>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Install the ICE servers reported by the backend for the typed browser peer.
/// Native publish/view requests carry their bounded ICE list directly into the
/// WebView bridge, so the compatibility setter does not maintain native state.
/// If never called on web, `rtc_config_with_stun` uses its STUN default.
#[cfg(target_arch = "wasm32")]
pub fn set_ice_servers(servers: Vec<IceServerConfig>) {
    ICE_SERVERS.with(|s| *s.borrow_mut() = servers);
}
#[cfg(not(target_arch = "wasm32"))]
pub fn set_ice_servers(_servers: Vec<IceServerConfig>) {}

#[cfg(target_arch = "wasm32")]
pub fn current_ice_servers() -> Vec<IceServerConfig> {
    ICE_SERVERS.with(|s| s.borrow().clone())
}
#[cfg(not(target_arch = "wasm32"))]
pub fn current_ice_servers() -> Vec<IceServerConfig> {
    Vec::new()
}

// ---------------------------------------------------------------------------
// Simulcast encoding tiers (pure, target-independent so it is unit-testable)
// ---------------------------------------------------------------------------

/// One simulcast layer the publisher offers to the SFU. Pure data so the tier
/// table can be unit-tested on host without any `web_sys` plumbing.
///
/// `rid` is the RTP stream id the SFU keys the layer on; `scale` is
/// `scaleResolutionDownBy` (1 = full res, 2 = half, 4 = quarter); `max_bitrate`
/// is the per-layer cap in bits-per-second.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SimulcastTier {
    pub rid: &'static str,
    pub scale: f32,
    pub max_bitrate: u32,
}

/// The default 3-layer simulcast ladder used for the main camera track.
///
/// Ordered LOWâ†’HIGH because the spec (and Chrome) expects `sendEncodings`
/// ordered from the lowest-quality layer to the highest. The rids `q`/`h`/`f`
/// (quarter / half / full) are the conventional libwebrtc names that MediaMTX
/// and most SFUs recognise. Bitrate caps are conservative so a teacher on a
/// modest uplink can still sustain all three layers:
///
///   * `q` â€” 1/4 resolution, 150 kbps  (thumbnail / poor-network viewers)
///   * `h` â€” 1/2 resolution, 500 kbps  (default for most viewers)
///   * `f` â€” full resolution, 1.5 Mbps (full-screen / good-network viewers)
/// The video codec the SFU can actually remux, in `RTCRtpCodecCapability.mimeType`
/// form (the comparison is case-insensitive).
pub const PREFERRED_VIDEO_CODEC: &str = "video/H264";

/// Reorder a list of codec mime types so every H264 entry comes first, keeping
/// the browser's relative order within each group.
///
/// **Why the publisher must state a preference at all:** HLS and fMP4 cannot
/// carry VP8, and Chrome offers VP8 first. Published as VP8, MediaMTX ingests
/// the stream fine over WebRTC but silently drops the video everywhere else --
/// the HLS master playlist comes back AUDIO-ONLY (measured: `CODECS="opus"`
/// with no video variant) and the recorder logs `skipping track (VP8)`, so the
/// HLS fallback player and the session recording both lose the picture. With
/// H264 the same publish yields `CODECS="avc1.42c01f,opus"` and a real
/// `video1_stream.m3u8`.
///
/// Returns indices into `mime_types` rather than reordering values, so the
/// caller can reorder the matching JS capability objects. Pure, so the ordering
/// rule is unit-testable without `web_sys`.
pub fn h264_first_order(mime_types: &[String]) -> Vec<usize> {
    let is_h264 = |m: &String| m.eq_ignore_ascii_case(PREFERRED_VIDEO_CODEC);
    let preferred = mime_types.iter().enumerate().filter(|(_, m)| is_h264(m));
    let rest = mime_types.iter().enumerate().filter(|(_, m)| !is_h264(m));
    preferred.chain(rest).map(|(i, _)| i).collect()
}

/// Whether to actually OFFER the simulcast ladder to the SFU.
///
/// **Disabled, because MediaMTX cannot consume it.** MediaMTX 1.18.2 accepts a
/// RID simulcast offer and even mirrors `a=simulcast`/`a=rid` back in the
/// answer -- but it performs NO layer selection. It ingests each RID as an
/// INDEPENDENT track on the path (`stream is available and online, 3 tracks
/// (VP8, VP8, Opus)`), while a WHEP viewer's offer has exactly ONE video
/// m-line, so the viewer is bound to whichever VP8 track MediaMTX happened to
/// match first.
///
/// Chromium brings the ladder up bandwidth-limited, so the full-resolution
/// `f` layer routinely sends nothing at all. When that dead layer is the one
/// the viewer gets bound to, the student sees a permanently black video --
/// measured end-to-end in the real app: WHEP 201, `srcObject` attached,
/// `videoWidth` 0, `framesDecoded` 0, while the teacher was publishing fine.
/// Which layer wins depends on RTP arrival order inside MediaMTX's 2s
/// track-gather window, so the failure is INTERMITTENT.
///
/// Offering one encoding instead makes the path carry exactly one video track,
/// which is what MediaMTX and the WHEP viewer both expect, and it delivers the
/// full-resolution image rather than a quarter-res layer.
///
/// The ladder is deliberately kept below (`simulcast_tiers`, and the
/// `sendEncodings` plumbing) so a simulcast-aware SFU re-enables it by
/// flipping this one constant.
pub const SIMULCAST_ENABLED: bool = false;

/// The encodings actually offered on the video transceiver.
///
/// With simulcast enabled this is the full ladder, lowest layer first (the
/// order the spec and Chrome expect). With it disabled it is the TOP tier
/// alone, so the publisher sends one full-resolution stream and keeps the
/// bitrate cap. Pure, so the choice is unit-testable without `web_sys`.
pub fn send_encoding_plan() -> Vec<SimulcastTier> {
    if SIMULCAST_ENABLED {
        simulcast_tiers().to_vec()
    } else {
        // The top tier is the full-resolution one (scale 1.0).
        vec![simulcast_tiers()[2]]
    }
}

pub fn simulcast_tiers() -> [SimulcastTier; 3] {
    [
        SimulcastTier {
            rid: "q",
            scale: 4.0,
            max_bitrate: 150_000,
        },
        SimulcastTier {
            rid: "h",
            scale: 2.0,
            max_bitrate: 500_000,
        },
        SimulcastTier {
            rid: "f",
            scale: 1.0,
            max_bitrate: 1_500_000,
        },
    ]
}

#[cfg(target_arch = "wasm32")]
mod imp {
    use wasm_bindgen::{JsCast, JsValue};
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{
        MediaStream, RtcConfiguration, RtcIceServer, RtcPeerConnection, RtcRtpEncodingParameters,
        RtcRtpTransceiver, RtcRtpTransceiverDirection, RtcRtpTransceiverInit, RtcSdpType,
        RtcSessionDescriptionInit,
    };

    /// Public STUN servers used for ICE candidate gathering so media can
    /// traverse common NATs. TURN relays (for symmetric NATs) come later;
    /// extend this list when they land.
    const STUN_URLS: &[&str] = &[
        "stun:stun.l.google.com:19302",
        "stun:stun1.l.google.com:19302",
    ];

    /// Build an `RtcConfiguration` populated with the public STUN servers in
    /// `STUN_URLS`. Each server's `urls` is a JS array of stun URLs.
    fn rtc_config_with_stun() -> RtcConfiguration {
        let cfg = RtcConfiguration::new();
        let servers = js_sys::Array::new();

        let installed = crate::live_room_whip::current_ice_servers();
        if installed.is_empty() {
            // Fallback: hard-coded public STUN (unchanged behavior).
            let urls = js_sys::Array::new();
            for u in STUN_URLS {
                urls.push(&wasm_bindgen::JsValue::from_str(u));
            }
            let ice_server = RtcIceServer::new();
            ice_server.set_urls(&urls);
            servers.push(&ice_server);
        } else {
            for entry in &installed {
                let urls = js_sys::Array::new();
                for u in &entry.urls {
                    urls.push(&wasm_bindgen::JsValue::from_str(u));
                }
                let ice_server = RtcIceServer::new();
                ice_server.set_urls(&urls);
                if let Some(user) = &entry.username {
                    ice_server.set_username(user);
                }
                if let Some(cred) = &entry.credential {
                    ice_server.set_credential(cred);
                }
                servers.push(&ice_server);
            }
        }
        cfg.set_ice_servers(&servers);
        cfg
    }

    /// Whether this engine exposes `RTCPeerConnection.addTransceiver`, which we
    /// need to attach `sendEncodings` (simulcast) to the video sender. All
    /// evergreen browsers have it; the check keeps us safe on exotic / old
    /// webviews where only the legacy `addTrack` exists, in which case the
    /// caller publishes a single encoding.
    fn supports_add_transceiver(pc: &RtcPeerConnection) -> bool {
        js_sys::Reflect::has(pc, &JsValue::from_str("addTransceiver")).unwrap_or(false)
    }

    /// Build the JS `sendEncodings` array from the pure tier table. Each entry
    /// is an `RTCRtpEncodingParameters` with rid + scaleResolutionDownBy +
    /// maxBitrate + active=true. Kept here (wasm side) because it touches
    /// `web_sys`; the tier *values* live in the pure `simulcast_tiers()`.
    fn build_send_encodings() -> js_sys::Array {
        let arr = js_sys::Array::new();
        let plan = crate::live_room_whip::send_encoding_plan();
        // A single-encoding offer must NOT carry a rid: a lone `a=rid` still
        // makes Chrome emit the simulcast/rid attributes, which is exactly the
        // multi-track shape MediaMTX mishandles. rids are only meaningful when
        // there is more than one layer to distinguish.
        let multi = plan.len() > 1;
        for tier in plan {
            let enc = RtcRtpEncodingParameters::new();
            if multi {
                enc.set_rid(tier.rid);
            }
            enc.set_scale_resolution_down_by(tier.scale);
            enc.set_max_bitrate(tier.max_bitrate);
            enc.set_active(true);
            arr.push(&enc);
        }
        arr
    }

    /// Add the camera/video `track` to `pc` as a **send-only simulcast
    /// transceiver** so the SFU (MediaMTX) can forward whichever spatial layer
    /// each viewer needs. Falls back to a plain single-encoding `addTrack` when
    /// `addTransceiver` is unavailable, so video always publishes.
    ///
    /// The transceiver is associated with `stream` (via `streams`) so the
    /// outgoing SDP carries the stream id, matching the previous `add_track`
    /// behaviour that the answer/WHEP side relies on.
    fn add_video_with_simulcast(
        pc: &RtcPeerConnection,
        track: &web_sys::MediaStreamTrack,
        stream: &MediaStream,
    ) {
        if !supports_add_transceiver(pc) {
            // Capability fallback: single encoding via the legacy path.
            pc.add_track_0(track, stream);
            return;
        }
        let init = RtcRtpTransceiverInit::new();
        // Publishers only send; never receive on this m-line.
        init.set_direction(RtcRtpTransceiverDirection::Sendonly);
        init.set_send_encodings(&build_send_encodings());
        let streams = js_sys::Array::new();
        streams.push(stream);
        init.set_streams(&streams);
        // `addTransceiver` does not throw for valid args. Keep the returned
        // transceiver: it is the only handle that can state a codec preference
        // before the offer is created.
        let transceiver = pc.add_transceiver_with_media_stream_track_and_init(track, &init);
        prefer_remuxable_video_codec(&transceiver);
    }

    /// Offer H264 ahead of every other video codec on this transceiver.
    ///
    /// Must run BEFORE `create_offer`. A no-op when the engine exposes no H264
    /// (publishing then continues exactly as before, just without HLS video) or
    /// when `setCodecPreferences` is unavailable, so this can only improve the
    /// negotiated result -- it never blocks a publish. Every non-H264 codec is
    /// retained behind H264, so a peer without H264 still negotiates VP8.
    fn prefer_remuxable_video_codec(transceiver: &RtcRtpTransceiver) {
        let Some(caps) = web_sys::RtcRtpSender::get_capabilities("video") else {
            return;
        };
        // `RtcRtpCapabilities` is bound as a DICTIONARY in web-sys: it exposes
        // builder setters (`codecs(&mut self, val)`), not getters, so the codec
        // list has to be read reflectively rather than via `caps.codecs()`.
        let Ok(codecs) = js_sys::Reflect::get(&caps, &JsValue::from_str("codecs")) else {
            return;
        };
        let Ok(codecs) = codecs.dyn_into::<js_sys::Array>() else {
            return;
        };
        let mimes: Vec<String> = (0..codecs.length())
            .map(|i| {
                js_sys::Reflect::get(&codecs.get(i), &JsValue::from_str("mimeType"))
                    .ok()
                    .and_then(|v| v.as_string())
                    .unwrap_or_default()
            })
            .collect();
        // Nothing to prefer: leave the browser's own ordering untouched.
        if !mimes
            .iter()
            .any(|m| m.eq_ignore_ascii_case(crate::live_room_whip::PREFERRED_VIDEO_CODEC))
        {
            return;
        }
        let ordered = js_sys::Array::new();
        for i in crate::live_room_whip::h264_first_order(&mimes) {
            ordered.push(&codecs.get(i as u32));
        }
        let _ = transceiver.set_codec_preferences(ordered.as_ref());
    }

    pub struct WhipPublisher {
        pub pc: RtcPeerConnection,
        pub resource_url: Option<String>,
        closed: bool,
    }

    pub async fn publish(
        whip_url: &str,
        password: &str,
        local_stream: &MediaStream,
    ) -> Result<WhipPublisher, String> {
        let cfg = rtc_config_with_stun();
        let pc = RtcPeerConnection::new_with_configuration(&cfg)
            .map_err(|e| format!("RtcPeerConnection: {e:?}"))?;
        let tracks = local_stream.get_tracks();
        for i in 0..tracks.length() {
            let track = tracks.get(i);
            let track: web_sys::MediaStreamTrack =
                track.dyn_into().map_err(|_| "track cast".to_string())?;
            // Video â†’ publish as a simulcast transceiver (multiple spatial
            // layers the SFU can forward selectively). Audio (and any future
            // non-video kind) â†’ plain add_track, which is what it has always
            // been. `add_video_with_simulcast` falls back to add_track_0 when
            // the browser/SFU can't take send_encodings, so video always flows.
            if track.kind() == "video" {
                add_video_with_simulcast(&pc, &track, local_stream);
            } else {
                pc.add_track_0(&track, local_stream);
            }
        }

        let offer = JsFuture::from(pc.create_offer())
            .await
            .map_err(|e| format!("create_offer: {e:?}"))?;
        // create_offer() resolves to an RTCSessionDescriptionInit *dictionary*
        // (a plain {type, sdp} object), not a class instance, so dyn_into()'s
        // instanceof check fails ("offer cast"). The value is known to be the
        // offer, so cast unchecked.
        let offer: RtcSessionDescriptionInit = offer.unchecked_into();
        JsFuture::from(pc.set_local_description(&offer))
            .await
            .map_err(|e| format!("set_local_description: {e:?}"))?;

        // WHIP is a ONE-SHOT exchange: the SDP POSTed below is the only
        // opportunity to tell MediaMTX where to send its connectivity checks,
        // and `set_local_description` resolves BEFORE gathering has produced
        // anything. Wait for gathering to complete so the offer carries every
        // host / server-reflexive / RELAY candidate. Without this the offer
        // ships with zero candidates (measured), and the session can only
        // connect when the browser reaches one of MediaMTX's own candidates
        // directly and MediaMTX learns ours as peer-reflexive -- which is why a
        // cross-network teacher died with MediaMTX's
        // "deadline exceeded while waiting connection".
        //
        // TURN/STUN configuration still comes from current_ice_servers(); this
        // wait is what lets those relay candidates actually reach the server.
        let gathered = crate::live_room_ice::await_ice_gathering(
            &pc,
            crate::live_room_ice::ICE_GATHERING_TIMEOUT_MS,
        )
        .await;

        // Typed read of the local description (per design Q1-C / typed SDP
        // section). `pc.local_description()` returns `Option<RtcSessionDescription>`
        // and `.sdp()` is a typed accessor â€” no Reflect.get fallback needed.
        let local_sdp = pc
            .local_description()
            .ok_or_else(|| "no local SDP".to_string())?
            .sdp();

        // A gathering timeout means a STUN/TURN server did not answer in time;
        // a zero-candidate offer means the session can only work same-LAN.
        // Logging both turns "the stream is just black" into a diagnosable
        // event instead of a silent degradation.
        let candidates = crate::live_room_ice::candidate_count(&local_sdp);
        if !gathered || candidates == 0 {
            web_sys::console::warn_1(
                &format!(
                    "[live_room_whip] publishing an incomplete offer: gathering_complete={gathered} candidates={candidates}"
                )
                .into(),
            );
        }

        let init = web_sys::RequestInit::new();
        init.set_method("POST");
        init.set_body(&wasm_bindgen::JsValue::from_str(&local_sdp));
        let headers = web_sys::Headers::new().map_err(|e| format!("headers: {e:?}"))?;
        headers
            .set("Content-Type", "application/sdp")
            .map_err(|e| format!("set Content-Type: {e:?}"))?;
        if !password.is_empty() {
            use base64::Engine;
            let token = base64::engine::general_purpose::STANDARD.encode(format!(":{password}"));
            headers
                .set("Authorization", &format!("Basic {token}"))
                .map_err(|e| format!("set Authorization: {e:?}"))?;
        }
        init.set_headers(&headers);
        let req = web_sys::Request::new_with_str_and_init(whip_url, &init)
            .map_err(|e| format!("Request: {e:?}"))?;
        let win = web_sys::window().ok_or_else(|| "no window".to_string())?;
        let resp_value = JsFuture::from(win.fetch_with_request(&req))
            .await
            .map_err(|e| format!("fetch: {e:?}"))?;
        let resp: web_sys::Response = resp_value
            .dyn_into()
            .map_err(|_| "response cast".to_string())?;
        if !resp.ok() {
            return Err(format!("WHIP returned {}", resp.status()));
        }
        // MediaMTX answers with a RELATIVE Location (measured:
        // `/aula/<tenant>/<course>/<session>/whip/<uuid>`). Resolving it
        // against `whip_url` keeps teardown pointed at the MEDIA host -- handed
        // to `fetch()` raw it would resolve against the PAGE origin, so the
        // DELETE never reached MediaMTX and publisher sessions leaked until the
        // next publish evicted them with "closing existing publisher".
        let location = resp
            .headers()
            .get("Location")
            .ok()
            .flatten()
            .and_then(|loc| crate::live_room_ice::resolve_resource_url(whip_url, &loc));
        let answer_text_promise = resp.text().map_err(|e| format!("body text: {e:?}"))?;
        let answer_text_value = JsFuture::from(answer_text_promise)
            .await
            .map_err(|e| format!("body await: {e:?}"))?;
        let answer_sdp = answer_text_value
            .as_string()
            .ok_or_else(|| "non-string SDP".to_string())?;

        let answer_init = RtcSessionDescriptionInit::new(RtcSdpType::Answer);
        answer_init.set_sdp(&answer_sdp);
        JsFuture::from(pc.set_remote_description(&answer_init))
            .await
            .map_err(|e| format!("set_remote_description: {e:?}"))?;

        let mut publisher = WhipPublisher {
            pc,
            resource_url: location,
            closed: false,
        };

        // A 201 only proves the SDP was accepted. ICE/DTLS can still fail
        // afterwards, and until now that failure was invisible: publish()
        // returned Ok, the teacher's UI said "live", and viewers got
        // `404 no stream is available on path ...`. Verify the connection
        // really came up, and release the server-side session when it did not
        // so MediaMTX is not left holding a dead publisher.
        if let Err(e) = crate::live_room_ice::await_connected(
            &publisher.pc,
            crate::live_room_ice::CONNECT_TIMEOUT_MS,
        )
        .await
        {
            let _ = publisher.close().await;
            return Err(format!("WHIP publish did not connect: {e}"));
        }

        Ok(publisher)
    }

    impl WhipPublisher {
        /// Swap the outbound **video** track without renegotiating SDP.
        ///
        /// Finds the `RtcRtpSender` currently carrying a video track and calls
        /// `replaceTrack(newTrack)`. This is how in-call camera switching (and
        /// background-FX toggling, which produces a fresh canvas track) avoids
        /// a full WHIP re-publish: the transceiver / SSRC stays put, only the
        /// source changes. Returns `Err` when there is no video sender yet.
        ///
        /// The old track is intentionally NOT stopped here: when switching FX
        /// on/off the caller may want to keep the raw camera track alive to
        /// switch back to. Device switching stops the prior track at the call
        /// site after the swap succeeds.
        pub async fn replace_video_track(
            &self,
            track: &web_sys::MediaStreamTrack,
        ) -> Result<(), String> {
            let sender = self
                .video_sender()
                .ok_or_else(|| "no video sender on this publisher".to_string())?;
            JsFuture::from(sender.replace_track(Some(track)))
                .await
                .map_err(|e| format!("replace_track: {e:?}"))?;
            Ok(())
        }

        /// The `RtcRtpSender` whose current track is a video track, if any.
        /// Walks `getSenders()` and matches on `track.kind() == "video"`.
        fn video_sender(&self) -> Option<web_sys::RtcRtpSender> {
            let senders = self.pc.get_senders();
            for i in 0..senders.length() {
                if let Ok(sender) = senders.get(i).dyn_into::<web_sys::RtcRtpSender>() {
                    if let Some(track) = sender.track() {
                        if track.kind() == "video" {
                            return Some(sender);
                        }
                    }
                }
            }
            None
        }

        /// Poll the underlying peer connection's `getStats()` and reduce it to
        /// a coarse `NetQuality`. Thin accessor so the broadcast view can show
        /// a publisher-side network-quality badge without reaching into `pc`.
        pub async fn network_quality(&self) -> crate::live_room_stats::NetQuality {
            crate::live_room_stats::probe(&self.pc).await
        }

        /// Release every resource owned by this publisher:
        ///
        ///   1. Stop every local `MediaStreamTrack` so the browser releases
        ///      the camera / microphone indicator.
        ///   2. `RTCPeerConnection.close()` so the ICE / DTLS stack tears down.
        ///   3. `DELETE resource_url` so the WHIP server frees the session.
        ///
        /// Idempotent â€” second call is a no-op. Errors during DELETE are
        /// downgraded to `Ok(())` because there is nothing meaningful the
        /// caller can do (we have already released the local resources).
        pub async fn close(&mut self) -> Result<(), String> {
            if self.closed {
                return Ok(());
            }
            self.closed = true;

            // 1. Stop every track attached to this peer connection. We walk
            // the senders because the `MediaStream` is not stored on the
            // publisher; senders are the canonical handle the PC exposes.
            let senders = self.pc.get_senders();
            for i in 0..senders.length() {
                let sender_val = senders.get(i);
                if let Ok(sender) = sender_val.dyn_into::<web_sys::RtcRtpSender>() {
                    if let Some(track) = sender.track() {
                        track.stop();
                    }
                }
            }

            // 2. Close the peer connection.
            self.pc.close();

            // 3. Best-effort DELETE on the WHIP resource URL.
            if let Some(url) = self.resource_url.take() {
                let init = web_sys::RequestInit::new();
                init.set_method("DELETE");
                if let Some(win) = web_sys::window() {
                    if let Ok(req) = web_sys::Request::new_with_str_and_init(&url, &init) {
                        let _ = JsFuture::from(win.fetch_with_request(&req)).await;
                    }
                }
            }
            Ok(())
        }
    }

    impl Drop for WhipPublisher {
        /// Best-effort cleanup on Drop. Stops local tracks and closes the PC
        /// synchronously, then spawns the async DELETE so the server learns
        /// about the close. The spawned task is not awaited â€” if the page is
        /// closing the browser will cancel it, which is fine: the local
        /// resources are already released.
        fn drop(&mut self) {
            if self.closed {
                return;
            }
            self.closed = true;

            // Stop tracks + close PC inline.
            let senders = self.pc.get_senders();
            for i in 0..senders.length() {
                let sender_val = senders.get(i);
                if let Ok(sender) = sender_val.dyn_into::<web_sys::RtcRtpSender>() {
                    if let Some(track) = sender.track() {
                        track.stop();
                    }
                }
            }
            self.pc.close();

            // Best-effort DELETE in a detached task.
            if let Some(url) = self.resource_url.take() {
                wasm_bindgen_futures::spawn_local(async move {
                    let init = web_sys::RequestInit::new();
                    init.set_method("DELETE");
                    if let Some(win) = web_sys::window() {
                        if let Ok(req) = web_sys::Request::new_with_str_and_init(&url, &init) {
                            let _ = JsFuture::from(win.fetch_with_request(&req)).await;
                        }
                    }
                });
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod imp {
    pub struct WhipPublisher {
        key: String,
        element_id: Option<String>,
        closed: bool,
    }

    impl WhipPublisher {
        pub(crate) fn native(key: impl Into<String>, element_id: Option<String>) -> Self {
            Self {
                key: key.into(),
                element_id,
                closed: false,
            }
        }
    }

    pub async fn publish(_w: &str, _p: &str, _ls: &()) -> Result<WhipPublisher, String> {
        Err("native WHIP requires a WebView-owned media stream".into())
    }
    impl WhipPublisher {
        pub async fn close(&mut self) -> Result<(), String> {
            if self.closed {
                return Ok(());
            }
            self.closed = true;
            crate::live_room_native::stop_publisher(&self.key, self.element_id.as_deref()).await
        }
        pub async fn replace_video_track(&self, _track: &()) -> Result<(), String> {
            Err("use the native camera-switch bridge with a device id".into())
        }
        pub async fn network_quality(&self) -> crate::live_room_stats::NetQuality {
            crate::live_room_native::publisher_quality(&self.key).await
        }
    }
    // Async resource teardown is performed explicitly by room/session
    // lifecycle code; `live_room_native::close_all` is the route-drop safety
    // net because Rust Drop cannot await the WebView cleanup response.
}

pub use imp::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simulcast_ladder_is_three_layers_low_to_high() {
        let tiers = simulcast_tiers();
        assert_eq!(tiers.len(), 3);
        // rids match the conventional libwebrtc quarter/half/full names.
        assert_eq!(tiers[0].rid, "q");
        assert_eq!(tiers[1].rid, "h");
        assert_eq!(tiers[2].rid, "f");
    }

    #[test]
    fn simulcast_scale_decreases_as_quality_increases() {
        let tiers = simulcast_tiers();
        // scaleResolutionDownBy must shrink monotonically toward 1.0 (full res)
        // as we go from the lowest to the highest layer.
        assert!(tiers[0].scale > tiers[1].scale);
        assert!(tiers[1].scale > tiers[2].scale);
        assert_eq!(tiers[2].scale, 1.0, "top layer must be full resolution");
    }

    #[test]
    fn simulcast_bitrate_increases_with_quality() {
        let tiers = simulcast_tiers();
        // Bitrate caps grow with resolution so the SFU has a meaningful ladder.
        assert!(tiers[0].max_bitrate < tiers[1].max_bitrate);
        assert!(tiers[1].max_bitrate < tiers[2].max_bitrate);
    }

    #[test]
    fn h264_is_ordered_ahead_of_every_other_codec() {
        let mimes: Vec<String> = ["video/VP8", "video/rtx", "video/H264", "video/VP9"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        // H264 first, then the rest in their original relative order.
        assert_eq!(h264_first_order(&mimes), vec![2, 0, 1, 3]);
    }

    #[test]
    fn h264_matching_is_case_insensitive() {
        let mimes: Vec<String> = ["video/VP8", "video/h264"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(h264_first_order(&mimes), vec![1, 0]);
    }

    #[test]
    fn every_codec_survives_the_reorder() {
        // Nothing may be dropped: a peer without H264 must still find VP8.
        let mimes: Vec<String> = ["video/VP8", "video/H264", "video/AV1", "video/H264"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let order = h264_first_order(&mimes);
        assert_eq!(order.len(), mimes.len());
        let mut sorted = order.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, vec![0, 1, 2, 3], "reorder must be a permutation");
        // Both H264 entries lead.
        assert_eq!(&order[..2], &[1, 3]);
    }

    #[test]
    fn order_is_stable_when_no_h264_is_present() {
        let mimes: Vec<String> = ["video/VP8", "video/VP9"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(h264_first_order(&mimes), vec![0, 1]);
    }

    #[test]
    fn send_encoding_plan_is_a_single_full_resolution_layer_by_default() {
        // MediaMTX does no layer selection, so offering the ladder produces a
        // multi-VP8 path that a one-m-line WHEP viewer cannot reliably consume.
        assert!(!SIMULCAST_ENABLED, "default must stay off against MediaMTX");
        let plan = send_encoding_plan();
        assert_eq!(plan.len(), 1, "one encoding => one video track on the path");
        assert_eq!(plan[0].scale, 1.0, "viewers must get full resolution");
        assert_eq!(plan[0].max_bitrate, 1_500_000);
    }

    #[test]
    fn send_encoding_plan_still_exposes_the_full_ladder() {
        // The ladder itself must remain intact so a simulcast-capable SFU can
        // be enabled by flipping SIMULCAST_ENABLED alone.
        let tiers = simulcast_tiers();
        assert_eq!(tiers.len(), 3);
        assert_eq!(
            send_encoding_plan().len(),
            if SIMULCAST_ENABLED { 3 } else { 1 }
        );
    }

    #[test]
    fn simulcast_rids_are_unique() {
        let tiers = simulcast_tiers();
        for i in 0..tiers.len() {
            for j in (i + 1)..tiers.len() {
                assert_ne!(tiers[i].rid, tiers[j].rid, "rids must be distinct");
            }
        }
    }
}



