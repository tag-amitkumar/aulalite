// crates/features-courses/src/live_room_whep.rs
//! WHEP (WebRTC HTTP Egress) client. Symmetric to WHIP but for receivers.

/// WHEP reuses the WHIP module's ICE registry so a single `set_ice_servers`
/// call configures both publish and view peers. Re-export the type + setter for
/// ergonomic call sites (`live_room_whep::set_ice_servers(...)`).
pub use crate::live_room_whip::{set_ice_servers, IceServerConfig};

#[cfg(target_arch = "wasm32")]
mod imp {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{
        MediaStream, RtcConfiguration, RtcIceServer, RtcPeerConnection, RtcSdpType,
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

    pub struct WhepViewer {
        pub pc: RtcPeerConnection,
        pub remote_stream: MediaStream,
        pub resource_url: Option<String>,
        closed: bool,
    }

    /// Subscribe, waiting for the publisher's path to become readable.
    ///
    /// Use this wherever a publisher is EXPECTED (the teacher's main feed, a
    /// promoted student): MediaMTX only makes a path readable up to
    /// `webrtcTrackGatherTimeout` after the publisher's ICE connects, so an
    /// immediate 404 there means "not yet", not "never".
    pub async fn view(whep_url: &str, viewer_jwt: &str) -> Result<WhepViewer, String> {
        view_inner(
            whep_url,
            viewer_jwt,
            crate::live_room_ice::PATH_READY_TIMEOUT_MS,
        )
        .await
    }

    /// Subscribe with NO readiness wait: a 404 fails immediately.
    ///
    /// Use this for SPECULATIVE attaches where "no publisher" is the normal
    /// case -- the screen-share feed is probed on every join even though the
    /// teacher is usually not sharing. Waiting 20s there would burn ~40 futile
    /// requests per student join against the media host and tell us nothing.
    pub async fn view_once(whep_url: &str, viewer_jwt: &str) -> Result<WhepViewer, String> {
        view_inner(whep_url, viewer_jwt, 0).await
    }

    async fn view_inner(
        whep_url: &str,
        viewer_jwt: &str,
        ready_timeout_ms: u32,
    ) -> Result<WhepViewer, String> {
        let cfg = rtc_config_with_stun();
        let pc = RtcPeerConnection::new_with_configuration(&cfg)
            .map_err(|e| format!("RtcPeerConnection: {e:?}"))?;
        // See the matching guard in live_room_whip::publish: closes `pc` if any
        // `?` below bails out before the WhepViewer takes ownership.
        let pc_guard = crate::live_room_ice::PcCloseGuard::new(pc.clone());

        let remote_stream = MediaStream::new().map_err(|e| format!("MediaStream::new: {e:?}"))?;

        let t_init_video = web_sys::RtcRtpTransceiverInit::new();
        t_init_video.set_direction(web_sys::RtcRtpTransceiverDirection::Recvonly);
        let _ = pc.add_transceiver_with_str_and_init("video", &t_init_video);
        let t_init_audio = web_sys::RtcRtpTransceiverInit::new();
        t_init_audio.set_direction(web_sys::RtcRtpTransceiverDirection::Recvonly);
        let _ = pc.add_transceiver_with_str_and_init("audio", &t_init_audio);

        // NOT `remote_stream.clone()`: that calls the DOM's MediaStream.clone()
        // and yields a detached copy, so every received track landed on a
        // stream nobody rendered. See live_room_ice::same_stream.
        let remote_stream_clone = crate::live_room_ice::same_stream(&remote_stream);
        let on_track_cb = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::RtcTrackEvent)>::new(
            move |evt: web_sys::RtcTrackEvent| {
                let _ = remote_stream_clone.add_track(&evt.track());
            },
        );
        pc.set_ontrack(Some(on_track_cb.as_ref().unchecked_ref()));
        on_track_cb.forget();

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
        // Same one-shot constraint as WHIP: this POST is the only chance to
        // hand MediaMTX our candidates, and `set_local_description` resolves
        // before any have been gathered. Reading the SDP immediately (as this
        // did) produced an offer with ZERO candidates, so a viewer could only
        // ever connect when it reached one of MediaMTX's own candidates
        // directly -- a viewer needing a TURN relay had no viable pair at all,
        // which is the "201 on WHEP, black video" failure.
        let gathered = crate::live_room_ice::await_ice_gathering(
            &pc,
            crate::live_room_ice::ICE_GATHERING_TIMEOUT_MS,
        )
        .await;

