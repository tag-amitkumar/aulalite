# Live Room Cycle 2 Screen Share Whiteboard Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add teacher screen share, promoted-student WHEP viewer-JWT correctness, and a teacher-controlled ephemeral whiteboard to the existing live-room experience.

**Architecture:** Extend the current Axum live-room WebSocket protocol and Dioxus live-room components. Media stays on the existing MediaMTX main and `/screen` paths; ephemeral whiteboard strokes ride through the existing `LiveRoomBroker` and are stored only in client memory.

**Tech Stack:** Rust 2021, Dioxus 0.7.4, Axum WebSocket, SQLx/Postgres tests, MediaMTX WHIP/WHEP helpers, `web-sys` for media capture, SVG rendering for whiteboard.

---

## Scope Check

This plan covers one subsystem: the live classroom runtime. It intentionally avoids saved whiteboards, replay sync, student drawing permissions, migrations, TURN, and MediaMTX topology changes.

## Current Baseline

The worktree already contains uncommitted live-room video/auth changes:

- `crates/backend/src/handlers/live_sessions.rs`
- `crates/features-courses/src/live_room_session.rs`
- `crates/features-courses/src/live_room_view.rs`

Do not revert these. Continue from them. Stage only files changed by each task.

## File Structure

**Create**

- `crates/features-courses/src/live_room_whiteboard.rs` - pure Rust/Dioxus whiteboard state reducer and UI.

**Modify**

- `crates/core-types/src/live_room.rs` - shared whiteboard wire structs/events.
- `crates/backend/src/services/live_room.rs` - backend broker event variants and validation helper.
- `crates/backend/src/handlers/live_sessions.rs` - client whiteboard commands and server-side auth/broadcast handling.
- `crates/features-courses/src/lib.rs` - export whiteboard module.
- `crates/features-courses/src/live_room_socket.rs` - frontend socket event variants.
- `crates/features-courses/src/live_room_session.rs` - screen publisher/viewer ownership and `attach_student` viewer-JWT fix.
- `crates/features-courses/src/live_room_broadcast.rs` - teacher screen share controls, local preview, whiteboard controls.
- `crates/features-courses/src/live_room_view.rs` - student stage layout, screen WHEP attach, whiteboard rendering/event handling.
- `crates/design-system/assets/components.css` - live-room stage, screen tile, whiteboard styling.
- `crates/shell-web/public/assets/components.css` - mirror CSS asset for the running web shell.
- `crates/features-courses/tests/live_room_smoke.rs` - SSR tests for screen share and whiteboard.
- `crates/backend/tests/live_room.rs` - integration tests for whiteboard command authorization/broadcast.

---

## Task 1: Baseline Guard

**Files:**
- Inspect only.

- [ ] **Step 1: Record worktree state**

Run:

```powershell
git status --short
git diff --stat HEAD
```

Expected: existing uncommitted live-room files and generated logs are visible. Do not stage or revert unrelated files.

- [ ] **Step 2: Confirm design spec exists**

Run:

```powershell
Test-Path 'docs\superpowers\specs\2026-06-01-live-room-cycle-2-screen-share-whiteboard-design.md'
```

Expected: output is `True`.

- [ ] **Step 3: Run focused compile baseline**

Run:

```powershell
cargo check -p features-courses
cargo check -p backend
```

Expected: both pass. If either fails, inspect whether the failure comes from the pre-existing live-room diffs. Fix compile errors only when they block this plan and keep the fix in the task that first touches the file.

---

## Task 2: Shared Whiteboard Wire Types

**Files:**
- Modify: `crates/core-types/src/live_room.rs`
- Modify: `crates/backend/src/services/live_room.rs`
- Modify: `crates/features-courses/src/live_room_socket.rs`

- [ ] **Step 1: Add shared whiteboard structs and event variants**

In `crates/core-types/src/live_room.rs`, add these structs after `StudentDemoted`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum WhiteboardTool {
    Pen,
    Eraser,
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
}
```

Then add these variants to `ServerEvent`:

```rust
    WhiteboardStroke {
        stroke: WhiteboardStroke,
    },
    WhiteboardClear,
