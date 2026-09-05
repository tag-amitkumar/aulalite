// crates/features-courses/tests/live_room_smoke.rs
//! SSR smoke tests confirm that LiveRoomShell, LiveRoomLobby, and
//! LiveRoomView render expected text/HTML for each branch.

use dioxus::prelude::*;
use features_courses::api::ApiContext;
use features_courses::live_room_shell::{
    CallerRole, LiveRoomShell, LiveRoomShellProps, SessionStatus,
};

#[derive(Props, Clone, PartialEq)]
struct LiveRoomTestHarnessProps {
    shell: LiveRoomShellProps,
}

#[allow(non_snake_case)]
fn LiveRoomTestHarness(props: LiveRoomTestHarnessProps) -> Element {
    use_context_provider(|| {
        Signal::new(ApiContext {
            base_url: "http://localhost:8080".into(),
            id_token: "ssr-test-token".into(),
        })
    });
    rsx! { LiveRoomShell { ..props.shell } }
}

fn render_shell(role: CallerRole, status: SessionStatus) -> String {
    let is_teacher = matches!(role, CallerRole::Teacher);
    let props = LiveRoomShellProps {
        session_id: "00000000-0000-0000-0000-000000000000".to_string(),
        course_slug: "test-course".to_string(),
        caller_role: role,
        status,
        course_title: "Algebra 1".to_string(),
        instructor_name: Some("Ms. Smith".to_string()),
        scheduled_starts_at_iso: "2026-05-08T18:00:00Z".to_string(),
        transport_mode: "webrtc".to_string(),
        viewer_jwt: Some("dummy.jwt.value".to_string()),
        main_url: Some("http://localhost:8889/aula/x/y/z/whep".to_string()),
        screen_url: None,
        has_recording: false,
        is_teacher,
    };
    let mut vdom = VirtualDom::new_with_props(
        LiveRoomTestHarness,
        LiveRoomTestHarnessProps { shell: props },
    );
    vdom.rebuild_in_place();
    dioxus_ssr::render(&vdom)
}

#[test]
fn lobby_renders_class_will_begin() {
    let html = render_shell(CallerRole::Student, SessionStatus::Scheduled);
    assert!(html.contains("Class will begin shortly"), "got: {html}");
    assert!(html.contains("Algebra 1"), "got: {html}");
    assert!(html.contains("Ms. Smith"), "got: {html}");
}

#[test]
fn live_renders_video_tag_for_webrtc() {
    let html = render_shell(CallerRole::Student, SessionStatus::Live);
    assert!(html.contains("live-video-main"), "got: {html}");
    assert!(html.contains("● LIVE"), "got: {html}");
    assert!(html.contains("live-room-stage"), "got: {html}");
    assert!(html.contains("live-room-sidebars"), "got: {html}");
}

#[test]
fn student_live_renders_screen_and_whiteboard_surfaces() {
    let html = render_shell(CallerRole::Student, SessionStatus::Live);
    assert!(html.contains("live-room-main-video"), "got: {html}");
    assert!(html.contains("live-room-screen-video"), "got: {html}");
    assert!(html.contains("live-room-whiteboard"), "got: {html}");
}

#[test]
fn ended_renders_ended_message_for_any_role() {
    let html_t = render_shell(CallerRole::Teacher, SessionStatus::Ended);
    let html_s = render_shell(CallerRole::Student, SessionStatus::Ended);
    assert!(html_t.contains("Class has ended"), "got: {html_t}");
    assert!(html_s.contains("Class has ended"), "got: {html_s}");
}

#[test]
fn cancelled_renders_cancelled_message() {
    let html = render_shell(CallerRole::Student, SessionStatus::Cancelled);
    assert!(html.contains("Class was cancelled"), "got: {html}");
}

#[test]
fn teacher_scheduled_renders_broadcast_with_go_live() {
    let html = render_shell(CallerRole::Teacher, SessionStatus::Scheduled);
    // The Idle state now shows the prejoin device-test gate (its "Go live"
    // button kicks off the broadcast) rather than a bare "Go Live" button.
    assert!(html.contains("live-room-prejoin"), "got: {html}");
    assert!(html.contains("Go live"), "got: {html}");
    assert!(html.contains("Broadcast"), "got: {html}");
    assert!(html.contains("live-room-broadcast"), "got: {html}");
}

#[test]
fn teacher_live_renders_capability_gated_screen_share_control() {
    let html = render_shell(CallerRole::Teacher, SessionStatus::Live);
    // Host SSR has no WebView capability probe, so the shared native branch
    // must render the disabled control rather than pretending capture works.
    assert!(html.contains("Screen share unavailable"), "got: {html}");
    assert!(html.contains("broadcast-capability-note"), "got: {html}");
}