        // Typed read of the local description (no Reflect.get fallback).
        let local_sdp = pc
            .local_description()
            .ok_or_else(|| "no local SDP".to_string())?
            .sdp();

        let candidates = crate::live_room_ice::candidate_count(&local_sdp);
        if !gathered || candidates == 0 {
            web_sys::console::warn_1(
                &format!(
                    "[live_room_whep] subscribing with an incomplete offer: gathering_complete={gathered} candidates={candidates}"
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
            .map_err(|e| format!("set ct: {e:?}"))?;
        // Send the viewer JWT as HTTP Basic auth (empty user, password = JWT),
        // matching the WHIP publisher. MediaMTX forwards the Basic password
        // into the auth-callback `password` field, which the backend read path
        // accepts as a raw JWT. A `Bearer` header is NOT forwarded the same
        // way by MediaMTX, which left WHEP reads unauthenticated (403/401).
        use base64::Engine;
        let basic = base64::engine::general_purpose::STANDARD.encode(format!(":{viewer_jwt}"));
        headers
            .set("Authorization", &format!("Basic {basic}"))
            .map_err(|e| format!("set authz: {e:?}"))?;
        init.set_headers(&headers);
        let win = web_sys::window().ok_or_else(|| "no window".to_string())?;

        // A 404 from MediaMTX means "no stream is available on path ..." --
        // i.e. the publisher has not been gathered YET, not that it will never
        // arrive. The teacher's path only becomes readable up to
        // `webrtcTrackGatherTimeout` AFTER their ICE connects, so a viewer who
        // joins as the class goes live reliably loses that race. Asking once
        // and giving up is exactly the reported "WHEP returned 404" against a
        // teacher who is in fact publishing correctly.
        //
        // Every other status is terminal: 401/403 are auth failures that
        // retrying cannot fix (and which previously surfaced promptly), so
        // only 404 is retried. `Request` consumes its body, so each attempt
        // needs a fresh one; the SDP offer itself stays valid.
        // A zero budget collapses to exactly one attempt.
        let max_attempts = (ready_timeout_ms / crate::live_room_ice::PATH_READY_RETRY_MS).max(1);
        let mut attempt: u32 = 0;
        let resp: web_sys::Response = loop {
            let req = web_sys::Request::new_with_str_and_init(whep_url, &init)
                .map_err(|e| format!("Request: {e:?}"))?;
            let resp_value = JsFuture::from(win.fetch_with_request(&req))
                .await
                .map_err(|e| format!("fetch: {e:?}"))?;
            let candidate: web_sys::Response = resp_value
                .dyn_into()
                .map_err(|_| "response cast".to_string())?;
            if candidate.ok() {
                break candidate;
            }
            if candidate.status() != 404 || attempt + 1 >= max_attempts {
                return Err(format!("WHEP returned {}", candidate.status()));
            }
            if attempt == 0 {
                web_sys::console::log_1(
                    &"[live_room_whep] path not ready yet (404); waiting for the publisher".into(),
                );
            }
            attempt += 1;
            gloo_timers::future::TimeoutFuture::new(crate::live_room_ice::PATH_READY_RETRY_MS)
                .await;
        };
        // Relative Location, as on the WHIP side: resolve against the request
        // URL so the teardown DELETE reaches the media host rather than the
        // page origin, and MediaMTX frees the egress session.
        let location = resp
            .headers()
            .get("Location")
            .ok()
            .flatten()
            .and_then(|loc| crate::live_room_ice::resolve_resource_url(whep_url, &loc));
        let answer_promise = resp.text().map_err(|e| format!("body text: {e:?}"))?;
        let answer_value = JsFuture::from(answer_promise)
            .await
            .map_err(|e| format!("body await: {e:?}"))?;
        let answer_sdp = answer_value
            .as_string()
            .ok_or_else(|| "non-string SDP".to_string())?;
        let answer_init = RtcSessionDescriptionInit::new(RtcSdpType::Answer);
        answer_init.set_sdp(&answer_sdp);
        JsFuture::from(pc.set_remote_description(&answer_init))
            .await
            .map_err(|e| format!("set_remote_description: {e:?}"))?;

        let mut viewer = WhepViewer {
            pc,
            remote_stream,
            resource_url: location,
            closed: false,
        };

        // Turn a silent black video into a reportable error. The call site
        // (live_room_view.rs) already surfaces Err as "Couldn't connect to the
        // video stream", so failing here is strictly better than handing back
        // a viewer whose media will never arrive. Release the server-side
        // egress session on failure.
        // The viewer owns the connection from here.
        pc_guard.disarm();

        if let Err(e) = crate::live_room_ice::await_connected(&viewer.pc).await {
            let _ = viewer.close().await;
            return Err(format!("WHEP subscribe did not connect: {e}"));
        }

        Ok(viewer)
    }

    impl WhepViewer {
        /// Mirror of `WhipPublisher::close`. Stops every track on the remote
        /// stream, closes the peer connection, and DELETEs the WHEP resource
        /// URL so the server frees the egress. Idempotent.
        pub async fn close(&mut self) -> Result<(), String> {
            if self.closed {
                return Ok(());
            }
            self.closed = true;

            // 1. Stop every remote track. The browser keeps the receiver
            // socket alive until the track itself is stopped, so closing the
            // PC is not enough on its own.
            let tracks = self.remote_stream.get_tracks();
            for i in 0..tracks.length() {
                if let Ok(track) = tracks.get(i).dyn_into::<web_sys::MediaStreamTrack>() {
                    track.stop();
                }
            }

            // 2. Close the peer connection.
            self.pc.close();

            // 3. Best-effort DELETE on the WHEP resource URL.
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

    impl Drop for WhepViewer {
        /// Best-effort cleanup on Drop. Stops remote tracks and closes the
        /// PC synchronously, then spawns the async DELETE.
        fn drop(&mut self) {
            if self.closed {
                return;
            }
            self.closed = true;

            let tracks = self.remote_stream.get_tracks();
            for i in 0..tracks.length() {
                if let Ok(track) = tracks.get(i).dyn_into::<web_sys::MediaStreamTrack>() {
                    track.stop();
                }
            }
            self.pc.close();

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
    pub struct WhepViewer {
        key: String,
        element_id: String,
        closed: bool,
    }

    /// Native has no readiness loop of its own -- the WebView bridge owns
    /// retry -- so `view_once` is the same call. Kept so cross-target call
    /// sites do not need `cfg` branches.
    pub async fn view_once(url: &str, jwt: &str) -> Result<WhepViewer, String> {
        view(url, jwt).await
    }

    pub async fn view(url: &str, jwt: &str) -> Result<WhepViewer, String> {
        let key = format!("viewer-{}", uuid::Uuid::new_v4());
        let element_id = "live-room-main-video".to_string();
        crate::live_room_native::view(crate::live_room_native::ViewRequest {
            key: key.clone(),
            url: url.to_string(),
            password: jwt.to_string(),
            element_id: element_id.clone(),
            ice_servers: Vec::new(),
        })
        .await?;
        Ok(WhepViewer {
            key,
            element_id,
            closed: false,
        })
    }

    impl WhepViewer {
        pub async fn close(&mut self) -> Result<(), String> {
            if self.closed {
                return Ok(());
            }
            self.closed = true;
            crate::live_room_native::stop_viewer(&self.key, Some(&self.element_id)).await
        }
    }
    // Native route/session lifecycle performs async close explicitly. The
    // shared WebView runtime's `close_all` is the route-drop safety net.
}

pub use imp::*;
