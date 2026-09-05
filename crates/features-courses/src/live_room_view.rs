// crates/features-courses/src/live_room_view.rs
//! Student watch UI. Picks WebRTC (WHEP) or HLS based on transport_mode.
//! The shared browser/native view owns WHEP/HLS attachment, a reconnecting room
//! socket, chat/presence/hands/polls/breakouts, and collaborative whiteboard
//! state. `LiveRoomSession` provides route-level media teardown ownership.

use crate::live_room_chat::{ChatMessage, LiveRoomChat};
use crate::live_room_hand_raise::{HandRaiseEntry, LiveRoomHandRaise};
use crate::live_room_presence::{LiveRoomPresence, PresenceParticipant};
use design_system::{Badge, BadgeTone};
use dioxus::prelude::*;

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
extern "C" {
    #[wasm_bindgen(
        js_namespace = ["aula", "features"],
        js_name = attachHls,
        catch
    )]
    async fn attach_hls(
        video_id: &str,
        source: &str,
    ) -> Result<wasm_bindgen::JsValue, wasm_bindgen::JsValue>;

    #[wasm_bindgen(js_namespace = ["aula", "features"], js_name = detachHls)]
    fn detach_hls(video_id: &str);
}

// ---------------------------------------------------------------------------
// Tabbed-stage + dock UI state (cross-target; renders under SSR)
// ---------------------------------------------------------------------------

/// Which surface fills the stage. Default = Camera. (For students, "Camera" is
/// the teacher's main feed; there is no self-publish.)
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum StageTab {
    Camera,
    Screen,
    Whiteboard,
}

/// Which right-rail dock panel is active. (Students have no Breakout panel; the
/// variant is shared with the teacher view but never selected here.)
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DockTab {
    Chat,
    People,
    Hands,
    Breakout,
}

// ---------------------------------------------------------------------------
// State shared across sidebars
// ---------------------------------------------------------------------------

/// State driving the live-room sidebars (chat / presence / hand-raise) plus
/// the inline toasts (`banner`, `command_error`). Made public so SSR tests
/// can seed state and render `LiveRoomSidebars` directly.
#[derive(Clone, Default, PartialEq)]
pub struct LiveRoomState {
    pub messages: Vec<ChatMessage>,
    pub presence_count: u32,
    pub participants: Option<Vec<PresenceParticipant>>,
    pub queue: Vec<HandRaiseEntry>,
    pub my_hand_raised: bool,
    pub is_teacher: bool,
    pub whiteboard: crate::live_room_whiteboard::WhiteboardState,
    /// Last server-side command failure reason, surfaced as a
    /// `.system-state--error` toast. `None` clears the toast.
    pub command_error: Option<String>,
    /// Set when the teacher ends the class. The stage swaps to a "session
    /// has ended" panel and the socket stops reconnecting.
    pub session_ended: bool,
    /// In-flight floating emoji reactions (capped; see `live_room_reactions`).
    pub reactions: Vec<crate::live_room_reactions::FloatingReaction>,
    /// Monotonic sequence backing the stable keys of `reactions`.
    pub reaction_seq: u64,
    /// Live per-user whiteboard cursors (ephemeral, not persisted).
    pub cursors: Vec<crate::live_room_whiteboard::RemoteCursor>,
    /// Whether the teacher has opened the whiteboard for student drawing.
    pub draw_open: bool,
    /// Active in-class poll (ephemeral, broker-driven; see `live_room_polls`).
    pub polls: crate::live_room_polls::PollClientState,
    /// Live breakout-room state (ephemeral, broker-driven). Drives the student
    /// banner + the WHEP re-subscribe on assignment / close.
    pub breakout: crate::breakout_rooms::BreakoutClientState,
    /// The main-room WHEP URL this client subscribes to by default. Seeded at
    /// mount from the join response so a breakout close can re-attach the main
    /// feed. Empty when unavailable (HLS / SSR).
    pub main_url: String,
}

// ---------------------------------------------------------------------------
// Props
// ---------------------------------------------------------------------------

#[derive(Props, Clone, PartialEq)]
pub struct LiveRoomViewProps {
    pub session_id: String,
    pub transport_mode: String,
    pub viewer_jwt: Option<String>,
    pub main_url: Option<String>,
    pub screen_url: Option<String>,
}

// ---------------------------------------------------------------------------
// Root component
// ---------------------------------------------------------------------------