use features_courses::live_room_chat::{ChatMessage, LiveRoomChat};
use features_courses::live_room_hand_raise::{HandRaiseEntry, LiveRoomHandRaise};
use features_courses::live_room_presence::{LiveRoomPresence, PresenceParticipant};

#[test]
fn chat_renders_messages() {
    fn app() -> Element {
        rsx! {
            LiveRoomChat {
                messages: vec![
                    ChatMessage { id: "1".into(), sender_display_name: "Alice".into(),
                                  body: "Hello!".into(), created_at: "2026".into(), deleted: false },
                ],
                is_teacher: false,
                on_send: |_: String| {},
                on_delete: |_: String| {},
            }
        }
    }
    let mut vdom = VirtualDom::new(app);
    vdom.rebuild_in_place();
    let html = dioxus_ssr::render(&vdom);
    assert!(html.contains("Alice"), "got: {html}");
    assert!(html.contains("Hello!"), "got: {html}");
}

#[test]
fn chat_renders_empty_state() {
    fn app() -> Element {
        rsx! {
            LiveRoomChat {
                messages: vec![],
                is_teacher: false,
                on_send: |_: String| {},
                on_delete: |_: String| {},
            }
        }
    }
    let mut vdom = VirtualDom::new(app);
    vdom.rebuild_in_place();
    let html = dioxus_ssr::render(&vdom);
    assert!(html.contains("No messages yet"), "got: {html}");
}

#[test]
fn presence_renders_count_for_student() {
    let mut vdom = VirtualDom::new_with_props(
        LiveRoomPresence,
        features_courses::live_room_presence::LiveRoomPresenceProps {
            count: 32,
            participants: None,
            is_teacher: false,
        },
    );
    vdom.rebuild_in_place();
    let html = dioxus_ssr::render(&vdom);
    assert!(html.contains("32 watching"), "got: {html}");
    assert!(
        !html.contains("Participants"),
        "students should not see list: {html}"
    );
}

#[test]
fn presence_renders_list_for_teacher() {
    let participants = vec![PresenceParticipant {
        user_id: "u1".into(),
        display_name: "Alice".into(),
        role: "student".into(),
    }];
    let mut vdom = VirtualDom::new_with_props(
        LiveRoomPresence,
        features_courses::live_room_presence::LiveRoomPresenceProps {
            count: 1,
            participants: Some(participants),
            is_teacher: true,
        },
    );
    vdom.rebuild_in_place();
    let html = dioxus_ssr::render(&vdom);
    assert!(html.contains("Alice"), "got: {html}");
    assert!(
        html.contains("Participants"),
        "teacher should see list heading: {html}"
    );
}

#[test]
fn hand_raise_renders_button_for_student() {
    fn app() -> Element {
        rsx! {
            LiveRoomHandRaise {
                is_teacher: false,
                my_hand_raised: false,
                queue: vec![],
                on_raise: |_: bool| {},
                on_accept: |_: String| {},
                on_demote: |_: String| {},
            }
        }
    }
    let mut vdom = VirtualDom::new(app);
    vdom.rebuild_in_place();
    let html = dioxus_ssr::render(&vdom);
    assert!(html.contains("Raise hand"), "got: {html}");
}

#[test]
fn hand_raise_renders_queue_for_teacher() {
    fn app() -> Element {
        rsx! {
            LiveRoomHandRaise {
                is_teacher: true,
                my_hand_raised: false,
                queue: vec![
                    HandRaiseEntry { user_id: "u1".into(), display_name: "Alice".into() },
                ],
                on_raise: |_: bool| {},
                on_accept: |_: String| {},
                on_demote: |_: String| {},
            }
        }
    }
    let mut vdom = VirtualDom::new(app);
    vdom.rebuild_in_place();
    let html = dioxus_ssr::render(&vdom);
    assert!(html.contains("Alice"), "got: {html}");
    assert!(html.contains("Accept"), "got: {html}");
}

use features_courses::live_room_replay::LiveRoomReplay;
use features_courses::live_room_view::{LiveRoomSidebars, LiveRoomState};
use features_courses::live_room_whiteboard::{
    LiveRoomWhiteboard, WhiteboardKind, WhiteboardPoint, WhiteboardState, WhiteboardStroke,
    WhiteboardTool,
};

