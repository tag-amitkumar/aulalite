# AulaLite Live-Class Health and Recovery Design

## Context

AulaLite already has the core live-class product surface: teacher prejoin, camera
and microphone selection, WHIP publishing, WHEP viewing, screen share,
whiteboard, chat, hand raise, presence, WebSocket room events, HLS fallback
support, MediaMTX path helpers, and recording processing. The next product
improvement should not add another classroom feature before making the existing
live class feel trustworthy while it is running.

The current room can show local publish errors and some connection banners, but
teachers still lack one compact place to answer the practical teaching
questions:

- Am I live?
- Are my camera and microphone captured?
- Is the stream reaching the media server?
- Are students likely to see and hear me?
- Is recording working?
- What should I do when one of those checks is wrong?

This design prioritizes the requested product order: teacher confidence first,
student reliability second, classroom engagement third, and post-class value
fourth.

## Goals

- Add a compact teacher-facing health strip inside the live room.
- Add a diagnostic sheet with detailed checks and recovery actions.
- Make recording state visible during and after class.
- Make media-server and stream-path health visible to teachers.
- Improve student waiting and error states when the same health signals can
  prevent blank or confusing room surfaces.
- Reuse the existing live-room architecture instead of introducing a new media
  stack.

## Non-Goals

- No full live-room redesign.
- No replacement of MediaMTX, WHIP, WHEP, or the WebSocket broker.
- No full automated failover system in this pass.
- No load-testing framework in this pass.
- No saved whiteboard replay or richer post-class authoring.
- No new operational dashboard outside the live-room experience.

## Recommended Approach

Build a **Live-Class Health and Recovery Layer**.

The room will expose a simple, teacher-readable health summary while keeping
technical detail one click away. The backend will report server-side facts such
as session lifecycle, MediaMTX reachability, path status, and recording state.
The frontend will merge those facts with browser-side facts such as capture
state, publish state, WebSocket state, and network quality.

This is preferred over a pure UI polish pass because the teacher needs evidence
that the live stack is working, not just clearer wording. It is preferred over a
full resilience pass because the system should first make failures visible and
actionable before adding more automatic recovery behavior.

## Architecture

The feature extends the existing live-room boundary.

Backend responsibilities:

- Authorize the current user for teacher or admin diagnostics on the session.
- Load session lifecycle and course context.
- Build the existing MediaMTX main and screen paths.
- Probe MediaMTX health and path readiness through `MediaMtxClient`.
- Fetch the current recording row, if any.
- Return conservative health DTOs that do not expose secrets or raw provider
  payloads.

Frontend responsibilities:

- Track local browser state from prejoin, publish, WebSocket, WHEP, and network
  stats.
- Poll server health while the teacher room is live.
- Merge browser health and backend health into a small set of categories.
- Render a compact health strip and a detailed diagnostic sheet.
- Offer recovery actions only when the system has enough context to perform
  them safely.

## Backend Design

Add:

```text
GET /v1/sessions/{id}/health
```

The first version is teacher/admin only. It should use the same session access
shape as the live-room teacher actions and deny students. Student-facing state
can continue to be derived from join responses and local viewer errors until a
minimal student-safe health contract is explicitly needed.

### DTO Shape

Use stable string statuses for the API contract:

```text
ok
warning
error
unknown
not_applicable
```

The response should include:

- `session`: lifecycle state, scheduled time, actual start, actual end.
- `media_server`: MediaMTX API reachability.
- `main_stream`: main path status.
- `screen_stream`: screen path status and whether inactive is expected.
- `recording`: enabled flag, processing status, failure message summary, retry
  eligibility.
- `checked_at`: server timestamp for freshness.

The server should map facts conservatively:

- MediaMTX health probe succeeds: `ok`.
- MediaMTX health probe fails: `error`.
- Main path active while session is live: `ok`.
- Main path inactive while session is live: `error`.
- Main path while the session is still scheduled: `not_applicable`.
- Screen path active: `ok`.
- Screen path inactive by itself: `not_applicable`, because the backend does not
  know whether the browser is actively sharing a screen.
- Recording `pending`, `remuxing`, or `uploading`: `warning` with progress copy.
- Recording `available`: `ok`.
- Recording `failed`: `error` and retry eligible for teacher/admin after class.
- No recording row while the session is live and recording is enabled:
  `warning`, because the final row may not be created until the session ends.

### Backend Boundaries

The endpoint should not call browser-only concepts. It only reports server-side
facts. Browser-side capture, WHIP local publish, WebSocket state, and network
quality remain frontend facts.

The endpoint should not expose raw ffmpeg output, full provider responses, JWTs,
publish passwords, or MediaMTX shared secrets. If a recording failed, the DTO can
carry a short sanitized summary from `processing_error`.

## Frontend Design

Add focused live-room health UI rather than a dashboard page.

### Components

- `LiveRoomHealthStrip`: compact teacher row near existing broadcast status
  controls. Shows the current worst status for camera/mic, stream, room socket,
  student visibility, recording, and connection quality.
- `LiveRoomDiagnosticsSheet`: detail panel opened from the strip. Groups checks
  by local device, publishing, media server, student viewing, recording, and
  room connection.
- `HealthCheckRow`: reusable row for a status, label, explanation, freshness,
  and optional action.
- `RecordingStatusPanel`: focused recording state and retry affordance.
- `StreamStateNotice`: student-facing state surface for waiting, connecting,
  retrying, and failed video states.

### Teacher Health Strip

The strip should stay compact and classroom-focused. It should not become an
operator dashboard. Suggested categories:

- Devices: camera and microphone captured.
- Publish: teacher WHIP publish active or failed.
- Media: MediaMTX reachable and main path active.
- Room: WebSocket connected or reconnecting.
- Students: student viewing status inferred from main stream and room state.
- Recording: disabled, recording expected, processing, available, or failed.
- Quality: existing `NetworkQualityBadge`.

### Diagnostics Sheet

The sheet gives the teacher the details needed to act:

- Check name.
- Status.
- Last checked time.
- Plain explanation.
- Recommended action.

Recovery actions:

- `Refresh room health`: force server health refresh.
- `Recheck devices`: return to a prejoin-like device check without ending class.
- `Retry publish`: rerun camera/mic publish when local capture or WHIP publish
  failed.
- `Restart screen share`: stop and restart screen sharing when screen publish is
  active but unhealthy.
- `Retry recording`: allowed only after class ends and only when backend reports
  a failed recording.

### Student Reliability States

Student-facing changes should be small and tied to the same health effort:

- If the teacher is not live yet, show a waiting state.
- If the stream is connecting, show a connecting state.
- If WHEP attach fails, show retry instead of leaving a blank video area.
- If HLS fallback is unavailable or still connecting, explain that state.

## Data Flow

1. Teacher opens or starts the live room.
2. Frontend initializes local browser health from prejoin and publish state.
3. When session status is live, frontend polls `GET /v1/sessions/{id}/health`
   every 10 to 15 seconds.
4. Frontend also refreshes health after recovery actions.
5. Frontend merges backend health with local browser health.
6. `LiveRoomHealthStrip` renders category summaries.
7. `LiveRoomDiagnosticsSheet` renders detailed rows and action buttons.
8. Student video states use local viewer errors and known session state to avoid
   blank room surfaces.

## Status Merge Rules

Health categories should use the worst meaningful status:

- `error` beats `warning`.
- `warning` beats `unknown`.
- `unknown` beats `ok` only when no better source exists.
- `not_applicable` should not downgrade the room.

Examples:

- Browser capture is active but backend main path is inactive while live:
  publish/media status is `error`.
- Backend health polling fails but browser publish is still active: media server
  is `unknown`, not `error`, unless the last known server check was an error.
- Screen path is inactive when the teacher is not sharing: screen status is
  `not_applicable`.
- Screen path is inactive while the browser has an active screen publisher:
  screen status is `warning`, because the frontend has extra local context that
  the backend intentionally does not infer.
- Recording is remuxing after class: recording status is `warning`, not `error`.

## Error Handling

- Health polling failure should not hide local state.
- MediaMTX API failure should show a clear media-server error.
- Main path inactive while teacher is live should offer retry publish.
- Screen path inactive should be a warning only when the teacher is actively
  sharing screen.
- Recording processing states should use progress wording.
- Recording retry should not appear unless the backend says retry is allowed.
- Student video errors should render a state with retry guidance, not a blank
  stage.
- Recovery action failures should be shown inline in the diagnostics sheet and
  as existing live-room toasts where appropriate.

## Testing Strategy

Backend tests:

- Pure status mapping for MediaMTX up/down.
- Pure status mapping for main and screen path active/inactive combinations.
- Pure status mapping for recording states.
- Handler test that teacher/admin can read health.
- Handler test that a student cannot read full teacher diagnostics.
- Handler test that the health endpoint does not expose secrets.

Frontend tests:

- SSR render of health strip with all-ok state.
- SSR render of health strip with mixed warning/error states.
- SSR render of diagnostics sheet rows and action labels.
- Pure merge tests for backend health plus browser health.
- Student stream notice tests for waiting, connecting, retrying, and failed
  states.

Verification commands:

```bash
cargo test --workspace
cargo check -p shell-web --target wasm32-unknown-unknown
cargo test -p backend --lib live_sessions
cargo test -p features-courses --lib live_room
cargo test -p shell-web --lib
```

DB-backed endpoint tests should run under:

```bash
cargo test -p backend --features db-tests --test live_room -- --nocapture
```

## Acceptance Criteria

- Teacher sees a compact health strip in the live room while teaching.
- Teacher can open diagnostics without leaving the class.
- The diagnostics distinguish local device, publish, media server, room socket,
  student visibility, and recording issues.
- Main stream inactive while live is visible as a serious problem.
- Screen stream inactive is only treated as a problem while screen share is
  active.
- Recording state is visible and retry is available only for failed recordings
  after class.
- Students no longer see an unexplained blank video area for known waiting or
  connection-failure states.
- Existing live-room tests continue to pass.

## Risks and Mitigations

- Risk: The teacher UI becomes too technical.
  - Mitigation: keep the strip simple and move details into the sheet.
- Risk: Backend health probes slow down the room.
  - Mitigation: poll modestly, keep MediaMTX calls bounded, and preserve local
    browser state when polling fails.
- Risk: Server health and browser health disagree.
  - Mitigation: show both in diagnostics and mark disagreement as actionable.
- Risk: Retry publish creates duplicate media resources.
  - Mitigation: route retry through existing `LiveRoomSession` ownership and
    close previous publishers before replacing them.
- Risk: Recording status causes false alarm while processing.
  - Mitigation: treat pending/remuxing/uploading as progress warnings, not
    failures.

## Implementation Sequence

1. Add DTOs and pure status mapping helpers.
2. Add `GET /v1/sessions/{id}/health`.
3. Add frontend health model and merge helpers.
4. Add health strip and diagnostics sheet.
5. Wire teacher browser-state sources into the health model.
6. Wire recovery actions.
7. Improve student stream notices.
8. Add targeted tests and run verification.