```

- [ ] **Step 2: Add core-type tests**

Append to `crates/core-types/src/live_room.rs` test module:

```rust
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
            },
        };
        let s = serde_json::to_string(&evt).unwrap();
        assert!(s.contains("\"type\":\"whiteboard_stroke\""));
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
```

- [ ] **Step 3: Run the shared type tests**

Run:

```powershell
cargo test -p core-types whiteboard_
```

Expected: pass.

- [ ] **Step 4: Mirror backend broker variants**

In `crates/backend/src/services/live_room.rs`, add:

```rust
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum WhiteboardTool {
    Pen,
    Eraser,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct WhiteboardPoint {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct WhiteboardStroke {
    pub id: String,
    pub points: Vec<WhiteboardPoint>,
    pub color: String,
    pub width: f32,
    pub tool: WhiteboardTool,
}
```

Place them after `HandRaiseEntry`. Then add `BrokerEvent` variants:

```rust
    WhiteboardStroke {
        stroke: WhiteboardStroke,
    },
    WhiteboardClear,
```

- [ ] **Step 5: Add backend validation helper**

In `crates/backend/src/services/live_room.rs`, add after `matches_wildcard_path`:

```rust
pub fn validate_whiteboard_stroke(stroke: &WhiteboardStroke) -> Result<(), &'static str> {
    if stroke.id.trim().is_empty() || stroke.id.len() > 96 {
        return Err("stroke id is invalid");
    }
    if stroke.points.len() < 2 {
        return Err("stroke must contain at least two points");
    }
    if stroke.points.len() > 512 {
        return Err("stroke contains too many points");
    }
    if stroke.color.len() > 32 || !stroke.color.starts_with('#') {
        return Err("stroke color is invalid");
    }
    if !stroke.width.is_finite() || stroke.width < 1.0 || stroke.width > 32.0 {
        return Err("stroke width is invalid");
    }
    for p in &stroke.points {
        if !p.x.is_finite() || !p.y.is_finite() {
            return Err("stroke point is invalid");
        }
        if !(0.0..=1.0).contains(&p.x) || !(0.0..=1.0).contains(&p.y) {
            return Err("stroke point is outside the board");
        }
    }
    Ok(())
}
```

- [ ] **Step 6: Add backend unit tests**

Append to the `tests` module in `crates/backend/src/services/live_room.rs`:

```rust
    fn sample_stroke() -> WhiteboardStroke {
        WhiteboardStroke {
            id: "stroke-1".into(),
            points: vec![
                WhiteboardPoint { x: 0.1, y: 0.2 },
                WhiteboardPoint { x: 0.3, y: 0.4 },
            ],
            color: "#111827".into(),
            width: 4.0,
            tool: WhiteboardTool::Pen,
        }
    }

    #[test]
    fn broker_event_whiteboard_stroke_round_trips_json() {
        let evt = BrokerEvent::WhiteboardStroke {
            stroke: sample_stroke(),
        };
        let s = serde_json::to_string(&evt).unwrap();
        assert!(s.contains("\"type\":\"whiteboard_stroke\""));
        let d: BrokerEvent = serde_json::from_str(&s).unwrap();
        match d {
            BrokerEvent::WhiteboardStroke { stroke } => {
                assert_eq!(stroke.id, "stroke-1");
                assert_eq!(stroke.points.len(), 2);
            }
            other => panic!("wrong event: {other:?}"),
        }
    }

    #[test]
    fn broker_event_whiteboard_clear_round_trips_json() {
        let evt = BrokerEvent::WhiteboardClear;
        let s = serde_json::to_string(&evt).unwrap();
        assert_eq!(s, "{\"type\":\"whiteboard_clear\"}");
        let d: BrokerEvent = serde_json::from_str(&s).unwrap();
        assert!(matches!(d, BrokerEvent::WhiteboardClear));
    }

    #[test]
    fn validate_whiteboard_stroke_accepts_normal_stroke() {
        assert_eq!(validate_whiteboard_stroke(&sample_stroke()), Ok(()));
    }

    #[test]
    fn validate_whiteboard_stroke_rejects_oversized_points() {
        let mut stroke = sample_stroke();
        stroke.points = (0..513)
            .map(|i| WhiteboardPoint {
                x: (i as f32 % 100.0) / 100.0,
                y: 0.5,
            })
            .collect();
        assert_eq!(
            validate_whiteboard_stroke(&stroke),
            Err("stroke contains too many points")
        );
    }
```

- [ ] **Step 7: Mirror frontend socket event variants**

In `crates/features-courses/src/live_room_socket.rs`, add these structs above `ServerEvent`:

```rust
#[derive(Debug, Deserialize, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WhiteboardTool {
    Pen,
    Eraser,
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
}
```

Then add these variants to `ServerEvent`:

```rust
    WhiteboardStroke {
        stroke: WhiteboardStroke,
    },
    WhiteboardClear,
```

- [ ] **Step 8: Add frontend parse tests**

Append to `crates/features-courses/src/live_room_socket.rs` tests:

```rust
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
    fn parses_whiteboard_clear_event() {
        match parse_event(r#"{"type":"whiteboard_clear"}"#).unwrap() {
            ServerEvent::WhiteboardClear => {}
            other => panic!("wrong variant: {other:?}"),
        }
    }
```

- [ ] **Step 9: Verify and commit**

Run:

```powershell
cargo test -p core-types whiteboard_
cargo test -p backend live_room::tests::broker_event_whiteboard
cargo test -p features-courses live_room_socket::tests::parses_whiteboard
```

Expected: all pass.

Commit:

```powershell
git add -- crates/core-types/src/live_room.rs crates/backend/src/services/live_room.rs crates/features-courses/src/live_room_socket.rs
git commit -m "feat(live-room): add whiteboard socket event types"
```

---

## Task 3: Backend Whiteboard Command Handling

**Files:**
- Modify: `crates/backend/src/handlers/live_sessions.rs`
- Test: `crates/backend/tests/live_room.rs`

- [ ] **Step 1: Add client envelope variants**

In `crates/backend/src/handlers/live_sessions.rs`, extend `ClientEnvelope`:

```rust
    WhiteboardStroke {
        stroke: crate::services::live_room::WhiteboardStroke,
    },
    WhiteboardClear,
```

- [ ] **Step 2: Add command handling**

In `handle_client_envelope`, add these match arms before `ClientEnvelope::Kick`:

```rust
        ClientEnvelope::WhiteboardStroke { stroke } => {
            if !is_teacher {
                send_command_failed(
                    sender,
                    "whiteboard_stroke",
                    &ApiError::Forbidden.user_facing(),
                )
                .await;
                return;
            }
            if let Err(reason) = crate::services::live_room::validate_whiteboard_stroke(&stroke) {
                send_command_failed(sender, "whiteboard_stroke", reason).await;
                return;
            }
            let _ = broker
                .publish(session_id, BrokerEvent::WhiteboardStroke { stroke })
                .await;
        }
        ClientEnvelope::WhiteboardClear => {
            if !is_teacher {
                send_command_failed(
                    sender,
                    "whiteboard_clear",
                    &ApiError::Forbidden.user_facing(),
                )
                .await;
                return;
            }
            let _ = broker.publish(session_id, BrokerEvent::WhiteboardClear).await;
        }
```

- [ ] **Step 3: Add backend integration tests**

Append this helper and three tests to `crates/backend/tests/live_room.rs` near the existing socket tests:

```rust
struct WhiteboardSocketFixture {
    session: uuid::Uuid,
    teacher_ws: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    student_ws: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
}

