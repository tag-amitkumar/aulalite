// crates/core-types/src/live_room.rs
//! Canonical wire types for the live-room socket protocol.
//!
//! This is the shared cross-crate definition. The backend (`BrokerEvent` in
//! `backend/src/services/live_room.rs`) and frontend (`ServerEvent` in
//! `features-courses/src/live_room_socket.rs`) currently maintain their own
//! mirror types; migrating them to consume this module is tracked in the
//! follow-up tasks of the 2026-05-15 live-room safety pass.
//!
//! These types are intentionally additive on the wire (no
//! `#[serde(deny_unknown_fields)]`) so new variants and fields can roll out
//! without breaking older clients.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Hand-raise state change for a single student.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandRaiseChanged {
    pub user_id: Uuid,
    pub raised: bool,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue_position: Option<u32>,
}

/// Notification that a promoted student has started publishing media.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StudentPublishing {
    pub user_id: Uuid,
    pub publish_path: String,
    pub display_name: String,
}

/// Notification that a previously-promoted student was demoted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StudentDemoted {
    pub user_id: Uuid,
    pub display_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum WhiteboardTool {
    Pen,
    Eraser,
}

/// Geometric vocabulary for a whiteboard element. `Freehand` is the original
/// pen/eraser polyline (every point matters). The shape kinds use only the
/// first and last point of `points` as anchors; `Text` additionally carries a
/// `text` body rendered as an SVG `<text>` at the first point. `Image`
/// references an uploaded file asset (`asset_id`) and anchors on two points
/// (top-left + bottom-right bounds), like a rect.
///
/// `#[serde(default)]` on the owning struct's `kind` field keeps older pen
/// strokes (which serialize no `kind`) parsing as `Freehand`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WhiteboardPoint {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WhiteboardStroke {
    pub id: String,
    pub points: Vec<WhiteboardPoint>,
    pub color: String,
    pub width: f32,
    pub tool: WhiteboardTool,
    /// Geometric kind. Omitted on the wire by older pen-only clients, which
    /// `#[serde(default)]` parses as `WhiteboardKind::Freehand`.
    #[serde(default)]
    pub kind: WhiteboardKind,
    /// Body for `WhiteboardKind::Text` elements. Omitted for every other kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// File-asset id for `WhiteboardKind::Image` elements (the uploaded image is
    /// referenced by id rather than embedded as a data-URL, to keep the socket
    /// payload small). Omitted for every other kind. `#[serde(default)]` keeps
    /// older streams (which never carried it) parsing as `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_id: Option<String>,
    /// The user who authored this stroke. Added additively for per-author
    /// undo/redo; `#[serde(default)]` keeps older strokes (no `author`)
    /// parsing as `None`, which falls back to the global undo behavior.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<Uuid>,
}

/// Server → client events on the live-room WebSocket.
///
/// New variants are additive. `#[serde(deny_unknown_fields)]` is intentionally
/// NOT present so old clients keep parsing newer event streams.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerEvent {
    HandRaiseChanged {
        user_id: Uuid,
        raised: bool,
        display_name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        queue_position: Option<u32>,
    },
    StudentPublishing {
        user_id: Uuid,
        publish_path: String,
        display_name: String,
    },
    StudentDemoted {
        user_id: Uuid,
        display_name: String,
    },
    WhiteboardStroke {
        stroke: WhiteboardStroke,
    },
    WhiteboardClear,
    /// Ephemeral per-user cursor position over the whiteboard. NOT persisted
    /// (no snapshot hydration) — it is only meaningful in real time. `x`/`y`
    /// are normalized board coordinates in `0.0..=1.0`.
    WhiteboardCursor {
        user_id: Uuid,
        display_name: String,
        x: f32,
        y: f32,
    },
    /// The teacher toggled whether non-teachers may draw on the board. The
    /// server gates `WhiteboardStroke`/`WhiteboardErase` from non-teachers on
    /// this flag and hydrates the current value on connect.
    DrawPermissionChanged {
        open: bool,
    },
    /// A client-issued command failed server-side. The client should surface
    /// `reason` (already user-facing) and not retry automatically.
    CommandFailed {
        command: String,
        reason: String,
    },
    /// A teacher started an in-class poll. `options` is the ordered list of
    /// answer choices (2..=6); votes reference an option by its index.
    /// Ephemeral — not hydrated from any board snapshot; late joiners simply
    /// miss a poll that started before they connected (acceptable for a
    /// real-time prompt).
    PollStarted {
        poll_id: Uuid,
        question: String,
        options: Vec<String>,
    },
    /// Live tally update for the active poll: `counts[i]` is the vote count for
    /// `options[i]`. Broadcast to everyone after each accepted vote.
    PollResults {
        poll_id: Uuid,
        counts: Vec<u32>,
    },
    /// The teacher ended the poll; `counts` carries the final tally. Clients
    /// freeze the result and stop accepting votes for this `poll_id`.
    PollEnded {
        poll_id: Uuid,
        counts: Vec<u32>,
    },
}

