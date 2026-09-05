# Live Room Cycle 2 - Screen Share + Ephemeral Whiteboard Design

**Date:** 2026-06-01
**Status:** Approved design; implementation plan pending.
**Scope:** Add teacher screen share, fix promoted-student WHEP token usage, and ship a teacher-controlled ephemeral whiteboard inside the existing live-room architecture.

## Goal

Cycle 2 completes the next live-class surface after the video auth fix: teachers can share a screen alongside their camera, and teachers can draw on a pure-Rust whiteboard that students see live. The work stays inside the current Dioxus/Rust/Axum/MediaMTX live-room system and lands in one frontend/backend rebuild.

## Approved Decisions

| Topic | Decision |
| --- | --- |
| Architecture | Extend the existing live-room pipeline and WebSocket broker. |
| Screen share layout | Show screen share alongside camera, with screen as the large stage while active. |
| Whiteboard write access | Teacher/admin controlled only in Cycle 2. Students view live strokes. |
| Whiteboard persistence | Ephemeral only. No saved board or replay sync in Cycle 2. |
| Whiteboard rendering | Rust/Dioxus-managed board state and SVG rendering; no JS drawing library. |

## Out of Scope

- Saved whiteboard snapshots or stroke logs.
- Time-synced whiteboard replay alongside recordings.
- Student drawing permissions or collaborative board moderation.
- New database tables or migrations.
- TURN server work, MediaMTX HA, or broader WebRTC topology changes.
- Full live-room UI redesign beyond the controls and stage layout needed for screen share and whiteboard.

## Architecture

Cycle 2 extends the current live-room implementation instead of introducing a separate media service or whiteboard service.

Media paths remain the existing MediaMTX paths:

```text
aula/<tenant>/<course>/<session>          # teacher camera + mic
aula/<tenant>/<course>/<session>/screen   # teacher screen share
```

The backend already returns `screen_publish_url` from `/go-live` and `screen_url` from `/join`. The missing work is frontend ownership, rendering, retry behavior, and screen lifecycle cleanup.

`LiveRoomSession` becomes the owning aggregate for all route-scoped live media resources:

- main WHIP publisher for teacher camera/mic
- screen WHIP publisher for teacher display media
- main WHEP viewer for students watching the teacher feed
- screen WHEP viewer for students watching the screen feed
- promoted-student WHEP viewers
- existing room socket ownership and cleanup surface

`attach_main` already passes the join-minted `viewer_jwt` to WHEP. `attach_student` must receive the same token treatment instead of using the backend API token. Screen viewing uses a new `attach_screen(screen_url, viewer_jwt)` method that mirrors `attach_main`.

The whiteboard uses the existing live-room WebSocket and broker. The backend validates whiteboard commands and broadcasts accepted whiteboard events to connected clients. Clients keep in-memory board state for the active session.

## Screen Share Design

### Teacher Flow

1. Teacher starts the class as today, publishing camera/mic to the main WHIP path.
2. Teacher clicks `Share screen`.
3. The frontend calls `getDisplayMedia({ video: true, audio: false })`.
4. `LiveRoomSession::start_screen_share(screen_publish_url, publish_password, stream)` opens a second WHIP publisher.
5. The teacher UI shows a local screen preview alongside the camera preview.
6. If the browser display track ends or the teacher clicks `Stop sharing`, the frontend closes the screen publisher and clears the active screen state.

The same publish password returned by `/go-live` is used for the screen WHIP path, matching the existing backend contract.

### Student Flow

1. While the session is live, `/join` returns `main_url`, `screen_url`, and `viewer_jwt`.
2. `LiveRoomView` attaches `main_url` through `attach_main(main_url, viewer_jwt)`.
3. If `screen_url` exists, the view attempts `attach_screen(screen_url, viewer_jwt)`.
4. If the screen path is not publishing yet, the failure is treated as inactive screen share, not as a room failure.
5. When screen share becomes available, students see it as the large stage with teacher camera retained as a smaller tile.

### Screen States

- `inactive`: no screen publisher, no screen stage.
- `starting`: teacher has approved browser capture and WHIP is connecting.
- `active`: screen WHIP is publishing and screen preview is visible.
- `stopping`: publisher close is in progress.
- `error`: capture or publish failed; camera stream remains active.

Students mirror simpler states:

- `not_shared`: no screen currently available.
- `connecting`: trying WHEP on `screen_url`.
- `active`: screen WHEP stream attached.
- `reconnecting`: screen WHEP dropped and retry is scheduled.
- `error`: auth or repeated connection failure, surfaced without breaking main video.

## Whiteboard Design

### Frontend Module

Create `crates/features-courses/src/live_room_whiteboard.rs`.

The module owns:

- board state reducer
- whiteboard props and Dioxus component
- tool state: pen, eraser
- color and stroke width
- pointer event handling
- SVG rendering for committed strokes and in-progress stroke
- serialization structs for socket events if frontend-local types remain mirrored

The whiteboard is pure Rust/WASM at the app layer: stroke state, event handling, validation, serialization, and DOM/SVG output are all Rust code. It does not use a JS drawing library.

### State Model

```rust
pub struct WhiteboardState {
    pub strokes: Vec<WhiteboardStroke>,
    pub active_stroke: Option<WhiteboardStroke>,
    pub tool: WhiteboardTool,
    pub color: String,
    pub width: f32,
}

pub struct WhiteboardStroke {
    pub id: String,
    pub points: Vec<WhiteboardPoint>,
    pub color: String,
    pub width: f32,
    pub tool: WhiteboardTool,
}

pub struct WhiteboardPoint {
    pub x: f32,
    pub y: f32,
}
```