async fn whiteboard_socket_fixture() -> WhiteboardSocketFixture {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, fb_t, em_t) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (student, fb_s, em_s) = create_user(&pool).await;
    attach_membership(&pool, tenant, student, "student").await;
    let (course, session) =
        course_with_session(&pool, tenant, teacher, chrono::Utc::now()).await;
    sqlx::query(
        "INSERT INTO course_memberships (course_id, user_id, tenant_id, role)
         VALUES ($1, $2, $3, 'student')",
    )
    .bind(course)
    .bind(student)
    .bind(tenant)
    .execute(&pool)
    .await
    .unwrap();

    let mediamtx = Arc::new(backend::services::mediamtx::MockMediaMtxClient::new());
    let signer = Arc::new(backend::services::mediamtx::JwtSigner::new_ephemeral());
    let shared_broker: Arc<dyn backend::services::live_room::LiveRoomBroker> =
        Arc::new(backend::services::live_room::MockLiveRoomBroker::new());

    let teacher_router = backend::handlers::live_sessions::live_room_router_for_tests_with_broker(
        pool.clone(),
        mediamtx.clone(),
        signer.clone(),
        "http://localhost:8889".into(),
        "http://localhost:8888".into(),
        shared_broker.clone(),
    );
    let student_router = backend::handlers::live_sessions::live_room_router_for_tests_with_broker(
        pool.clone(),
        mediamtx,
        signer,
        "http://localhost:8889".into(),
        "http://localhost:8888".into(),
        shared_broker,
    );

    let teacher_addr = bind_test_server(
        teacher_router,
        StubAuth {
            pool: pool.clone(),
            user_id: teacher,
            firebase_uid: fb_t,
            email: em_t,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Teacher),
        },
    )
    .await;
    let student_addr = bind_test_server(
        student_router,
        StubAuth {
            pool: pool.clone(),
            user_id: student,
            firebase_uid: fb_s,
            email: em_s,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Student),
        },
    )
    .await;

    let (teacher_ws, _) = tokio_tungstenite::connect_async(format!(
        "ws://{teacher_addr}/v1/sessions/{session}/socket"
    ))
    .await
    .unwrap();
    let (student_ws, _) = tokio_tungstenite::connect_async(format!(
        "ws://{student_addr}/v1/sessions/{session}/socket"
    ))
    .await
    .unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    WhiteboardSocketFixture {
        session,
        teacher_ws,
        student_ws,
    }
}

async fn next_ws_event_type(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    expected_type: &str,
) -> serde_json::Value {
    for _ in 0..20 {
        match tokio::time::timeout(std::time::Duration::from_secs(2), ws.next()).await {
            Ok(Some(Ok(tokio_tungstenite::tungstenite::Message::Text(t)))) => {
                let value: serde_json::Value = serde_json::from_str(&t).unwrap();
                if value["type"] == expected_type {
                    return value;
                }
            }
            _ => continue,
        }
    }
    panic!("did not receive expected websocket event type {expected_type}");
}

#[tokio::test]
async fn teacher_whiteboard_stroke_broadcasts() {
    let mut f = whiteboard_socket_fixture().await;
    f.teacher_ws
        .send(tokio_tungstenite::tungstenite::Message::Text(
            r##"{"type":"whiteboard_stroke","stroke":{"id":"s1","points":[{"x":0.1,"y":0.2},{"x":0.3,"y":0.4}],"color":"#111827","width":4.0,"tool":"pen"}}"##.to_string(),
        ))
        .await
        .unwrap();

    let msg = next_ws_event_type(&mut f.student_ws, "whiteboard_stroke").await;
    assert_eq!(msg["stroke"]["id"], "s1");
}

#[tokio::test]
async fn student_whiteboard_stroke_gets_command_failed() {
    let mut f = whiteboard_socket_fixture().await;
    f.student_ws
        .send(tokio_tungstenite::tungstenite::Message::Text(
            r##"{"type":"whiteboard_stroke","stroke":{"id":"s1","points":[{"x":0.1,"y":0.2},{"x":0.3,"y":0.4}],"color":"#111827","width":4.0,"tool":"pen"}}"##.to_string(),
        ))
        .await
        .unwrap();

    let msg = next_ws_event_type(&mut f.student_ws, "command_failed").await;
    assert_eq!(msg["command"], "whiteboard_stroke");
}

#[tokio::test]
async fn teacher_whiteboard_clear_broadcasts() {
    let mut f = whiteboard_socket_fixture().await;
    f.teacher_ws
        .send(tokio_tungstenite::tungstenite::Message::Text(
            r#"{"type":"whiteboard_clear"}"#.to_string(),
        ))
        .await
        .unwrap();

    let msg = next_ws_event_type(&mut f.student_ws, "whiteboard_clear").await;
    assert_eq!(msg["type"], "whiteboard_clear");
}
```

- [ ] **Step 4: Run backend tests**

Run:

```powershell
cargo test -p backend --test live_room whiteboard
```

Expected: the three whiteboard tests pass.

- [ ] **Step 5: Commit**

```powershell
git add -- crates/backend/src/handlers/live_sessions.rs crates/backend/tests/live_room.rs
git commit -m "feat(live-room): authorize and broadcast whiteboard commands"
```

---

## Task 4: Frontend Whiteboard Module

**Files:**
- Create: `crates/features-courses/src/live_room_whiteboard.rs`
- Modify: `crates/features-courses/src/lib.rs`
- Test: `crates/features-courses/tests/live_room_smoke.rs`

- [ ] **Step 1: Add module export**

In `crates/features-courses/src/lib.rs`, add:

```rust
pub mod live_room_whiteboard;
```

- [ ] **Step 2: Create whiteboard reducer and component**

Create `crates/features-courses/src/live_room_whiteboard.rs`:

```rust
use dioxus::prelude::*;

#[derive(Clone, PartialEq, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WhiteboardTool {
    Pen,
    Eraser,
}

impl Default for WhiteboardTool {
    fn default() -> Self {
        Self::Pen
    }
}

#[derive(Clone, PartialEq, Debug, serde::Serialize, serde::Deserialize)]
pub struct WhiteboardPoint {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, PartialEq, Debug, serde::Serialize, serde::Deserialize)]
pub struct WhiteboardStroke {
    pub id: String,
    pub points: Vec<WhiteboardPoint>,
    pub color: String,
    pub width: f32,
    pub tool: WhiteboardTool,
}

