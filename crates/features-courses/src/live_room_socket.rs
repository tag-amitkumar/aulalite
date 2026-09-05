// crates/features-courses/src/live_room_socket.rs
//! WebSocket client for the live room. Pure helpers in this file are
//! cross-platform and unit-tested; the wasm32 connection driver is gated.
//!
//! The canonical token-aware URL builder lives in `live_room_session.rs`
//! (Task 17). The 2-arg `build_ws_url` here is kept for the legacy
//! view/broadcast call sites that have not yet migrated to the session
//! aggregate; do not add new callers — use the session helper instead.

use serde::Deserialize;

#[derive(Debug, Deserialize, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WhiteboardTool {
    Pen,
    Eraser,
}

/// Geometric kind of a whiteboard element. `Freehand` is the original pen
/// polyline; shape kinds anchor on the first/last point; `Text` carries a
/// `text` body; `Image` references an uploaded file asset (`asset_id`) and
/// anchors on two points like a rect. `#[serde(default)]` keeps older pen-only
/// event streams parsing as `Freehand`.
#[derive(Debug, Deserialize, Clone, PartialEq, serde::Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WhiteboardKind {
    #[default]
    Freehand,
    Line,
    Rect,
    Ellipse,
    Arrow,
    Text,
    Image,
}

#[derive(Debug, Deserialize, Clone, PartialEq, serde::Serialize)]
pub struct WhiteboardPoint {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Deserialize, Clone, PartialEq, serde::Serialize)]
pub struct WhiteboardStroke {
    pub id: String,
    pub points: Vec<WhiteboardPoint>,
    pub color: String,
    pub width: f32,
    pub tool: WhiteboardTool,
    #[serde(default)]
    pub kind: WhiteboardKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// File-asset id for `WhiteboardKind::Image` elements. Omitted for every
    /// other kind; `#[serde(default)]` keeps older event streams parsing as
    /// `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_id: Option<String>,
    /// User who authored this stroke (UUID string). Added additively for
    /// per-author undo/redo; `#[serde(default)]` keeps older event streams
    /// (no `author`) parsing as `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
}

#[derive(Debug, Deserialize, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerEvent {
    Chat {
        id: String,
        sender_user_id: String,
        sender_display_name: String,
        body: String,
        created_at: String,
    },
    ChatDeleted {
        id: String,
    },
    HandRaiseChanged {
        user_id: String,
        raised: bool,
        /// Display name of the student whose hand-raise state changed.
        /// `#[serde(default)]` keeps older servers (which did not emit this
        /// field) parseable; the UI falls back to a placeholder when empty.
        #[serde(default)]
        display_name: String,
        queue_position: Option<u32>,
    },
    PresenceCount {
        count: u32,
    },
    PresenceList {
        participants: Vec<PresenceParticipant>,
    },
    Promoted {
        user_id: String,
        publish_url: String,
        publish_password: String,
    },
    Demoted {
        user_id: String,
        #[serde(default)]
        display_name: String,
    },
    StudentPublishing {
        user_id: String,
        path: String,
        /// Full WHEP URL for the promoted student's feed, built server-side
        /// from the public WebRTC base. Empty on older servers, in which case
        /// the client derives the URL from `path`.
        #[serde(default)]
        whep_url: String,
        #[serde(default)]
        display_name: String,
    },
    WhiteboardStroke {
        stroke: WhiteboardStroke,
    },
    /// Full board state, sent right after this socket subscribes so late
    /// joiners and reconnects hydrate the existing drawing.
    WhiteboardSnapshot {
        strokes: Vec<WhiteboardStroke>,
    },
    /// A single stroke was removed (teacher undo).
    WhiteboardStrokeRemoved {
        stroke_id: String,
    },
    WhiteboardClear,
    /// Ephemeral per-user cursor over the whiteboard. NOT persisted; rendered
    /// as a small labeled dot for every user except the local one. `x`/`y` are
    /// normalized board coordinates in `0.0..=1.0`.
    WhiteboardCursor {
        user_id: String,
        #[serde(default)]
        display_name: String,
        x: f32,
        y: f32,
    },
    /// The teacher toggled whether non-teachers may draw on the board.
    DrawPermissionChanged {
        open: bool,
    },
    /// An ephemeral emoji reaction from a participant — rendered as a brief
    /// floating burst for everyone in the room.
    Reaction {
        user_id: String,
        emoji: String,
        #[serde(default)]
        display_name: String,
    },
    Kicked {
        user_id: String,
    },
    SessionEnded,
    RateLimited {
        retry_after_ms: u64,
    },
    Error {
        code: String,
        message: String,
    },
    /// A client-issued command failed server-side. The client should surface
    /// `reason` (already user-facing) and not retry automatically.
    CommandFailed {
        command: String,
        reason: String,
    },
    /// A teacher started an in-class poll. `options` are the ordered choices;
    /// votes reference an option by its index. Ephemeral (not snapshot-hydrated).
    PollStarted {
        poll_id: String,
        question: String,
        options: Vec<String>,
    },
    /// Live tally for the active poll: `counts[i]` is the vote count for
    /// `options[i]`. Broadcast after each accepted vote.
    PollResults {
        poll_id: String,
        counts: Vec<u32>,
    },
    /// The teacher ended the poll; `counts` carries the final tally.
    PollEnded {
        poll_id: String,
        counts: Vec<u32>,
    },
    /// The teacher opened breakout rooms. Carries the full layout so every
    /// client renders the same set of rooms + assignments.
    BreakoutOpened {
        rooms: Vec<BreakoutRoom>,
    },
    /// The breakout layout changed while open (rooms created/renamed, or a
    /// participant reassigned). Same full-layout payload as `BreakoutOpened`.
    BreakoutUpdated {
        rooms: Vec<BreakoutRoom>,
    },
    /// The teacher closed all breakouts; everyone returns to the main room.
    BreakoutClosed,
    /// Full breakout state sent point-to-point on (re)connect. `open` is false
    /// (and `rooms` empty) when no breakouts are running.
    BreakoutSnapshot {
        open: bool,
        rooms: Vec<BreakoutRoom>,
    },
    /// Targeted at a single participant: their breakout assignment changed. The
    /// client (re)subscribes its main WHEP viewer to `whep_url` while assigned,
    /// and falls back to the main room when `room_id` is `None`.
    BreakoutAssignment {
        #[serde(default)]
        user_id: String,
        #[serde(default)]
        room_id: Option<String>,
        #[serde(default)]
        room_key: String,
        #[serde(default)]
        whep_url: String,
        #[serde(default)]
        room_name: String,
    },
}