/// Bounds on a poll's option list, shared by the start handler and the UI
/// composer so both reject the same shapes.
pub const POLL_MIN_OPTIONS: usize = 2;
pub const POLL_MAX_OPTIONS: usize = 6;
/// Max length (chars) of a poll question or any single option label.
pub const POLL_QUESTION_MAX_LEN: usize = 240;
pub const POLL_OPTION_MAX_LEN: usize = 120;

/// WebSocket close codes used by the live-room socket.
///
/// Values are in the application-private range (4000-4999) per RFC 6455.
pub mod close_codes {
    /// The client's JWT was valid at connect time but has since expired.
    /// The client should refresh and reconnect.
    pub const AUTH_EXPIRED: u16 = 4001;

    /// The client's JWT was invalid (signature, audience, or principal).
    /// The client should not auto-reconnect.
    pub const AUTH_INVALID: u16 = 4003;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_failed_round_trips_json() {
        let evt = ServerEvent::CommandFailed {
            command: "demote_hand".into(),
            reason: "broker unavailable".into(),
        };
        let s = serde_json::to_string(&evt).unwrap();
        let d: ServerEvent = serde_json::from_str(&s).unwrap();
        assert_eq!(d, evt);
    }

    #[test]
    fn hand_raise_changed_includes_display_name() {
        let evt = ServerEvent::HandRaiseChanged {
            user_id: Uuid::nil(),
            raised: true,
            display_name: "Ada".into(),
            queue_position: Some(2),
        };
        let s = serde_json::to_string(&evt).unwrap();
        assert!(s.contains("\"display_name\":\"Ada\""));
        let d: ServerEvent = serde_json::from_str(&s).unwrap();
        assert_eq!(d, evt);
    }