pub fn LiveRoomView(props: LiveRoomViewProps) -> Element {
    let state = use_signal(LiveRoomState::default);

    // Tabbed-stage UI state. Default surface = Camera. PiP starts visible.
    let mut stage_tab = use_signal(|| StageTab::Camera);
    let pip_hidden = use_signal(|| false);

    // Right-rail dock state.
    let dock_tab = use_signal(|| DockTab::Chat);
    let mut dock_collapsed = use_signal(|| false);
    let chat_seen = use_signal(|| 0usize);

    // The Screen tab is enabled only when a screen feed exists AND we're on the
    // WebRTC transport. HLS is a single-feed fallback: `render_hls` emits only a
    // Camera surface (no #live-room-screen-video), so a clickable Screen tab
    // would blank the stage. Gating on webrtc keeps the Screen tab disabled for
    // HLS. The render-time `active` resolution in `render_webrtc` already maps
    // Screen->Camera when no screen feed is present, so the shown surface and
    // `aria-selected` never disagree (props.screen_url is fixed per mount here).
    let screen_present = props.transport_mode == "webrtc"
        && !props.screen_url.clone().unwrap_or_default().is_empty();

    // Auto-expand the dock when a command error arrives so the toast is seen.
    {
        let state = state;
        let mut dock_collapsed = dock_collapsed;
        use_effect(move || {
            if state.read().command_error.is_some() && *dock_collapsed.peek() {
                dock_collapsed.set(false);
            }
        });
    }

    // Seed the main-room WHEP URL once so a breakout close can re-attach the
    // main feed (the breakout assignment carries its own whep_url). Done in a
    // mount-only effect so a later state mutation doesn't clobber it.
    {
        let main_url_seed = props.main_url.clone().unwrap_or_default();
        let mut state_seed = state;
        use_effect(move || {
            if state_seed.read().main_url != main_url_seed {
                state_seed.write().main_url = main_url_seed.clone();
            }
        });
    }

    // Signed-in user's id (sub claim) for per-author whiteboard undo/redo +
    // cursor self-filter. Read from the ApiContext token; empty if unavailable
    // (SSR/tests) via try_consume_context, in which case the whiteboard falls
    // back to the global path.
    let local_user_id = try_consume_context::<Signal<crate::api::ApiContext>>()
        .map(|sig| {
            crate::live_room_whiteboard::user_id_from_jwt(&sig.read().id_token).unwrap_or_default()
        })
        .unwrap_or_default();

    // Holds the WhipPublisher for the current promotion, so Demoted can close()
    // it (releases the mic capture + tears down the WHIP PeerConnection).
    let publisher: Signal<Option<crate::live_room_whip::WhipPublisher>> = use_signal(|| None);

    // Inline banner for transient WebSocket-side notifications
    // (RateLimited / Error). Auto-clears after a few seconds.
    let banner: Signal<Option<String>> = use_signal(|| None);

    // Establish the persistent WebSocket on browser and native WebView targets.
    let socket = use_persistent_socket(
        props.session_id.clone(),
        props.viewer_jwt.clone().unwrap_or_default(),
        state,
        publisher,
        banner,
    );

    // Build renderer-neutral socket actions.
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

    let on_raise = {
        let socket = socket.clone();
        move |raise: bool| {
            let payload = serde_json::json!({"type": "hand_raise", "raise": raise}).to_string();
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

    let on_react = {
        let socket = socket.clone();
        move |emoji: String| {
            let payload = serde_json::json!({"type": "reaction", "emoji": emoji}).to_string();
            if let Some(s) = socket.borrow().as_ref() {
                let _ = s.send_text(&payload);
            }
        }
    };

    // Cast a vote in the active poll: optimistically record the local vote
    // (disables the buttons immediately) and send `poll_vote` to the server,
    // which dedupes and broadcasts the authoritative tally back to everyone.
    let on_poll_vote = {
        #[cfg(target_arch = "wasm32")]
        let socket = socket.clone();
        let mut state = state;
        move |option_index: usize| {
            let poll_id = state
                .read()
                .polls
                .active
                .as_ref()
                .map(|p| p.poll_id.clone());
            // Record local vote; bail if it was rejected (already voted/ended).
            let recorded = state.write().polls.record_local_vote(option_index);
            if recorded {
                if let Some(poll_id) = poll_id {
                    let payload = serde_json::json!({
                        "type": "poll_vote",
                        "poll_id": poll_id,
                        "option_index": option_index,
                    })
                    .to_string();
                    if let Some(s) = socket.borrow().as_ref() {
                        let _ = s.send_text(&payload);
                    }
                }
            }
        }
    };

    // Students don't accept/demote — these are teacher-only actions; no-op here.
    let on_accept = move |_uid: String| {};
    let on_demote = move |_uid: String| {};

    let video_body = match props.transport_mode.as_str() {
        "webrtc" => render_webrtc(&props, stage_tab, pip_hidden),
        "hls" => render_hls(&props, stage_tab),
        other => rsx! { div { class: "form-error", "Unknown transport mode: {other}" } },
    };

    // Transient WebSocket-side notifications (Error / RateLimited).
    let banner_body: Element = match banner.read().as_ref() {
        Some(msg) => {
            let msg = msg.clone();
            rsx! {
                div { class: "live-room-banner live-room-banner--warn", "{msg}" }
            }
        }
        None => rsx! {},
    };

    let session_ended = state.read().session_ended;

    rsx! {
        div { class: if dock_collapsed() { "live-room-view live-room-view--dock-collapsed" } else { "live-room-view" },
            div { class: "live-room-status-row",
                if session_ended {
                    Badge {
                        label: "Ended".to_string(),
                        tone: BadgeTone::Neutral,
                    }
                } else {
                    Badge {
                        label: "\u{25cf} LIVE".to_string(),
                        tone: BadgeTone::Live,
                    }
                    // The backend records every session path to MinIO, so the
                    // class is always being recorded while live — surface it so
                    // participants know (and it appears in Recordings afterward).
                    span {
                        class: "live-rec-badge",
                        title: "This session is being recorded",
                        "\u{25cf} REC"
                    }
                }
            }

            if !session_ended {
                {banner_body}
            }

            // Breakout banner — visible only while breakouts are open. Tells the
            // student whether they've been moved to a named sub-room (their feed
            // follows automatically via the WHEP re-subscribe) or are waiting in
            // the main room for an assignment.
            if !session_ended {
                crate::breakout_rooms::BreakoutBanner {
                    state: state.read().breakout.clone(),
                }
            }

            // Main video area
            if session_ended {
                div { class: "live-room-video-area live-room-stage live-room-stage--ended",
                    div { class: "live-room-ended",
                        h3 { class: "live-room-ended-title", "This session has ended" }
                        p { class: "live-room-ended-copy",
                            "Thanks for joining. A recording will appear in the course's recordings page once processing finishes."
                        }
                    }
                }
            } else {
                div { class: "live-room-video-area live-room-stage live-stage live-stage--tabbed",
                    nav {
                        class: "live-stage-tabs",
                        role: "tablist",
                        "aria-label": "Stage view",
                        button {
                            r#type: "button",
                            role: "tab",
                            id: "stage-tab-camera",
                            "aria-controls": "stage-panel-camera",
                            class: if *stage_tab.read() == StageTab::Camera || (!screen_present && *stage_tab.read() == StageTab::Screen) { "live-stage-tab live-stage-tab--active" } else { "live-stage-tab" },
                            "aria-selected": (*stage_tab.read() == StageTab::Camera || (!screen_present && *stage_tab.read() == StageTab::Screen)).to_string(),
                            onclick: move |_| { stage_tab.set(StageTab::Camera); },
                            "Camera"
                        }
                        button {
                            r#type: "button",
                            role: "tab",
                            id: "stage-tab-screen",
                            "aria-controls": "stage-panel-screen",
                            class: if !screen_present { "live-stage-tab live-stage-tab--disabled" } else if *stage_tab.read() == StageTab::Screen { "live-stage-tab live-stage-tab--active" } else { "live-stage-tab" },
                            "aria-selected": (screen_present && *stage_tab.read() == StageTab::Screen).to_string(),
                            disabled: !screen_present,
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
                    // Positioned stage viewport: wraps the active surface (camera/
                    // screen via {video_body}, or the whiteboard) AND anchors the
                    // corner PiP (which lives inside {video_body}). Because the
                    // whiteboard is a CHILD here, the PiP overlays it correctly on
                    // the Whiteboard tab, and ReactionFloats covers the viewport.
                    div { class: "live-stage-body",
                        {video_body}
                        // Whiteboard surface — conditionally MOUNTED so onmounted
                        // measures correct dimensions. The cfg!(not(wasm32)) arm
                        // forces SSR to render it (default tab is Camera under SSR)
                        // so the `live-room-whiteboard` smoke-test substring is
                        // present; in wasm that arm is false, so it still mounts
                        // only when the Whiteboard tab is active -> correct measure.
                        // `key` ties mount identity to dock_collapsed so a collapse
                        // (stage width change with no resize event) forces a
                        // remount -> fresh measure.
                        if *stage_tab.read() == StageTab::Whiteboard || cfg!(not(target_arch = "wasm32")) {
                            div {
                                class: "live-stage-surface live-stage-surface--whiteboard live-stage-surface--active",
                                role: "tabpanel",
                                id: "stage-panel-whiteboard",
                                "aria-labelledby": "stage-tab-whiteboard",
                                key: "wb-{dock_collapsed()}",
                                crate::live_room_whiteboard::LiveRoomWhiteboard {
                                    state: state.read().whiteboard.clone(),
                                    is_teacher: false,
                                    local_user_id: local_user_id.clone(),
                                    session_id: props.session_id.clone(),
                                    cursors: state.read().cursors.clone(),
                                    draw_open: state.read().draw_open,
                                    on_emit_stroke: EventHandler::new(on_whiteboard_stroke),
                                    on_clear: EventHandler::new(on_whiteboard_clear),
                                    on_cursor: EventHandler::new(on_whiteboard_cursor),
                                }
                            }
                        }
                        crate::live_room_reactions::ReactionFloats {
                            reactions: state.read().reactions.clone(),
                        }
                    }
                }
                crate::live_room_reactions::ReactionBar {
                    on_react: EventHandler::new(on_react),
                }
                // Live poll card — only present while a poll is active. Anyone
                // may vote once; the card flips to a live bar chart after voting
                // or when the teacher ends the poll.
                if let Some(poll) = state.read().polls.active.clone() {
                    crate::live_room_polls::PollVoterCard {
                        poll,
                        on_vote: EventHandler::new(on_poll_vote),
                    }
                }
            }

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
                        r#type: "button", role: "tab",
                        id: "dock-tab-chat",
                        "aria-controls": "dock-panel-chat",
                        class: if *dock_tab.read() == DockTab::Chat { "live-dock-tab live-dock-tab--active" } else { "live-dock-tab" },
                        "aria-selected": (*dock_tab.read() == DockTab::Chat).to_string(),
                        onclick: move |_| { let mut t = dock_tab; t.set(DockTab::Chat); let mut s = chat_seen; s.set(state.read().messages.len()); dock_collapsed.set(false); },
                        "Chat"
                        {
                            let unread = if *dock_tab.read() == DockTab::Chat { 0 } else { state.read().messages.len().saturating_sub(chat_seen()) };
                            if unread > 0 { rsx! { span { class: "live-dock-tab__unread", "{unread}" } } } else { rsx! {} }
                        }
                    }
                    button {
                        r#type: "button", role: "tab",
                        id: "dock-tab-people",
                        "aria-controls": "dock-panel-people",
                        class: if *dock_tab.read() == DockTab::People { "live-dock-tab live-dock-tab--active" } else { "live-dock-tab" },
                        "aria-selected": (*dock_tab.read() == DockTab::People).to_string(),
                        onclick: move |_| { let mut t = dock_tab; t.set(DockTab::People); dock_collapsed.set(false); },
                        "People"
                    }
                    button {
                        r#type: "button", role: "tab",
                        id: "dock-tab-hands",
                        "aria-controls": "dock-panel-hands",
                        class: if *dock_tab.read() == DockTab::Hands { "live-dock-tab live-dock-tab--active" } else { "live-dock-tab" },
                        "aria-selected": (*dock_tab.read() == DockTab::Hands).to_string(),
                        onclick: move |_| { let mut t = dock_tab; t.set(DockTab::Hands); dock_collapsed.set(false); },
                        "Hands"
                    }
                }
                {match state.read().command_error.as_ref() {
                    Some(reason) => rsx! { div { class: "system-state system-state--error", "Command failed: {reason}" } },
                    None => rsx! {},
                }}
                div { class: "live-dock-panels",
                    div {
                        class: if *dock_tab.read() == DockTab::Chat { "live-dock-panel live-dock-panel--active" } else { "live-dock-panel" },
                        role: "tabpanel",
                        id: "dock-panel-chat",
                        "aria-labelledby": "dock-tab-chat",
                        LiveRoomChat {
                            messages: state.read().messages.clone(),
                            is_teacher: false,
                            on_send: EventHandler::new(on_send),
                            on_delete: EventHandler::new(on_delete),
                        }
                    }
                    div {
                        class: if *dock_tab.read() == DockTab::People { "live-dock-panel live-dock-panel--active" } else { "live-dock-panel" },
                        role: "tabpanel",
                        id: "dock-panel-people",
                        "aria-labelledby": "dock-tab-people",
                        LiveRoomPresence {
                            count: state.read().presence_count,
                            participants: state.read().participants.clone(),
                            is_teacher: false,
                        }
                    }
                    div {
                        class: if *dock_tab.read() == DockTab::Hands { "live-dock-panel live-dock-panel--active" } else { "live-dock-panel" },
                        role: "tabpanel",
                        id: "dock-panel-hands",
                        "aria-labelledby": "dock-tab-hands",
                        LiveRoomHandRaise {
                            is_teacher: false,
                            my_hand_raised: state.read().my_hand_raised,
                            queue: state.read().queue.clone(),
                            on_raise: EventHandler::new(on_raise),
                            on_accept: EventHandler::new(on_accept),
                            on_demote: EventHandler::new(on_demote),
                        }
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// LiveRoomSidebars — extracted so tests can render the sidebar block with
// pre-seeded state. The handlers default to no-ops, which is sufficient for
// SSR smoke tests that only assert on rendered HTML.
// ---------------------------------------------------------------------------

#[derive(Props, Clone, PartialEq)]
pub struct LiveRoomSidebarsProps {
    pub state: Signal<LiveRoomState>,
    #[props(default)]
    pub on_send: EventHandler<String>,
    #[props(default)]
    pub on_delete: EventHandler<String>,
    #[props(default)]
    pub on_raise: EventHandler<bool>,
    #[props(default)]
    pub on_accept: EventHandler<String>,
    #[props(default)]
    pub on_demote: EventHandler<String>,
}

#[allow(non_snake_case)]
pub fn LiveRoomSidebars(props: LiveRoomSidebarsProps) -> Element {
    let snapshot = props.state.read().clone();

    // `CommandFailed` toast — rendered above the right-rail sidebars so the
    // operator notices it before scanning the queue.
    let command_toast: Element = match snapshot.command_error.as_ref() {
        Some(reason) => rsx! {
            div { class: "system-state system-state--error",
                "Command failed: {reason}"
            }
        },
        None => rsx! {},
    };

    rsx! {
        div { class: "live-room-sidebars",
            {command_toast}
            LiveRoomChat {
                messages: snapshot.messages.clone(),
                is_teacher: snapshot.is_teacher,
                on_send: props.on_send,
                on_delete: props.on_delete,
            }
            LiveRoomPresence {
                count: snapshot.presence_count,
                participants: snapshot.participants.clone(),
                is_teacher: snapshot.is_teacher,
            }
            LiveRoomHandRaise {
                is_teacher: snapshot.is_teacher,
                my_hand_raised: snapshot.my_hand_raised,
                queue: snapshot.queue.clone(),
                on_raise: props.on_raise,
                on_accept: props.on_accept,
                on_demote: props.on_demote,
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Persistent socket — browser and native WebView renderers
// ---------------------------------------------------------------------------

fn use_persistent_socket(
    session_id: String,
    viewer_jwt: String,
    state: Signal<LiveRoomState>,
    publisher: Signal<Option<crate::live_room_whip::WhipPublisher>>,
    banner: Signal<Option<String>>,
) -> std::rc::Rc<std::cell::RefCell<Option<crate::live_room_socket::conn::LiveRoomSocket>>> {
    use std::cell::RefCell;
    use std::rc::Rc;

    // Allocate the Rc once per component mount via `use_hook` so the SAME
    // Rc is returned on every render. Without this, the function would
    // re-allocate a fresh empty Rc each render — the live socket would
    // sit inside the first-render Rc captured by `use_effect`, while the
    // on_send / on_raise closures created on each subsequent render would
    // close over a brand-new empty Rc and silently drop user actions.
    let socket: Rc<RefCell<Option<crate::live_room_socket::conn::LiveRoomSocket>>> =
        use_hook(|| Rc::new(RefCell::new(None)));

    // Optional session — present when this view is rendered inside the
    // route-owned `LiveSessionShell` wrapper. When present, the
    // `StudentPublishing` handler asks the session to attach a WHEP viewer
    // to the promoted student's feed.
    let session: Option<Signal<crate::live_room_session::LiveRoomSession>> =
        try_consume_context::<Signal<crate::live_room_session::LiveRoomSession>>();

    // Read the current access token from the ApiContext signal so the WS URL
    // carries `?access_token=<jwt>`. The backend auth middleware
    // (`require_auth`) requires this query param for WebSocket upgrades —
    // browsers cannot attach an `Authorization` header to a WS handshake,
    // so without the query param the upgrade returns 401 and chat /
    // presence / hand-raise events never reach the client.
    let api_ctx_signal = use_context::<Signal<crate::api::ApiContext>>();

    // Lifetime guard for the reconnect loop: set on unmount so any pending
    // backoff/reconnect task stops instead of resurrecting a socket.
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
        // Guard: launch the reconnecting socket only once per mount.
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

        // URL builder — re-reads the freshest ApiContext token each connect,
        // or uses the override handed back by a 4001 token refresh.
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

        // Per-message handler (unchanged hydration of component state).
        // Signals are `Copy`; copy into a local `mut` inside the `Fn` closure
        // so we don't mutably borrow a captured variable.
        let state_cb = state;
        let publisher_cb = publisher;
        let banner_cb = banner;
        let session_cb = session;
        let viewer_jwt_cb = viewer_jwt.clone();
        let socket_evt = socket_for_effect.clone();
        let closed_cb = closed_for_effect.clone();
        let on_event: Rc<dyn Fn(crate::live_room_socket::ServerEvent)> = Rc::new(move |evt| {
            let is_session_end = matches!(evt, crate::live_room_socket::ServerEvent::SessionEnded);
            {
                let mut state_w = state_cb;
                let mut current = state_w.write();
                apply_event_to_state(
                    &mut current,
                    evt,
                    &socket_evt,
                    publisher_cb,
                    banner_cb,
                    session_cb,
                    viewer_jwt_cb.clone(),
                );
            }
            // The class ended on purpose — stop the reconnect loop so the
            // server's close isn't retried (and surfaced) as a failure.
            if is_session_end {
                closed_cb.set(true);
                if let Some(mut s) = socket_evt.borrow_mut().take() {
                    s.close();
                }
            }
        });

        let refresh: Rc<
            dyn Fn() -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<String>>>>,
        > = Rc::new(
            || -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<String>>>> {
                Box::pin(crate::api::refresh_access_token())
            },
        );

        // Surface connection status through the banner so a dropped socket is
        // visible instead of silent.
        let status_banner = banner;
        let on_status: Rc<dyn Fn(crate::live_room_socket::ConnStatus)> = Rc::new(move |st| {
            use crate::live_room_socket::ConnStatus;
            let mut b = status_banner;
            match st {
                ConnStatus::Connected => b.set(None),
                ConnStatus::Reconnecting => b.set(Some("Reconnecting to the live room…".into())),
                ConnStatus::Disconnected => b.set(Some(
                    "Disconnected from the live room — reload to rejoin.".into(),
                )),
            }
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
// Event → state mutation
// ---------------------------------------------------------------------------

fn apply_event_to_state(
    state: &mut LiveRoomState,
    evt: crate::live_room_socket::ServerEvent,
    _socket: &std::rc::Rc<
        std::cell::RefCell<Option<crate::live_room_socket::conn::LiveRoomSocket>>,
    >,
    mut publisher: Signal<Option<crate::live_room_whip::WhipPublisher>>,
    mut banner: Signal<Option<String>>,
    session: Option<Signal<crate::live_room_session::LiveRoomSession>>,
    viewer_jwt: String,
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
            if raised && queue_position.is_some() {
                if !state.queue.iter().any(|e| e.user_id == user_id) {
                    // Prefer the server-supplied display_name; fall back to a
                    // short id only when the server omits it (older builds).
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
        ServerEvent::Promoted {
            publish_url,
            publish_password,
            ..
        } => {
            // Kick off audio publish for the promoted student and store the
            // returned WhipPublisher so Demoted can close() it.
            let mut publisher = publisher;
            spawn(async move {
                match crate::live_room_audio_publisher::publish_audio(
                    &publish_url,
                    &publish_password,
                )
                .await
                {
                    Ok(p) => publisher.set(Some(p)),
                    Err(e) => {
                        tracing::warn!(error = %e, "promoted-student audio publish failed");
                    }
                }
            });
        }
        ServerEvent::Demoted { user_id, .. } => {
            // Take the publisher out and close it. close(self) is async, so
            // we move it into a spawn_local task: it stops the local mic
            // capture and tears down the WHIP PeerConnection (DELETE
            // resource_url).
            if let Some(mut p) = publisher.write().take() {
                spawn(async move {
                    let _ = p.close().await;
                });
            }
            // Every other participant watching the demoted student closes the
            // WHEP viewer they opened on `StudentPublishing`, so the audio/tile
            // stops cleanly instead of lingering as a dead peer connection.
            if let Some(mut session_sig) = session {
                if let Ok(parsed) = uuid::Uuid::parse_str(&user_id) {
                    spawn(async move {
                        session_sig.write().detach_student(parsed).await;
                    });
                }
            }
        }
        ServerEvent::Kicked { .. } => {
            // Server closes the socket; nothing more to do client-side.
        }
        ServerEvent::SessionEnded => {
            // Swap the stage to the ended panel and stop the reconnect loop —
            // without the flag the socket driver would treat the server's
            // close as a drop and show "Disconnected — reload to rejoin",
            // which reads as a failure rather than the class simply ending.
            state.session_ended = true;
            banner.set(None);
        }
        ServerEvent::StudentPublishing {
            user_id,
            path,
            whep_url,
            ..
        } => {
            // Ask the session aggregate (if provided by the route shell) to
            // open a WHEP subscription onto the promoted student's feed so
            // their tile appears in the strip without us having to manage a
            // viewer locally.
            if let Some(mut session_sig) = session {
                let uid = user_id.clone();
                let viewer_jwt = viewer_jwt.clone();
                // Prefer the server-supplied full WHEP URL (built from the
                // public WebRTC base). Fall back to deriving it from `path`
                // against the session origin for older servers that omit it.
                let url = if whep_url.is_empty() {
                    crate::live_room_session::whep_url_for_path(
                        &session_sig.read().config().api_origin,
                        &path,
                    )
                } else {
                    whep_url.clone()
                };
                spawn(async move {
                    if let Ok(parsed) = uuid::Uuid::parse_str(&uid) {
                        let _ = session_sig
                            .write()
                            .attach_student(parsed, &url, &viewer_jwt)
                            .await;
                    }
                });
            }
        }
        ServerEvent::RateLimited { retry_after_ms } => {
            banner.set(Some(format!(
                "Rate limited — try again in {} ms",
                retry_after_ms
            )));
            let mut banner = banner;
            spawn(async move {
                transient_banner_delay().await;
                banner.set(None);
            });
        }
        ServerEvent::Error { code, message } => {
            banner.set(Some(format!("[{code}] {message}")));
            let mut banner = banner;
            spawn(async move {
                transient_banner_delay().await;
                banner.set(None);
            });
        }
        ServerEvent::WhiteboardStroke { stroke } => {
            let mapped = map_socket_stroke(stroke);
            if let Err(e) =
                crate::live_room_whiteboard::append_stroke(&mut state.whiteboard, mapped)
            {
                state.command_error = Some(format!("whiteboard_stroke: {e}"));
            }
        }
        ServerEvent::WhiteboardSnapshot { strokes } => {
            // Socket hydration: replace the board so late joiners and
            // reconnects see what was drawn before they arrived.
            let mapped = strokes.into_iter().map(map_socket_stroke).collect();
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
            state.breakout.on_opened(map_socket_breakout_rooms(rooms));
        }
        ServerEvent::BreakoutUpdated { rooms } => {
            state.breakout.on_updated(map_socket_breakout_rooms(rooms));
        }
        ServerEvent::BreakoutClosed => {
            state.breakout.on_closed();
            // Everyone returns to the main room — re-attach the main WHEP feed
            // so the student's video flips back from the breakout sub-room.
            reattach_main_whep(session, &state.main_url, &viewer_jwt);
        }
        ServerEvent::BreakoutSnapshot { open, rooms } => {
            state
                .breakout
                .on_snapshot(open, map_socket_breakout_rooms(rooms));
        }
        ServerEvent::BreakoutAssignment {
            room_id,
            room_name,
            whep_url,
            ..
        } => {
            // The targeted assignment is already filtered to this recipient
            // server-side. Update local state and re-subscribe the main WHEP
            // viewer to the assigned room (or back to the main feed).
            let resub = state
                .breakout
                .on_my_assignment(room_id, room_name, whep_url);
            match resub {
                // Assigned to a breakout room → subscribe to its feed.
                Some(url) if !url.is_empty() => {
                    reattach_main_whep(session, &url, &viewer_jwt);
                }
                // Back to the main room (or an empty url) → main feed.
                _ => {
                    reattach_main_whep(session, &state.main_url, &viewer_jwt);
                }
            }
        }
        ServerEvent::CommandFailed { command, reason } => {
            // Surface server-side command failures as a `.system-state--error`
            // toast in the right rail. The reason is already user-facing per
            // the server contract; the command name lands in the same line
            // so operators can correlate. Operators dismiss the toast by
            // re-issuing the command or by the next CommandFailed event
            // overwriting the slot.
            state.command_error = Some(format!("{command}: {reason}"));
        }
    }
}

async fn transient_banner_delay() {
    #[cfg(target_arch = "wasm32")]
    gloo_timers::future::TimeoutFuture::new(5_000).await;
    #[cfg(not(target_arch = "wasm32"))]
    crate::live_room_native::delay(5_000).await;
}

/// Re-attach the main WHEP viewer to `url` (the breakout sub-room feed while
/// assigned, or the main-room feed on return / close), then wire the resulting
/// remote stream onto the `#live-room-main-video` element. No-op when the
/// session aggregate is absent (SSR) or `url` is empty.
fn reattach_main_whep(
    session: Option<Signal<crate::live_room_session::LiveRoomSession>>,
    url: &str,
    viewer_jwt: &str,
) {
    let Some(mut session_sig) = session else {
        return;
    };
    if url.is_empty() {
        return;
    }
    let url = url.to_string();
    let viewer_jwt = viewer_jwt.to_string();
    spawn(async move {
        if let Err(e) = session_sig.write().attach_main(&url, &viewer_jwt).await {
            #[cfg(target_arch = "wasm32")]
            web_sys::console::error_1(&format!("breakout re-attach_main failed: {e}").into());
            #[cfg(not(target_arch = "wasm32"))]
            tracing::warn!(error = %e, "native breakout WHEP re-attach failed");
            #[cfg(target_arch = "wasm32")]
            return;
        }
        #[cfg(target_arch = "wasm32")]
        {
            // Wire the freshly-attached remote stream onto the main video element.
            let stream = session_sig.read().main_remote_stream();
            if let (Some(stream), Some(doc)) =
                (stream, web_sys::window().and_then(|w| w.document()))
            {
                if let Some(el) = doc.get_element_by_id("live-room-main-video") {
                    use wasm_bindgen::JsCast;
                    if let Ok(media_el) = el.dyn_into::<web_sys::HtmlMediaElement>() {
                        media_el.set_src_object(Some(&stream));
                    }
                }
            }
        }
    });
}

/// Map socket-layer breakout rooms into the breakout component vocabulary.
/// Pure + cross-target so it is unit-testable on host and reusable by the
/// broadcast view.
pub(crate) fn map_socket_breakout_rooms(
    rooms: Vec<crate::live_room_socket::BreakoutRoom>,
) -> Vec<crate::breakout_rooms::BreakoutRoomView> {
    rooms
        .into_iter()
        .map(|r| crate::breakout_rooms::BreakoutRoomView {
            id: r.id,
            name: r.name,
            members: r.members,
        })
        .collect()
}

/// Map a socket-layer stroke into the whiteboard component vocabulary.
pub(crate) fn map_socket_stroke(
    stroke: crate::live_room_socket::WhiteboardStroke,
) -> crate::live_room_whiteboard::WhiteboardStroke {
    crate::live_room_whiteboard::WhiteboardStroke {
        id: stroke.id,
        points: stroke
            .points
            .into_iter()
            .map(|p| crate::live_room_whiteboard::WhiteboardPoint { x: p.x, y: p.y })
            .collect(),
        color: stroke.color,
        width: stroke.width,
        tool: match stroke.tool {
            crate::live_room_socket::WhiteboardTool::Pen => {
                crate::live_room_whiteboard::WhiteboardTool::Pen
            }
            crate::live_room_socket::WhiteboardTool::Eraser => {
                crate::live_room_whiteboard::WhiteboardTool::Eraser
            }
        },
        kind: {
            use crate::live_room_socket::WhiteboardKind as SK;
            use crate::live_room_whiteboard::WhiteboardKind as CK;
            match stroke.kind {
                SK::Freehand => CK::Freehand,
                SK::Line => CK::Line,
                SK::Rect => CK::Rect,
                SK::Ellipse => CK::Ellipse,
                SK::Arrow => CK::Arrow,
                SK::Text => CK::Text,
                SK::Image => CK::Image,
            }
        },
        text: stroke.text,
        asset_id: stroke.asset_id,
        author: stroke.author,
    }
}

// ---------------------------------------------------------------------------
// Private video-branch helpers (unchanged from Task 23 baseline)
// ---------------------------------------------------------------------------

fn render_webrtc(
    props: &LiveRoomViewProps,
    stage_tab: Signal<StageTab>,
    pip_hidden: Signal<bool>,
) -> Element {
    let main_url = props.main_url.clone().unwrap_or_default();
    let screen_url = props.screen_url.clone().unwrap_or_default();
    let viewer_jwt = props.viewer_jwt.clone().unwrap_or_default();
    let waiting_for_stream = main_url.is_empty();

    // Surfaced when the WHEP attach fails (expired viewer token after sitting
    // in the lobby, backend restart invalidating the signer, ICE failure…).
    // Previously these errors only hit the console and the student stared at
    // a black rectangle. Retrying reloads the route, which re-joins and mints
    // a fresh viewer JWT.
    let mut video_error: Signal<Option<String>> = use_signal(|| None);

    #[cfg(target_arch = "wasm32")]
    let picture_in_picture_supported = true;
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
    #[cfg(not(target_arch = "wasm32"))]
    let picture_in_picture_supported = native_capabilities
        .read()
        .as_ref()
        .is_some_and(|capabilities| capabilities.picture_in_picture);

    // When the route wraps us in a `LiveSessionShell`, hand the viewer off to
    // the session so it owns the WHEP connection and can close it on route
    // exit. Falls back to the legacy per-component spawn when the wrapper is
    // absent (e.g. SSR tests).
    let session = try_consume_context::<Signal<crate::live_room_session::LiveRoomSession>>();

    use_effect(move || {
        let main_url = main_url.clone();
        let screen_url = screen_url.clone();
        let viewer_jwt = viewer_jwt.clone();
        #[cfg(target_arch = "wasm32")]
        {
            let session = session;
            wasm_bindgen_futures::spawn_local(async move {
                if main_url.is_empty() {
                    return;
                }
                if let Some(mut session_sig) = session {
                    // Session-owned path: attach_main stores the viewer
                    // inside the aggregate so route exit / Drop can close it.
                    if let Err(e) = session_sig
                        .write()
                        .attach_main(&main_url, &viewer_jwt)
                        .await
                    {
                        web_sys::console::error_1(&format!("attach_main failed: {e}").into());
                        video_error
                            .set(Some(format!("Couldn't connect to the video stream ({e})")));
                        return;
                    }
                    if !screen_url.is_empty() {
                        let screen_attach = session_sig
                            .write()
                            .attach_screen(&screen_url, &viewer_jwt)
                            .await;
                        match screen_attach {
                            Ok(()) => {
                                let stream = session_sig.read().screen_remote_stream();
                                if let (Some(stream), Some(doc)) =
                                    (stream, web_sys::window().and_then(|w| w.document()))
                                {
                                    if let Some(el) =
                                        doc.get_element_by_id("live-room-screen-video")
                                    {
                                        use wasm_bindgen::JsCast;
                                        if let Ok(media_el) =
                                            el.dyn_into::<web_sys::HtmlMediaElement>()
                                        {
                                            media_el.set_src_object(Some(&stream));
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                web_sys::console::debug_1(
                                    &format!("screen WHEP inactive or unavailable: {e}").into(),
                                );
                            }
                        }
                    }
                    // Wire the remote stream onto the <video> element. Until
                    // this was added, the session-owned path attached the
                    // WHEP viewer for lifecycle but never rendered the feed,
                    // so the student saw a blank black video even when
                    // signaling and ICE succeeded.
                    let stream = session_sig.read().main_remote_stream();
                    if let (Some(stream), Some(doc)) =
                        (stream, web_sys::window().and_then(|w| w.document()))
                    {
                        if let Some(el) = doc.get_element_by_id("live-room-main-video") {
                            use wasm_bindgen::JsCast;
                            if let Ok(media_el) = el.dyn_into::<web_sys::HtmlMediaElement>() {
                                media_el.set_src_object(Some(&stream));
                            }
                        }
                    }
                    return;
                }
                match crate::live_room_whep::view(&main_url, &viewer_jwt).await {
                    Ok(viewer) => {
                        if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
                            if let Some(el) = doc.get_element_by_id("live-room-main-video") {
                                use wasm_bindgen::JsCast;
                                if let Ok(media_el) = el.dyn_into::<web_sys::HtmlMediaElement>() {
                                    media_el.set_src_object(Some(&viewer.remote_stream));
                                }
                            }
                        }
                    }
                    Err(e) => {
                        web_sys::console::error_1(&format!("WHEP failed: {e}").into());
                        video_error
                            .set(Some(format!("Couldn't connect to the video stream ({e})")));
                    }
                }
            });
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let session = session;
            spawn(async move {
                if main_url.is_empty() {
                    return;
                }
                let result = if let Some(mut session) = session {
                    let main = session.write().attach_main(&main_url, &viewer_jwt).await;
                    if main.is_ok() && !screen_url.is_empty() {
                        if let Err(error) = session
                            .write()
                            .attach_screen(&screen_url, &viewer_jwt)
                            .await
                        {
                            tracing::debug!(%error, "native screen WHEP feed is not active");
                        }
                    }
                    main
                } else {
                    crate::live_room_native::view(crate::live_room_native::ViewRequest {
                        key: "main".into(),
                        url: main_url,
                        password: viewer_jwt,
                        element_id: "live-room-main-video".into(),
                        ice_servers: Vec::new(),
                    })
                    .await
                };
                match result {
                    Ok(()) => video_error.set(None),
                    Err(error) => {
                        tracing::warn!(%error, "native WHEP attach failed");
                        video_error.set(Some(format!(
                            "Couldn't connect to the video stream ({error})"
                        )));
                    }
                }
            });
        }
    });

    if waiting_for_stream {
        return rsx! {
            div { class: "live-stage-surfaces",
                crate::live_room_health::StreamStateNotice {
                    state: crate::live_room_health::StreamNoticeState::Waiting,
                    title: "Waiting for stream".to_string(),
                    detail: "The teacher stream is not available yet.".to_string(),
                    on_retry: move |_| {},
                }
            }
        };
    }

    #[cfg(not(target_arch = "wasm32"))]
    let retry_main_url = props.main_url.clone().unwrap_or_default();
    #[cfg(not(target_arch = "wasm32"))]
    let retry_viewer_jwt = props.viewer_jwt.clone().unwrap_or_default();
    let error_overlay: Element = match video_error.read().as_ref() {
        Some(msg) => rsx! {
            div { class: "live-video-error",
                crate::live_room_health::StreamStateNotice {
                    state: crate::live_room_health::StreamNoticeState::Failed,
                    title: "Stream unavailable".to_string(),
                    detail: msg.clone(),
                    on_retry: move |_| {
                        #[cfg(target_arch = "wasm32")]
                        if let Some(win) = web_sys::window() {
                            let _ = win.location().reload();
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        {
                            let mut error = video_error;
                            error.set(Some("Reconnecting to the video stream…".into()));
                            let session = session;
                            let main_url = retry_main_url.clone();
                            let viewer_jwt = retry_viewer_jwt.clone();
                            spawn(async move {
                                let result = if let Some(mut session) = session {
                                    session.write().attach_main(&main_url, &viewer_jwt).await
                                } else {
                                    crate::live_room_native::view(
                                        crate::live_room_native::ViewRequest {
                                            key: "main".into(),
                                            url: main_url,
                                            password: viewer_jwt,
                                            element_id: "live-room-main-video".into(),
                                            ice_servers: Vec::new(),
                                        },
                                    )
                                    .await
                                };
                                error.set(result.err().map(|message| format!(
                                    "Couldn't connect to the video stream ({message})"
                                )));
                            });
                        }
                    },
                }
            }
        },
        None => rsx! {},
    };

    let screen_present = !props.screen_url.clone().unwrap_or_default().is_empty();
    // Resolve the SHOWN tab once; the tab bar (Edit S2) uses this SAME value for
    // aria-selected so the announced tab and the visible surface never disagree.
    let active = match (*stage_tab.read(), screen_present) {
        (StageTab::Screen, false) => StageTab::Camera,
        (t, _) => t,
    };
    rsx! {
        // Block container for the stage surfaces + the inline/PiP person video.
        // NOT positioned and NOT a grid (one surface shows at a time): the PiP
        // anchors to the parent `.live-stage-body` (added in LiveRoomView) which
        // also wraps the whiteboard surface, so the corner PiP overlays whichever
        // surface is active and the camera feed fills the full stage width.
        div { class: "live-stage-surfaces",
            {error_overlay}
            // Camera surface — display:none on non-Camera tabs. Holds NO
            // stream element (the person video lives in the sibling live-pip).
            div {
                class: if active == StageTab::Camera { "live-stage-surface live-stage-surface--camera live-stage-surface--active" } else { "live-stage-surface live-stage-surface--camera" },
                role: "tabpanel",
                id: "stage-panel-camera",
                "aria-labelledby": "stage-tab-camera",
            }
            // The SINGLE main-video element (the teacher feed the student sees).
            // Inline-large on Camera; floating top-right corner PiP on
            // Screen/Whiteboard. Top-right clears the bottom native-controls bar.
            // One DOM node always -> WHEP stream stays attached (Invariant B).
            div {
                class: {
                    let off = active != StageTab::Camera;
                    match (off, pip_hidden()) {
                        (true, true) => "live-pip live-pip--active live-pip--hidden live-pip-corner--tr",
                        (true, false) => "live-pip live-pip--active live-pip-corner--tr",
                        (false, _) => "live-pip",
                    }
                },
                if active != StageTab::Camera {
                    button {
                        r#type: "button",
                        class: "live-pip-toggle",
                        "aria-label": if pip_hidden() { "Show teacher video".to_string() } else { "Hide teacher video".to_string() },
                        title: if pip_hidden() { "Show teacher video".to_string() } else { "Hide teacher video".to_string() },
                        onclick: move |_| { let mut h = pip_hidden; let n = !h(); h.set(n); },
                        if pip_hidden() { "Show" } else { "Hide" }
                    }
                }
                // OS-level Picture-in-Picture. Hidden by CSS while the PiP is in
                // corner (--active) mode (OS-PiP from a thumbnail is nonsensical);
                // only usable on the inline Camera tab.
                button {
                    r#type: "button",
                    class: "live-pip-button",
                    disabled: !picture_in_picture_supported,
                    title: if picture_in_picture_supported {
                        "Pop the teacher video into a floating window"
                    } else {
                        "System picture-in-picture is unavailable on this device"
                    },
                    onclick: move |_| {
                        spawn(async {
                            let _ = crate::live_room_devices::request_picture_in_picture(
                                "live-room-main-video",
                            )
                            .await;
                        });
                    },
                    if picture_in_picture_supported { "Pop out video" } else { "PiP unavailable" }
                }
                video {
                    id: "live-room-main-video",
                    autoplay: true,
                    playsinline: true,
                    controls: true,
                    class: "live-video-main",
                }
            }
            // Screen surface — element stays mounted so its WHEP stream stays
            // attached; hidden via display:none when not the active tab.
            div {
                class: if active == StageTab::Screen { "live-stage-surface live-stage-surface--screen live-stage-surface--active" } else { "live-stage-surface live-stage-surface--screen" },
                role: "tabpanel",
                id: "stage-panel-screen",
                "aria-labelledby": "stage-tab-screen",
                video {
                    id: "live-room-screen-video",
                    autoplay: true,
                    playsinline: true,
                    controls: true,
                    class: "live-video-screen",
                }
            }
        }
    }
}

fn render_hls(props: &LiveRoomViewProps, stage_tab: Signal<StageTab>) -> Element {
    let main_url = props.main_url.clone().unwrap_or_default();

    let main_url_for_effect = main_url.clone();
    use_effect(move || {
        let main_url = main_url_for_effect.clone();
        #[cfg(target_arch = "wasm32")]
        {
            wasm_bindgen_futures::spawn_local(async move {
                if !main_url.is_empty() {
                    let _ = attach_hls("live-room-main-video", &main_url).await;
                }
            });
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            spawn(async move {
                if !main_url.is_empty() {
                    let _ = crate::live_room_native::attach_hls("live-room-main-video", &main_url)
                        .await;
                }
            });
        }
    });

    // hls.js owns workers, timers and network requests. Tear it down when the
    // fallback surface unmounts instead of leaking work across route changes.
    #[cfg(target_arch = "wasm32")]
    use_drop(move || detach_hls("live-room-main-video"));
    #[cfg(not(target_arch = "wasm32"))]
    use_drop(move || {
        spawn(async {
            let _ = crate::live_room_native::detach_hls("live-room-main-video").await;
        });
    });

    if main_url.is_empty() {
        return rsx! {
            div { class: "live-stage-surfaces",
                crate::live_room_health::StreamStateNotice {
                    state: crate::live_room_health::StreamNoticeState::Waiting,
                    title: "Waiting for stream".to_string(),
                    detail: "The class stream is not available yet.".to_string(),
                    on_retry: move |_| {},
                }
            }
        };
    }

    rsx! {
        // HLS has a single feed and no PiP; wrap the video in a Camera surface so
        // the Whiteboard tab hides it (display:none keeps hls.js + audio alive)
        // instead of overlaying the whiteboard on top of a still-visible video.
        div { class: "live-stage-surfaces",
            div {
                class: if *stage_tab.read() == StageTab::Camera { "live-stage-surface live-stage-surface--camera live-stage-surface--active" } else { "live-stage-surface live-stage-surface--camera" },
                role: "tabpanel",
                id: "stage-panel-camera",
                "aria-labelledby": "stage-tab-camera",
                video {
                    id: "live-room-main-video",
                    autoplay: true,
                    playsinline: true,
                    controls: true,
                    class: "live-video-main",
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live_room_socket::BreakoutRoom as WireRoom;

    #[test]
    fn map_socket_breakout_rooms_preserves_fields() {
        let wire = vec![
            WireRoom {
                id: "r1".into(),
                name: "Group 1".into(),
                members: vec!["u1".into(), "u2".into()],
            },
            WireRoom {
                id: "r2".into(),
                name: "Group 2".into(),
                members: vec![],
            },
        ];
        let view = map_socket_breakout_rooms(wire);
        assert_eq!(view.len(), 2);
        assert_eq!(view[0].id, "r1");
        assert_eq!(view[0].name, "Group 1");
        assert_eq!(view[0].members.len(), 2);
        assert!(view[1].members.is_empty());
    }

    #[test]
    fn breakout_banner_renders_when_state_open_and_assigned() {
        // SSR smoke test: a state seeded with an open breakout + this client's
        // assignment renders the banner with the room name.
        fn app() -> Element {
            let mut bk = crate::breakout_rooms::BreakoutClientState::default();
            bk.on_opened(vec![crate::breakout_rooms::BreakoutRoomView {
                id: "r1".into(),
                name: "Group 1".into(),
                members: vec!["u1".into()],
            }]);
            bk.on_my_assignment(Some("r1".into()), "Group 1".into(), "url".into());
            rsx! { crate::breakout_rooms::BreakoutBanner { state: bk } }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("Group 1"), "room name missing: {html}");
    }
}
