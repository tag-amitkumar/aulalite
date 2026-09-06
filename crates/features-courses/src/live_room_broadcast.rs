// crates/features-courses/src/live_room_broadcast.rs
//! Teacher publish UI shared by web and native WebView renderers.
//! Task 25: Adds chat / presence / hand-raise sidebars (is_teacher = true).
//! Task 4a: Persistent WebSocket connection + state hydration.

use crate::live_room_chat::{ChatMessage, LiveRoomChat};
use crate::live_room_hand_raise::{HandRaiseEntry, LiveRoomHandRaise};
use crate::live_room_presence::{LiveRoomPresence, PresenceParticipant};
use crate::live_room_shell::SessionStatus;
use design_system::{
    Badge, BadgeTone, Button, ButtonVariant, FormError, HeadingLevel, Loading, PageHeader, Select,
};
use dioxus::prelude::*;

// ---------------------------------------------------------------------------
// Tabbed-stage + dock UI state (cross-target; renders under SSR)
// ---------------------------------------------------------------------------

/// Which surface fills the stage. Default = Camera.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum StageTab {
    Camera,
    Screen,
    Whiteboard,
}

/// Which right-rail dock panel is active. `Breakout` is teacher-only.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DockTab {
    Chat,
    People,
    Hands,
    Breakout,
}

// ---------------------------------------------------------------------------
// Broadcast publish state (unchanged)
// ---------------------------------------------------------------------------

#[derive(Clone, PartialEq)]
enum PublishState {
    Idle,
    GoingLive,
    Live {
        main_active: bool,
        screen_active: bool,
    },
    Error(String),
}

// ---------------------------------------------------------------------------
// Sidebar state for the teacher view
// ---------------------------------------------------------------------------

#[derive(Clone, Default, PartialEq)]
struct BroadcastSidebarState {
    messages: Vec<ChatMessage>,
    presence_count: u32,
    participants: Option<Vec<PresenceParticipant>>,
    queue: Vec<HandRaiseEntry>,
    /// Last server-side command failure reason, surfaced as a
    /// `.system-state--error` toast above the teacher's sidebars.
    command_error: Option<String>,
    /// The teacher's whiteboard, maintained from server events (snapshot on
    /// connect, echoed strokes, undo removals, clears) so a reload or
    /// reconnect restores the board instead of starting blank.
    whiteboard: crate::live_room_whiteboard::WhiteboardState,
    /// In-flight floating emoji reactions (capped; see `live_room_reactions`).
    reactions: Vec<crate::live_room_reactions::FloatingReaction>,
    /// Monotonic sequence backing the stable keys of `reactions`.
    reaction_seq: u64,
    /// Live per-user whiteboard cursors (ephemeral, not persisted).
    cursors: Vec<crate::live_room_whiteboard::RemoteCursor>,
    /// Whether the teacher has opened the whiteboard for student drawing.
    draw_open: bool,
    /// Active in-class poll (ephemeral, broker-driven; see `live_room_polls`).
    polls: crate::live_room_polls::PollClientState,
    /// Live breakout-room state (ephemeral, broker-driven). Drives the teacher
    /// breakout panel.
    breakout: crate::breakout_rooms::BreakoutClientState,
}

// ---------------------------------------------------------------------------
// Props
// ---------------------------------------------------------------------------

#[derive(Props, Clone, PartialEq)]
pub struct LiveRoomBroadcastProps {
    pub session_id: String,
    pub status: SessionStatus,
}

// ---------------------------------------------------------------------------
// Root component
// ---------------------------------------------------------------------------