Coordinates are normalized to the board viewport, so strokes resize predictably across teacher and student viewports.

### Socket Events

Additive server event variants:

```json
{ "type": "whiteboard_stroke", "stroke": { ... } }
{ "type": "whiteboard_clear" }
```

Additive client command variants:

```json
{ "type": "whiteboard_stroke", "stroke": { ... } }
{ "type": "whiteboard_clear" }
```

Backend rules:

- Teachers/admins can submit `whiteboard_stroke` and `whiteboard_clear`.
- Students cannot submit whiteboard commands; the server replies with `CommandFailed` and does not broadcast.
- Malformed, empty, or oversized strokes are rejected.
- Accepted events are broadcast through the existing `LiveRoomBroker`.

### UX Behavior

Teacher:

- Sees whiteboard controls: pen/eraser, color, width, clear.
- Draws locally immediately while pointer is down.
- Sends the completed stroke on pointer up.
- Clear resets local state and broadcasts `whiteboard_clear`.

Student:

- Sees the whiteboard surface and received strokes.
- Does not see drawing controls.
- Cannot emit whiteboard commands.

Reconnect behavior is intentionally simple in Cycle 2: because the board is ephemeral and broker-only, a client that reconnects after missing earlier strokes starts from whatever events it receives after reconnect. Saved state replay is deferred.

## Stage Layout

The live room becomes a multi-surface stage:

- main teacher camera remains visible whenever available
- screen share appears alongside camera and becomes the large stage while active
- whiteboard appears as a classroom surface controlled by teacher
- chat, presence, and hand-raise sidebars remain in the right rail

On desktop, the main content area uses a stable layout with a large primary surface and smaller secondary tiles. On mobile, surfaces stack vertically with stable heights so controls and text do not overlap.

## Error Handling

### Screen Share

- Browser denies screen capture: show inline teacher error; camera stream continues.
- Screen WHIP publish fails: stop display tracks immediately and clear screen state.
- Browser-level "Stop sharing": close the screen publisher and update the UI.
- Student screen WHEP returns no stream: treat as inactive and retry quietly.
- Student screen WHEP returns 401/403: surface compact auth error and log it; do not break the main feed.
- Network/ICE failure: show reconnecting state with capped backoff.
- Route exit/end class: close main and screen publishers/viewers.

### Whiteboard

- Unauthorized student write: backend emits `CommandFailed`.
- Oversized stroke: backend rejects; teacher sees a command failure.
- Pointer cancellation: discard the active stroke without broadcasting.
- Excessive in-memory stroke count: client blocks additional strokes and asks teacher to clear the board.

## Testing Plan

### Backend

- `BrokerEvent::WhiteboardStroke` JSON round-trips.
- `BrokerEvent::WhiteboardClear` JSON round-trips.
- Teacher whiteboard stroke broadcasts to subscribers.
- Student whiteboard stroke emits `CommandFailed` and does not broadcast.
- Teacher whiteboard clear broadcasts.
- Oversized or malformed stroke validation rejects cleanly.

### Frontend

- SSR: teacher broadcast renders `Share screen`.
- SSR: student live view renders main, screen, and whiteboard stage containers.
- SSR: whiteboard renders seeded strokes.
- SSR: teacher whiteboard controls render.
- SSR: student whiteboard controls do not render.
- Host tests: whiteboard reducer appends and clears strokes deterministically.
- Host tests or compile checks: `attach_student` and screen attach paths use `viewer_jwt`, not `ApiContext.id_token`.

### Manual Browser Verification

1. Hard-refresh the live-class page to avoid a cached WASM bundle.
2. Teacher starts camera/mic.
3. Student sees teacher feed.
4. Teacher starts screen share.
5. Student sees screen share alongside teacher camera.
6. Teacher stops screen share; student camera feed remains.
7. Teacher draws on whiteboard.
8. Student sees the strokes.
9. Student cannot draw.
10. End class releases camera, screen, and WHEP resources.

## Delivery Sequence

1. Add design spec and implementation plan.
2. Extend shared/backend socket event types for whiteboard.
3. Add screen-share ownership to `LiveRoomSession`.
4. Apply `viewer_jwt` fix to `attach_student`.
5. Add screen share controls and teacher local preview.
6. Add student screen WHEP attachment and stage rendering.
7. Add whiteboard reducer, component, and socket handling.
8. Add tests.
9. Rebuild frontend image and deploy the new bundle.

## Risks And Mitigations

| Risk | Mitigation |
| --- | --- |
| Screen WHEP errors look like fatal video failure. | Treat screen path as optional and keep main feed independent. |
| Screen publisher leaks display track after WHIP failure. | Stop tracks on every failure and close path on route exit. |
| `attach_student` repeats the earlier wrong-token bug. | Change method signature/state so promoted-student WHEP receives `viewer_jwt`. |
| Whiteboard reconnect loses prior strokes. | Accepted for ephemeral Cycle 2; saved state is a later feature. |
| Whiteboard stroke payloads grow too large. | Client and backend caps for points/strokes. |
| UI gets crowded. | Use primary/secondary stage layout and keep sidebars unchanged. |

## Acceptance Criteria

- Teacher can start/stop screen share without interrupting camera/mic.
- Student sees screen share alongside teacher camera when active.
- Stopping screen share removes the screen surface and leaves the camera feed alive.
- Promoted-student WHEP viewing uses the join-minted `viewer_jwt`.
- Teacher can draw and clear the whiteboard.
- Students see teacher whiteboard strokes live.
- Students cannot draw in Cycle 2.
- Route exit and end class close main, screen, and promoted-student media resources.
- Relevant backend/frontend tests pass.
- Manual browser smoke passes against the deployed stack.
