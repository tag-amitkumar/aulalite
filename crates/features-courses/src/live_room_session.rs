// crates/features-courses/src/live_room_session.rs
//! `LiveRoomSession` — owning aggregate for the per-route live-room
//! resources (WebSocket, WHIP publisher, WHEP viewer for the teacher feed,
//! and a map of WHEP viewers for promoted students).
//!
//! Wiring this aggregate into the route is the load-bearing step that stops
//! the live-room from leaking RTC peer connections, media tracks, and
//! WebSockets on every Go-Live and every route exit. The current screens
//! manage these connections individually inside `use_signal` / `use_effect`
//! and never release them.
//!
//! This module ships the aggregate. Tasks 18 / 19 / 20 wire it into the
//! socket, WHIP / WHEP clients, and the broadcast / view routes.
//!
//! Browser builds own typed WebRTC handles directly. Native builds keep those
//! media objects inside the WebView bridge, while this aggregate still owns
//! the room socket and invokes the same explicit route teardown contract.

use crate::api::ApiContext;
#[cfg(target_arch = "wasm32")]
use std::collections::HashMap;
use uuid::Uuid;

/// Minimal config the session needs to operate. Carries the session id,
/// the API origin (used to build the WebSocket URL), and the current
/// access token. Kept small so the host-side tests can construct one
/// without pulling in dioxus / wasm types.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionConfig {
    pub session_id: String,
    pub api_origin: String,
    pub access_token: String,
    pub viewer_jwt: Option<String>,
}

/// Build the WebSocket URL the client uses to connect to the live room.
/// Pure function — no I/O — so it is unit-testable on host.
///
/// Percent-encodes the access token so that JWTs containing `+`, `/`,
/// or `=` survive the URL boundary intact. The server (after the
/// auth-middleware change in the design doc) decodes the value with
/// `percent_encoding::percent_decode_str`.
pub fn build_ws_url(
    api_origin: &str,
    session_id: &str,
    access_token: &str,
    workspace_id: Option<&str>,
) -> String {
    let scheme = if api_origin.starts_with("https") {
        "wss"
    } else {
        "ws"
    };
    let host = api_origin
        .trim_start_matches("http://")
        .trim_start_matches("https://");
    let mut url = format!(
        "{scheme}://{host}/v1/sessions/{session_id}/socket?access_token={}",
        urlencoding::encode(access_token)
    );
    if let Some(workspace_id) = workspace_id.filter(|value| !value.trim().is_empty()) {
        url.push_str("&workspace_id=");
        url.push_str(&urlencoding::encode(workspace_id.trim()));
    }
    url
}