    #[test]
    fn unknown_fields_are_ignored_on_server_event() {
        // Additive-on-the-wire guarantee: unknown future fields must not break
        // the parser.
        let raw = r#"{"type":"student_demoted","user_id":"00000000-0000-0000-0000-000000000000","display_name":"Bo","future_field":42}"#;
        let d: ServerEvent = serde_json::from_str(raw).unwrap();
        match d {
            ServerEvent::StudentDemoted { display_name, .. } => {
                assert_eq!(display_name, "Bo");
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn close_codes_match_design() {
        assert_eq!(close_codes::AUTH_EXPIRED, 4001);
        assert_eq!(close_codes::AUTH_INVALID, 4003);
    }

    #[test]
    fn whiteboard_stroke_round_trips_json() {
        let evt = ServerEvent::WhiteboardStroke {
            stroke: WhiteboardStroke {
                id: "stroke-1".into(),
                points: vec![
                    WhiteboardPoint { x: 0.1, y: 0.2 },
                    WhiteboardPoint { x: 0.3, y: 0.4 },
                ],
                color: "#1f2937".into(),
                width: 4.0,
                tool: WhiteboardTool::Pen,
                kind: WhiteboardKind::Freehand,
                text: None,
                asset_id: None,
                author: None,
            },
        };
        let s = serde_json::to_string(&evt).unwrap();
        assert!(s.contains("\"type\":\"whiteboard_stroke\""));
        let d: ServerEvent = serde_json::from_str(&s).unwrap();
        assert_eq!(d, evt);
    }

    #[test]
    fn whiteboard_stroke_without_kind_defaults_to_freehand() {
        // Backward-compat: a pen-only client serializes no `kind`/`text`. The
        // additive fields must default rather than fail to parse.
        let raw = r##"{"type":"whiteboard_stroke","stroke":{"id":"s1","points":[{"x":0.1,"y":0.2},{"x":0.3,"y":0.4}],"color":"#111827","width":4.0,"tool":"pen"}}"##;
        match serde_json::from_str::<ServerEvent>(raw).unwrap() {
            ServerEvent::WhiteboardStroke { stroke } => {
                assert_eq!(stroke.kind, WhiteboardKind::Freehand);
                assert_eq!(stroke.text, None);
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn whiteboard_text_element_round_trips_json() {
        let evt = ServerEvent::WhiteboardStroke {
            stroke: WhiteboardStroke {
                id: "t1".into(),
                points: vec![
                    WhiteboardPoint { x: 0.2, y: 0.3 },
                    WhiteboardPoint { x: 0.5, y: 0.4 },
                ],
                color: "#111827".into(),
                width: 4.0,
                tool: WhiteboardTool::Pen,
                kind: WhiteboardKind::Text,
                text: Some("hello".into()),
                asset_id: None,
                author: None,
            },
        };
        let s = serde_json::to_string(&evt).unwrap();
        assert!(s.contains("\"kind\":\"text\""));
        assert!(s.contains("\"text\":\"hello\""));
        let d: ServerEvent = serde_json::from_str(&s).unwrap();
        assert_eq!(d, evt);
    }

    #[test]
    fn whiteboard_image_element_round_trips_json() {
        let evt = ServerEvent::WhiteboardStroke {
            stroke: WhiteboardStroke {
                id: "img1".into(),
                points: vec![
                    WhiteboardPoint { x: 0.2, y: 0.3 },
                    WhiteboardPoint { x: 0.5, y: 0.6 },
                ],
                color: "#111827".into(),
                width: 4.0,
                tool: WhiteboardTool::Pen,
                kind: WhiteboardKind::Image,
                text: None,
                asset_id: Some("asset-123".into()),
                author: None,
            },
        };
        let s = serde_json::to_string(&evt).unwrap();
        assert!(s.contains("\"kind\":\"image\""));
        assert!(s.contains("\"asset_id\":\"asset-123\""));
        let d: ServerEvent = serde_json::from_str(&s).unwrap();
        assert_eq!(d, evt);
    }

    #[test]
    fn whiteboard_clear_round_trips_json() {
        let evt = ServerEvent::WhiteboardClear;
        let s = serde_json::to_string(&evt).unwrap();
        assert_eq!(s, "{\"type\":\"whiteboard_clear\"}");
        let d: ServerEvent = serde_json::from_str(&s).unwrap();
        assert_eq!(d, evt);
    }

    #[test]
    fn whiteboard_cursor_round_trips_json() {
        let evt = ServerEvent::WhiteboardCursor {
            user_id: Uuid::nil(),
            display_name: "Ada".into(),
            x: 0.42,
            y: 0.73,
        };
        let s = serde_json::to_string(&evt).unwrap();
        assert!(s.contains("\"type\":\"whiteboard_cursor\""));
        let d: ServerEvent = serde_json::from_str(&s).unwrap();
        assert_eq!(d, evt);
    }

    #[test]
    fn draw_permission_changed_round_trips_json() {
        let evt = ServerEvent::DrawPermissionChanged { open: true };
        let s = serde_json::to_string(&evt).unwrap();
        assert!(s.contains("\"type\":\"draw_permission_changed\""));
        assert!(s.contains("\"open\":true"));
        let d: ServerEvent = serde_json::from_str(&s).unwrap();
        assert_eq!(d, evt);
    }

    #[test]
    fn poll_events_round_trip_json() {
        let started = ServerEvent::PollStarted {
            poll_id: Uuid::nil(),
            question: "Favorite language?".into(),
            options: vec!["Rust".into(), "Go".into(), "Python".into()],
        };
        let s = serde_json::to_string(&started).unwrap();
        assert!(s.contains("\"type\":\"poll_started\""));
        assert_eq!(serde_json::from_str::<ServerEvent>(&s).unwrap(), started);

        let results = ServerEvent::PollResults {
            poll_id: Uuid::nil(),
            counts: vec![3, 1, 0],
        };
        let s = serde_json::to_string(&results).unwrap();
        assert!(s.contains("\"type\":\"poll_results\""));
        assert_eq!(serde_json::from_str::<ServerEvent>(&s).unwrap(), results);

        let ended = ServerEvent::PollEnded {
            poll_id: Uuid::nil(),
            counts: vec![3, 1, 0],
        };
        let s = serde_json::to_string(&ended).unwrap();
        assert!(s.contains("\"type\":\"poll_ended\""));
        assert_eq!(serde_json::from_str::<ServerEvent>(&s).unwrap(), ended);
    }

    #[test]
    fn whiteboard_stroke_with_author_round_trips_and_legacy_defaults_none() {
        // With an author present.
        let evt = ServerEvent::WhiteboardStroke {
            stroke: WhiteboardStroke {
                id: "a1".into(),
                points: vec![
                    WhiteboardPoint { x: 0.1, y: 0.2 },
                    WhiteboardPoint { x: 0.3, y: 0.4 },
                ],
                color: "#111827".into(),
                width: 4.0,
                tool: WhiteboardTool::Pen,
                kind: WhiteboardKind::Freehand,
                text: None,
                asset_id: None,
                author: Some(Uuid::nil()),
            },
        };
        let s = serde_json::to_string(&evt).unwrap();
        assert!(s.contains("\"author\":"));
        let d: ServerEvent = serde_json::from_str(&s).unwrap();
        assert_eq!(d, evt);

        // Legacy stroke (no `author`) defaults to None.
        let raw = r##"{"type":"whiteboard_stroke","stroke":{"id":"s1","points":[{"x":0.1,"y":0.2},{"x":0.3,"y":0.4}],"color":"#111827","width":4.0,"tool":"pen"}}"##;
        match serde_json::from_str::<ServerEvent>(raw).unwrap() {
            ServerEvent::WhiteboardStroke { stroke } => assert_eq!(stroke.author, None),
            other => panic!("wrong variant: {other:?}"),
        }
    }
}