/// One breakout room as it arrives on the wire. Mirrors the backend
/// `services::live_room::BreakoutRoom` (UUIDs serialize as strings).
#[derive(Debug, Deserialize, Clone, PartialEq, serde::Serialize)]
pub struct BreakoutRoom {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub members: Vec<String>,
}

#[derive(Debug, Deserialize, Clone, PartialEq)]
pub struct PresenceParticipant {
    pub user_id: String,
    pub display_name: String,
    pub role: String,
}

pub fn parse_event(text: &str) -> Result<ServerEvent, String> {
    serde_json::from_str(text).map_err(|e| e.to_string())
}

/// Returns the next backoff delay (in ms) given the attempt count.
/// Sequence: 1000, 2000, 4000, 8000, 30000, 30000, ...
pub fn backoff_delay_ms(attempt: u32) -> u64 {
    match attempt {
        0 => 1000,
        1 => 2000,
        2 => 4000,
        3 => 8000,
        _ => 30000,
    }
}

/// Maximum number of reconnect attempts before giving up and surfacing
/// the disconnect to the UI. Matches the design doc cap.
pub const MAX_RECONNECT_ATTEMPTS: u32 = 5;

/// Keep presence fresh across mobile radio changes and intermediary timeouts.
/// This is intentionally slower than media stats polling and safely below the
/// server's stale-presence window.
pub const PRESENCE_HEARTBEAT_INTERVAL_MS: u32 = 15_000;
pub const PRESENCE_HEARTBEAT_PAYLOAD: &str = r#"{"type":"heartbeat"}"#;

/// Coarse connection state surfaced to the live-room UI so a transient
/// WebSocket drop (network blip, 1006, or a 4001 token expiry) is visible
/// instead of silently killing chat / presence / hand-raise.
///
/// * `Connected` — socket is open; the room is fully live.
/// * `Reconnecting` — the socket dropped and an exponential-backoff
///   reconnect is scheduled (see [`backoff_delay_ms`]).
/// * `Disconnected` — terminal: either the close was `Surface`
///   (4003 auth-invalid) or [`MAX_RECONNECT_ATTEMPTS`] was exhausted. The
///   user must reload to rejoin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConnStatus {
    #[default]
    Connected,
    Reconnecting,
    Disconnected,
}

/// What the live-room socket should do in response to a close-event code.
///
/// The dispatch matches the protocol convention in `core-types`:
/// * `4001` (`AUTH_EXPIRED`) — refresh the JWT then reconnect.
/// * `4003` (`AUTH_INVALID`) — surface to the UI; do not reconnect.
/// * other codes (incl. `1006` abnormal-close) — exponential-backoff
///   reconnect (capped by [`MAX_RECONNECT_ATTEMPTS`] and [`backoff_delay_ms`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseAction {
    /// Reconnect after acquiring a fresh access token.
    RefreshToken,
    /// Reconnect immediately (subject to backoff in the caller).
    Reconnect,
    /// Stop reconnecting and surface the close reason to the UI.
    Surface,
}

/// Pure dispatch for a WebSocket close code → the action the live-room
/// session should take. Kept free of `web_sys` so it can be unit-tested
/// on the host target.
pub fn classify_close_action(code: u16) -> CloseAction {
    match code {
        // Keep these in sync with `core_types::live_room::close_codes`.
        4001 => CloseAction::RefreshToken,
        4003 => CloseAction::Surface,
        _ => CloseAction::Reconnect,
    }
}