pub fn whep_url_for_path(api_origin: &str, path: &str) -> String {
    format!(
        "{}/{}/whep",
        api_origin.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}

// ---------------------------------------------------------------------------
// LiveRoomSession
// ---------------------------------------------------------------------------

/// Aggregate owning the per-route live-room resources.
///
/// Media object ownership is renderer-specific, but the lifecycle and socket
/// ownership surface is uniform.
pub struct LiveRoomSession {
    config: SessionConfig,
    api: ApiContext,
    socket: Option<crate::live_room_socket::conn::LiveRoomSocket>,
    #[cfg(target_arch = "wasm32")]
    publisher: Option<crate::live_room_whip::WhipPublisher>,
    #[cfg(target_arch = "wasm32")]
    screen_publisher: Option<crate::live_room_whip::WhipPublisher>,
    /// Student-side view of the teacher feed.
    #[cfg(target_arch = "wasm32")]
    viewer: Option<crate::live_room_whep::WhepViewer>,
    #[cfg(target_arch = "wasm32")]
    screen_viewer: Option<crate::live_room_whep::WhepViewer>,
    /// Promoted-student viewers, keyed by student user id.
    #[cfg(target_arch = "wasm32")]
    students: HashMap<Uuid, crate::live_room_whep::WhepViewer>,
    closed: bool,
}

impl LiveRoomSession {
    pub fn new(config: SessionConfig, api: ApiContext) -> Self {
        Self {
            config,
            api,
            socket: None,
            #[cfg(target_arch = "wasm32")]
            publisher: None,
            #[cfg(target_arch = "wasm32")]
            screen_publisher: None,
            #[cfg(target_arch = "wasm32")]
            viewer: None,
            #[cfg(target_arch = "wasm32")]
            screen_viewer: None,
            #[cfg(target_arch = "wasm32")]
            students: HashMap::new(),
            closed: false,
        }
    }

    pub fn config(&self) -> &SessionConfig {
        &self.config
    }

    pub fn api(&self) -> &ApiContext {
        &self.api
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }

    fn begin_close(&mut self) -> bool {
        if self.closed {
            false
        } else {
            self.closed = true;
            true
        }
    }

    /// Open the live-room WebSocket and start receiving events.
    ///
    /// The UI-level persistent driver adds token refresh and reconnect policy;
    /// this lower-level method replaces any previously owned connection.
    pub async fn connect_socket(
        &mut self,
        on_event: impl FnMut(crate::live_room_socket::ServerEvent) + 'static,
    ) -> Result<(), String> {
        let workspace_id = crate::api::selected_workspace_id();
        let url = build_ws_url(
            &self.config.api_origin,
            &self.config.session_id,
            &self.config.access_token,
            workspace_id.as_deref(),
        );
        let sock = crate::live_room_socket::conn::LiveRoomSocket::connect(&url, on_event)?;
        // Replace any prior socket; the old one drops on the next line.
        let old = self.socket.replace(sock);
        if let Some(mut s) = old {
            s.close();
        }
        Ok(())
    }

    /// Set the active WHIP publisher (handing ownership to the session so
    /// `close` can release it). Browser publish orchestration hands the live
    /// publisher over immediately after negotiation.
    #[cfg(target_arch = "wasm32")]
    pub fn set_publisher(&mut self, publisher: crate::live_room_whip::WhipPublisher) {
        self.publisher = Some(publisher);
    }

    /// Stop only the main camera/microphone publisher while keeping the room
    /// socket and any viewer state alive. This is intentionally narrower than
    /// [`Self::close`]: diagnostics can recover a failed WHIP publish without
    /// tearing down chat, presence, or forcing a full page reload.
    #[cfg(target_arch = "wasm32")]
    pub async fn stop_publishing(&mut self) {
        if let Some(mut publisher) = self.publisher.take() {
            let _ = publisher.close().await;
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub async fn stop_publishing(&mut self) {
        let _ = crate::live_room_native::stop_publisher("main", Some("broadcast-self-video")).await;
    }

    /// In-call camera switch: replace the publisher's outbound video track
    /// with `track` without renegotiating. Delegates to
    /// `WhipPublisher::replace_video_track`. Errors when no publisher is set
    /// (not yet live) or when the swap fails.
    #[cfg(target_arch = "wasm32")]
    pub async fn replace_main_video_track(
        &self,
        track: &web_sys::MediaStreamTrack,
    ) -> Result<(), String> {
        let publisher = self
            .publisher
            .as_ref()
            .ok_or_else(|| "no active publisher".to_string())?;
        publisher.replace_video_track(track).await
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub async fn replace_main_video_track(&self, _track: &()) -> Result<(), String> {
        Ok(())
    }

    /// Open a fresh **video-only** camera stream on the chosen `device_id` via
    /// `getUserMedia` — and nothing else. No track replacement, no publisher
    /// touch. This is the clean primitive the broadcast uses to drive an
    /// FX-aware camera switch: the broadcast decides whether to feed the new
    /// stream into the blur bridge (`video_fx::set_source`) or to replace the
    /// publisher's raw video track (`replace_main_video_track`), then handles
    /// stopping the old camera track and updating its own signals.
    ///
    /// Video-only because mic switching is a separate concern; the publisher /
    /// blur bridge keeps whatever audio it already carries. Does NOT require an
    /// active publisher, so it can be called even before/independently of WHIP.
    #[cfg(target_arch = "wasm32")]
    pub async fn open_camera(&self, device_id: &str) -> Result<web_sys::MediaStream, String> {
        use wasm_bindgen::JsCast;
        use wasm_bindgen_futures::JsFuture;

        let win = web_sys::window().ok_or_else(|| "no window".to_string())?;
        let media = win
            .navigator()
            .media_devices()
            .map_err(|e| format!("media_devices: {e:?}"))?;

        // Constrain to the requested camera; request video-only so the prior
        // mic / audio pipeline is never disturbed.
        let video_constraints = web_sys::MediaTrackConstraints::new();
        if !device_id.is_empty() {
            video_constraints.set_device_id(&wasm_bindgen::JsValue::from_str(device_id));
        }
        let constraints = web_sys::MediaStreamConstraints::new();
        constraints.set_video(video_constraints.as_ref());
        constraints.set_audio(&wasm_bindgen::JsValue::FALSE);

        let stream_promise = media
            .get_user_media_with_constraints(&constraints)
            .map_err(|e| format!("getUserMedia: {e:?}"))?;
        let stream_value = JsFuture::from(stream_promise)
            .await
            .map_err(|e| format!("getUserMedia await: {e:?}"))?;
        let stream: web_sys::MediaStream = stream_value
            .dyn_into()
            .map_err(|_| "stream cast".to_string())?;
        Ok(stream)
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub async fn open_camera(&self, _device_id: &str) -> Result<(), String> {
        Ok(())
    }

    /// In-call camera switch by device id for the **non-FX path**: open a fresh
    /// camera stream on the chosen `device_id` (via [`open_camera`]) and hand
    /// its video track to the publisher via `replace_video_track`. The audio
    /// track is left untouched (mic switching is a separate concern). Returns
    /// the new `MediaStream` so the caller can wire it onto the local self-view
    /// `<video>` and stop the now-orphaned old camera track.
    ///
    /// NOTE: This BYPASSES the background-blur canvas — when FX is active the
    /// published track is the bridge's canvas captureStream, so replacing the
    /// publisher's video track here would publish the raw camera and break the
    /// blur. The broadcast must call `video_fx::is_active()` first and, when
    /// true, route through `open_camera` + `video_fx::set_source` instead of
    /// this method. See the broadcast camera-switch handler.
    ///
    /// [`open_camera`]: Self::open_camera
    #[cfg(target_arch = "wasm32")]
    pub async fn switch_camera(&self, device_id: &str) -> Result<web_sys::MediaStream, String> {
        use wasm_bindgen::JsCast;

        let publisher = self
            .publisher
            .as_ref()
            .ok_or_else(|| "no active publisher".to_string())?;

        let stream = self.open_camera(device_id).await?;

        let tracks = stream.get_video_tracks();
        let new_track: web_sys::MediaStreamTrack = tracks
            .get(0)
            .dyn_into()
            .map_err(|_| "no video track on new camera stream".to_string())?;

        publisher.replace_video_track(&new_track).await?;
        Ok(stream)
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub async fn switch_camera(&self, device_id: &str) -> Result<(), String> {
        crate::live_room_native::switch_camera(device_id, None, "broadcast-self-video").await
    }

    /// Poll the publisher-side connection quality. `None` when not live.
    #[cfg(target_arch = "wasm32")]
    pub async fn publisher_quality(&self) -> Option<crate::live_room_stats::NetQuality> {
        let publisher = self.publisher.as_ref()?;
        Some(publisher.network_quality().await)
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub async fn publisher_quality(&self) -> Option<crate::live_room_stats::NetQuality> {
        Some(crate::live_room_native::publisher_quality("main").await)
    }

    /// Clone of the publisher's `RtcPeerConnection` (a cheap JS-handle clone),
    /// or `None` when not publishing. Callers poll quality off the returned
    /// handle so they don't hold a `Signal` read guard across the async
    /// `getStats()` await (which could race a concurrent session write).
    #[cfg(target_arch = "wasm32")]
    pub fn publisher_pc(&self) -> Option<web_sys::RtcPeerConnection> {
        self.publisher.as_ref().map(|p| p.pc.clone())
    }

    /// Student-side: store the WHEP viewer for the teacher's feed, returning
    /// the one it replaced so the CALLER can close it.
    ///
    /// Deliberately synchronous, and deliberately not an `attach_main` that
    /// opens the viewer itself. A `Signal<LiveRoomSession>` write guard is
    /// alive for the whole of `sig.write().method(..).await`, and the WHEP
    /// handshake now legitimately takes tens of seconds (ICE gathering +
    /// path-readiness retries + connection verification). Holding the guard
    /// across that made any concurrent read/write of the same signal -- a
    /// promoted student arriving on the socket, or route exit calling
    /// `close()` -- a generational-box double borrow, which on wasm32
    /// (`panic = "abort"`) traps the module instead of unwinding.
    ///
    /// The open-then-store split lives in `live_room_view::attach_main_unguarded`.
    /// Returning the previous viewer keeps its async `close()` outside the
    /// guard too.
    ///
    /// (MediaMTX's WHEP read-auth only accepts the join-minted viewer JWT, not
    /// the backend API session token; passing the latter gave 403 -> 401 and a
    /// blank student feed.)
    #[cfg(target_arch = "wasm32")]
    #[must_use = "close the returned viewer outside the signal write guard"]
    pub fn set_main_viewer(
        &mut self,
        viewer: crate::live_room_whep::WhepViewer,
    ) -> Option<crate::live_room_whep::WhepViewer> {
        self.viewer.replace(viewer)
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub async fn attach_main(&mut self, _url: &str, _viewer_jwt: &str) -> Result<(), String> {
        crate::live_room_native::view(crate::live_room_native::ViewRequest {
            key: "main".into(),
            url: _url.into(),
            password: _viewer_jwt.into(),
            element_id: "live-room-main-video".into(),
            ice_servers: Vec::new(),
        })
        .await
    }

    /// The remote `MediaStream` of the main (teacher) WHEP viewer, if one
    /// has been attached. The caller (typically `LiveRoomView::render_webrtc`)
    /// wires it onto the `<video>` element after the session-owned
    /// `attach_main` completes.
    ///
    /// Uses `live_room_ice::same_stream`, NOT `.clone()`: on
    /// `web_sys::MediaStream` the inherent DOM `clone()` shadows the `Clone`
    /// impl and returns a detached copy with cloned tracks, so the element
    /// would be handed a stream that never receives the incoming tracks.
    #[cfg(target_arch = "wasm32")]
    pub fn main_remote_stream(&self) -> Option<web_sys::MediaStream> {
        self.viewer
            .as_ref()
            .map(|v| crate::live_room_ice::same_stream(&v.remote_stream))
    }

    /// Speculative: every join probes the screen path even though the teacher
    /// is usually not sharing, and the caller treats the error as routine
    /// (`screen WHEP inactive or unavailable`). So this uses `view_once` --
    /// a 404 means "not sharing" and must fail fast, not wait for a publisher
    /// that was never coming.
    /// Store the screen-share viewer. Same guard discipline as
    /// `set_main_viewer`; the open half is
    /// `live_room_view::attach_screen_unguarded`, which uses `view_once`.
    #[cfg(target_arch = "wasm32")]
    #[must_use = "close the returned viewer outside the signal write guard"]
    pub fn set_screen_viewer(
        &mut self,
        viewer: crate::live_room_whep::WhepViewer,
    ) -> Option<crate::live_room_whep::WhepViewer> {
        self.screen_viewer.replace(viewer)
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub async fn attach_screen(&mut self, _url: &str, _viewer_jwt: &str) -> Result<(), String> {
        crate::live_room_native::view(crate::live_room_native::ViewRequest {
            key: "screen".into(),
            url: _url.into(),
            password: _viewer_jwt.into(),
            element_id: "live-room-screen-video".into(),
            ice_servers: Vec::new(),
        })
        .await
    }

    #[cfg(target_arch = "wasm32")]
    pub fn screen_remote_stream(&self) -> Option<web_sys::MediaStream> {
        self.screen_viewer
            .as_ref()
            .map(|v| crate::live_room_ice::same_stream(&v.remote_stream))
    }

    #[cfg(target_arch = "wasm32")]
    pub fn set_screen_publisher(&mut self, publisher: crate::live_room_whip::WhipPublisher) {
        self.screen_publisher = Some(publisher);
    }

    #[cfg(target_arch = "wasm32")]
    pub async fn stop_screen_share(&mut self) {
        if let Some(mut publisher) = self.screen_publisher.take() {
            let _ = publisher.close().await;
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub async fn stop_screen_share(&mut self) {
        let _ =
            crate::live_room_native::stop_publisher("screen", Some("broadcast-screen-video")).await;
    }

    /// Open a WHEP subscription onto a promoted student so this participant
    /// hears (and sees) them. `whep_url` is the full URL the server reports in
    /// `StudentPublishing` (built from the public WebRTC base).
    ///
    /// The `StudentPublishing` event fires the instant MediaMTX authorizes the
    /// student's WHIP publish — a hair before the path is actually readable —
    /// so the WHEP attach can lose that race. That readiness retry now lives
    /// inside `live_room_whep::view` (bounded by
    /// `live_room_ice::PATH_READY_TIMEOUT_MS`), which fixes the same race for
    /// the main teacher feed, the screen feed and breakouts as well. The outer
    /// backoff loop that used to live here is gone deliberately: stacking it on
    /// top of the inner budget multiplied the worst-case wait, and it also
    /// retried terminal auth failures that cannot succeed.
    #[cfg(target_arch = "wasm32")]
    #[must_use = "close the returned viewer outside the signal write guard"]
    pub fn set_student_viewer(
        &mut self,
        user_id: Uuid,
        viewer: crate::live_room_whep::WhepViewer,
    ) -> Option<crate::live_room_whep::WhepViewer> {
        self.students.insert(user_id, viewer)
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub async fn attach_student(
        &mut self,
        user_id: Uuid,
        whep_url: &str,
        viewer_jwt: &str,
    ) -> Result<(), String> {
        let key = format!("student-{user_id}");
        crate::live_room_native::view(crate::live_room_native::ViewRequest {
            element_id: format!("live-room-student-audio-{user_id}"),
            key,
            url: whep_url.into(),
            password: viewer_jwt.into(),
            ice_servers: Vec::new(),
        })
        .await
    }

    /// Drop a student WHEP viewer (e.g. when the student is demoted).
    #[cfg(target_arch = "wasm32")]
    /// Remove a student's viewer and hand it back for the caller to close
    /// outside the write guard.
    #[must_use = "close the returned viewer outside the signal write guard"]
    pub fn take_student(&mut self, user_id: Uuid) -> Option<crate::live_room_whep::WhepViewer> {
        self.students.remove(&user_id)
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub async fn detach_student(&mut self, user_id: Uuid) {
        let key = format!("student-{user_id}");
        let element = format!("live-room-student-audio-{user_id}");
        let _ = crate::live_room_native::stop_viewer(&key, Some(&element)).await;
    }

    /// Teacher-side end-class. Closes local resources **first** (so the
    /// publisher / viewer release before the backend tears the session
    /// down) then POSTs `/end-class`.
    pub async fn end_class(&mut self) -> Result<(), String> {
        self.close().await;
        use crate::api::{fetch_json, ApiError};
        let _: serde_json::Value = fetch_json(
            &self.api,
            "POST",
            &format!("/v1/sessions/{}/end-class", self.config.session_id),
            Some(&serde_json::json!({})),
        )
        .await
        .map_err(|e: ApiError| e.to_string())?;
        Ok(())
    }

    /// Release every owned resource. Idempotent: a second call is a no-op.
    ///
    /// `LiveRoomSocket::close` is synchronous and idempotent; browser
    /// `WhipPublisher` / `WhepViewer::close` are async, so we `take()` and await
    /// them. Native media objects are closed by one bounded bridge operation.
    pub async fn close(&mut self) {
        if !self.begin_close() {
            return;
        }

        if let Some(mut socket) = self.socket.take() {
            socket.close();
        }

        #[cfg(target_arch = "wasm32")]
        {
            if let Some(mut p) = self.publisher.take() {
                let _ = p.close().await;
            }
            if let Some(mut p) = self.screen_publisher.take() {
                let _ = p.close().await;
            }
            if let Some(mut v) = self.viewer.take() {
                let _ = v.close().await;
            }
            if let Some(mut v) = self.screen_viewer.take() {
                let _ = v.close().await;
            }
            let students = std::mem::take(&mut self.students);
            for (_uid, mut v) in students {
                let _ = v.close().await;
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = crate::live_room_native::close_all().await;
        }
    }
}

impl Drop for LiveRoomSession {
    fn drop(&mut self) {
        if self.closed {
            return;
        }
        // Synchronously stop the room socket on every renderer. Browser media
        // handles finish in a local task; native routes call `close()` so the
        // WebView bridge can await its teardown requests.
        if let Some(mut socket) = self.socket.take() {
            socket.close();
        }
        #[cfg(target_arch = "wasm32")]
        {
            // Take the resources out so the async block owns them; the
            // session itself is being dropped right now and we can't borrow
            // `&mut self` across the await.
            let publisher = self.publisher.take();
            let screen_publisher = self.screen_publisher.take();
            let viewer = self.viewer.take();
            let screen_viewer = self.screen_viewer.take();
            let students = std::mem::take(&mut self.students);
            wasm_bindgen_futures::spawn_local(async move {
                if let Some(mut p) = publisher {
                    let _ = p.close().await;
                }
                if let Some(mut p) = screen_publisher {
                    let _ = p.close().await;
                }
                if let Some(mut v) = viewer {
                    let _ = v.close().await;
                }
                if let Some(mut v) = screen_viewer {
                    let _ = v.close().await;
                }
                for (_uid, mut sv) in students {
                    let _ = sv.close().await;
                }
            });
        }
        self.closed = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_api() -> ApiContext {
        ApiContext {
            base_url: String::new(),
            id_token: String::new(),
        }
    }

    fn dummy_config() -> SessionConfig {
        SessionConfig {
            session_id: "session-1".into(),
            api_origin: "http://localhost:3000".into(),
            access_token: "tok".into(),
            viewer_jwt: Some("viewer.jwt".into()),
        }
    }

    /// The pure lifecycle gate makes a second close a no-op before renderer
    /// resources are touched. Actual native teardown requires a Dioxus runtime
    /// and is covered by the bridge contract tests.
    #[test]
    fn close_is_idempotent() {
        let mut session = LiveRoomSession::new(dummy_config(), dummy_api());
        assert!(!session.is_closed());
        assert!(session.begin_close());
        assert!(session.is_closed());
        assert!(!session.begin_close());
        assert!(session.is_closed());
    }

    /// `build_ws_url` must percent-encode the access token so JWT
    /// characters (`+`, `/`, `=`) survive the URL boundary.
    #[test]
    fn build_ws_url_urlencodes_token() {
        let url = build_ws_url(
            "http://localhost:3000",
            "session-1",
            "abc+def/ghi=jkl",
            None,
        );
        assert!(
            url.starts_with("ws://localhost:3000/v1/sessions/session-1/socket?access_token="),
            "unexpected prefix: {url}",
        );
        assert!(url.contains("abc%2Bdef%2Fghi%3Djkl"), "not encoded: {url}");
        // Confirm raw special chars do not survive.
        assert!(!url.contains("abc+def"), "raw + leaked: {url}");
        assert!(!url.contains("ghi=jkl"), "raw = leaked: {url}");
    }

    #[test]
    fn build_ws_url_picks_wss_for_https_origin() {
        let url = build_ws_url("https://api.example.com", "s", "t", None);
        assert!(url.starts_with("wss://api.example.com/"), "got: {url}");
    }

    #[test]
    fn build_ws_url_carries_workspace_selection() {
        let workspace = "11111111-1111-1111-1111-111111111111";
        let url = build_ws_url("https://api.example.com", "s", "t", Some(workspace));
        assert!(
            url.ends_with(&format!("&workspace_id={workspace}")),
            "got: {url}"
        );
    }

    #[test]
    fn whep_url_for_path_trims_duplicate_slashes() {
        assert_eq!(
            whep_url_for_path("http://localhost:8889/", "/aula/t/c/s/student/u"),
            "http://localhost:8889/aula/t/c/s/student/u/whep"
        );
    }

    #[test]
    fn session_config_carries_viewer_jwt() {
        assert_eq!(dummy_config().viewer_jwt.as_deref(), Some("viewer.jwt"));
    }
}