#[derive(Clone, PartialEq, Debug)]
pub struct WhiteboardState {
    pub strokes: Vec<WhiteboardStroke>,
    pub active_stroke: Option<WhiteboardStroke>,
    pub tool: WhiteboardTool,
    pub color: String,
    pub width: f32,
}

impl Default for WhiteboardState {
    fn default() -> Self {
        Self {
            strokes: Vec::new(),
            active_stroke: None,
            tool: WhiteboardTool::Pen,
            color: "#111827".into(),
            width: 4.0,
        }
    }
}

pub const MAX_STROKES: usize = 256;
pub const MAX_POINTS_PER_STROKE: usize = 512;

pub fn append_stroke(state: &mut WhiteboardState, stroke: WhiteboardStroke) -> Result<(), String> {
    validate_stroke(&stroke)?;
    if state.strokes.len() >= MAX_STROKES {
        return Err("clear the board before adding more strokes".into());
    }
    state.strokes.push(stroke);
    Ok(())
}

pub fn clear_board(state: &mut WhiteboardState) {
    state.strokes.clear();
    state.active_stroke = None;
}

pub fn validate_stroke(stroke: &WhiteboardStroke) -> Result<(), String> {
    if stroke.id.trim().is_empty() || stroke.id.len() > 96 {
        return Err("stroke id is invalid".into());
    }
    if stroke.points.len() < 2 {
        return Err("stroke must contain at least two points".into());
    }
    if stroke.points.len() > MAX_POINTS_PER_STROKE {
        return Err("stroke contains too many points".into());
    }
    if stroke.color.len() > 32 || !stroke.color.starts_with('#') {
        return Err("stroke color is invalid".into());
    }
    if !stroke.width.is_finite() || stroke.width < 1.0 || stroke.width > 32.0 {
        return Err("stroke width is invalid".into());
    }
    for point in &stroke.points {
        if !point.x.is_finite() || !point.y.is_finite() {
            return Err("stroke point is invalid".into());
        }
        if !(0.0..=1.0).contains(&point.x) || !(0.0..=1.0).contains(&point.y) {
            return Err("stroke point is outside the board".into());
        }
    }
    Ok(())
}

fn points_attr(stroke: &WhiteboardStroke) -> String {
    stroke
        .points
        .iter()
        .map(|p| format!("{:.3},{:.3}", p.x * 1000.0, p.y * 600.0))
        .collect::<Vec<_>>()
        .join(" ")
}

fn event_point(evt: &Event<MouseData>) -> WhiteboardPoint {
    let element = evt.data().element_coordinates();
    let x = (element.x / 1000.0).clamp(0.0, 1.0) as f32;
    let y = (element.y / 600.0).clamp(0.0, 1.0) as f32;
    WhiteboardPoint { x, y }
}

#[derive(Props, Clone, PartialEq)]
pub struct LiveRoomWhiteboardProps {
    pub state: WhiteboardState,
    pub is_teacher: bool,
    #[props(default)]
    pub on_emit_stroke: EventHandler<WhiteboardStroke>,
    #[props(default)]
    pub on_clear: EventHandler<()>,
}