pub fn build_ws_url(api_origin: &str, session_id: &str) -> String {
    let scheme = if api_origin.starts_with("https") {
        "wss"
    } else {
        "ws"
    };
    let host = api_origin
        .trim_start_matches("http://")
        .trim_start_matches("https://");
    format!("{scheme}://{host}/v1/sessions/{session_id}/socket")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_chat_event() {
        let raw = r#"{"type":"chat","id":"00000000-0000-0000-0000-000000000000","sender_user_id":"00000000-0000-0000-0000-000000000000","sender_display_name":"A","body":"hi","created_at":"2026-05-09T12:00:00Z"}"#;
        match parse_event(raw).unwrap() {
            ServerEvent::Chat {
                body,
                sender_display_name,
                ..
            } => {
                assert_eq!(body, "hi");
                assert_eq!(sender_display_name, "A");
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn parses_session_ended() {
        match parse_event(r#"{"type":"session_ended"}"#).unwrap() {
            ServerEvent::SessionEnded => {}
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn parses_kicked() {
        match parse_event(r#"{"type":"kicked","user_id":"abc"}"#).unwrap() {
            ServerEvent::Kicked { user_id } => assert_eq!(user_id, "abc"),
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn rejects_unknown_type() {
        assert!(parse_event(r#"{"type":"definitely_unknown"}"#).is_err());
    }

    #[test]
    fn backoff_sequence() {
        assert_eq!(backoff_delay_ms(0), 1000);
        assert_eq!(backoff_delay_ms(1), 2000);
        assert_eq!(backoff_delay_ms(2), 4000);
        assert_eq!(backoff_delay_ms(3), 8000);
        assert_eq!(backoff_delay_ms(4), 30000);
        assert_eq!(backoff_delay_ms(99), 30000);
    }

    #[test]
    fn presence_heartbeat_matches_the_server_protocol() {
        let value: serde_json::Value =
            serde_json::from_str(PRESENCE_HEARTBEAT_PAYLOAD).expect("valid heartbeat json");
        assert_eq!(value["type"], "heartbeat");
        assert_eq!(PRESENCE_HEARTBEAT_INTERVAL_MS, 15_000);
    }

    /// 4001 = AUTH_EXPIRED. The socket should request a fresh token and
    /// queue a reconnect — surfaced here by classifying the close as
    /// `RefreshToken`.
    #[test]
    fn onclose_4001_triggers_reconnect_with_fresh_token() {
        assert_eq!(classify_close_action(4001), CloseAction::RefreshToken);
    }

    /// 4003 = AUTH_INVALID. The socket must NOT reconnect; the UI shows
    /// a permanent error.
    #[test]
    fn onclose_4003_does_not_reconnect() {
        assert_eq!(classify_close_action(4003), CloseAction::Surface);
    }

    /// 1006 is the standard "abnormal closure" code that browsers emit
    /// when the TCP socket dies under the WebSocket. The session retries
    /// with backoff.
    #[test]
    fn onclose_1006_triggers_plain_reconnect() {
        assert_eq!(classify_close_action(1006), CloseAction::Reconnect);
    }

    #[test]
    fn onclose_1000_normal_close_classifies_as_reconnect() {
        // 1000 is normal close; in practice the session sets `closed = true`
        // before close arrives so this branch is rare, but the classifier
        // itself should not single-case-it.
        assert_eq!(classify_close_action(1000), CloseAction::Reconnect);
    }

    #[test]
    fn parses_whiteboard_stroke_event() {
        let raw = r##"{"type":"whiteboard_stroke","stroke":{"id":"s1","points":[{"x":0.1,"y":0.2},{"x":0.3,"y":0.4}],"color":"#111827","width":4.0,"tool":"pen"}}"##;
        match parse_event(raw).unwrap() {
            ServerEvent::WhiteboardStroke { stroke } => {
                assert_eq!(stroke.id, "s1");
                assert_eq!(stroke.points.len(), 2);
                assert_eq!(stroke.tool, WhiteboardTool::Pen);
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn parses_whiteboard_shape_and_text_events() {
        // Shape (line) — anchors on two points, kind present.
        let line = r##"{"type":"whiteboard_stroke","stroke":{"id":"l1","points":[{"x":0.1,"y":0.2},{"x":0.4,"y":0.5}],"color":"#111827","width":4.0,"tool":"pen","kind":"line"}}"##;
        match parse_event(line).unwrap() {
            ServerEvent::WhiteboardStroke { stroke } => {
                assert_eq!(stroke.kind, WhiteboardKind::Line);
                assert_eq!(stroke.text, None);
            }
            other => panic!("wrong variant: {other:?}"),
        }

        // Text element — carries a body.
        let text = r##"{"type":"whiteboard_stroke","stroke":{"id":"t1","points":[{"x":0.2,"y":0.3}],"color":"#111827","width":4.0,"tool":"pen","kind":"text","text":"hi"}}"##;
        match parse_event(text).unwrap() {
            ServerEvent::WhiteboardStroke { stroke } => {
                assert_eq!(stroke.kind, WhiteboardKind::Text);
                assert_eq!(stroke.text.as_deref(), Some("hi"));
            }
            other => panic!("wrong variant: {other:?}"),
        }

        // Image element — carries an asset id, anchors on two points.
        let image = r##"{"type":"whiteboard_stroke","stroke":{"id":"i1","points":[{"x":0.2,"y":0.3},{"x":0.6,"y":0.7}],"color":"#111827","width":4.0,"tool":"pen","kind":"image","asset_id":"asset-9"}}"##;
        match parse_event(image).unwrap() {
            ServerEvent::WhiteboardStroke { stroke } => {
                assert_eq!(stroke.kind, WhiteboardKind::Image);
                assert_eq!(stroke.asset_id.as_deref(), Some("asset-9"));
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn parses_legacy_pen_stroke_without_kind() {
        // Backward-compat: an older server omits `kind`/`text`; the additive
        // fields default rather than fail to parse.
        let raw = r##"{"type":"whiteboard_stroke","stroke":{"id":"s1","points":[{"x":0.1,"y":0.2},{"x":0.3,"y":0.4}],"color":"#111827","width":4.0,"tool":"pen"}}"##;
        match parse_event(raw).unwrap() {
            ServerEvent::WhiteboardStroke { stroke } => {
                assert_eq!(stroke.kind, WhiteboardKind::Freehand);
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn parses_whiteboard_clear_event() {
        match parse_event(r#"{"type":"whiteboard_clear"}"#).unwrap() {
            ServerEvent::WhiteboardClear => {}
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn parses_whiteboard_cursor_event() {
        let raw =
            r#"{"type":"whiteboard_cursor","user_id":"u1","display_name":"Ada","x":0.25,"y":0.5}"#;
        match parse_event(raw).unwrap() {
            ServerEvent::WhiteboardCursor {
                user_id,
                display_name,
                x,
                y,
            } => {
                assert_eq!(user_id, "u1");
                assert_eq!(display_name, "Ada");
                assert!((x - 0.25).abs() < 1e-6);
                assert!((y - 0.5).abs() < 1e-6);
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn parses_draw_permission_changed_event() {
        match parse_event(r#"{"type":"draw_permission_changed","open":true}"#).unwrap() {
            ServerEvent::DrawPermissionChanged { open } => assert!(open),
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn parses_whiteboard_stroke_with_author_and_legacy_defaults_none() {
        let with_author = r##"{"type":"whiteboard_stroke","stroke":{"id":"a1","points":[{"x":0.1,"y":0.2},{"x":0.3,"y":0.4}],"color":"#111827","width":4.0,"tool":"pen","author":"u1"}}"##;
        match parse_event(with_author).unwrap() {
            ServerEvent::WhiteboardStroke { stroke } => {
                assert_eq!(stroke.author.as_deref(), Some("u1"));
            }
            other => panic!("wrong variant: {other:?}"),
        }
        let legacy = r##"{"type":"whiteboard_stroke","stroke":{"id":"s1","points":[{"x":0.1,"y":0.2},{"x":0.3,"y":0.4}],"color":"#111827","width":4.0,"tool":"pen"}}"##;
        match parse_event(legacy).unwrap() {
            ServerEvent::WhiteboardStroke { stroke } => assert_eq!(stroke.author, None),
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn parses_poll_events() {
        let started =
            r#"{"type":"poll_started","poll_id":"p1","question":"Q?","options":["A","B","C"]}"#;
        match parse_event(started).unwrap() {
            ServerEvent::PollStarted {
                poll_id,
                question,
                options,
            } => {
                assert_eq!(poll_id, "p1");
                assert_eq!(question, "Q?");
                assert_eq!(options, vec!["A", "B", "C"]);
            }
            other => panic!("wrong variant: {other:?}"),
        }

        let results = r#"{"type":"poll_results","poll_id":"p1","counts":[2,0,1]}"#;
        match parse_event(results).unwrap() {
            ServerEvent::PollResults { poll_id, counts } => {
                assert_eq!(poll_id, "p1");
                assert_eq!(counts, vec![2, 0, 1]);
            }
            other => panic!("wrong variant: {other:?}"),
        }

        let ended = r#"{"type":"poll_ended","poll_id":"p1","counts":[2,0,1]}"#;
        match parse_event(ended).unwrap() {
            ServerEvent::PollEnded { poll_id, counts } => {
                assert_eq!(poll_id, "p1");
                assert_eq!(counts, vec![2, 0, 1]);
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn parses_breakout_events() {
        let opened = r#"{"type":"breakout_opened","rooms":[{"id":"r1","name":"Group 1","members":["u1","u2"]}]}"#;
        match parse_event(opened).unwrap() {
            ServerEvent::BreakoutOpened { rooms } => {
                assert_eq!(rooms.len(), 1);
                assert_eq!(rooms[0].name, "Group 1");
                assert_eq!(rooms[0].members.len(), 2);
            }
            other => panic!("wrong variant: {other:?}"),
        }

        let updated = r#"{"type":"breakout_updated","rooms":[]}"#;
        assert!(matches!(
            parse_event(updated).unwrap(),
            ServerEvent::BreakoutUpdated { .. }
        ));

        let closed = r#"{"type":"breakout_closed"}"#;
        assert!(matches!(
            parse_event(closed).unwrap(),
            ServerEvent::BreakoutClosed
        ));

        let snapshot = r#"{"type":"breakout_snapshot","open":true,"rooms":[{"id":"r1","name":"A","members":[]}]}"#;
        match parse_event(snapshot).unwrap() {
            ServerEvent::BreakoutSnapshot { open, rooms } => {
                assert!(open);
                assert_eq!(rooms.len(), 1);
            }
            other => panic!("wrong variant: {other:?}"),
        }

        // Assignment to a room.
        let assigned = r#"{"type":"breakout_assignment","user_id":"u1","room_id":"r1","room_key":"aula/t/c/s/breakout/r1","whep_url":"http://w/aula/t/c/s/breakout/r1/whep","room_name":"A"}"#;
        match parse_event(assigned).unwrap() {
            ServerEvent::BreakoutAssignment {
                user_id,
                room_id,
                whep_url,
                ..
            } => {
                assert_eq!(user_id, "u1");
                assert_eq!(room_id.as_deref(), Some("r1"));
                assert!(whep_url.ends_with("/whep"));
            }
            other => panic!("wrong variant: {other:?}"),
        }

        // Assignment back to the main room (room_id null).
        let main = r#"{"type":"breakout_assignment","user_id":"u1","room_id":null,"whep_url":""}"#;
        match parse_event(main).unwrap() {
            ServerEvent::BreakoutAssignment {
                room_id, whep_url, ..
            } => {
                assert_eq!(room_id, None);
                assert!(whep_url.is_empty());
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn parses_student_publishing_with_whep_url() {
        // The server now sends a full `whep_url`; the client prefers it over
        // re-deriving the URL from `path`.
        let raw = r#"{"type":"student_publishing","user_id":"u1","path":"aula/t/c/s/student/u1","whep_url":"http://webrtc.example/aula/t/c/s/student/u1/whep","display_name":"Ada"}"#;
        match parse_event(raw).unwrap() {
            ServerEvent::StudentPublishing {
                user_id, whep_url, ..
            } => {
                assert_eq!(user_id, "u1");
                assert_eq!(whep_url, "http://webrtc.example/aula/t/c/s/student/u1/whep");
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn parses_student_publishing_without_whep_url_defaults_empty() {
        // Backward-compat: an older server omits `whep_url`; serde default
        // leaves it empty and the client falls back to deriving from `path`.
        let raw = r#"{"type":"student_publishing","user_id":"u1","path":"aula/t/c/s/student/u1"}"#;
        match parse_event(raw).unwrap() {
            ServerEvent::StudentPublishing { whep_url, path, .. } => {
                assert!(whep_url.is_empty());
                assert_eq!(path, "aula/t/c/s/student/u1");
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }
}

// wasm32 connection driver
#[cfg(target_arch = "wasm32")]
pub mod conn {
    use super::*;
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen::JsCast;
    use web_sys::{CloseEvent, MessageEvent, WebSocket};

    /// Thin owning wrapper around a `web_sys::WebSocket`.
    ///
    /// Lifetime contract: `close()` is synchronous, idempotent, and drops
    /// every registered closure. `Drop` calls `close()` so that an
    /// abandoned `LiveRoomSocket` cannot leak a live connection (the
    /// browser would otherwise keep the upgrade open until GC).
    pub struct LiveRoomSocket {
        ws: WebSocket,
        // Hold each closure for the WebSocket's lifetime so the JS side
        // can still invoke it; on `close()` we drop them, which unregisters
        // them on the JS side.
        on_message: Option<Closure<dyn FnMut(MessageEvent)>>,
        on_close: Option<Closure<dyn FnMut(CloseEvent)>>,
        closed: bool,
    }

    impl LiveRoomSocket {
        pub fn connect(
            url: &str,
            mut on_event: impl FnMut(ServerEvent) + 'static,
        ) -> Result<Self, String> {
            let ws = WebSocket::new(url).map_err(|e| format!("ws new: {e:?}"))?;
            let on_message = Closure::<dyn FnMut(MessageEvent)>::new(move |evt: MessageEvent| {
                if let Ok(s) = evt.data().dyn_into::<js_sys::JsString>() {
                    if let Some(rs) = s.as_string() {
                        match parse_event(&rs) {
                            Ok(parsed) => on_event(parsed),
                            Err(err) => {
                                // Previously swallowed silently — surface
                                // to console at debug level so dropped
                                // events are visible during diagnostics.
                                web_sys::console::debug_2(
                                    &"[live_room_socket] parse_event error:".into(),
                                    &err.into(),
                                );
                            }
                        }
                    }
                }
            });
            ws.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
            Ok(Self {
                ws,
                on_message: Some(on_message),
                on_close: None,
                closed: false,
            })
        }

        pub fn send_text(&self, text: &str) -> Result<(), String> {
            self.ws
                .send_with_str(text)
                .map_err(|e| format!("ws send: {e:?}"))
        }

        /// Register a close-event listener. The callback receives the
        /// classified action (per [`classify_close_action`]) so call
        /// sites do not duplicate the dispatch table.
        ///
        /// Replacing the close handler drops any previously registered
        /// one, which unregisters it on the JS side.
        pub fn set_onclose(&mut self, mut handler: impl FnMut(CloseAction, u16) + 'static) {
            let closure = Closure::<dyn FnMut(CloseEvent)>::new(move |evt: CloseEvent| {
                let code = evt.code();
                let action = classify_close_action(code);
                handler(action, code);
            });
            self.ws.set_onclose(Some(closure.as_ref().unchecked_ref()));
            // Drop the previous closure (if any) by overwriting.
            self.on_close = Some(closure);
        }

        /// Synchronously close the socket and clear all registered
        /// closures. Idempotent: subsequent calls are no-ops.
        pub fn close(&mut self) {
            if self.closed {
                return;
            }
            self.closed = true;
            // Detach handlers from the JS side first so a final close
            // event cannot fire into a freed closure.
            self.ws.set_onmessage(None);
            self.ws.set_onclose(None);
            let _ = self.ws.close();
            // Drop the closures — this also unregisters them on the JS
            // side if any reference still lingered.
            self.on_message.take();
            self.on_close.take();
        }
    }

    impl Drop for LiveRoomSocket {
        fn drop(&mut self) {
            // Best-effort: if the caller forgot to `close()`, do it now
            // so the browser does not keep the upgrade alive past Rust's
            // ownership boundary.
            self.close();
        }
    }

    /// Owns a self-healing live-room WebSocket: connects, surfaces status,
    /// and on an unexpected close reconnects with exponential backoff
    /// (refreshing the access token first on `4001 AUTH_EXPIRED`). The whole
    /// lifecycle is a single `spawn_local` task driven, per connection, by a
    /// oneshot fed from `set_onclose` — there are NO self-referential
    /// closures, so the task and all of its captures drop cleanly once
    /// `closed` is set (the caller does this in `use_drop`) and the awaited
    /// oneshot resolves `Canceled`. This replaces the previous one-shot
    /// connect where any WS drop silently and permanently killed chat /
    /// presence / hand-raise.
    ///
    /// * `socket`    — shared cell the caller reads for `send_text`; this
    ///                 function writes each live socket into it.
    /// * `closed`    — set `true` by the caller on unmount to stop reconnects.
    /// * `make_url`  — builds the WS URL; the argument is an optional token
    ///                 override (used right after a refresh), else the call
    ///                 should read the freshest `ApiContext` token itself.
    /// * `on_event`  — per-message handler (hydrates component state).
    /// * `refresh`   — yields a fresh access token for the `4001` path.
    /// * `on_status` — connection-state callback for the UI banner.
    pub fn connect_reconnecting(
        socket: std::rc::Rc<std::cell::RefCell<Option<LiveRoomSocket>>>,
        closed: std::rc::Rc<std::cell::Cell<bool>>,
        make_url: std::rc::Rc<dyn Fn(Option<String>) -> String>,
        on_event: std::rc::Rc<dyn Fn(ServerEvent)>,
        refresh: std::rc::Rc<
            dyn Fn() -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<String>>>>,
        >,
        on_status: std::rc::Rc<dyn Fn(ConnStatus)>,
    ) {
        // One heartbeat task spans every reconnect generation. It always reads
        // the current shared socket, so a reconnect automatically redirects
        // heartbeats to the replacement connection without spawning duplicates.
        let heartbeat_socket = socket.clone();
        let heartbeat_closed = closed.clone();
        wasm_bindgen_futures::spawn_local(async move {
            loop {
                gloo_timers::future::TimeoutFuture::new(PRESENCE_HEARTBEAT_INTERVAL_MS).await;
                if heartbeat_closed.get() {
                    break;
                }
                if let Some(socket) = heartbeat_socket.borrow().as_ref() {
                    let _ = socket.send_text(PRESENCE_HEARTBEAT_PAYLOAD);
                }
            }
        });

        wasm_bindgen_futures::spawn_local(async move {
            let mut attempt: u32 = 0;
            let mut next_token: Option<String> = None;
            loop {
                if closed.get() {
                    break;
                }
                let url = make_url(next_token.take());
                let (tx, rx) = futures_channel::oneshot::channel::<(CloseAction, u16)>();
                let on_event_cb = on_event.clone();
                match LiveRoomSocket::connect(&url, move |evt| on_event_cb(evt)) {
                    Ok(mut s) => {
                        // Feed the (single) close event into the oneshot. Held
                        // in an Option so the FnMut fires at most once.
                        let mut tx_once = Some(tx);
                        s.set_onclose(move |action, code| {
                            if let Some(tx) = tx_once.take() {
                                let _ = tx.send((action, code));
                            }
                        });
                        *socket.borrow_mut() = Some(s);
                        attempt = 0;
                        on_status(ConnStatus::Connected);

                        // Block until this connection closes — or the caller
                        // drops the socket on unmount (Sender dropped →
                        // `Canceled`).
                        let outcome = rx.await;
                        if closed.get() {
                            break;
                        }
                        match outcome {
                            // Sender dropped without a close event: the socket
                            // was torn down out from under us — stop.
                            Err(_) => break,
                            Ok((CloseAction::Surface, _)) => {
                                on_status(ConnStatus::Disconnected);
                                break;
                            }
                            Ok((action, _)) => {
                                if attempt >= MAX_RECONNECT_ATTEMPTS {
                                    on_status(ConnStatus::Disconnected);
                                    break;
                                }
                                on_status(ConnStatus::Reconnecting);
                                if matches!(action, CloseAction::RefreshToken) {
                                    next_token = refresh().await;
                                }
                                gloo_timers::future::TimeoutFuture::new(
                                    backoff_delay_ms(attempt) as u32
                                )
                                .await;
                                attempt += 1;
                                // loop → reconnect
                            }
                        }
                    }
                    Err(_) => {
                        // Could not open the socket at all. Retry with backoff.
                        if attempt >= MAX_RECONNECT_ATTEMPTS {
                            on_status(ConnStatus::Disconnected);
                            break;
                        }
                        on_status(ConnStatus::Reconnecting);
                        gloo_timers::future::TimeoutFuture::new(backoff_delay_ms(attempt) as u32)
                            .await;
                        attempt += 1;
                    }
                }
            }
        });
    }
}

// Native Dioxus renderers use the WebView's WebSocket implementation. This
// retains browser-equivalent proxy/TLS behavior while structured Eval messages
// keep the Rust state reducer renderer-neutral.
#[cfg(not(target_arch = "wasm32"))]
pub mod conn {
    use super::*;
    use dioxus::prelude::*;
    use serde::{Deserialize, Serialize};
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    const NATIVE_SOCKET_SCRIPT: &str = r#"
const config = await dioxus.recv();
const MAX_MESSAGE_BYTES = 65536;
const MAX_RECONNECTS = 8;
let ws = null;
let closed = false;
let reconnects = 0;
let reconnectTimer = null;
let heartbeatTimer = null;
let currentUrl = String(config.url || "");
const allowInsecureLoopback = config.allow_insecure_loopback === true;
const bytes = (value) => new TextEncoder().encode(String(value || "")).byteLength;
const isLoopbackHost = (hostname) => hostname === "localhost"
  || hostname === "::1"
  || hostname === "10.0.2.2"
  || /^127(?:\.[0-9]{1,3}){3}$/.test(hostname);
const validUrl = (value) => {
  if (bytes(value) > 8192) return false;
  try {
    const parsed = new URL(value);
    return parsed.protocol === "wss:"
      || (allowInsecureLoopback && parsed.protocol === "ws:" && isLoopbackHost(parsed.hostname));
  }
  catch (_) { return false; }
};
const emit = (value) => dioxus.send(value);
const stopTimers = () => {
  if (reconnectTimer) clearTimeout(reconnectTimer);
  if (heartbeatTimer) clearInterval(heartbeatTimer);
  reconnectTimer = null;
  heartbeatTimer = null;
};
const connect = () => {
  if (closed || !validUrl(currentUrl)) {
    emit({ kind: "fatal", message: "The live-room socket URL is invalid" });
    return;
  }
  if (ws) { ws.onopen = ws.onmessage = ws.onclose = ws.onerror = null; try { ws.close(); } catch (_) {} }
  emit({ kind: "status", status: reconnects ? "reconnecting" : "reconnecting" });
  ws = new WebSocket(currentUrl);
  ws.onopen = () => {
    reconnects = 0;
    emit({ kind: "status", status: "connected" });
    if (heartbeatTimer) clearInterval(heartbeatTimer);
    heartbeatTimer = setInterval(() => {
      if (ws && ws.readyState === WebSocket.OPEN) ws.send('{"type":"heartbeat"}');
    }, 15000);
  };
  ws.onmessage = (event) => {
    if (typeof event.data !== "string" || bytes(event.data) > MAX_MESSAGE_BYTES) return;
    emit({ kind: "message", data: event.data });
  };
  ws.onerror = () => {};
  ws.onclose = (event) => {
    if (heartbeatTimer) clearInterval(heartbeatTimer);
    heartbeatTimer = null;
    emit({ kind: "close", code: Number(event.code || 1006) });
    if (closed || event.code === 4003 || event.code === 4001) {
      emit({ kind: "status", status: "disconnected" });
      return;
    }
    if (reconnects >= MAX_RECONNECTS) {
      emit({ kind: "status", status: "disconnected" });
      return;
    }
    const delay = Math.min(30000, 1000 * (2 ** reconnects++));
    emit({ kind: "status", status: "reconnecting" });
    reconnectTimer = setTimeout(connect, delay);
  };
};
connect();
while (!closed) {
  const command = await dioxus.recv();
  if (!command || typeof command.type !== "string") continue;
  if (command.type === "send") {
    const body = String(command.body || "");
    if (bytes(body) <= MAX_MESSAGE_BYTES && ws && ws.readyState === WebSocket.OPEN) ws.send(body);
  } else if (command.type === "reconnect") {
    const next = String(command.url || "");
    if (validUrl(next)) { currentUrl = next; reconnects = 0; connect(); }
  } else if (command.type === "close") {
    closed = true;
    stopTimers();
    if (ws) { ws.onclose = null; try { ws.close(1000, "route closed"); } catch (_) {} }
  }
}
return true;
"#;

    #[derive(Serialize)]
    struct SocketConfig<'a> {
        url: &'a str,
        allow_insecure_loopback: bool,
    }

    #[derive(Serialize)]
    #[serde(tag = "type", rename_all = "snake_case")]
    enum SocketCommand<'a> {
        Send { body: &'a str },
        Reconnect { url: &'a str },
        Close,
    }

    #[derive(Deserialize)]
    #[serde(tag = "kind", rename_all = "snake_case")]
    enum SocketEvent {
        Message { data: String },
        Close { code: u16 },
        Status { status: String },
        Fatal { message: String },
    }

    type CloseHandler = Rc<RefCell<Option<Box<dyn FnMut(CloseAction, u16)>>>>;
    type StatusHandler = Rc<RefCell<Option<Box<dyn FnMut(ConnStatus)>>>>;

    fn valid_native_socket_url(value: &str) -> bool {
        if value.len() > 8192 {
            return false;
        }
        let Ok(url) = reqwest::Url::parse(value) else {
            return false;
        };
        if url.scheme() == "wss" {
            return true;
        }
        cfg!(debug_assertions)
            && url.scheme() == "ws"
            && (matches!(url.host_str(), Some("localhost" | "::1" | "10.0.2.2"))
                || url.host_str().is_some_and(|host| host.starts_with("127.")))
    }

    pub struct LiveRoomSocket {
        eval: document::Eval,
        close_handler: CloseHandler,
        status_handler: StatusHandler,
        closed: Rc<Cell<bool>>,
    }

    impl LiveRoomSocket {
        pub fn connect(
            url: &str,
            mut on_event: impl FnMut(ServerEvent) + 'static,
        ) -> Result<Self, String> {
            if !valid_native_socket_url(url) {
                return Err("The live-room socket URL must use WSS".into());
            }
            let eval = document::eval(NATIVE_SOCKET_SCRIPT);
            eval.send(SocketConfig {
                url,
                allow_insecure_loopback: cfg!(debug_assertions),
            })
            .map_err(|_| "native live-room socket bridge unavailable".to_string())?;
            let close_handler: CloseHandler = Rc::new(RefCell::new(None));
            let status_handler: StatusHandler = Rc::new(RefCell::new(None));
            let closed = Rc::new(Cell::new(false));
            let close_task = close_handler.clone();
            let status_task = status_handler.clone();
            let closed_task = closed.clone();
            let mut receiver = eval;
            spawn(async move {
                while !closed_task.get() {
                    let Ok(event) = receiver.recv::<SocketEvent>().await else {
                        break;
                    };
                    match event {
                        SocketEvent::Message { data } if data.len() <= 65_536 => {
                            if let Ok(event) = parse_event(&data) {
                                on_event(event);
                            }
                        }
                        SocketEvent::Close { code } => {
                            if let Some(handler) = close_task.borrow_mut().as_mut() {
                                handler(classify_close_action(code), code);
                            }
                        }
                        SocketEvent::Status { status } => {
                            let status = match status.as_str() {
                                "connected" => ConnStatus::Connected,
                                "reconnecting" => ConnStatus::Reconnecting,
                                _ => ConnStatus::Disconnected,
                            };
                            if let Some(handler) = status_task.borrow_mut().as_mut() {
                                handler(status);
                            }
                        }
                        SocketEvent::Fatal { message } => {
                            tracing::warn!(error = %message, "native live-room socket failed");
                            if let Some(handler) = status_task.borrow_mut().as_mut() {
                                handler(ConnStatus::Disconnected);
                            }
                        }
                        SocketEvent::Message { .. } => {}
                    }
                }
            });
            Ok(Self {
                eval,
                close_handler,
                status_handler,
                closed,
            })
        }

        pub fn send_text(&self, text: &str) -> Result<(), String> {
            if text.len() > 65_536 {
                return Err("live-room message exceeded the safe size limit".into());
            }
            self.eval
                .send(SocketCommand::Send { body: text })
                .map_err(|_| "live-room socket is unavailable".into())
        }

        pub fn reconnect(&self, url: &str) -> Result<(), String> {
            if !valid_native_socket_url(url) {
                return Err("The live-room socket URL must use WSS".into());
            }
            self.eval
                .send(SocketCommand::Reconnect { url })
                .map_err(|_| "live-room socket is unavailable".into())
        }

        pub fn set_onclose(&mut self, handler: impl FnMut(CloseAction, u16) + 'static) {
            *self.close_handler.borrow_mut() = Some(Box::new(handler));
        }

        pub fn set_onstatus(&mut self, handler: impl FnMut(ConnStatus) + 'static) {
            *self.status_handler.borrow_mut() = Some(Box::new(handler));
        }

        pub fn close(&mut self) {
            if self.closed.replace(true) {
                return;
            }
            let _ = self.eval.send(SocketCommand::Close);
            self.close_handler.borrow_mut().take();
            self.status_handler.borrow_mut().take();
        }
    }

    impl Drop for LiveRoomSocket {
        fn drop(&mut self) {
            self.close();
        }
    }

    #[allow(clippy::type_complexity)]
    pub fn connect_reconnecting(
        socket: Rc<RefCell<Option<LiveRoomSocket>>>,
        closed: Rc<Cell<bool>>,
        make_url: Rc<dyn Fn(Option<String>) -> String>,
        on_event: Rc<dyn Fn(ServerEvent)>,
        refresh: Rc<
            dyn Fn() -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<String>>>>,
        >,
        on_status: Rc<dyn Fn(ConnStatus)>,
    ) {
        if closed.get() || socket.borrow().is_some() {
            return;
        }
        let url = make_url(None);
        let event_cb = on_event.clone();
        let Ok(mut connected) = LiveRoomSocket::connect(&url, move |event| event_cb(event)) else {
            on_status(ConnStatus::Disconnected);
            return;
        };
        let socket_for_close = socket.clone();
        let make_url_for_close = make_url.clone();
        let refresh_for_close = refresh.clone();
        let status_for_close = on_status.clone();
        let closed_for_close = closed.clone();
        connected.set_onclose(move |action, _code| match action {
            CloseAction::RefreshToken if !closed_for_close.get() => {
                let socket = socket_for_close.clone();
                let make_url = make_url_for_close.clone();
                let refresh = refresh_for_close.clone();
                let on_status = status_for_close.clone();
                spawn(async move {
                    on_status(ConnStatus::Reconnecting);
                    match refresh().await {
                        Some(token) => {
                            let url = make_url(Some(token));
                            if socket
                                .borrow()
                                .as_ref()
                                .and_then(|socket| socket.reconnect(&url).err())
                                .is_some()
                            {
                                on_status(ConnStatus::Disconnected);
                            }
                        }
                        None => on_status(ConnStatus::Disconnected),
                    }
                });
            }
            CloseAction::RefreshToken => status_for_close(ConnStatus::Disconnected),
            CloseAction::Surface => status_for_close(ConnStatus::Disconnected),
            CloseAction::Reconnect => {}
        });
        let status_cb = on_status.clone();
        connected.set_onstatus(move |status| status_cb(status));
        *socket.borrow_mut() = Some(connected);
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn native_socket_bridge_bounds_messages_and_recovers_connections() {
            for contract in [
                "MAX_MESSAGE_BYTES",
                "MAX_RECONNECTS",
                "heartbeat",
                "event.code === 4001",
                "event.code === 4003",
                "await dioxus.recv()",
                "allowInsecureLoopback",
                "isLoopbackHost",
            ] {
                assert!(
                    NATIVE_SOCKET_SCRIPT.contains(contract),
                    "missing {contract}"
                );
            }
        }

        #[test]
        fn native_socket_transport_policy_rejects_plaintext_remote_hosts() {
            assert!(valid_native_socket_url("wss://live.example.test/room"));
            assert!(!valid_native_socket_url("ws://live.example.test/room"));
            if cfg!(debug_assertions) {
                assert!(valid_native_socket_url("ws://127.0.0.1:8080/room"));
                assert!(valid_native_socket_url("ws://10.0.2.2:8080/room"));
            }
        }
    }
}