#[test]
fn renders_command_failed_toast() {
    // Render the sidebar block with a CommandFailed reason already in state.
    // The view must surface a .system-state--error toast carrying the reason.
    fn app() -> Element {
        let state = use_signal(|| LiveRoomState {
            command_error: Some("broker unavailable".into()),
            ..LiveRoomState::default()
        });
        rsx! { LiveRoomSidebars { state: state } }
    }
    let mut vdom = VirtualDom::new(app);
    vdom.rebuild_in_place();
    let html = dioxus_ssr::render(&vdom);
    assert!(
        html.contains("system-state--error"),
        "expected .system-state--error toast in: {html}"
    );
    assert!(
        html.contains("broker unavailable"),
        "expected reason text in: {html}"
    );
}

#[test]
fn hand_raise_shows_real_display_name() {
    // Seed the queue with an entry built from a HandRaiseChanged payload that
    // carried display_name = "Student Sam". The view must render that name and
    // must NOT fall back to a "user-<short>" placeholder.
    fn app() -> Element {
        let state = use_signal(|| LiveRoomState {
            is_teacher: true,
            queue: vec![HandRaiseEntry {
                user_id: "abcdef1234567890".into(),
                display_name: "Student Sam".into(),
            }],
            ..LiveRoomState::default()
        });
        rsx! { LiveRoomSidebars { state: state } }
    }
    let mut vdom = VirtualDom::new(app);
    vdom.rebuild_in_place();
    let html = dioxus_ssr::render(&vdom);
    assert!(
        html.contains("Student Sam"),
        "expected real display_name in: {html}"
    );
    assert!(
        !html.contains("user-abcdef12"),
        "fallback short id leaked into UI: {html}"
    );
}

#[test]
fn replay_renders_loading_initially() {
    fn app() -> Element {
        // Provide a stub Signal<ApiContext> so use_api() (which reads a
        // Signal<ApiContext> from context) does not panic. On non-wasm32,
        // fetch_json always returns an error immediately, so use_resource starts
        // in the None state and the component renders "Loading recording…".
        use_context_provider(|| {
            Signal::new(ApiContext {
                base_url: String::new(),
                id_token: String::new(),
            })
        });
        rsx! {
            LiveRoomReplay {
                session_id: "00000000-0000-0000-0000-000000000000".to_string(),
                course_title: "Algebra 1".to_string(),
                instructor_name: Some("Ms. Smith".to_string()),
                is_teacher: false,
            }
        }
    }
    let mut vdom = VirtualDom::new(app);
    vdom.rebuild_in_place();
    let html = dioxus_ssr::render(&vdom);
    assert!(
        html.contains("Loading recording") || html.contains("live-room-replay"),
        "got: {html}"
    );
}

#[test]
fn whiteboard_renders_seeded_strokes() {
    fn app() -> Element {
        rsx! {
            LiveRoomWhiteboard {
                state: WhiteboardState {
                    strokes: vec![WhiteboardStroke {
                        id: "stroke-1".into(),
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
                        author: None,
                    }],
                    ..WhiteboardState::default()
                },
                is_teacher: false,
                on_emit_stroke: |_: WhiteboardStroke| {},
                on_clear: |_: ()| {},
            }
        }
    }
    let mut vdom = VirtualDom::new(app);
    vdom.rebuild_in_place();
    let html = dioxus_ssr::render(&vdom);
    assert!(html.contains("live-room-whiteboard"), "got: {html}");
    assert!(html.contains("polyline"), "got: {html}");
}

#[test]
fn whiteboard_controls_render_for_teacher_only() {
    fn teacher_app() -> Element {
        rsx! {
            LiveRoomWhiteboard {
                state: WhiteboardState::default(),
                is_teacher: true,
                on_emit_stroke: |_: WhiteboardStroke| {},
                on_clear: |_: ()| {},
            }
        }
    }
    fn student_app() -> Element {
        rsx! {
            LiveRoomWhiteboard {
                state: WhiteboardState::default(),
                is_teacher: false,
                on_emit_stroke: |_: WhiteboardStroke| {},
                on_clear: |_: ()| {},
            }
        }
    }

    let mut teacher_vdom = VirtualDom::new(teacher_app);
    teacher_vdom.rebuild_in_place();
    let teacher_html = dioxus_ssr::render(&teacher_vdom);
    assert!(teacher_html.contains("Clear"), "got: {teacher_html}");

    let mut student_vdom = VirtualDom::new(student_app);
    student_vdom.rebuild_in_place();
    let student_html = dioxus_ssr::render(&student_vdom);
    assert!(
        !student_html.contains("Clear"),
        "student should not see clear control: {student_html}"
    );
}