pub fn LiveRoomWhiteboard(props: LiveRoomWhiteboardProps) -> Element {
    let mut board = use_signal(|| props.state.clone());
    let mut rendered = board.read().strokes.clone();
    if let Some(active) = board.read().active_stroke.clone() {
        rendered.push(active);
    }

    let start_draw = {
        let is_teacher = props.is_teacher;
        move |evt: Event<MouseData>| {
            if !is_teacher {
                return;
            }
            let mut state = board.write();
            state.active_stroke = Some(WhiteboardStroke {
                id: format!("stroke-{}", uuid::Uuid::new_v4().simple()),
                points: vec![event_point(&evt)],
                color: state.color.clone(),
                width: state.width,
                tool: state.tool.clone(),
            });
        }
    };

    let move_draw = {
        let is_teacher = props.is_teacher;
        move |evt: Event<MouseData>| {
            if !is_teacher {
                return;
            }
            let mut state = board.write();
            if let Some(stroke) = state.active_stroke.as_mut() {
                if stroke.points.len() < MAX_POINTS_PER_STROKE {
                    stroke.points.push(event_point(&evt));
                }
            }
        }
    };

    let finish_draw = {
        let is_teacher = props.is_teacher;
        let on_emit = props.on_emit_stroke;
        move |evt: Event<MouseData>| {
            if !is_teacher {
                return;
            }
            let mut state = board.write();
            if let Some(mut stroke) = state.active_stroke.take() {
                stroke.points.push(event_point(&evt));
                if append_stroke(&mut state, stroke.clone()).is_ok() {
                    on_emit.call(stroke);
                }
            }
        }
    };

    let cancel_draw = {
        let is_teacher = props.is_teacher;
        move |_| {
            if is_teacher {
                board.write().active_stroke = None;
            }
        }
    };

    rsx! {
        section { class: "live-room-whiteboard live-panel",
            div { class: "live-panel-header",
                h3 { class: "live-panel-title", "Whiteboard" }
                if props.is_teacher {
                    div { class: "whiteboard-toolbar",
                        button {
                            class: "ds-button ds-button--secondary whiteboard-tool",
                            r#type: "button",
                            onclick: move |_| {
                                clear_board(&mut board.write());
                                props.on_clear.call(());
                            },
                            "Clear"
                        }
                    }
                }
            }
            svg {
                class: "whiteboard-canvas",
                view_box: "0 0 1000 600",
                role: "img",
                aria_label: "Live whiteboard",
                onmousedown: start_draw,
                onmousemove: move_draw,
                onmouseup: finish_draw,
                onmouseleave: cancel_draw,
                for stroke in rendered.iter() {
                    polyline {
                        key: "{stroke.id}",
                        points: "{points_attr(stroke)}",
                        fill: "none",
                        stroke: "{stroke.color}",
                        stroke_width: "{stroke.width}",
                        stroke_linecap: "round",
                        stroke_linejoin: "round",
                        opacity: if stroke.tool == WhiteboardTool::Eraser { "0.35" } else { "1" },
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stroke(id: &str) -> WhiteboardStroke {
        WhiteboardStroke {
            id: id.into(),
            points: vec![
                WhiteboardPoint { x: 0.1, y: 0.2 },
                WhiteboardPoint { x: 0.3, y: 0.4 },
            ],
            color: "#111827".into(),
            width: 4.0,
            tool: WhiteboardTool::Pen,
        }
    }

    #[test]
    fn reducer_appends_and_clears_strokes() {
        let mut state = WhiteboardState::default();
        append_stroke(&mut state, stroke("s1")).unwrap();
        assert_eq!(state.strokes.len(), 1);
        clear_board(&mut state);
        assert!(state.strokes.is_empty());
    }

    #[test]
    fn reducer_rejects_single_point_stroke() {
        let mut bad = stroke("s1");
        bad.points.truncate(1);
        assert_eq!(
            validate_stroke(&bad),
            Err("stroke must contain at least two points".into())
        );
    }
}
```

- [ ] **Step 3: Add SSR tests**

Append to `crates/features-courses/tests/live_room_smoke.rs`:

```rust
use features_courses::live_room_whiteboard::{
    LiveRoomWhiteboard, LiveRoomWhiteboardProps, WhiteboardPoint, WhiteboardState,
    WhiteboardStroke, WhiteboardTool,
};

#[test]
fn whiteboard_renders_seeded_strokes() {
    fn app() -> Element {
        let state = WhiteboardState {
            strokes: vec![WhiteboardStroke {
                id: "s1".into(),
                points: vec![
                    WhiteboardPoint { x: 0.1, y: 0.2 },
                    WhiteboardPoint { x: 0.3, y: 0.4 },
                ],
                color: "#111827".into(),
                width: 4.0,
                tool: WhiteboardTool::Pen,
            }],
            ..WhiteboardState::default()
        };
        rsx! { LiveRoomWhiteboard { state: state, is_teacher: false } }
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
        let state = WhiteboardState::default();
        rsx! { LiveRoomWhiteboard { state: state, is_teacher: true } }
    }
    fn student_app() -> Element {
        let state = WhiteboardState::default();
        rsx! { LiveRoomWhiteboard { state: state, is_teacher: false } }
    }
    let mut teacher = VirtualDom::new(teacher_app);
    teacher.rebuild_in_place();
    let teacher_html = dioxus_ssr::render(&teacher);
    assert!(teacher_html.contains("Clear"), "got: {teacher_html}");

    let mut student = VirtualDom::new(student_app);
    student.rebuild_in_place();
    let student_html = dioxus_ssr::render(&student);
    assert!(!student_html.contains("Clear"), "got: {student_html}");
}
```

- [ ] **Step 4: Verify and commit**

Run:

```powershell
cargo test -p features-courses live_room_whiteboard
cargo test -p features-courses --test live_room_smoke whiteboard
```

Expected: pass.

Commit:

```powershell
git add -- crates/features-courses/src/lib.rs crates/features-courses/src/live_room_whiteboard.rs crates/features-courses/tests/live_room_smoke.rs
git commit -m "feat(live-room): add ephemeral whiteboard component"
```

---

## Task 5: LiveRoomSession Screen Ownership And Viewer JWT Fix

**Files:**
- Modify: `crates/features-courses/src/live_room_session.rs`

- [ ] **Step 1: Extend session config**

In `SessionConfig`, add:

```rust
    pub viewer_jwt: Option<String>,
```

Update `dummy_config()` in the same file tests:

```rust
            viewer_jwt: Some("viewer.jwt".into()),
```

Update `crates/shell-web/src/routes/live_session.rs` where `SessionConfig` is built:

```rust
        viewer_jwt: viewer_jwt.clone(),
```

- [ ] **Step 2: Add screen fields**

In `LiveRoomSession`, add wasm-only fields:

```rust
    #[cfg(target_arch = "wasm32")]
    screen_publisher: Option<crate::live_room_whip::WhipPublisher>,
    #[cfg(target_arch = "wasm32")]
    screen_viewer: Option<crate::live_room_whep::WhepViewer>,
```

Initialize them in `new`:

```rust
            #[cfg(target_arch = "wasm32")]
            screen_publisher: None,
            #[cfg(target_arch = "wasm32")]
            screen_viewer: None,
```

- [ ] **Step 3: Add host-visible URL helper**

Add this pure helper near `build_ws_url`:

```rust
pub fn whep_url_for_path(api_origin: &str, path: &str) -> String {
    format!(
        "{}/{}/whep",
        api_origin.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}
```

Add tests:

```rust
    #[test]
    fn whep_url_for_path_trims_duplicate_slashes() {
        assert_eq!(
            whep_url_for_path("http://localhost:8889/", "/aula/t/c/s/student/u"),
            "http://localhost:8889/aula/t/c/s/student/u/whep"
        );
    }
```

- [ ] **Step 4: Change `attach_student` to use viewer JWT**

Replace the wasm `attach_student` body with:

```rust
    pub async fn attach_student(
        &mut self,
        user_id: Uuid,
        path: &str,
        viewer_jwt: &str,
    ) -> Result<(), String> {
        let url = whep_url_for_path(&self.config.api_origin, path);
        let viewer = crate::live_room_whep::view(&url, viewer_jwt).await?;
        if let Some(mut prev) = self.students.insert(user_id, viewer) {
            let _ = prev.close().await;
        }
        Ok(())
    }
```

Replace the non-wasm signature with:

```rust
    pub async fn attach_student(
        &mut self,
        _user_id: Uuid,
        _path: &str,
        _viewer_jwt: &str,
    ) -> Result<(), String> {
        Ok(())
    }
```

- [ ] **Step 5: Add screen attach and ownership methods**

Add after `main_remote_stream`:

```rust
    #[cfg(target_arch = "wasm32")]
    pub async fn attach_screen(&mut self, url: &str, viewer_jwt: &str) -> Result<(), String> {
        let viewer = crate::live_room_whep::view(url, viewer_jwt).await?;
        if let Some(mut prev) = self.screen_viewer.take() {
            let _ = prev.close().await;
        }
        self.screen_viewer = Some(viewer);
        Ok(())
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub async fn attach_screen(&mut self, _url: &str, _viewer_jwt: &str) -> Result<(), String> {
        Ok(())
    }

    #[cfg(target_arch = "wasm32")]
    pub fn screen_remote_stream(&self) -> Option<web_sys::MediaStream> {
        self.screen_viewer.as_ref().map(|v| v.remote_stream.clone())
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
    pub async fn stop_screen_share(&mut self) {}
```

- [ ] **Step 6: Update cleanup paths**

In `close`, after publisher cleanup, add:

```rust
            if let Some(mut p) = self.screen_publisher.take() {
                let _ = p.close().await;
            }
            if let Some(mut v) = self.screen_viewer.take() {
                let _ = v.close().await;
            }
```

In `Drop`, take the new fields and close them:

```rust
            let screen_publisher = self.screen_publisher.take();
            let screen_viewer = self.screen_viewer.take();
```

Inside the spawned async block:

```rust
                if let Some(mut p) = screen_publisher {
                    let _ = p.close().await;
                }
                if let Some(mut v) = screen_viewer {
                    let _ = v.close().await;
                }
```

- [ ] **Step 7: Update `attach_student` call site**

In `crates/features-courses/src/live_room_view.rs`, replace:

```rust
let _ = session_sig.write().attach_student(parsed, &path).await;
```

with:

```rust
let viewer_jwt = viewer_jwt_for_student.clone();
let _ = session_sig
    .write()
    .attach_student(parsed, &path, &viewer_jwt)
    .await;
```

Capture `viewer_jwt_for_student` in `use_persistent_socket` by adding a parameter `viewer_jwt: String` and passing `props.viewer_jwt.clone().unwrap_or_default()` from `LiveRoomView`.

- [ ] **Step 8: Verify and commit**

Run:

```powershell
cargo test -p features-courses live_room_session
cargo check -p features-courses
cargo check -p shell-web
```

Expected: pass.

Commit:

```powershell
git add -- crates/features-courses/src/live_room_session.rs crates/features-courses/src/live_room_view.rs crates/shell-web/src/routes/live_session.rs
git commit -m "fix(live-room): use viewer jwt for all WHEP viewers"
```

---

## Task 6: Teacher Screen Share Controls

**Files:**
- Modify: `crates/features-courses/src/live_room_broadcast.rs`
- Test: `crates/features-courses/tests/live_room_smoke.rs`

- [ ] **Step 1: Deserialize `screen_publish_url`**

In `go_live_flow`'s local `GoLiveResp`, add:

```rust
        screen_publish_url: String,
```

- [ ] **Step 2: Add screen stream signal**

In `LiveRoomBroadcast`, after `self_stream`, add:

```rust
    #[cfg(target_arch = "wasm32")]
    let screen_stream: Signal<Option<web_sys::MediaStream>> = use_signal(|| None);
```

- [ ] **Step 3: Add screen preview effect**

After the self-preview effect, add:

```rust
    #[cfg(target_arch = "wasm32")]
    {
        let stream_eff = screen_stream;
        use_effect(move || {
            let stream_opt = stream_eff.read().clone();
            if let Some(stream) = stream_opt {
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
```

- [ ] **Step 4: Add start/stop screen handlers**

Add these helper functions near `go_live_flow`:

```rust
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
    let mut constraints = web_sys::DisplayMediaStreamConstraints::new();
    constraints.video(&wasm_bindgen::JsValue::TRUE);
    constraints.audio(&wasm_bindgen::JsValue::FALSE);
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

    screen_stream.set(Some(stream.clone()));
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
```

- [ ] **Step 5: Wire buttons in live state**

Inside `PublishState::Live`, after the self-preview block, add:

```rust
                        if screen_active {
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
                            Button {
                                label: "Stop sharing".to_string(),
                                variant: ButtonVariant::Secondary,
                                on_click: {
                                    let session = session;
                                    let screen_stream = screen_stream;
                                    move |_| {
                                        let mut state_for_async = state;
                                        #[cfg(target_arch = "wasm32")]
                                        wasm_bindgen_futures::spawn_local(async move {
                                            stop_screen_share_flow(session, screen_stream).await;
                                            state_for_async.set(PublishState::Live {
                                                main_active: true,
                                                screen_active: false,
                                            });
                                        });
                                    }
                                },
                            }
                        } else {
                            Button {
                                label: "Share screen".to_string(),
                                variant: ButtonVariant::Secondary,
                                on_click: {
                                    let session_id = session_id.clone();
                                    let session = session;
                                    let screen_stream = screen_stream;
                                    move |_| {
                                        let mut state_for_async = state;
                                        #[cfg(target_arch = "wasm32")]
                                        wasm_bindgen_futures::spawn_local(async move {
                                            match start_screen_share_flow(&session_id, session, screen_stream).await {
                                                Ok(()) => state_for_async.set(PublishState::Live {
                                                    main_active: true,
                                                    screen_active: true,
                                                }),
                                                Err(e) => state_for_async.set(PublishState::Error(e)),
                                            }
                                        });
                                    }
                                },
                            }
                        }
```

- [ ] **Step 6: Render teacher whiteboard and send commands**

In `LiveRoomBroadcast`, after `sidebar_state`, add:

```rust
    let whiteboard_state = use_signal(crate::live_room_whiteboard::WhiteboardState::default);
```

After `on_kick`, add:

```rust
    let on_whiteboard_stroke = {
        #[cfg(target_arch = "wasm32")]
        let socket = socket.clone();
        move |stroke: crate::live_room_whiteboard::WhiteboardStroke| {
            #[cfg(target_arch = "wasm32")]
            {
                let payload = serde_json::json!({
                    "type": "whiteboard_stroke",
                    "stroke": stroke,
                })
                .to_string();
                if let Some(s) = socket.borrow().as_ref() {
                    let _ = s.send_text(&payload);
                }
            }
            #[cfg(not(target_arch = "wasm32"))]
            let _ = stroke;
        }
    };

    let on_whiteboard_clear = {
        #[cfg(target_arch = "wasm32")]
        let socket = socket.clone();
        move |_| {
            #[cfg(target_arch = "wasm32")]
            {
                if let Some(s) = socket.borrow().as_ref() {
                    let _ = s.send_text(r#"{"type":"whiteboard_clear"}"#);
                }
            }
        }
    };
```

Render the teacher whiteboard after `live-room-broadcast-controls` and before `live-room-sidebars`:

```rust
            crate::live_room_whiteboard::LiveRoomWhiteboard {
                state: whiteboard_state.read().clone(),
                is_teacher: true,
                on_emit_stroke: EventHandler::new(on_whiteboard_stroke),
                on_clear: EventHandler::new(on_whiteboard_clear),
            }
```

- [ ] **Step 7: Add SSR test**

In `crates/features-courses/tests/live_room_smoke.rs`, append:

```rust
#[test]
fn teacher_live_renders_share_screen_control() {
    let html = render_shell(CallerRole::Teacher, SessionStatus::Live);
    assert!(html.contains("Share screen"), "got: {html}");
}
```

- [ ] **Step 8: Verify and commit**

Run:

```powershell
cargo test -p features-courses --test live_room_smoke teacher_live_renders_share_screen_control
cargo check -p features-courses
```

Expected: pass.

Commit:

```powershell
git add -- crates/features-courses/src/live_room_broadcast.rs crates/features-courses/tests/live_room_smoke.rs
git commit -m "feat(live-room): add teacher screen share controls"
```

---

## Task 7: Student Stage Screen Share And Whiteboard

**Files:**
- Modify: `crates/features-courses/src/live_room_view.rs`
- Test: `crates/features-courses/tests/live_room_smoke.rs`

- [ ] **Step 1: Extend view state**

In `LiveRoomState`, add:

```rust
    pub whiteboard: crate::live_room_whiteboard::WhiteboardState,
```

Because `WhiteboardState` implements `Default`, the existing derive remains valid.

- [ ] **Step 2: Handle whiteboard socket events**

In `apply_event_to_state`, add arms before `CommandFailed`:

```rust
        ServerEvent::WhiteboardStroke { stroke } => {
            let mapped = crate::live_room_whiteboard::WhiteboardStroke {
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
            };
            if let Err(e) = crate::live_room_whiteboard::append_stroke(&mut state.whiteboard, mapped)
            {
                state.command_error = Some(format!("whiteboard_stroke: {e}"));
            }
        }
        ServerEvent::WhiteboardClear => {
            crate::live_room_whiteboard::clear_board(&mut state.whiteboard);
        }
```

- [ ] **Step 3: Send teacher whiteboard commands**

In `LiveRoomView`, create whiteboard handlers after hand-raise handlers:

```rust
    let on_whiteboard_stroke = {
        #[cfg(target_arch = "wasm32")]
        let socket = socket.clone();
        move |stroke: crate::live_room_whiteboard::WhiteboardStroke| {
            #[cfg(target_arch = "wasm32")]
            {
                let payload = serde_json::json!({
                    "type": "whiteboard_stroke",
                    "stroke": stroke,
                })
                .to_string();
                if let Some(s) = socket.borrow().as_ref() {
                    let _ = s.send_text(&payload);
                }
            }
            #[cfg(not(target_arch = "wasm32"))]
            let _ = stroke;
        }
    };

    let on_whiteboard_clear = {
        #[cfg(target_arch = "wasm32")]
        let socket = socket.clone();
        move |_| {
            #[cfg(target_arch = "wasm32")]
            {
                if let Some(s) = socket.borrow().as_ref() {
                    let _ = s.send_text(r#"{"type":"whiteboard_clear"}"#);
                }
            }
        }
    };
```

- [ ] **Step 4: Render whiteboard below stage**

After the stage div and before `LiveRoomSidebars`, render:

```rust
            crate::live_room_whiteboard::LiveRoomWhiteboard {
                state: state.read().whiteboard.clone(),
                is_teacher: false,
                on_emit_stroke: EventHandler::new(on_whiteboard_stroke),
                on_clear: EventHandler::new(on_whiteboard_clear),
            }
```

- [ ] **Step 5: Add screen video element**

In `render_webrtc`, read `screen_url`:

```rust
    let screen_url = props.screen_url.clone().unwrap_or_default();
```

Inside the wasm effect, after main attach succeeds, add:

```rust
                if !screen_url.is_empty() {
                    if let Some(mut session_sig) = session {
                        match session_sig.write().attach_screen(&screen_url, &viewer_jwt).await {
                            Ok(()) => {
                                let stream = session_sig.read().screen_remote_stream();
                                if let (Some(stream), Some(doc)) = (
                                    stream,
                                    web_sys::window().and_then(|w| w.document()),
                                ) {
                                    if let Some(el) = doc.get_element_by_id("live-room-screen-video") {
                                        use wasm_bindgen::JsCast;
                                        if let Ok(media_el) = el.dyn_into::<web_sys::HtmlMediaElement>() {
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
                }
```

Update the returned RSX from one video to:

```rust
    rsx! {
        div { class: "live-room-video-grid",
            video {
                id: "live-room-main-video",
                autoplay: true,
                playsinline: true,
                controls: true,
                class: "live-video-main",
            }
            video {
                id: "live-room-screen-video",
                autoplay: true,
                playsinline: true,
                controls: true,
                class: "live-video-screen",
            }
        }
    }
```

- [ ] **Step 6: Add SSR tests**

Append:

```rust
#[test]
fn student_live_renders_screen_and_whiteboard_surfaces() {
    let html = render_shell(CallerRole::Student, SessionStatus::Live);
    assert!(html.contains("live-room-main-video"), "got: {html}");
    assert!(html.contains("live-room-screen-video"), "got: {html}");
    assert!(html.contains("live-room-whiteboard"), "got: {html}");
}
```

- [ ] **Step 7: Verify and commit**

Run:

```powershell
cargo test -p features-courses --test live_room_smoke student_live_renders_screen_and_whiteboard_surfaces
cargo check -p features-courses
```

Expected: pass.

Commit:

```powershell
git add -- crates/features-courses/src/live_room_view.rs crates/features-courses/tests/live_room_smoke.rs
git commit -m "feat(live-room): render student screen share and whiteboard"
```

---

## Task 8: Styling

**Files:**
- Modify: `crates/design-system/assets/components.css`
- Modify: `crates/shell-web/public/assets/components.css`

- [ ] **Step 1: Add CSS to design-system asset**

Append near existing live-room styles in `crates/design-system/assets/components.css`:

```css
.live-room-video-grid {
  display: grid;
  grid-template-columns: minmax(0, 2fr) minmax(220px, 0.8fr);
  gap: var(--space-3);
  align-items: stretch;
}

.live-video-screen {
  width: 100%;
  min-height: 220px;
  background: var(--color-ink-stage-deep);
  border-radius: var(--radius-md);
}

.broadcast-screen-wrap {
  position: relative;
  margin: var(--space-4) 0;
  border-radius: var(--radius-md);
  overflow: hidden;
  border: 1px solid var(--color-on-ink-subtle);
  box-shadow: var(--shadow-lg);
}

.broadcast-screen-video {
  display: block;
  width: 100%;
  min-height: 260px;
  max-height: 58vh;
  object-fit: contain;
  background: var(--color-ink-stage-deep);
}

.live-room-whiteboard {
  grid-column: 1 / -1;
}

.whiteboard-toolbar {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  flex-wrap: wrap;
}

.whiteboard-canvas {
  display: block;
  width: 100%;
  min-height: 280px;
  aspect-ratio: 5 / 3;
  background: #fffdf8;
  border: 1px solid var(--color-rule);
  border-radius: var(--radius-md);
  touch-action: none;
}

@media (max-width: 900px) {
  .live-room-video-grid {
    grid-template-columns: 1fr;
  }

  .live-video-screen,
  .broadcast-screen-video,
  .whiteboard-canvas {
    min-height: 220px;
  }
}
```

- [ ] **Step 2: Mirror CSS to shell-web public asset**

Apply the same CSS block to `crates/shell-web/public/assets/components.css`.

- [ ] **Step 3: Verify CSS contains expected classes**

Run:

```powershell
rg -n "live-room-video-grid|whiteboard-canvas|broadcast-screen-video" crates/design-system/assets/components.css crates/shell-web/public/assets/components.css
```

Expected: all three classes appear in both files.

- [ ] **Step 4: Commit**

```powershell
git add -- crates/design-system/assets/components.css crates/shell-web/public/assets/components.css
git commit -m "style(live-room): add screen share and whiteboard surfaces"
```

---

## Task 9: Focused Verification

**Files:**
- Inspect only.

- [ ] **Step 1: Format**

Run:

```powershell
cargo fmt --all --check
```

Expected: pass. If it fails, run:

```powershell
cargo fmt --all
```

Then commit only formatting changes:

```powershell
git add -- crates
git commit -m "chore: format live room cycle 2 changes"
```

- [ ] **Step 2: Focused Rust tests**

Run:

```powershell
cargo test -p core-types whiteboard_
cargo test -p backend live_room::tests::broker_event_whiteboard
cargo test -p features-courses live_room_whiteboard
cargo test -p features-courses live_room_socket::tests::parses_whiteboard
cargo test -p features-courses --test live_room_smoke
```

Expected: pass.

- [ ] **Step 3: Compile checks**

Run:

```powershell
cargo check -p backend
cargo check -p features-courses
cargo check -p shell-web
```

Expected: pass.

- [ ] **Step 4: Broader tests when services are available**

Run when Postgres/Redis/MediaMTX are up:

```powershell
cargo test -p backend --test live_room whiteboard
cargo test -p backend --test live_room whep
```

Expected: pass. If a service is unavailable, record the exact failing command and service error.

- [ ] **Step 5: Worktree summary**

Run:

```powershell
git status --short
git log -8 --oneline
```

Expected: task commits are present. Unrelated generated logs may remain untracked; do not include them.

---

## Task 10: Frontend Rebuild And Manual Browser Smoke

**Files:**
- Inspect/deploy only unless a verification failure points to a concrete code fix.

- [ ] **Step 1: Build frontend image**

Run the repo's established frontend Docker build command. On this host, use the same BuildKit secret pattern already used by previous frontend rebuild logs:

```powershell
docker build --secret id=gh_token,env=GH_TOKEN -f crates/shell-web/Dockerfile -t aulalite-shell-web:cycle2 .
```

Expected: image builds successfully. If `GH_TOKEN` is missing, set it in the shell from the user's configured secret source before rerunning.

- [ ] **Step 2: Deploy frontend container**

Run:

```powershell
docker compose up -d frontend
docker compose ps
```

Expected: all containers healthy and frontend serving on port `3000`.

- [ ] **Step 3: Manual browser smoke**

In a hard-refreshed browser session:

1. Teacher opens the live-class page, clicks `Go Live`, and allows camera/mic.
2. Student opens the same class in another browser/incognito and sees teacher feed.
3. Teacher clicks `Share screen` and chooses a screen/window.
4. Student sees the screen alongside teacher camera.
5. Teacher clicks `Stop sharing`; student screen surface stops while camera remains.
6. Teacher draws on the whiteboard.
7. Student sees the whiteboard stroke.
8. Student has no drawing controls.
9. Teacher ends class; camera and screen indicators turn off.

Expected: no WHEP `401` for main, screen, or promoted-student viewers. A not-yet-published screen path may report no stream before the teacher starts sharing; that is acceptable.

- [ ] **Step 4: Capture final result**

Run:

```powershell
git status --short
```

Expected: only unrelated pre-existing logs/screenshots are untracked. Summarize verification results in the final response.