pub fn LiveRoomBroadcast(props: LiveRoomBroadcastProps) -> Element {
    let mut state = use_signal(|| {
        if props.status == SessionStatus::Live {
            PublishState::Live {
                main_active: true,
                screen_active: false,
            }
        } else {
            PublishState::Idle
        }
    });

    let sidebar_state = use_signal(BroadcastSidebarState::default);

    // Signed-in user's id (sub claim) for per-author whiteboard undo/redo +
    // cursor self-filter. Empty if unavailable (SSR/tests, via try_consume_context)
    // -> global fallback.
    let local_user_id = try_consume_context::<Signal<crate::api::ApiContext>>()
        .map(|sig| {
            crate::live_room_whiteboard::user_id_from_jwt(&sig.read().id_token).unwrap_or_default()
        })
        .unwrap_or_default();

    // Mic / camera toggles. They flip `enabled` on the captured tracks —
    // the WHIP publish keeps running, so muting is instant and reversible
    // without renegotiation. Reset to on whenever a new stream is captured.
    let mic_enabled = use_signal(|| true);
    let cam_enabled = use_signal(|| true);

    // The teacher's own camera/mic stream, captured by `go_live_flow` via
    // getUserMedia. Held here so the broadcast UI can render a local
    // self-preview `<video>` — previously the stream was published to MediaMTX
    // but never shown back to the teacher, so the broadcaster saw no video of
    // themselves at all.
    #[cfg(target_arch = "wasm32")]
    let self_stream: Signal<Option<web_sys::MediaStream>> = use_signal(|| None);

    #[cfg(target_arch = "wasm32")]
    let screen_stream: Signal<Option<web_sys::MediaStream>> = use_signal(|| None);

    // Raw camera/mic stream from getUserMedia, kept separate from `self_stream`
    // (which holds what is previewed + published — possibly a background-FX
    // canvas stream). We need the raw handle to stop the camera tracks on End
    // Class: with background FX the camera track lives inside the JS bridge, not
    // in the WHIP sender, so `publisher.close()` alone would leave the camera
    // light on.
    #[cfg(target_arch = "wasm32")]
    let camera_stream: Signal<Option<web_sys::MediaStream>> = use_signal(|| None);

    // Selected background effect (Normal / Blur / Studio virtual background).
    // Applied at go-live and switched live through the JS bridge. Declared on
    // both targets so the control renders under host SSR (where it's inert).
    let blur_mode = use_signal(|| crate::live_room_video_fx::FxMode::Off);

    // Teacher-side network quality, polled from the publisher's getStats while
    // live (see the poll effect below). Cross-target so the badge renders under
    // host SSR.
    let quality = use_signal(|| crate::live_room_stats::NetQuality::Unknown);

    // A live class needs an operator-facing view of the whole delivery chain,
    // not just a WebRTC quality badge. These signals combine browser state with
    // the backend's MediaMTX/recording probe in `live_room_health` below.
    let socket_status = use_signal(|| crate::live_room_socket::ConnStatus::Disconnected);
    let server_health = use_signal(|| None::<crate::live_room_health::LiveSessionHealthDto>);
    let health_fetch_error = use_signal(|| None::<String>);
    // Ordering machinery for "has the server actually looked at this publisher
    // yet?". Sequence numbers rather than timestamps: the server's `checked_at`
    // comes off a different clock than the browser's, and the two can disagree
    // by more than the window being measured.
    //
    // `health_poll_seq` counts polls STARTED, `health_snapshot_seq` records
    // which poll produced the snapshot currently held, and `publisher_seen_at`
    // freezes `health_poll_seq` at the instant the publisher appeared. Counting
    // starts, not completions, is what makes a poll that was already in flight
    // when the publisher landed come back correctly marked as pre-publisher --
    // it queried a media server that had no path yet, however late it lands.
    let health_poll_seq = use_signal(|| 0u64);
    let health_snapshot_seq = use_signal(|| None::<u64>);
    let publisher_seen_at = use_signal(|| None::<u64>);
    let mut diagnostics_open = use_signal(|| false);

    #[cfg(not(target_arch = "wasm32"))]
    let native_capabilities = use_signal(|| None::<crate::live_room_native::NativeCapabilities>);
    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut capabilities = native_capabilities;
        use_future(move || async move {
            if let Ok(value) = crate::live_room_native::capabilities().await {
                capabilities.set(Some(value));
            }
        });
    }

    // In-call camera picker: the available cameras + the selected device id.
    // Populated on first Live entry (see the effect below). Switching is
    // FX-aware. Cross-target so the <select> renders under host SSR.
    let cameras = use_signal(Vec::<crate::live_room_devices::MediaDevice>::new);
    let selected_camera = use_signal(String::new);

    // Mobile front/back camera selection. Starts on the front (selfie) camera;
    // the in-call flip button toggles it and re-captures via `facingMode`. Only
    // surfaced on mobile/touch devices (see `is_mobile()` below). Cross-target so
    // the control renders inert under host SSR.
    let facing = use_signal(|| crate::live_room_devices::Facing::Front);

    // Attach the local stream to the self-preview element once we're live. Runs
    // after render (so the `<video>` exists) and re-runs when either the
    // publish state or the captured stream changes. Muted to avoid the teacher
    // hearing their own mic.
    #[cfg(target_arch = "wasm32")]
    {
        let state_eff = state;
        let stream_eff = self_stream;
        use_effect(move || {
            let is_live = matches!(*state_eff.read(), PublishState::Live { .. });
            let stream_opt = stream_eff.read().clone();
            if let (true, Some(stream)) = (is_live, stream_opt) {
                if let Some(el) = web_sys::window()
                    .and_then(|w| w.document())
                    .and_then(|d| d.get_element_by_id("broadcast-self-video"))
                {
                    use wasm_bindgen::JsCast;
                    if let Ok(media_el) = el.dyn_into::<web_sys::HtmlMediaElement>() {
                        media_el.set_src_object(Some(&stream));
                        media_el.set_muted(true);
                    }
                }
            }
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        let state = state;
        use_effect(move || {
            let (main_active, screen_active) = match *state.read() {
                PublishState::Live {
                    main_active,
                    screen_active,
                } => (main_active, screen_active),
                _ => return,
            };
            spawn(async move {
                if main_active {
                    let _ =
                        crate::live_room_native::attach_publisher("main", "broadcast-self-video")
                            .await;
                }
                if screen_active {
                    let _ = crate::live_room_native::attach_publisher(
                        "screen",
                        "broadcast-screen-video",
                    )
                    .await;
                }
            });
        });
    }

    // A freshly captured stream starts with all tracks enabled — reflect that
    // in the toggle state (e.g. after re-entering Live for a new class).
    #[cfg(target_arch = "wasm32")]
    {
        let stream_eff = self_stream;
        let mut mic = mic_enabled;
        let mut cam = cam_enabled;
        use_effect(move || {
            if stream_eff.read().is_some() {
                mic.set(true);
                cam.set(true);
            }
        });
    }

    #[cfg(target_arch = "wasm32")]
    {
        let state_eff = state;
        let stream_eff = screen_stream;
        use_effect(move || {
            // Re-run on BOTH the publish state and the captured screen stream.
            // `screen_stream` is set inside `start_screen_share_flow` BEFORE the
            // publish state flips to `screen_active: true`, which is what mounts
            // the `<video id="broadcast-screen-video">` element. If this effect
            // only depended on `screen_stream` (as it once did) it would run
            // while the element is still unmounted, `set_src_object` on nothing,
            // and never re-fire — so the teacher saw their own camera but never
            // the screen they were sharing. Subscribing to `state` here makes
            // the effect re-run once the element mounts, mirroring the working
            // self-camera effect above.
            let screen_active = matches!(
                *state_eff.read(),
                PublishState::Live {
                    screen_active: true,
                    ..
                }
            );
            let stream_opt = stream_eff.read().clone();
            if let (true, Some(stream)) = (screen_active, stream_opt) {
                if let Some(el) = web_sys::window()
                    .and_then(|w| w.document())
                    .and_then(|d| d.get_element_by_id("broadcast-screen-video"))
                {
                    use wasm_bindgen::JsCast;
                    if let Ok(media_el) = el.dyn_into::<web_sys::HtmlMediaElement>() {
                        media_el.set_src_object(Some(&stream));
                        media_el.set_muted(true);
                    }
                }
            }
        });
    }

    // Pick up the route-owned session aggregate provided by `LiveSessionShell`.
    // When present, `go_live_flow` / `end_class_flow` route through the
    // session so it owns the publisher and the POST-then-close ordering.
    let session: Option<Signal<crate::live_room_session::LiveRoomSession>> =
        try_consume_context::<Signal<crate::live_room_session::LiveRoomSession>>();

    // Auto-capture + publish when the room is Live but this client hasn't
    // published yet. The broadcast mounts directly in `Live` whenever the
    // session is already live (e.g. started via "Start session now", or
    // re-opened mid-class) — in that path the explicit "Go Live" button never
    // appears, so without this the teacher's camera is never captured and
    // nothing is streamed or previewed. `go_live_flow` re-POSTs /go-live (a
    // no-op refresh on an already-live session the teacher owns) and runs
    // getUserMedia + WHIP. Guarded so it fires once per live entry.
    #[cfg(target_arch = "wasm32")]
    {
        let state_auto = state;
        let self_stream_auto = self_stream;
        let camera_stream_auto = camera_stream;
        let blur_mode_auto = blur_mode;
        let session_auto = session;
        let session_id_auto = props.session_id.clone();
        let publishing: std::rc::Rc<std::cell::Cell<bool>> =
            use_hook(|| std::rc::Rc::new(std::cell::Cell::new(false)));
        use_effect(move || {
            let is_live = matches!(*state_auto.read(), PublishState::Live { .. });
            if !is_live {
                // Reset the guard so re-entering Live (new class) re-publishes.
                publishing.set(false);
                return;
            }
            if self_stream_auto.read().is_some() || publishing.get() {
                return;
            }
            publishing.set(true);
            let mut state_for_auto = state_auto;
            let session_id = session_id_auto.clone();
            let session = session_auto;
            let self_stream = self_stream_auto;
            let camera_stream = camera_stream_auto;
            // `peek` (non-subscribing) so a later background-FX change doesn't
            // re-run this auto-publish effect.
            let fx_mode = *blur_mode_auto.peek();
            // The task no longer clears `publishing` on failure -- it parks the
            // room in PublishState::Error, and the effect's own `!is_live`
            // branch resets the guard. So the task needs no handle on it.
            wasm_bindgen_futures::spawn_local(async move {
                if let Err(e) = go_live_flow(
                    &session_id,
                    session,
                    self_stream,
                    camera_stream,
                    fx_mode,
                    String::new(),
                    String::new(),
                    String::new(),
                )
                .await
                {
                    web_sys::console::warn_1(
                        &format!("[live_room_broadcast] auto-publish on live failed: {e}").into(),
                    );
                    // Park in a TERMINAL error state instead of clearing the
                    // guard.
                    //
                    // Clearing `publishing` used to be harmless: publish()
                    // returned Ok even when ICE never came up, so this arm was
                    // only reached on a hard WHIP 4xx. Now that publish()
                    // verifies the connection, the common real-world failure --
                    // teacher and SFU on different networks with no working
                    // TURN relay -- lands here every time. And because
                    // `go_live_flow` also resets `self_stream` to None on
                    // failure, both of this effect's guards would be clear on
                    // the next render: it would re-acquire the camera, re-POST
                    // /go-live, leave another MediaMTX session to time out, and
                    // burn another CONNECT_TIMEOUT_MS, forever, while the UI
                    // still claimed to be Live and the only trace was this
                    // console line.
                    //
                    // PublishState::Error also drives the effect's own
                    // `!is_live` branch, which clears `publishing` for us, so
                    // the teacher's explicit retry still works. This mirrors
                    // `reset_main_publish`, which parks at Idle for exactly
                    // this reason ("so a denied permission does not enter a
                    // retry loop").
                    state_for_auto.set(PublishState::Error(e));
                }
            });
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        let current_state = state;
        let session_id = props.session_id.clone();
        let session = session;
        let started: std::rc::Rc<std::cell::Cell<bool>> =
            use_hook(|| std::rc::Rc::new(std::cell::Cell::new(false)));
        use_effect(move || {
            if !matches!(*current_state.read(), PublishState::Live { .. }) || started.get() {
                return;
            }
            started.set(true);
            let session_id = session_id.clone();
            let started_on_error = started.clone();
            let mut state = current_state;
            spawn(async move {
                if let Err(message) = go_live_flow_native(
                    &session_id,
                    session,
                    crate::live_room_prejoin::PrejoinChoice::default(),
                )
                .await
                {
                    started_on_error.set(false);
                    state.set(PublishState::Error(message));
                }
            });
        });
    }

    // Establish the persistent room socket on web and native renderers.
    let socket =
        use_persistent_socket_broadcast(props.session_id.clone(), sidebar_state, socket_status);

    // Probe the server-side half of the room immediately and then at a modest
    // cadence. Fifteen seconds is fast enough for an instructor to recover a
    // class while avoiding a per-frame/per-stats-poll API load multiplier.
    {
        let health_api = use_context::<Signal<crate::api::ApiContext>>();
        let health_session_id = props.session_id.clone();
        let health_w = server_health;
        let error_w = health_fetch_error;
        let poll_seq_w = health_poll_seq;
        let snapshot_seq_w = health_snapshot_seq;
        use_future(move || {
            let session_id = health_session_id.clone();
            async move {
                loop {
                    refresh_server_health(health_api, session_id.clone(), health_w, error_w, poll_seq_w, snapshot_seq_w).await;
                    #[cfg(target_arch = "wasm32")]
                    gloo_timers::future::TimeoutFuture::new(15_000).await;
                    #[cfg(not(target_arch = "wasm32"))]
                    crate::live_room_native::delay(15_000).await;
                }
            }
        });
    }

    // Stamp the poll sequence at the instant the publisher appears (and clear
    // it when the publisher goes away, so a republish is re-evaluated from
    // scratch rather than inheriting the previous run's confirmation).
    #[cfg(target_arch = "wasm32")]
    {
        let session_watch = session;
        let poll_seq = health_poll_seq;
        let mut seen_at = publisher_seen_at;
        use_effect(move || {
            let ready = session_watch.and_then(|s| s.read().publisher_pc()).is_some();
            let current = *poll_seq.read();
            // `peek`, not `read`: writing a signal this effect subscribes to
            // would re-trigger it forever. Bind before matching -- a guard held
            // in the scrutinee lives for the whole `match`, and the arms write
            // the same signal.
            let recorded = *seen_at.peek();
            match (ready, recorded) {
                (true, None) => seen_at.set(Some(current)),
                (false, Some(_)) => seen_at.set(None),
                _ => {}
            }
        });
    }

    // Build sidebar action closures — teacher view, so is_teacher = true.
    let on_send = {
        let socket = socket.clone();
        move |body: String| {
            let payload = serde_json::json!({"type": "chat", "body": body}).to_string();
            if let Some(s) = socket.borrow().as_ref() {
                let _ = s.send_text(&payload);
            }
        }
    };

    let on_delete = {
        let socket = socket.clone();
        move |id: String| {
            let payload =
                serde_json::json!({"type": "delete_message", "message_id": id}).to_string();
            if let Some(s) = socket.borrow().as_ref() {
                let _ = s.send_text(&payload);
            }
        }
    };

    // Teachers don't raise their own hand; on_raise is required by the prop but
    // the teacher branch of LiveRoomHandRaise never invokes it.
    let on_raise = move |_raise: bool| {};

    let on_accept = {
        let socket = socket.clone();
        move |uid: String| {
            let payload = serde_json::json!({"type": "accept_hand", "user_id": uid}).to_string();
            if let Some(s) = socket.borrow().as_ref() {
                let _ = s.send_text(&payload);
            }
        }
    };

    let on_demote = {
        let socket = socket.clone();
        move |uid: String| {
            let payload = serde_json::json!({"type": "demote_hand", "user_id": uid}).to_string();
            if let Some(s) = socket.borrow().as_ref() {
                let _ = s.send_text(&payload);
            }
        }
    };

    // on_kick closure — no broadcast UI button yet; closure exists for future use.
    #[allow(unused_variables)]
    let on_kick = {
        let socket = socket.clone();
        move |uid: String| {
            let payload = serde_json::json!({"type": "kick", "user_id": uid}).to_string();
            if let Some(s) = socket.borrow().as_ref() {
                let _ = s.send_text(&payload);
            }
        }
    };

    let on_whiteboard_stroke = {
        let socket = socket.clone();
        move |stroke: crate::live_room_whiteboard::WhiteboardStroke| {
            let payload = serde_json::json!({
                "type": "whiteboard_stroke",
                "stroke": stroke,
            })
            .to_string();
            if let Some(s) = socket.borrow().as_ref() {
                let _ = s.send_text(&payload);
            }
        }
    };

    let on_whiteboard_clear = {
        let socket = socket.clone();
        move |_| {
            if let Some(s) = socket.borrow().as_ref() {
                let _ = s.send_text(r#"{"type":"whiteboard_clear"}"#);
            }
        }
    };

    let on_whiteboard_undo = {
        let socket = socket.clone();
        move |_| {
            if let Some(s) = socket.borrow().as_ref() {
                let _ = s.send_text(r#"{"type":"whiteboard_undo"}"#);
            }
        }
    };

    let on_react = {
        let socket = socket.clone();
        move |emoji: String| {
            let payload = serde_json::json!({"type": "reaction", "emoji": emoji}).to_string();
            if let Some(s) = socket.borrow().as_ref() {
                let _ = s.send_text(&payload);
            }
        }
    };

    let on_whiteboard_erase = {
        let socket = socket.clone();
        move |stroke_id: String| {
            let payload = serde_json::json!({
                "type": "whiteboard_erase",
                "stroke_id": stroke_id,
            })
            .to_string();
            if let Some(s) = socket.borrow().as_ref() {
                let _ = s.send_text(&payload);
            }
        }
    };

    let on_whiteboard_cursor = {
        let socket = socket.clone();
        move |(x, y): (f32, f32)| {
            let payload =
                serde_json::json!({"type": "whiteboard_cursor", "x": x, "y": y}).to_string();
            if let Some(s) = socket.borrow().as_ref() {
                let _ = s.send_text(&payload);
            }
        }
    };

    let on_set_draw_permission = {
        let socket = socket.clone();
        move |open: bool| {
            let payload =
                serde_json::json!({"type": "set_draw_permission", "open": open}).to_string();
            if let Some(s) = socket.borrow().as_ref() {
                let _ = s.send_text(&payload);
            }
        }
    };

    let on_start_poll = {
        let socket = socket.clone();
        move |(question, options): (String, Vec<String>)| {
            let payload = serde_json::json!({
                "type": "start_poll",
                "question": question,
                "options": options,
            })
            .to_string();
            if let Some(s) = socket.borrow().as_ref() {
                let _ = s.send_text(&payload);
            }
        }
    };

    let on_end_poll = {
        let socket = socket.clone();
        move |poll_id: String| {
            let payload = serde_json::json!({"type": "end_poll", "poll_id": poll_id}).to_string();
            if let Some(s) = socket.borrow().as_ref() {
                let _ = s.send_text(&payload);
            }
        }
    };

    // Breakout-room socket commands. Each mirrors the existing send-through-
    // socket pattern: build the JSON envelope and write it if the socket is up.
    let on_breakout_auto_split = {
        let socket = socket.clone();
        move |room_count: usize| {
            let payload = serde_json::json!({
                "type": "auto_split_breakouts",
                "room_count": room_count,
            })
            .to_string();
            if let Some(s) = socket.borrow().as_ref() {
                let _ = s.send_text(&payload);
            }
        }
    };

    let on_breakout_create = {
        let socket = socket.clone();
        move |names: Vec<String>| {
            let payload = serde_json::json!({
                "type": "create_breakouts",
                "names": names,
            })
            .to_string();
            if let Some(s) = socket.borrow().as_ref() {
                let _ = s.send_text(&payload);
            }
        }
    };

    let on_breakout_assign = {
        let socket = socket.clone();
        move |(user_id, room_id): (String, String)| {
            // An empty room_id means "back to the main room" → send `null`.
            let room_id_val = if room_id.is_empty() {
                serde_json::Value::Null
            } else {
                serde_json::Value::String(room_id)
            };
            let payload = serde_json::json!({
                "type": "assign_breakout",
                "user_id": user_id,
                "room_id": room_id_val,
            })
            .to_string();
            if let Some(s) = socket.borrow().as_ref() {
                let _ = s.send_text(&payload);
            }
        }
    };

    let on_breakout_open = {
        let socket = socket.clone();
        move |_| {
            if let Some(s) = socket.borrow().as_ref() {
                let _ = s.send_text(r#"{"type":"open_breakouts"}"#);
            }
        }
    };

    let on_breakout_close = {
        let socket = socket.clone();
        move |_| {
            if let Some(s) = socket.borrow().as_ref() {
                let _ = s.send_text(r#"{"type":"close_breakouts"}"#);
            }
        }
    };

    let session_id = props.session_id.clone();
    let on_go_live = {
        let session_id = session_id.clone();
        let session = session;
        move |choice: crate::live_room_prejoin::PrejoinChoice| {
            state.set(PublishState::GoingLive);
            #[cfg(target_arch = "wasm32")]
            {
                let session_id = session_id.clone();
                let session = session;
                let mut state_for_async = state;
                let self_stream = self_stream;
                let camera_stream = camera_stream;
                let fx_mode = blur_mode();
                let camera_id = choice.camera_id;
                let mic_id = choice.mic_id;
                wasm_bindgen_futures::spawn_local(async move {
                    let result = go_live_flow(
                        &session_id,
                        session,
                        self_stream,
                        camera_stream,
                        fx_mode,
                        String::new(),
                        camera_id,
                        mic_id,
                    )
                    .await;
                    match result {
                        Ok(_) => state_for_async.set(PublishState::Live {
                            main_active: true,
                            screen_active: false,
                        }),
                        Err(e) => state_for_async.set(PublishState::Error(e)),
                    }
                });
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                let session_id = session_id.clone();
                let session = session;
                let mut state_for_async = state;
                spawn(async move {
                    match go_live_flow_native(&session_id, session, choice).await {
                        Ok(()) => state_for_async.set(PublishState::Live {
                            main_active: true,
                            screen_active: false,
                        }),
                        Err(message) => state_for_async.set(PublishState::Error(message)),
                    }
                });
            }
        }
    };

    let session_id_for_end = session_id.clone();
    let sidebar_state_for_end = sidebar_state;
    let on_end = move |_| {
        #[cfg(target_arch = "wasm32")]
        {
            let session_id = session_id_for_end.clone();
            let session = session;
            let mut sidebar_for_async = sidebar_state_for_end;
            // Drop the local self-preview stream handle so the preview element
            // releases it (the publisher's close() stops the underlying tracks).
            let mut self_stream = self_stream;
            self_stream.set(None);
            let mut screen_stream = screen_stream;
            let mut camera_stream = camera_stream;
            // Tear down the background-FX pipeline and stop the RAW camera/mic
            // tracks. With FX the camera feeds the JS bridge (not the WHIP
            // sender), so this is what actually turns the camera light off.
            crate::live_room_video_fx::stop();
            let cam = camera_stream.write().take();
            wasm_bindgen_futures::spawn_local(async move {
                if let Some(stream) = screen_stream.write().take() {
                    stop_media_stream_tracks(&stream);
                }
                if let Some(stream) = cam {
                    stop_media_stream_tracks(&stream);
                }
                if let Err(e) = end_class_flow(&session_id, session).await {
                    // Surface the end-class failure to the operator via the
                    // existing `.system-state--error` toast.
                    let mut snap = sidebar_for_async.write();
                    snap.command_error = Some(format!("end_class: {e}"));
                }
            });
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let session_id = session_id_for_end.clone();
            let session = session;
            let mut sidebar = sidebar_state_for_end;
            spawn(async move {
                if let Err(message) = end_class_flow_native(&session_id, session).await {
                    sidebar.write().command_error = Some(format!("end_class: {message}"));
                }
            });
        }
        state.set(PublishState::Idle);
    };

    // Network-quality poll (teacher). Every 3s, clone the publisher's PC out of
    // the session (dropping the read guard immediately) and probe getStats off
    // the clone — so we never hold a Signal guard across the async await. A
    // use_drop guard stops the loop on unmount, and we re-check it before every
    // Signal read/write so nothing touches a dropped signal.
    #[cfg(target_arch = "wasm32")]
    {
        let session_poll = session;
        let mut quality_w = quality;
        let started: std::rc::Rc<std::cell::Cell<bool>> =
            use_hook(|| std::rc::Rc::new(std::cell::Cell::new(false)));
        let stop: std::rc::Rc<std::cell::Cell<bool>> =
            use_hook(|| std::rc::Rc::new(std::cell::Cell::new(false)));
        {
            let stop_d = stop.clone();
            use_drop(move || stop_d.set(true));
        }
        use_effect(move || {
            if started.get() {
                return;
            }
            started.set(true);
            let stop = stop.clone();
            wasm_bindgen_futures::spawn_local(async move {
                loop {
                    gloo_timers::future::TimeoutFuture::new(3000).await;
                    if stop.get() {
                        break;
                    }
                    // Clone the PC handle (guard dropped at the end of this let).
                    let pc = session_poll.and_then(|s| s.read().publisher_pc());
                    if let Some(pc) = pc {
                        let q = crate::live_room_stats::probe(&pc).await;
                        if stop.get() {
                            break;
                        }
                        quality_w.set(q);
                    }
                }
            });
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut quality_w = quality;
        let stop: std::rc::Rc<std::cell::Cell<bool>> =
            use_hook(|| std::rc::Rc::new(std::cell::Cell::new(false)));
        {
            let stop_on_drop = stop.clone();
            use_drop(move || stop_on_drop.set(true));
        }
        use_future(move || {
            let stop = stop.clone();
            async move {
                loop {
                    crate::live_room_native::delay(3_000).await;
                    if stop.get() {
                        break;
                    }
                    quality_w.set(crate::live_room_native::publisher_quality("main").await);
                }
            }
        });
    }

    // Populate the in-call camera list once Live (enumerateDevices yields
    // labels only after getUserMedia has granted permission, which
    // go_live_flow does). Re-arms when leaving Live.
    #[cfg(target_arch = "wasm32")]
    {
        let state_eff = state;
        let mut cameras_w = cameras;
        let mut selected_w = selected_camera;
        let loaded: std::rc::Rc<std::cell::Cell<bool>> =
            use_hook(|| std::rc::Rc::new(std::cell::Cell::new(false)));
        use_effect(move || {
            let is_live = matches!(*state_eff.read(), PublishState::Live { .. });
            if !is_live {
                loaded.set(false);
                return;
            }
            if loaded.get() {
                return;
            }
            loaded.set(true);
            let loaded = loaded.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let devices = crate::live_room_devices::enumerate().await;
                let (cams, _m, _s) = crate::live_room_devices::partition_by_kind(&devices);
                if cams.is_empty() {
                    loaded.set(false); // permission not ready yet — retry next render
                    return;
                }
                if selected_w.read().is_empty() {
                    if let Some(first) = cams.first() {
                        selected_w.set(first.device_id.clone());
                    }
                }
                cameras_w.set(cams);
            });
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        let live_state = state;
        let mut cameras = cameras;
        let mut selected = selected_camera;
        let loaded: std::rc::Rc<std::cell::Cell<bool>> =
            use_hook(|| std::rc::Rc::new(std::cell::Cell::new(false)));
        use_effect(move || {
            if !matches!(*live_state.read(), PublishState::Live { .. }) {
                loaded.set(false);
                return;
            }
            if loaded.replace(true) {
                return;
            }
            let loaded_on_error = loaded.clone();
            spawn(async move {
                let devices = crate::live_room_devices::enumerate().await;
                let (available, _, _) = crate::live_room_devices::partition_by_kind(&devices);
                if available.is_empty() {
                    loaded_on_error.set(false);
                    return;
                }
                if selected.read().is_empty() {
                    if let Some(first) = available.first() {
                        selected.set(first.device_id.clone());
                    }
                }
                cameras.set(available);
            });
        });
    }

    // Tabbed-stage UI state. Default surface = Camera. PiP starts visible.
    let mut stage_tab = use_signal(|| StageTab::Camera);
    let pip_hidden = use_signal(|| false);

    // Right-rail dock state.
    let dock_tab = use_signal(|| DockTab::Chat);
    let mut dock_collapsed = use_signal(|| false);

    // Unread chat marker (nice-to-have). Compared against message count.
    let chat_seen = use_signal(|| 0usize);

    // Edge-triggered auto-switch: track the previous sharing state so a manual
    // Camera-tab click during a share is NOT reverted. `.peek()` reads stage_tab
    // WITHOUT subscribing, so this effect only re-runs when `state` changes.
    let was_sharing = use_signal(|| false);
    {
        let state_eff = state;
        let mut stage_tab = stage_tab;
        let mut was_sharing = was_sharing;
        use_effect(move || {
            let now = matches!(
                *state_eff.read(),
                PublishState::Live {
                    screen_active: true,
                    ..
                }
            );
            let prev = *was_sharing.peek();
            if now && !prev {
                stage_tab.set(StageTab::Screen);
            } else if !now && prev && *stage_tab.peek() == StageTab::Screen {
                stage_tab.set(StageTab::Camera);
            }
            was_sharing.set(now);
        });
    }

    // Auto-expand the dock when a command error arrives so the toast is seen.
    {
        let sidebar_state = sidebar_state;
        let mut dock_collapsed = dock_collapsed;
        use_effect(move || {
            if sidebar_state.read().command_error.is_some() && *dock_collapsed.peek() {
                dock_collapsed.set(false);
            }
        });
    }

    let current = state.read().clone();
    let (publish_active, screen_publish_active, video_error) = match &current {
        PublishState::Live {
            main_active,
            screen_active,
        } => (*main_active, *screen_active, None),
        PublishState::Error(message) => (false, false, Some(message.clone())),
        PublishState::Idle | PublishState::GoingLive => (false, false, None),
    };
    // "Going live, not up yet."
    //
    // `publish_active` cannot answer this on its own: when the session is
    // already live this component OPTIMISTICALLY initialises
    // `Live { main_active: true }` (see the top of LiveRoomBroadcast) before
    // anything has actually been published, so it claims to be publishing during
    // the very window we need to identify. The session only holds a publisher
    // once `publish()` has returned Ok, so that is the honest signal -- and
    // reading it here subscribes the strip, so it flips to OK the moment the
    // publisher lands.
    #[cfg(target_arch = "wasm32")]
    let publisher_ready = session.and_then(|s| s.read().publisher_pc()).is_some();
    #[cfg(not(target_arch = "wasm32"))]
    let publisher_ready = publish_active;

    // Idle is NOT starting: nothing is being attempted, so there is nothing to
    // be not-ready about.
    let publish_starting = matches!(&current, PublishState::GoingLive)
        || (matches!(&current, PublishState::Live { .. }) && !publisher_ready);

    #[cfg(target_arch = "wasm32")]
    let devices_captured = self_stream.read().is_some();
    #[cfg(not(target_arch = "wasm32"))]
    let devices_captured = publish_active;

    let server_snapshot = server_health.read().clone();

    // Was the snapshot we are about to render taken before this publisher
    // existed? The poll runs every 15s from mount, so the snapshot sitting in
    // the signal when publishing completes was typically fetched before the
    // teacher even clicked Go live -- and the server reports "no path" as an
    // Error once the session is live, which is exactly the false red we are
    // eliminating.
    //
    // `None` for either sequence number means stale, not fresh:
    //   - no stamp yet: the publisher is up but the effect that records it has
    //     not run for this render, so nothing has been correlated with it. The
    //     honest answer for that frame is "unobserved", and guessing "fresh"
    //     would flash the Error for exactly one frame -- the original bug, just
    //     briefer and harder to catch.
    //   - no snapshot sequence: no successful poll has ever completed, so there
    //     is nothing to trust. (Guarded by `is_some()` so a room that has never
    //     fetched health keeps its existing Unknown reading.)
    let publisher_stamp = *publisher_seen_at.read();
    let snapshot_seq = *health_snapshot_seq.read();
    let server_stream_stale = publisher_ready
        && server_snapshot.is_some()
        && match (snapshot_seq, publisher_stamp) {
            (Some(seq), Some(stamp)) => seq < stamp,
            _ => true,
        };

    let mut health_model = crate::live_room_health::merge_live_room_health(
        server_snapshot.as_ref(),
        crate::live_room_health::BrowserRoomHealth {
            devices_captured,
            publish_starting,
            server_stream_stale,
            publish_active,
            screen_publish_active,
            socket_status: crate::live_room_health::SocketHealthStatus::from(*socket_status.read()),
            quality: *quality.read(),
            video_error,
        },
    );
    let health_error_snapshot = health_fetch_error.read().clone();
    if server_snapshot.is_none() {
        if let Some(error) = health_error_snapshot.as_deref() {
            health_model.media_server.detail =
                format!("The room health probe could not be reached: {error}");
        }
    }
    let show_health = publish_active
        || matches!(&current, PublishState::GoingLive | PublishState::Error(_))
        || props.status == SessionStatus::Live;

    #[cfg(target_arch = "wasm32")]
    let screen_share_supported = true;
    #[cfg(not(target_arch = "wasm32"))]
    let screen_share_supported = native_capabilities
        .read()
        .as_ref()
        .is_some_and(|capabilities| capabilities.screen_share);

    let health_api_for_actions = use_context::<Signal<crate::api::ApiContext>>();

    let refresh_session_id = props.session_id.clone();
    let on_health_refresh = move |_| {
        let session_id = refresh_session_id.clone();
        let health = server_health;
        let error = health_fetch_error;
        let poll_seq = health_poll_seq;
        let snapshot_seq = health_snapshot_seq;
        spawn(async move {
            refresh_server_health(health_api_for_actions, session_id, health, error, poll_seq, snapshot_seq).await;
        });
    };

    let on_recheck_devices = move |_| {
        #[cfg(target_arch = "wasm32")]
        {
            let session = session;
            let self_stream = self_stream;
            let camera_stream = camera_stream;
            let state = state;
            spawn(async move {
                reset_main_publish(session, self_stream, camera_stream, state).await;
            });
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let mut state = state;
            spawn(async move {
                let _ =
                    crate::live_room_native::stop_publisher("main", Some("broadcast-self-video"))
                        .await;
                state.set(PublishState::Idle);
            });
        }
    };

    let on_retry_publish = move |_| {
        #[cfg(target_arch = "wasm32")]
        {
            let session = session;
            let self_stream = self_stream;
            let camera_stream = camera_stream;
            let state = state;
            spawn(async move {
                reset_main_publish(session, self_stream, camera_stream, state).await;
            });
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let mut state = state;
            spawn(async move {
                let _ =
                    crate::live_room_native::stop_publisher("main", Some("broadcast-self-video"))
                        .await;
                state.set(PublishState::Idle);
            });
        }
    };

    let restart_session_id = props.session_id.clone();
    let on_restart_screen_share = move |_| {
        #[cfg(target_arch = "wasm32")]
        {
            let session_id = restart_session_id.clone();
            let session = session;
            let screen_stream = screen_stream;
            let mut state = state;
            let mut error = health_fetch_error;
            spawn(async move {
                stop_screen_share_flow(session, screen_stream).await;
                state.set(PublishState::Live {
                    main_active: true,
                    screen_active: false,
                });
                match start_screen_share_flow(&session_id, session, screen_stream).await {
                    Ok(()) => {
                        state.set(PublishState::Live {
                            main_active: true,
                            screen_active: true,
                        });
                        error.set(None);
                    }
                    Err(message) => {
                        error.set(Some(format!("Screen sharing could not restart: {message}")))
                    }
                }
            });
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let session_id = restart_session_id.clone();
            let session = session;
            let mut state = state;
            let mut error = health_fetch_error;
            spawn(async move {
                let _ = crate::live_room_native::stop_publisher(
                    "screen",
                    Some("broadcast-screen-video"),
                )
                .await;
                match start_screen_share_flow_native(&session_id, session).await {
                    Ok(()) => {
                        state.set(PublishState::Live {
                            main_active: true,
                            screen_active: true,
                        });
                        error.set(None);
                    }
                    Err(message) => {
                        error.set(Some(format!("Screen sharing could not restart: {message}")))
                    }
                }
            });
        }
    };

    let retry_recording_session_id = props.session_id.clone();
    let on_retry_recording = move |_| {
        let session_id = retry_recording_session_id.clone();
        let mut error = health_fetch_error;
        let health = server_health;
        let poll_seq = health_poll_seq;
        let snapshot_seq = health_snapshot_seq;
        spawn(async move {
            let api = health_api_for_actions.read().clone();
            let path = format!("/v1/sessions/{session_id}/recording/retry");
            match crate::api::fetch_json::<serde_json::Value>(&api, "POST", &path, None::<&()>)
                .await
            {
                Ok(_) => {
                    refresh_server_health(health_api_for_actions, session_id, health, error, poll_seq, snapshot_seq).await;
                }
                Err(err) => error.set(Some(format!("Recording recovery could not start: {err}"))),
            }
        });
    };

    rsx! {
        div { class: if dock_collapsed() { "live-room-broadcast live-room-broadcast--dock-collapsed" } else { "live-room-broadcast" },
            PageHeader {
                kicker: "Live session".to_string(),
                title: "Broadcast".to_string(),
                subtitle: "Go live when you're ready. Students join automatically.".to_string(),
                as_tag: HeadingLevel::H2,
            }

            if show_health {
                section { class: "live-room-health-console", "aria-label": "Broadcast health console",
                    div { class: "live-room-health-console__summary",
                        crate::live_room_health::LiveRoomHealthStrip {
                            model: health_model.clone(),
                        }
                        button {
                            r#type: "button",
                            class: "live-room-health-console__toggle",
                            "aria-expanded": diagnostics_open().to_string(),
                            onclick: move |_| {
                                let next = !diagnostics_open();
                                diagnostics_open.set(next);
                            },
                            if diagnostics_open() { "Hide diagnostics" } else { "Room diagnostics" }
                        }
                    }
                    if let Some(message) = health_error_snapshot.clone() {
                        p { class: "live-room-health-console__notice", role: "alert",
                            "Health refresh delayed. {message}"
                        }
                    }
                    if diagnostics_open() {
                        crate::live_room_health::LiveRoomDiagnosticsSheet {
                            model: health_model.clone(),
                            on_refresh: on_health_refresh,
                            on_recheck_devices: on_recheck_devices,
                            on_retry_publish: on_retry_publish,
                            on_restart_screen_share: on_restart_screen_share,
                            on_retry_recording: on_retry_recording,
                        }
                    }
                }
            }

            // Publish controls
            div { class: "live-room-broadcast-controls live-room-stage",
                match current {
                    PublishState::Idle => rsx! {
                        // Prejoin device test + picker. "Go live" fires on_join
                        // with the chosen camera/mic, which kick off go_live_flow.
                        crate::live_room_prejoin::LiveRoomPrejoin {
                            join_label: "Go live".to_string(),
                            heading: "Check your camera and mic before going live".to_string(),
                            on_join: on_go_live,
                        }
                    },
                    PublishState::GoingLive => rsx! {
                        Loading {
                            message: "Going live\u{2026} requesting camera + mic permissions.".to_string(),
                        }
                    },
                    PublishState::Live { main_active, screen_active } => rsx! {
                        // Stage tab bar: Camera always; Screen only while sharing;
                        // Whiteboard always. Edge-triggered auto-switch (see above)
                        // handles Camera<->Screen on share start/stop.
                        nav {
                            class: "live-stage-tabs",
                            role: "tablist",
                            "aria-label": "Stage view",
                            button {
                                r#type: "button",
                                role: "tab",
                                id: "stage-tab-camera",
                                "aria-controls": "stage-panel-camera",
                                class: if *stage_tab.read() == StageTab::Camera { "live-stage-tab live-stage-tab--active" } else { "live-stage-tab" },
                                "aria-selected": (*stage_tab.read() == StageTab::Camera).to_string(),
                                onclick: move |_| { stage_tab.set(StageTab::Camera); },
                                "Camera"
                            }
                            button {
                                r#type: "button",
                                role: "tab",
                                id: "stage-tab-screen",
                                "aria-controls": "stage-panel-screen",
                                class: if !screen_active { "live-stage-tab live-stage-tab--disabled" } else if *stage_tab.read() == StageTab::Screen { "live-stage-tab live-stage-tab--active" } else { "live-stage-tab" },
                                "aria-selected": (*stage_tab.read() == StageTab::Screen).to_string(),
                                disabled: !screen_active,
                                onclick: move |_| { stage_tab.set(StageTab::Screen); },
                                "Screen"
                            }
                            button {
                                r#type: "button",
                                role: "tab",
                                id: "stage-tab-whiteboard",
                                "aria-controls": "stage-panel-whiteboard",
                                class: if *stage_tab.read() == StageTab::Whiteboard { "live-stage-tab live-stage-tab--active" } else { "live-stage-tab" },
                                "aria-selected": (*stage_tab.read() == StageTab::Whiteboard).to_string(),
                                onclick: move |_| { stage_tab.set(StageTab::Whiteboard); },
                                "Whiteboard"
                            }
                        }
                        div { class: "broadcast-status",
                            Badge {
                                label: "\u{25cf} LIVE".to_string(),
                                tone: BadgeTone::Live,
                            }
                            span {
                                class: "live-rec-badge",
                                title: "This session is being recorded",
                                "\u{25cf} REC"
                            }
                            crate::live_room_stats::NetworkQualityBadge { quality: *quality.read() }
                            if main_active {
                                Badge {
                                    label: "camera+mic".to_string(),
                                    tone: BadgeTone::Success,
                                }
                            }
                            if screen_active {
                                Badge {
                                    label: "screen".to_string(),
                                    tone: BadgeTone::Info,
                                }
                            }
                        }
                        // ALWAYS-displayed positioning context. Holds the camera /
                        // screen / whiteboard surfaces AND the floating PiP as
                        // SIBLINGS, so the PiP is never trapped inside a
                        // display:none surface. position:relative anchors the PiP
                        // corner overlay. Closed in Edit T4 after the whiteboard.
                        div { class: "live-stage-body",
                            // Camera surface. display:none on non-Camera tabs.
                            // Holds NO stream-attached element (the person video
                            // lives in the sibling live-pip), so hiding it is safe.
                            div {
                                class: if *stage_tab.read() == StageTab::Camera { "live-stage-surface live-stage-surface--camera live-stage-surface--active" } else { "live-stage-surface live-stage-surface--camera" },
                                role: "tabpanel",
                                id: "stage-panel-camera",
                                "aria-labelledby": "stage-tab-camera",
                            }
                            // The SINGLE self-video element. Inline-large on the
                            // Camera tab; floating corner PiP on Screen/Whiteboard
                            // (live-pip--active). Element is one DOM node always —
                            // stream + audio survive every tab switch (Invariant B).
                            div {
                                class: {
                                    let off_camera = *stage_tab.read() != StageTab::Camera;
                                    match (off_camera, pip_hidden()) {
                                        (true, true) => "live-pip live-pip--active live-pip--hidden live-pip-corner--br broadcast-self-wrap",
                                        (true, false) => "live-pip live-pip--active live-pip-corner--br broadcast-self-wrap",
                                        (false, _) => "live-pip broadcast-self-wrap",
                                    }
                                },
                                video {
                                    id: "broadcast-self-video",
                                    class: "broadcast-self-video",
                                    autoplay: true,
                                    muted: true,
                                    playsinline: true,
                                }
                                span { class: "broadcast-self-tag", "You" }
                                if *stage_tab.read() != StageTab::Camera {
                                    button {
                                        r#type: "button",
                                        class: "live-pip-toggle",
                                        "aria-label": if pip_hidden() { "Show self-view".to_string() } else { "Hide self-view".to_string() },
                                        title: if pip_hidden() { "Show self-view".to_string() } else { "Hide self-view".to_string() },
                                        onclick: move |_| { let mut h = pip_hidden; let n = !h(); h.set(n); },
                                        if pip_hidden() { "Show" } else { "Hide" }
                                    }
                                }
                            }
                            // Screen surface — lives inside the stage body so the
                            // PiP overlays it. Mounted only while screen_active
                            // (unchanged); CSS-hidden when the Screen tab isn't the
                            // active one. The Stop/Share control sits in the toolbar
                            // below, outside the positioned body.
                            if screen_active {
                                div {
                                    class: if *stage_tab.read() == StageTab::Screen { "live-stage-surface live-stage-surface--screen live-stage-surface--active" } else { "live-stage-surface live-stage-surface--screen" },
                                    role: "tabpanel",
                                    id: "stage-panel-screen",
                                    "aria-labelledby": "stage-tab-screen",
                                    div { class: "broadcast-screen-wrap",
                                        video {
                                            id: "broadcast-screen-video",
                                            class: "broadcast-screen-video",
                                            autoplay: true,
                                            muted: true,
                                            playsinline: true,
                                        }
                                        span { class: "broadcast-self-tag", "Screen" }
                                    }
                                }
                            }
                            // Whiteboard surface — conditionally MOUNTED only when
                            // its tab is active, so `live-room-whiteboard-canvas`
                            // mounts visible and `onmounted` measures correct
                            // dimensions. Persistent stroke state lives in
                            // sidebar_state, so unmount/remount loses nothing. The
                            // `key` ties mount identity to dock_collapsed so a
                            // collapse (stage-width change with no window resize)
                            // forces a remount -> fresh measure.
                            if *stage_tab.read() == StageTab::Whiteboard {
                                div {
                                    class: "live-stage-surface live-stage-surface--whiteboard live-stage-surface--active",
                                    role: "tabpanel",
                                    id: "stage-panel-whiteboard",
                                    "aria-labelledby": "stage-tab-whiteboard",
                                    key: "wb-{dock_collapsed()}",
                                    crate::live_room_whiteboard::LiveRoomWhiteboard {
                                        state: sidebar_state.read().whiteboard.clone(),
                                        is_teacher: true,
                                        local_user_id: local_user_id.clone(),
                                        session_id: props.session_id.clone(),
                                        cursors: sidebar_state.read().cursors.clone(),
                                        draw_open: sidebar_state.read().draw_open,
                                        on_emit_stroke: EventHandler::new(on_whiteboard_stroke),
                                        on_clear: EventHandler::new(on_whiteboard_clear),
                                        on_undo: EventHandler::new(on_whiteboard_undo),
                                        on_erase: EventHandler::new(on_whiteboard_erase),
                                        on_cursor: EventHandler::new(on_whiteboard_cursor),
                                        on_set_draw_permission: EventHandler::new(on_set_draw_permission),
                                    }
                                }
                            }
                            // Floating emoji reactions overlay the stage viewport.
                            crate::live_room_reactions::ReactionFloats {
                                reactions: sidebar_state.read().reactions.clone(),
                            }
                        }
                        // Stage controls live OUTSIDE the positioned stage body so
                        // the corner PiP anchors to the stage viewport, not the
                        // full control column.
                        div { class: "broadcast-controls",
                            // Mic / camera toggles: flip `enabled` on the captured
                            // tracks (no renegotiation, instant for viewers).
                            div { class: "broadcast-av-toggles",
                                Button {
                                    label: if mic_enabled() { "Mute mic".to_string() } else { "Unmute mic".to_string() },
                                    variant: if mic_enabled() { ButtonVariant::Secondary } else { ButtonVariant::Danger },
                                    on_click: move |_| {
                                        #[cfg(target_arch = "wasm32")]
                                        {
                                            let mut mic_enabled = mic_enabled;
                                            let next = !mic_enabled();
                                            set_tracks_enabled(&self_stream, TrackKind::Audio, next);
                                            mic_enabled.set(next);
                                        }
                                        #[cfg(not(target_arch = "wasm32"))]
                                        {
                                            let mut mic_enabled = mic_enabled;
                                            let next = !mic_enabled();
                                            mic_enabled.set(next);
                                            spawn(async move {
                                                let _ = crate::live_room_native::set_track_enabled(
                                                    "main", "audio", next,
                                                )
                                                .await;
                                            });
                                        }
                                    },
                                }
                                Button {
                                    label: if cam_enabled() { "Camera off".to_string() } else { "Camera on".to_string() },
                                    variant: if cam_enabled() { ButtonVariant::Secondary } else { ButtonVariant::Danger },
                                    on_click: move |_| {
                                        #[cfg(target_arch = "wasm32")]
                                        {
                                            let mut cam_enabled = cam_enabled;
                                            let next = !cam_enabled();
                                            set_tracks_enabled(&self_stream, TrackKind::Video, next);
                                            cam_enabled.set(next);
                                        }
                                        #[cfg(not(target_arch = "wasm32"))]
                                        {
                                            let mut cam_enabled = cam_enabled;
                                            let next = !cam_enabled();
                                            cam_enabled.set(next);
                                            spawn(async move {
                                                let _ = crate::live_room_native::set_track_enabled(
                                                    "main", "video", next,
                                                )
                                                .await;
                                            });
                                        }
                                    },
                                }
                            }
                            // In-call camera switch. FX-aware: when blur is active
                            // it feeds the new camera into the bridge (the published
                            // canvas track is unchanged, so blur stays intact);
                            // otherwise it replaceTrack's the publisher. Shown only
                            // when more than one camera is available.
                            {
                                let cam_opts = crate::live_room_prejoin::device_options(&cameras.read());
                                if cam_opts.len() > 1 {
                                    rsx! {
                                        label { class: "broadcast-camera-switch",
                                            span { class: "broadcast-camera-switch-label", "Camera" }
                                            Select {
                                                value: selected_camera.read().clone(),
                                                options: cam_opts,
                                                on_change: move |new_id: String| {
                                                    let mut selected_camera = selected_camera;
                                                    selected_camera.set(new_id.clone());
                                                    #[cfg(target_arch = "wasm32")]
                                                    {
                                                        let session = session;
                                                        let self_stream = self_stream;
                                                        let camera_stream = camera_stream;
                                                        wasm_bindgen_futures::spawn_local(async move {
                                                            switch_camera_flow(session, self_stream, camera_stream, new_id).await;
                                                        });
                                                    }
                                                    #[cfg(not(target_arch = "wasm32"))]
                                                    spawn(async move {
                                                        let _ = crate::live_room_native::switch_camera(
                                                            &new_id,
                                                            None,
                                                            "broadcast-self-video",
                                                        )
                                                        .await;
                                                    });
                                                },
                                            }
                                        }
                                    }
                                } else {
                                    rsx! {}
                                }
                            }
                            // Mobile flip-camera control. On phones the deviceId
                            // picker above is hidden (opaque mobile ids), so this
                            // one-tap front/back toggle is the camera switch. Shown
                            // only when the UA looks mobile. FX-aware via
                            // `flip_camera_flow`.
                            if crate::live_room_devices::is_mobile() {
                                button {
                                    r#type: "button",
                                    class: "broadcast-camera-flip",
                                    title: match facing() {
                                        crate::live_room_devices::Facing::Front => "Switch to the back camera",
                                        crate::live_room_devices::Facing::Back => "Switch to the front camera",
                                    },
                                    onclick: move |_| {
                                        #[cfg(target_arch = "wasm32")]
                                        {
                                            let mut facing = facing;
                                            let next = facing().flipped();
                                            facing.set(next);
                                            let session = session;
                                            let self_stream = self_stream;
                                            let camera_stream = camera_stream;
                                            wasm_bindgen_futures::spawn_local(async move {
                                                flip_camera_flow(session, self_stream, camera_stream, next).await;
                                            });
                                        }
                                        #[cfg(not(target_arch = "wasm32"))]
                                        {
                                            let mut facing = facing;
                                            let next = facing().flipped();
                                            facing.set(next);
                                            spawn(async move {
                                                let _ = crate::live_room_native::switch_camera(
                                                    "",
                                                    Some(next.as_wire()),
                                                    "broadcast-self-video",
                                                )
                                                .await;
                                            });
                                        }
                                    },
                                    "Flip camera"
                                }
                            }
                            // Background effect: Normal / Blur / Studio virtual
                            // background. Rendered only when the MediaPipe bridge is
                            // available; otherwise the raw camera publishes and this
                            // control is hidden. Switching is instant (no
                            // renegotiation) because the published track is the
                            // bridge's canvas stream and we only change what it
                            // composites.
                            if crate::live_room_video_fx::is_supported() {
                                div {
                                    class: "broadcast-fx",
                                    role: "group",
                                    "aria-label": "Background effect",
                                    span { class: "broadcast-fx-label", "Background" }
                                    button {
                                        r#type: "button",
                                        class: if blur_mode() == crate::live_room_video_fx::FxMode::Off {
                                            "broadcast-fx-btn broadcast-fx-btn--active"
                                        } else {
                                            "broadcast-fx-btn"
                                        },
                                        onclick: move |_| {
                                            let mut blur_mode = blur_mode;
                                            blur_mode.set(crate::live_room_video_fx::FxMode::Off);
                                            #[cfg(target_arch = "wasm32")]
                                            crate::live_room_video_fx::set_mode("off");
                                        },
                                        "Normal"
                                    }
                                    button {
                                        r#type: "button",
                                        class: if blur_mode() == crate::live_room_video_fx::FxMode::Blur {
                                            "broadcast-fx-btn broadcast-fx-btn--active"
                                        } else {
                                            "broadcast-fx-btn"
                                        },
                                        onclick: move |_| {
                                            let mut blur_mode = blur_mode;
                                            blur_mode.set(crate::live_room_video_fx::FxMode::Blur);
                                            #[cfg(target_arch = "wasm32")]
                                            crate::live_room_video_fx::set_mode("blur");
                                        },
                                        "Blur"
                                    }
                                    button {
                                        r#type: "button",
                                        class: if blur_mode() == crate::live_room_video_fx::FxMode::Image {
                                            "broadcast-fx-btn broadcast-fx-btn--active"
                                        } else {
                                            "broadcast-fx-btn"
                                        },
                                        onclick: move |_| {
                                            let mut blur_mode = blur_mode;
                                            blur_mode.set(crate::live_room_video_fx::FxMode::Image);
                                            #[cfg(target_arch = "wasm32")]
                                            {
                                                crate::live_room_video_fx::set_background(&studio_bg_data_url());
                                                crate::live_room_video_fx::set_mode("image");
                                            }
                                        },
                                        "Studio"
                                    }
                                }
                            }
                            // Share / Stop screen control (the screen video itself
                            // lives in the stage body above).
                            if screen_active {
                                Button {
                                    label: "Stop sharing".to_string(),
                                    variant: ButtonVariant::Secondary,
                                    on_click: move |_| {
                                        #[cfg(target_arch = "wasm32")]
                                        {
                                            let session = session;
                                            let screen_stream = screen_stream;
                                            let mut state_for_async = state;
                                            wasm_bindgen_futures::spawn_local(async move {
                                                stop_screen_share_flow(session, screen_stream).await;
                                                state_for_async.set(PublishState::Live {
                                                    main_active: true,
                                                    screen_active: false,
                                                });
                                            });
                                        }
                                        #[cfg(not(target_arch = "wasm32"))]
                                        {
                                            let mut state = state;
                                            spawn(async move {
                                                let _ = crate::live_room_native::stop_publisher(
                                                    "screen",
                                                    Some("broadcast-screen-video"),
                                                )
                                                .await;
                                                state.set(PublishState::Live {
                                                    main_active: true,
                                                    screen_active: false,
                                                });
                                            });
                                        }
                                    },
                                }
                            } else {
                                Button {
                                    label: if screen_share_supported { "Share screen".to_string() } else { "Screen share unavailable".to_string() },
                                    variant: ButtonVariant::Secondary,
                                    disabled: !screen_share_supported,
                                    on_click: {
                                        let session_id = session_id.clone();
                                        move |_| {
                                            #[cfg(target_arch = "wasm32")]
                                            {
                                                let session = session;
                                                let screen_stream = screen_stream;
                                                let mut state_for_async = state;
                                                let session_id = session_id.clone();
                                                wasm_bindgen_futures::spawn_local(async move {
                                                    match start_screen_share_flow(
                                                        &session_id,
                                                        session,
                                                        screen_stream,
                                                    )
                                                    .await
                                                    {
                                                        Ok(()) => state_for_async.set(PublishState::Live {
                                                            main_active: true,
                                                            screen_active: true,
                                                        }),
                                                        Err(e) => state_for_async.set(PublishState::Error(e)),
                                                    }
                                                });
                                            }
                                            #[cfg(not(target_arch = "wasm32"))]
                                            {
                                                let session_id = session_id.clone();
                                                let session = session;
                                                let mut state_for_async = state;
                                                // Mount the screen video surface before the
                                                // WebView attaches the captured stream.
                                                state_for_async.set(PublishState::Live {
                                                    main_active: true,
                                                    screen_active: true,
                                                });
                                                spawn(async move {
                                                    if let Err(message) = start_screen_share_flow_native(
                                                        &session_id,
                                                        session,
                                                    )
                                                    .await
                                                    {
                                                        state_for_async.set(PublishState::Live {
                                                            main_active: true,
                                                            screen_active: false,
                                                        });
                                                        tracing::warn!(error = %message, "native screen share failed");
                                                    }
                                                });
                                            }
                                        }
                                    },
                                }
                                if !screen_share_supported {
                                    p { class: "broadcast-capability-note",
                                        if cfg!(target_os = "android") {
                                            "Android screen sharing is disabled until the MediaProjection host adapter is installed."
                                        } else if cfg!(target_os = "ios") {
                                            "iOS screen sharing is disabled until the ReplayKit host adapter is installed."
                                        } else {
                                            "This desktop WebView or operating system does not expose screen capture."
                                        }
                                    }
                                }
                            }
                            p { class: "broadcast-copy", "Streaming is active. Keep this room open until class ends." }
                            Button {
                                label: "End Class".to_string(),
                                variant: ButtonVariant::Danger,
                                on_click: on_end,
                            }
                            crate::live_room_reactions::ReactionBar {
                                on_react: EventHandler::new(on_react),
                            }
                            // Teacher poll composer: editor when idle, live tally +
                            // End control while a poll runs.
                            crate::live_room_polls::PollComposer {
                                active: sidebar_state.read().polls.active.clone(),
                                on_start: EventHandler::new(on_start_poll),
                                on_end: EventHandler::new(on_end_poll),
                            }
                        }
                    },
                    PublishState::Error(msg) => rsx! {
                        FormError { message: Some(msg) }
                        Button {
                            label: "Try again".to_string(),
                            variant: ButtonVariant::Secondary,
                            on_click: move |_| state.set(PublishState::Idle),
                        }
                    },
                }
            }

            // Teacher dock — tabbed, collapsible. All panels stay MOUNTED so
            // chat/presence/hand-raise/breakout keep accumulating socket events;
            // only `live-dock-panel--active` is shown (display:none on the rest).
            div {
                class: if dock_collapsed() { "live-room-sidebars live-dock live-dock--tabbed live-dock--collapsed" } else { "live-room-sidebars live-dock live-dock--tabbed" },
                div {
                    class: "live-dock-bar",
                    role: "tablist",
                    "aria-label": "Session panels",
                    button {
                        r#type: "button",
                        class: "live-dock-collapse",
                        title: if dock_collapsed() { "Expand panel".to_string() } else { "Collapse panel".to_string() },
                        "aria-label": if dock_collapsed() { "Expand panel".to_string() } else { "Collapse panel".to_string() },
                        "aria-expanded": (!dock_collapsed()).to_string(),
                        onclick: move |_| { let n = !dock_collapsed(); dock_collapsed.set(n); },
                        if dock_collapsed() { "\u{203a}" } else { "\u{2039}" }
                    }
                    button {
                        r#type: "button",
                        role: "tab",
                        id: "dock-tab-chat",
                        "aria-controls": "dock-panel-chat",
                        class: if *dock_tab.read() == DockTab::Chat { "live-dock-tab live-dock-tab--active" } else { "live-dock-tab" },
                        "aria-selected": (*dock_tab.read() == DockTab::Chat).to_string(),
                        onclick: move |_| {
                            let mut t = dock_tab; t.set(DockTab::Chat);
                            let mut seen = chat_seen; seen.set(sidebar_state.read().messages.len());
                            dock_collapsed.set(false);
                        },
                        "Chat"
                        {
                            let unread = if *dock_tab.read() == DockTab::Chat { 0 } else { sidebar_state.read().messages.len().saturating_sub(chat_seen()) };
                            if unread > 0 { rsx! { span { class: "live-dock-tab__unread", "{unread}" } } } else { rsx! {} }
                        }
                    }
                    button {
                        r#type: "button",
                        role: "tab",
                        id: "dock-tab-people",
                        "aria-controls": "dock-panel-people",
                        class: if *dock_tab.read() == DockTab::People { "live-dock-tab live-dock-tab--active" } else { "live-dock-tab" },
                        "aria-selected": (*dock_tab.read() == DockTab::People).to_string(),
                        onclick: move |_| { let mut t = dock_tab; t.set(DockTab::People); dock_collapsed.set(false); },
                        "People"
                    }
                    button {
                        r#type: "button",
                        role: "tab",
                        id: "dock-tab-hands",
                        "aria-controls": "dock-panel-hands",
                        class: if *dock_tab.read() == DockTab::Hands { "live-dock-tab live-dock-tab--active" } else { "live-dock-tab" },
                        "aria-selected": (*dock_tab.read() == DockTab::Hands).to_string(),
                        onclick: move |_| { let mut t = dock_tab; t.set(DockTab::Hands); dock_collapsed.set(false); },
                        "Hands"
                    }
                    button {
                        r#type: "button",
                        role: "tab",
                        id: "dock-tab-breakout",
                        "aria-controls": "dock-panel-breakout",
                        class: if *dock_tab.read() == DockTab::Breakout { "live-dock-tab live-dock-tab--active" } else { "live-dock-tab" },
                        "aria-selected": (*dock_tab.read() == DockTab::Breakout).to_string(),
                        onclick: move |_| { let mut t = dock_tab; t.set(DockTab::Breakout); dock_collapsed.set(false); },
                        "Breakout"
                    }
                }
                // Command-failed toast stays visible regardless of active tab AND
                // when collapsed (CSS does NOT hide .system-state on collapse).
                {match sidebar_state.read().command_error.as_ref() {
                    Some(reason) => rsx! {
                        div { class: "system-state system-state--error",
                            "Command failed: {reason}"
                        }
                    },
                    None => rsx! {},
                }}
                div { class: "live-dock-panels",
                    div {
                        class: if *dock_tab.read() == DockTab::Chat { "live-dock-panel live-dock-panel--active" } else { "live-dock-panel" },
                        role: "tabpanel",
                        id: "dock-panel-chat",
                        "aria-labelledby": "dock-tab-chat",
                        LiveRoomChat {
                            messages: sidebar_state.read().messages.clone(),
                            is_teacher: true,
                            on_send: on_send,
                            on_delete: on_delete,
                        }
                    }
                    div {
                        class: if *dock_tab.read() == DockTab::People { "live-dock-panel live-dock-panel--active" } else { "live-dock-panel" },
                        role: "tabpanel",
                        id: "dock-panel-people",
                        "aria-labelledby": "dock-tab-people",
                        LiveRoomPresence {
                            count: sidebar_state.read().presence_count,
                            participants: sidebar_state.read().participants.clone(),
                            is_teacher: true,
                        }
                    }
                    div {
                        class: if *dock_tab.read() == DockTab::Hands { "live-dock-panel live-dock-panel--active" } else { "live-dock-panel" },
                        role: "tabpanel",
                        id: "dock-panel-hands",
                        "aria-labelledby": "dock-tab-hands",
                        LiveRoomHandRaise {
                            is_teacher: true,
                            my_hand_raised: false,
                            queue: sidebar_state.read().queue.clone(),
                            on_raise: on_raise,
                            on_accept: on_accept,
                            on_demote: on_demote,
                        }
                    }
                    div {
                        class: if *dock_tab.read() == DockTab::Breakout { "live-dock-panel live-dock-panel--active" } else { "live-dock-panel" },
                        role: "tabpanel",
                        id: "dock-panel-breakout",
                        "aria-labelledby": "dock-tab-breakout",
                        // Teacher breakout-room panel. Participants come from the
                        // presence list (students only — the teacher manages, isn't
                        // assigned). Auto-split / create / assign / open / close all
                        // forward onto the live-room socket.
                        crate::breakout_rooms::BreakoutPanel {
                            state: sidebar_state.read().breakout.clone(),
                            participants: breakout_participants(&sidebar_state.read().participants),
                            on_auto_split: EventHandler::new(on_breakout_auto_split),
                            on_create: EventHandler::new(on_breakout_create),
                            on_assign: EventHandler::new(on_breakout_assign),
                            on_open: EventHandler::new(on_breakout_open),
                            on_close: EventHandler::new(on_breakout_close),
                        }
                    }
                }
            }
        }
    }
}

/// Build the `(user_id, display_name)` participant list the breakout panel
/// assigns from. Drawn from the presence list, filtered to students (the
/// teacher manages breakouts but is never assigned to one). Pure + testable.
fn breakout_participants(presence: &Option<Vec<PresenceParticipant>>) -> Vec<(String, String)> {
    presence
        .as_ref()
        .map(|list| {
            list.iter()
                .filter(|p| p.role != "teacher")
                .map(|p| (p.user_id.clone(), p.display_name.clone()))
                .collect()
        })
        .unwrap_or_default()
}

async fn refresh_server_health(
    api: Signal<crate::api::ApiContext>,
    session_id: String,
    mut health: Signal<Option<crate::live_room_health::LiveSessionHealthDto>>,
    mut error: Signal<Option<String>>,
    mut poll_seq: Signal<u64>,
    mut snapshot_seq: Signal<Option<u64>>,
) {
    // Claim this poll's sequence number BEFORE awaiting. The number records
    // when the request went out, not when it came back, so a poll that overlaps
    // the moment the publisher lands is correctly filed as pre-publisher -- it
    // asked the media server about a path that did not exist yet.
    let started_at = {
        let current = *poll_seq.peek();
        poll_seq.set(current.wrapping_add(1));
        current
    };
    let ctx = api.read().clone();
    match crate::live_room_health::fetch_session_health(&ctx, &session_id).await {
        Ok(snapshot) => {
            health.set(Some(snapshot));
            error.set(None);
            // Only a SUCCESSFUL fetch republishes the sequence: a failed poll
            // leaves the previous snapshot in place, so it has observed nothing
            // new and must not be allowed to look fresher than it is.
            snapshot_seq.set(Some(started_at));
        }
        Err(err) => {
            // Keep the last known snapshot visible during a transient probe
            // failure, but surface that its freshness could not be renewed.
            error.set(Some(err.to_string()));
        }
    }
}

#[cfg(target_arch = "wasm32")]
async fn reset_main_publish(
    session: Option<Signal<crate::live_room_session::LiveRoomSession>>,
    mut self_stream: Signal<Option<web_sys::MediaStream>>,
    mut camera_stream: Signal<Option<web_sys::MediaStream>>,
    mut state: Signal<PublishState>,
) {
    self_stream.set(None);
    crate::live_room_video_fx::stop();
    if let Some(stream) = camera_stream.write().take() {
        stop_media_stream_tracks(&stream);
    }
    if let Some(mut session) = session {
        session.write().stop_publishing().await;
    }
    // Return to the prejoin device check. The next publish is an explicit
    // operator action, so a denied permission does not enter a retry loop.
    state.set(PublishState::Idle);
}

// ---------------------------------------------------------------------------
// Persistent socket — browser and native WebView renderers
// ---------------------------------------------------------------------------

fn use_persistent_socket_broadcast(
    session_id: String,
    sidebar_state: Signal<BroadcastSidebarState>,
    socket_status: Signal<crate::live_room_socket::ConnStatus>,
) -> std::rc::Rc<std::cell::RefCell<Option<crate::live_room_socket::conn::LiveRoomSocket>>> {
    use std::cell::RefCell;
    use std::rc::Rc;

    // Persist the Rc across renders. Without `use_hook`, the function body
    // re-allocates a fresh Rc on every render — the live socket ends up
    // inside the first-render Rc (captured by the `use_effect` closure),
    // while the on_send / on_delete closures created on each subsequent
    // render close over a brand-new empty Rc. The user's chat sends then
    // hit `socket.borrow().is_none()` and silently drop. `use_hook`
    // returns the SAME Rc every render so all closures share state.
    let socket: Rc<RefCell<Option<crate::live_room_socket::conn::LiveRoomSocket>>> =
        use_hook(|| Rc::new(RefCell::new(None)));

    // Read the current access token from the ApiContext signal so the WS URL
    // carries `?access_token=<jwt>`. The backend auth middleware
    // (`require_auth`) requires this query param for WebSocket upgrades
    // because browsers cannot attach an `Authorization` header to a WS
    // handshake. Without the token the upgrade returns 401, the socket
    // never opens, and `send_text` calls silently fail — which is why
    // typed chat messages were not registering.
    let api_ctx_signal = use_context::<Signal<crate::api::ApiContext>>();

    // Lifetime guard: set on unmount so a pending reconnect task stops.
    let closed: Rc<std::cell::Cell<bool>> = use_hook(|| Rc::new(std::cell::Cell::new(false)));
    {
        let closed_d = closed.clone();
        let socket_d = socket.clone();
        use_drop(move || {
            closed_d.set(true);
            if let Some(mut s) = socket_d.borrow_mut().take() {
                s.close();
            }
        });
    }

    let socket_for_effect = socket.clone();
    let closed_for_effect = closed.clone();
    use_effect(move || {
        if socket_for_effect.borrow().is_some() {
            return;
        }
        #[cfg(target_arch = "wasm32")]
        let origin = {
            let configured_api_origin = api_ctx_signal.read().base_url.clone();
            if configured_api_origin.is_empty() {
                web_sys::window()
                    .and_then(|window| window.location().origin().ok())
                    .unwrap_or_default()
            } else {
                configured_api_origin
            }
        };
        #[cfg(not(target_arch = "wasm32"))]
        let origin = api_ctx_signal.read().base_url.clone();

        let session_id_url = session_id.clone();
        let api_ctx_for_url = api_ctx_signal;
        let make_url: Rc<dyn Fn(Option<String>) -> String> = Rc::new(move |override_token| {
            let token = override_token.unwrap_or_else(|| api_ctx_for_url.read().id_token.clone());
            crate::live_room_session::build_ws_url(
                &origin,
                &session_id_url,
                &token,
                crate::api::selected_workspace_id().as_deref(),
            )
        });

        // `Signal` is `Copy`; copy into a local `mut` inside the `Fn` closure
        // so we don't mutably borrow a captured variable.
        let state_cb = sidebar_state;
        let socket_evt = socket_for_effect.clone();
        let on_event: Rc<dyn Fn(crate::live_room_socket::ServerEvent)> = Rc::new(move |evt| {
            let mut state_w = state_cb;
            let mut current = state_w.write();
            apply_event_to_broadcast_state(&mut current, evt, &socket_evt);
        });

        let refresh: Rc<
            dyn Fn() -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<String>>>>,
        > = Rc::new(
            || -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<String>>>> {
                Box::pin(crate::api::refresh_access_token())
            },
        );

        let socket_status_w = socket_status;
        let on_status: Rc<dyn Fn(crate::live_room_socket::ConnStatus)> = Rc::new(move |st| {
            let mut status = socket_status_w;
            status.set(st);
            #[cfg(target_arch = "wasm32")]
            web_sys::console::debug_2(
                &"[live_room_broadcast] conn status:".into(),
                &format!("{st:?}").into(),
            );
        });

        crate::live_room_socket::conn::connect_reconnecting(
            socket_for_effect.clone(),
            closed_for_effect.clone(),
            make_url,
            on_event,
            refresh,
            on_status,
        );
    });

    socket
}

// ---------------------------------------------------------------------------
// Event → broadcast sidebar state mutation
// ---------------------------------------------------------------------------

fn apply_event_to_broadcast_state(
    state: &mut BroadcastSidebarState,
    evt: crate::live_room_socket::ServerEvent,
    _socket: &std::rc::Rc<
        std::cell::RefCell<Option<crate::live_room_socket::conn::LiveRoomSocket>>,
    >,
) {
    use crate::live_room_socket::ServerEvent;
    match evt {
        ServerEvent::Chat {
            id,
            sender_display_name,
            body,
            created_at,
            ..
        } => {
            state.messages.push(ChatMessage {
                id,
                sender_display_name,
                body,
                created_at,
                deleted: false,
            });
        }
        ServerEvent::ChatDeleted { id } => {
            for m in state.messages.iter_mut() {
                if m.id == id {
                    m.deleted = true;
                    m.body = "[deleted]".into();
                }
            }
        }
        ServerEvent::PresenceCount { count } => {
            state.presence_count = count;
        }
        ServerEvent::PresenceList { participants } => {
            state.participants = Some(
                participants
                    .into_iter()
                    .map(|p| PresenceParticipant {
                        user_id: p.user_id,
                        display_name: p.display_name,
                        role: p.role,
                    })
                    .collect(),
            );
        }
        ServerEvent::HandRaiseChanged {
            user_id,
            raised,
            display_name,
            queue_position,
        } => {
            // Teacher sees the queue; add/remove entries as students raise/lower.
            if raised && queue_position.is_some() {
                if !state.queue.iter().any(|e| e.user_id == user_id) {
                    let label = if display_name.is_empty() {
                        let short: String = user_id.chars().take(8).collect();
                        format!("user-{short}")
                    } else {
                        display_name.clone()
                    };
                    state.queue.push(HandRaiseEntry {
                        user_id: user_id.clone(),
                        display_name: label,
                    });
                }
            } else {
                state.queue.retain(|e| e.user_id != user_id);
            }
        }
        ServerEvent::CommandFailed { command, reason } => {
            // Surface server-side command failures (e.g. demote/accept/kick)
            // as a `.system-state--error` toast above the teacher's sidebars.
            state.command_error = Some(format!("{command}: {reason}"));
        }
        ServerEvent::WhiteboardStroke { stroke } => {
            // Echo of the teacher's own stroke (or another co-teacher tab).
            // `append_stroke` is idempotent in effect here: the component's
            // local board is replaced by this state on the next props sync,
            // and the stroke appears exactly once.
            let mapped = crate::live_room_view::map_socket_stroke(stroke);
            let _ = crate::live_room_whiteboard::append_stroke(&mut state.whiteboard, mapped);
        }
        ServerEvent::WhiteboardSnapshot { strokes } => {
            let mapped = strokes
                .into_iter()
                .map(crate::live_room_view::map_socket_stroke)
                .collect();
            crate::live_room_whiteboard::apply_snapshot(&mut state.whiteboard, mapped);
        }
        ServerEvent::WhiteboardStrokeRemoved { stroke_id } => {
            crate::live_room_whiteboard::remove_stroke(&mut state.whiteboard, &stroke_id);
        }
        ServerEvent::WhiteboardClear => {
            crate::live_room_whiteboard::clear_board(&mut state.whiteboard);
        }
        ServerEvent::Reaction { emoji, .. } => {
            crate::live_room_reactions::push_reaction(
                &mut state.reactions,
                &mut state.reaction_seq,
                emoji,
            );
        }
        ServerEvent::WhiteboardCursor {
            user_id,
            display_name,
            x,
            y,
        } => {
            crate::live_room_whiteboard::apply_cursor_event(
                &mut state.cursors,
                user_id,
                display_name,
                x,
                y,
            );
        }
        ServerEvent::DrawPermissionChanged { open } => {
            state.draw_open = open;
        }
        ServerEvent::PollStarted {
            poll_id,
            question,
            options,
        } => {
            state.polls.on_started(poll_id, question, options);
        }
        ServerEvent::PollResults { poll_id, counts } => {
            state.polls.on_results(&poll_id, counts);
        }
        ServerEvent::PollEnded { poll_id, counts } => {
            state.polls.on_ended(&poll_id, counts);
        }
        ServerEvent::BreakoutOpened { rooms } => {
            state
                .breakout
                .on_opened(crate::live_room_view::map_socket_breakout_rooms(rooms));
        }
        ServerEvent::BreakoutUpdated { rooms } => {
            state
                .breakout
                .on_updated(crate::live_room_view::map_socket_breakout_rooms(rooms));
        }
        ServerEvent::BreakoutClosed => {
            state.breakout.on_closed();
        }
        ServerEvent::BreakoutSnapshot { open, rooms } => {
            state.breakout.on_snapshot(
                open,
                crate::live_room_view::map_socket_breakout_rooms(rooms),
            );
        }
        // The teacher manages breakouts but is never assigned to one, so a
        // targeted assignment is a no-op for the teacher's panel state.
        ServerEvent::BreakoutAssignment { .. } => {}
        // Teacher doesn't get promoted/demoted — these events are student-facing.
        ServerEvent::Promoted { .. }
        | ServerEvent::Demoted { .. }
        | ServerEvent::StudentPublishing { .. }
        | ServerEvent::Kicked { .. }
        | ServerEvent::SessionEnded
        | ServerEvent::RateLimited { .. }
        | ServerEvent::Error { .. } => {
            // No-op for teacher sidebar state; handle in a future toast layer.
        }
    }
}

// ---------------------------------------------------------------------------
// Mic / camera track toggling — wasm32 only
// ---------------------------------------------------------------------------

#[cfg(target_arch = "wasm32")]
#[derive(Clone, Copy, PartialEq)]
enum TrackKind {
    Audio,
    Video,
}

/// Flip `enabled` on every captured track of the given kind. Disabling an
/// outgoing track makes WebRTC send silence/black frames — viewers see the
/// mute instantly and re-enabling needs no renegotiation.
#[cfg(target_arch = "wasm32")]
fn set_tracks_enabled(
    stream: &Signal<Option<web_sys::MediaStream>>,
    kind: TrackKind,
    enabled: bool,
) {
    use wasm_bindgen::JsCast;
    let Some(stream) = stream.read().clone() else {
        return;
    };
    let tracks = match kind {
        TrackKind::Audio => stream.get_audio_tracks(),
        TrackKind::Video => stream.get_video_tracks(),
    };
    for i in 0..tracks.length() {
        if let Ok(track) = tracks.get(i).dyn_into::<web_sys::MediaStreamTrack>() {
            track.set_enabled(enabled);
        }
    }
}

// ---------------------------------------------------------------------------
// Browser-owned media helpers. Native equivalents live in
// `live_room_native` and the renderer-neutral orchestration below.
// ---------------------------------------------------------------------------

/// Build a getUserMedia audio constraints object with the common quality
/// enhancements enabled (noise suppression, echo cancellation, auto gain),
/// returned as a `JsValue` object. Browsers ignore unknown keys, so this is
/// safe across engines and needs no extra web-sys binding.
/// One tagged console line per publish stage. A teacher's publisher failing is
/// almost always diagnosed from their console after the fact.
#[cfg(target_arch = "wasm32")]
fn log_publish(msg: &str) {
    web_sys::console::log_1(&format!("[live_room_publish] {msg}").into());
}

/// Classify a getUserMedia rejection and turn it into the user-facing text,
/// logging the raw DOMException name alongside. Never panics.
#[cfg(target_arch = "wasm32")]
fn gum_error_text(e: &wasm_bindgen::JsValue) -> String {
    let name = js_sys::Reflect::get(e, &wasm_bindgen::JsValue::from_str("name"))
        .ok()
        .and_then(|v| v.as_string())
        .unwrap_or_default();
    log_publish(&format!("getUserMedia rejected: {name} ({e:?})"));
    crate::live_room_capture::gum_failure_message(crate::live_room_capture::classify_gum_failure(
        &name,
    ))
}

/// `(live_audio, live_video)` counts for a captured stream.
#[cfg(target_arch = "wasm32")]
fn count_live_tracks(stream: &web_sys::MediaStream) -> (usize, usize) {
    use wasm_bindgen::JsCast;
    let tracks = stream.get_tracks();
    let (mut audio, mut video) = (0usize, 0usize);
    for i in 0..tracks.length() {
        let Ok(t) = tracks.get(i).dyn_into::<web_sys::MediaStreamTrack>() else {
            continue;
        };
        if t.ready_state() != web_sys::MediaStreamTrackState::Live {
            continue;
        }
        match t.kind().as_str() {
            "audio" => audio += 1,
            "video" => video += 1,
            _ => {}
        }
    }
    (audio, video)
}

#[cfg(target_arch = "wasm32")]
fn audio_constraints(mic_id: &str) -> wasm_bindgen::JsValue {
    let obj = js_sys::Object::new();
    for key in ["noiseSuppression", "echoCancellation", "autoGainControl"] {
        let _ = js_sys::Reflect::set(
            &obj,
            &wasm_bindgen::JsValue::from_str(key),
            &wasm_bindgen::JsValue::TRUE,
        );
    }
    // Honor a prejoin-selected microphone, if any.
    if !mic_id.is_empty() {
        let _ = js_sys::Reflect::set(
            &obj,
            &wasm_bindgen::JsValue::from_str("deviceId"),
            &wasm_bindgen::JsValue::from_str(mic_id),
        );
    }
    obj.into()
}

/// On-brand "Studio" virtual background as an SVG data URL (deep-green gradient
/// with a soft vignette). Encoded as base64 so the bridge's `Image` can load it
/// without a network round-trip and without tainting the compositing canvas.
#[cfg(target_arch = "wasm32")]
fn studio_bg_data_url() -> String {
    use base64::Engine;
    let svg = r##"<svg xmlns='http://www.w3.org/2000/svg' width='1280' height='720'><defs><linearGradient id='g' x1='0' y1='0' x2='1' y2='1'><stop offset='0' stop-color='#1f4a3d'/><stop offset='1' stop-color='#0d1f19'/></linearGradient><radialGradient id='v' cx='0.5' cy='0.42' r='0.72'><stop offset='0' stop-color='#2f6552' stop-opacity='0.55'/><stop offset='1' stop-color='#0d1f19' stop-opacity='0'/></radialGradient></defs><rect width='1280' height='720' fill='url(#g)'/><rect width='1280' height='720' fill='url(#v)'/></svg>"##;
    let b64 = base64::engine::general_purpose::STANDARD.encode(svg);
    format!("data:image/svg+xml;base64,{b64}")
}

#[cfg(target_arch = "wasm32")]
async fn go_live_flow(
    session_id: &str,
    session: Option<Signal<crate::live_room_session::LiveRoomSession>>,
    mut self_stream: Signal<Option<web_sys::MediaStream>>,
    mut camera_stream: Signal<Option<web_sys::MediaStream>>,
    fx_mode: crate::live_room_video_fx::FxMode,
    fx_bg: String,
    camera_id: String,
    mic_id: String,
) -> Result<(), String> {
    use crate::api::{fetch_json, ApiContext, ApiError};
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;

    // When the route wraps us in `LiveSessionShell`, the API context lives on
    // the session aggregate. Otherwise fall back to the bare context.
    let cx: ApiContext = match session.as_ref() {
        Some(s) => s.read().api().clone(),
        None => crate::api::use_api(),
    };

    #[derive(serde::Deserialize)]
    struct GoLiveResp {
        main_publish_url: String,
        #[serde(rename = "screen_publish_url")]
        _screen_publish_url: String,
        publish_password: String,
        #[serde(default)]
        ice_servers: Vec<crate::live_room_whip::IceServerConfig>,
    }
    let resp: GoLiveResp = fetch_json(
        &cx,
        "POST",
        &format!("/v1/sessions/{session_id}/go-live"),
        Some(&serde_json::json!({})),
    )
    .await
    .map_err(|e: ApiError| e.to_string())?;

    let win = web_sys::window().ok_or_else(|| "no window".to_string())?;
    let nav = win.navigator();
    let media = nav
        .media_devices()
        .map_err(|e| format!("media_devices: {e:?}"))?;
    // Enumerate BEFORE requesting, and ask only for what exists.
    //
    // The old code asked for `{audio, video}` unconditionally. That request is
    // all-or-nothing: on a machine with a microphone but no camera the browser
    // rejects the whole thing with NotFoundError, so the teacher lost the audio
    // they could have broadcast, and the surfaced error named neither device.
    // It also passed a saved deviceId straight through, so an unplugged camera
    // kept being requested by id forever.
    let pre = crate::live_room_devices::enumerate().await;
    let (pre_cams, pre_mics, _) = crate::live_room_devices::partition_by_kind(&pre);
    log_publish(&format!(
        "enumerateDevices: {} camera(s), {} microphone(s)",
        pre_cams.len(),
        pre_mics.len()
    ));
    let plan = match crate::live_room_capture::plan_capture(
        &pre_cams, &pre_mics, &camera_id, &mic_id,
    ) {
        crate::live_room_capture::CaptureDecision::NoInputDevices => {
            log_publish("no camera and no microphone -- refusing to publish");
            return Err(crate::live_room_capture::no_input_devices_message());
        }
        crate::live_room_capture::CaptureDecision::Capture(p) => p,
    };
    log_publish(&format!("requesting {}", plan.summary()));

    let constraints = web_sys::MediaStreamConstraints::new();
    if plan.want_video {
        match plan.camera_id.as_deref() {
            Some(id) => {
                let vc = web_sys::MediaTrackConstraints::new();
                vc.set_device_id(&wasm_bindgen::JsValue::from_str(id));
                constraints.set_video(vc.as_ref());
            }
            None => constraints.set_video(&wasm_bindgen::JsValue::TRUE),
        }
    } else {
        constraints.set_video(&wasm_bindgen::JsValue::FALSE);
    }
    // Audio polish (noise suppression, echo cancellation, auto gain) + the
    // prejoin-selected mic. A plain JS constraints object; browsers ignore
    // unknown keys.
    if plan.want_audio {
        constraints.set_audio(&audio_constraints(plan.mic_id.as_deref().unwrap_or("")));
    } else {
        constraints.set_audio(&wasm_bindgen::JsValue::FALSE);
    }
    let stream_promise = media
        .get_user_media_with_constraints(&constraints)
        .map_err(|e| gum_error_text(&e))?;
    let stream_value = JsFuture::from(stream_promise)
        .await
        .map_err(|e| gum_error_text(&e))?;
    let stream: web_sys::MediaStream = stream_value
        .dyn_into()
        .map_err(|_| "stream cast".to_string())?;

    // The stream must actually carry live media before we build a peer
    // connection around it. An ended track publishes a black/silent MediaMTX
    // path, which from the student side is indistinguishable from a broken
    // encoder -- reject it here, where the message can still be useful.
    let (live_audio, live_video) = count_live_tracks(&stream);
    log_publish(&format!(
        "captured tracks: {live_audio} live audio, {live_video} live video"
    ));
    crate::live_room_capture::publishable_verdict(live_audio, live_video)?;

    // Keep the RAW camera/mic handle for cleanup (and future device toggles).
    //
    // `same_stream`, never `.clone()`: on `web_sys::MediaStream` the inherent
    // DOM `clone()` shadows the `Clone` impl and hands back a stream of CLONED
    // tracks. Cleanup would then stop the clones while the real camera tracks
    // stayed live, leaving the capture indicator lit. See live_room_ice.
    camera_stream.set(Some(crate::live_room_ice::same_stream(&stream)));

    // Apply the background effect when the bridge is available. It returns a
    // processed (canvas) stream carrying the ORIGINAL audio; on any failure it
    // hands back the raw stream, so video always publishes.
    let published = if crate::live_room_video_fx::is_supported() {
        crate::live_room_video_fx::start(&stream, fx_mode.as_wire(), &fx_bg).await
    } else {
        crate::live_room_ice::same_stream(&stream)
    };

    // The self-preview shows exactly what is published (raw, or the FX canvas).
    // `same_stream` shares the handle; `.clone()` would NOT -- it is the DOM's
    // own MediaStream.clone(), which copies the tracks, so the preview would
    // show a detached duplicate that teardown never stops.
    self_stream.set(Some(crate::live_room_ice::same_stream(&published)));

    crate::live_room_whip::set_ice_servers(resp.ice_servers.clone());
    let publisher = match crate::live_room_whip::publish(
        &resp.main_publish_url,
        &resp.publish_password,
        &published,
    )
    .await
    {
        Ok(publisher) => publisher,
        Err(e) => {
            // Publish failed after capture: tear down FX and stop both the raw
            // camera and the (possibly distinct) processed track so no capture
            // indicator stays lit, then let the auto-publish effect retry (it
            // only re-fires when self_stream is None).
            crate::live_room_video_fx::stop();
            stop_media_stream_tracks(&stream);
            stop_media_stream_tracks(&published);
            self_stream.set(None);
            camera_stream.set(None);
            return Err(e);
        }
    };

    // Hand the publisher to the session aggregate so route exit (via
    // `use_drop`) and `end_class()` can close it. If the session is absent
    // (legacy mount path), the publisher drops at the end of this scope —
    // its `Drop` impl best-effort closes the PeerConnection.
    if let Some(mut s) = session {
        s.write().set_publisher(publisher);
    } else {
        drop(publisher);
    }
    Ok(())
}

#[cfg(target_arch = "wasm32")]
async fn start_screen_share_flow(
    session_id: &str,
    session: Option<Signal<crate::live_room_session::LiveRoomSession>>,
    mut screen_stream: Signal<Option<web_sys::MediaStream>>,
) -> Result<(), String> {
    use crate::api::{fetch_json, ApiContext, ApiError};
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;

    let cx: ApiContext = match session.as_ref() {
        Some(s) => s.read().api().clone(),
        None => crate::api::use_api(),
    };

    #[derive(serde::Deserialize)]
    struct GoLiveResp {
        screen_publish_url: String,
        publish_password: String,
        #[serde(default)]
        ice_servers: Vec<crate::live_room_whip::IceServerConfig>,
    }

    let resp: GoLiveResp = fetch_json(
        &cx,
        "POST",
        &format!("/v1/sessions/{session_id}/go-live"),
        Some(&serde_json::json!({})),
    )
    .await
    .map_err(|e: ApiError| e.to_string())?;

    let win = web_sys::window().ok_or_else(|| "no window".to_string())?;
    let media = win
        .navigator()
        .media_devices()
        .map_err(|e| format!("media_devices: {e:?}"))?;
    let constraints = web_sys::DisplayMediaStreamConstraints::new();
    constraints.set_video(&wasm_bindgen::JsValue::TRUE);
    // Request tab/system audio so sharing a video or slide-with-sound carries
    // its audio to students. If the teacher declines audio in the picker the
    // stream simply has no audio track (harmless) — the screen plays muted.
    constraints.set_audio(&wasm_bindgen::JsValue::TRUE);
    let stream_value = JsFuture::from(
        media
            .get_display_media_with_constraints(&constraints)
            .map_err(|e| format!("getDisplayMedia: {e:?}"))?,
    )
    .await
    .map_err(|e| format!("getDisplayMedia await: {e:?}"))?;
    let stream: web_sys::MediaStream = stream_value
        .dyn_into()
        .map_err(|_| "display stream cast".to_string())?;

    // The very stream handed to publish() below -- not a DOM clone of it, or
    // "stop sharing" would stop copies and leave the screen-capture indicator up.
    screen_stream.set(Some(crate::live_room_ice::same_stream(&stream)));
    crate::live_room_whip::set_ice_servers(resp.ice_servers.clone());
    let publisher = match crate::live_room_whip::publish(
        &resp.screen_publish_url,
        &resp.publish_password,
        &stream,
    )
    .await
    {
        Ok(publisher) => publisher,
        Err(e) => {
            stop_media_stream_tracks(&stream);
            screen_stream.set(None);
            return Err(e);
        }
    };

    if let Some(mut s) = session {
        s.write().set_screen_publisher(publisher);
    } else {
        drop(publisher);
    }
    Ok(())
}

#[cfg(target_arch = "wasm32")]
async fn stop_screen_share_flow(
    session: Option<Signal<crate::live_room_session::LiveRoomSession>>,
    mut screen_stream: Signal<Option<web_sys::MediaStream>>,
) {
    if let Some(stream) = screen_stream.write().take() {
        stop_media_stream_tracks(&stream);
    }
    if let Some(mut s) = session {
        s.write().stop_screen_share().await;
    }
}

/// Stop every track of a captured `MediaStream` so the browser releases the
/// camera / microphone. Used to clean up when publishing fails after capture
/// (the success path hands the stream to `WhipPublisher`, which stops the
/// tracks on close/drop instead).
#[cfg(target_arch = "wasm32")]
fn stop_media_stream_tracks(stream: &web_sys::MediaStream) {
    use wasm_bindgen::JsCast;
    let tracks = stream.get_tracks();
    for i in 0..tracks.length() {
        if let Ok(track) = tracks.get(i).dyn_into::<web_sys::MediaStreamTrack>() {
            track.stop();
        }
    }
}

/// In-call camera switch. Opens the chosen camera, then either feeds it INTO the
/// blur bridge (FX active → published canvas track unchanged, no renegotiation,
/// blur preserved) or replaces the publisher's outbound video track (FX off).
/// Only the OLD camera's video track is stopped; the mic is never touched.
#[cfg(target_arch = "wasm32")]
async fn switch_camera_flow(
    session: Option<Signal<crate::live_room_session::LiveRoomSession>>,
    mut self_stream: Signal<Option<web_sys::MediaStream>>,
    mut camera_stream: Signal<Option<web_sys::MediaStream>>,
    device_id: String,
) {
    use wasm_bindgen::JsCast;
    let Some(s) = session else {
        return;
    };

    let new_stream = match s.read().open_camera(&device_id).await {
        Ok(st) => st,
        Err(e) => {
            web_sys::console::warn_1(
                &format!("[live_room_broadcast] open_camera failed: {e}").into(),
            );
            return;
        }
    };

    let prev_camera = camera_stream.read().clone();

    if crate::live_room_video_fx::is_active() {
        // FX path: swap the bridge's INPUT camera; published canvas track stays.
        crate::live_room_video_fx::set_source(&new_stream);
        camera_stream.set(Some(new_stream));
    } else {
        // Non-FX path: replace the publisher's outbound video track.
        let track = new_stream
            .get_video_tracks()
            .get(0)
            .dyn_into::<web_sys::MediaStreamTrack>()
            .ok();
        let Some(track) = track else {
            stop_media_stream_tracks(&new_stream);
            return;
        };
        if let Err(e) = s.read().replace_main_video_track(&track).await {
            web_sys::console::warn_1(
                &format!("[live_room_broadcast] replace_main_video_track failed: {e}").into(),
            );
            stop_media_stream_tracks(&new_stream);
            return;
        }
        // Both signals must reference the SAME stream whose video track was
        // just pushed into the sender. A DOM `.clone()` here gave camera_stream
        // detached copies, so the next switch's "stop the previous camera"
        // stopped clones and left the real device open.
        camera_stream.set(Some(crate::live_room_ice::same_stream(&new_stream)));
        // Re-fires the self-preview effect (depends on self_stream) to attach
        // the new feed to #broadcast-self-video.
        self_stream.set(Some(new_stream));
    }

    // Stop ONLY the previous camera's VIDEO track(s); leave audio alone.
    if let Some(old) = prev_camera {
        let vtracks = old.get_video_tracks();
        for i in 0..vtracks.length() {
            if let Ok(t) = vtracks.get(i).dyn_into::<web_sys::MediaStreamTrack>() {
                t.stop();
            }
        }
    }
}

/// Mobile flip-camera switch. Opens the chosen front/back camera via
/// `open_camera_facing` (no audio — the mic is untouched), then either feeds it
/// INTO the blur bridge (FX active → published canvas track unchanged, blur
/// preserved) or replaces the publisher's outbound video track (FX off). Only
/// the OLD camera's video track is stopped. Mirrors `switch_camera_flow`, but
/// selects by `facingMode` instead of an opaque mobile deviceId.
#[cfg(target_arch = "wasm32")]
async fn flip_camera_flow(
    session: Option<Signal<crate::live_room_session::LiveRoomSession>>,
    mut self_stream: Signal<Option<web_sys::MediaStream>>,
    mut camera_stream: Signal<Option<web_sys::MediaStream>>,
    facing: crate::live_room_devices::Facing,
) {
    use wasm_bindgen::JsCast;

    let new_stream = match crate::live_room_devices::open_camera_facing(facing, false).await {
        Ok(st) => st,
        Err(e) => {
            web_sys::console::warn_1(
                &format!("[live_room_broadcast] flip open_camera_facing failed: {e}").into(),
            );
            return;
        }
    };

    let prev_camera = camera_stream.read().clone();

    if crate::live_room_video_fx::is_active() {
        // FX path: swap the bridge's INPUT camera; published canvas track stays.
        crate::live_room_video_fx::set_source(&new_stream);
        camera_stream.set(Some(new_stream));
    } else {
        // Non-FX path: replace the publisher's outbound video track.
        let track = new_stream
            .get_video_tracks()
            .get(0)
            .dyn_into::<web_sys::MediaStreamTrack>()
            .ok();
        let Some(track) = track else {
            stop_media_stream_tracks(&new_stream);
            return;
        };
        // No active publisher yet (not live) → just stop the freshly-opened
        // stream so the camera light doesn't stay on.
        let Some(s) = session else {
            stop_media_stream_tracks(&new_stream);
            return;
        };
        if let Err(e) = s.read().replace_main_video_track(&track).await {
            web_sys::console::warn_1(
                &format!("[live_room_broadcast] flip replace_main_video_track failed: {e}").into(),
            );
            stop_media_stream_tracks(&new_stream);
            return;
        }
        // Same handle-sharing requirement as switch_camera_flow.
        camera_stream.set(Some(crate::live_room_ice::same_stream(&new_stream)));
        // Re-fires the self-preview effect (depends on self_stream).
        self_stream.set(Some(new_stream));
    }

    // Stop ONLY the previous camera's VIDEO track(s); leave audio alone.
    if let Some(old) = prev_camera {
        let vtracks = old.get_video_tracks();
        for i in 0..vtracks.length() {
            if let Ok(t) = vtracks.get(i).dyn_into::<web_sys::MediaStreamTrack>() {
                t.stop();
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
async fn end_class_flow(
    session_id: &str,
    session: Option<Signal<crate::live_room_session::LiveRoomSession>>,
) -> Result<(), String> {
    use crate::api::{fetch_json, ApiError};

    // Prefer the session-driven path: it closes the publisher (and any
    // WHEP viewers / sockets) locally first, then POSTs `/end-class`, so a
    // server-side teardown cannot race against a still-publishing PC.
    if let Some(mut s) = session {
        return s.write().end_class().await;
    }

    let cx = crate::api::use_api();
    let _: serde_json::Value = fetch_json(
        &cx,
        "POST",
        &format!("/v1/sessions/{session_id}/end-class"),
        Some(&serde_json::json!({})),
    )
    .await
    .map_err(|e: ApiError| e.to_string())?;
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(serde::Deserialize)]
struct NativeGoLiveResponse {
    main_publish_url: String,
    screen_publish_url: String,
    publish_password: String,
    #[serde(default)]
    ice_servers: Vec<crate::live_room_whip::IceServerConfig>,
}

#[cfg(not(target_arch = "wasm32"))]
fn native_api_context(
    session: Option<Signal<crate::live_room_session::LiveRoomSession>>,
) -> crate::api::ApiContext {
    session
        .map(|session| session.read().api().clone())
        .unwrap_or_else(crate::api::use_api)
}

#[cfg(not(target_arch = "wasm32"))]
async fn native_go_live_response(
    session_id: &str,
    session: Option<Signal<crate::live_room_session::LiveRoomSession>>,
) -> Result<(crate::api::ApiContext, NativeGoLiveResponse), String> {
    use crate::api::{fetch_json, ApiError};
    let api = native_api_context(session);
    let response = fetch_json(
        &api,
        "POST",
        &format!("/v1/sessions/{session_id}/go-live"),
        Some(&serde_json::json!({})),
    )
    .await
    .map_err(|error: ApiError| error.to_string())?;
    Ok((api, response))
}

#[cfg(not(target_arch = "wasm32"))]
async fn go_live_flow_native(
    session_id: &str,
    session: Option<Signal<crate::live_room_session::LiveRoomSession>>,
    choice: crate::live_room_prejoin::PrejoinChoice,
) -> Result<(), String> {
    let (_, response) = native_go_live_response(session_id, session).await?;
    let _ = crate::live_room_native::stop_prejoin().await;
    crate::live_room_native::publish(crate::live_room_native::PublishRequest {
        key: "main".into(),
        url: response.main_publish_url,
        password: response.publish_password,
        element_id: Some("broadcast-self-video".into()),
        camera_id: choice.camera_id,
        mic_id: choice.mic_id,
        facing_mode: Some("user".into()),
        screen: false,
        audio_only: false,
        ice_servers: crate::live_room_native::map_ice_servers(&response.ice_servers),
    })
    .await
}

#[cfg(not(target_arch = "wasm32"))]
async fn start_screen_share_flow_native(
    session_id: &str,
    session: Option<Signal<crate::live_room_session::LiveRoomSession>>,
) -> Result<(), String> {
    let (_, response) = native_go_live_response(session_id, session).await?;
    crate::live_room_native::publish(crate::live_room_native::PublishRequest {
        key: "screen".into(),
        url: response.screen_publish_url,
        password: response.publish_password,
        element_id: Some("broadcast-screen-video".into()),
        camera_id: String::new(),
        mic_id: String::new(),
        facing_mode: None,
        screen: true,
        audio_only: false,
        ice_servers: crate::live_room_native::map_ice_servers(&response.ice_servers),
    })
    .await
}

#[cfg(not(target_arch = "wasm32"))]
async fn end_class_flow_native(
    session_id: &str,
    session: Option<Signal<crate::live_room_session::LiveRoomSession>>,
) -> Result<(), String> {
    use crate::api::{fetch_json, ApiError};
    let api = native_api_context(session);
    let _ = crate::live_room_native::close_all().await;
    let _: serde_json::Value = fetch_json(
        &api,
        "POST",
        &format!("/v1/sessions/{session_id}/end-class"),
        Some(&serde_json::json!({})),
    )
    .await
    .map_err(|error: ApiError| error.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(uid: &str, name: &str, role: &str) -> PresenceParticipant {
        PresenceParticipant {
            user_id: uid.into(),
            display_name: name.into(),
            role: role.into(),
        }
    }

    #[test]
    fn breakout_participants_filters_out_teacher() {
        let presence = Some(vec![
            p("t1", "Teacher", "teacher"),
            p("s1", "Ada", "student"),
            p("s2", "Babbage", "student"),
        ]);
        let got = breakout_participants(&presence);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0], ("s1".to_string(), "Ada".to_string()));
        assert_eq!(got[1], ("s2".to_string(), "Babbage".to_string()));
    }

    #[test]
    fn breakout_participants_empty_when_none() {
        assert!(breakout_participants(&None).is_empty());
    }
}
