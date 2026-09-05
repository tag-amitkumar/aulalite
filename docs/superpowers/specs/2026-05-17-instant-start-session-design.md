# Instant-Start Session — Design

**Status:** Draft for review
**Date:** 2026-05-17
**Author:** brainstorming pass with chiranjib.chaudhuri@geosapiens.com

## Problem

Today a teacher can only start a live class by first scheduling a Series:
`POST /v1/courses/:cid/sessions` requires `title`, `starts_at`, `frequency`,
`end_kind`, etc., producing a Series and one or more Occurrences. To start
a class "right now" a teacher must fill the series scheduler, pick a start
time of "now", set frequency to `none`, and submit — then navigate into
the live room and click **Go Live**.

We want a one-tap path for the common case: a teacher on a course page
who wants to start a session immediately.

## Goals

- Teacher can start a live session from the course page with one click.
- An optional "Customize…" path lets the teacher set title / duration /
  recording before starting.
- Only one live session per course at a time (atomic conflict check).
- Students on the course page see a "Live now" banner within ~15s.
- No new realtime infrastructure (reuse the existing live-room socket
  for in-session state, polling for course-level discovery).

## Non-goals

- External notifications (email / web push) for instant sessions.
- "Personal room" / course-agnostic instant sessions from the dashboard.
- Automatic recovery from abandoned sessions (teacher closes tab
  without End Class). Today's reaper / end-of-day cleanup is unchanged.
- Notifying enrolled students who are not currently on the course page.

## Architecture overview

Five new building blocks:

1. `StartNowButton` (`features-courses`) — split button rendered in
   `CourseDetail`'s `PageHeader` actions slot, gated on `can_admin`.
2. `StartNowModal` (`features-courses`) — editable title, duration,
   recording toggle. Reached via the split button's caret.
3. `POST /v1/courses/:cid/sessions/start-now` (`backend`) — atomic
   conflict check + one-off occurrence creation + transition to `live`,
   all in the same transaction.
4. `GET /v1/courses/:cid/active-session` (`backend`) — read used by
   both the teacher button (for 409 recovery) and the student banner.
5. `use_active_session_poll` hook + `LiveNowBanner` component (both in
   `features-courses`) — 15s polling loop that drives the student
   banner and the teacher button's conflict state.

Reused infrastructure:

- `create_series_inner` (`backend`) — produces the underlying series
  and occurrence rows.
- The existing live-room flow (`live_room_broadcast`,
  `LiveRoomSession`, `/go-live`, `/end-class`) — unchanged. Start-now
  lands the teacher in a session that is already `live`, so the
  broadcast view's existing "Go Live" path is short-circuited (it sees
  the live state on mount and proceeds straight to socket + WHIP
  attach).

## UI

### Teacher view — course-detail header

The `CourseDetail` `actions` slot today holds the status badge and an
"Edit" ghost button. We extend it to:

```
[ status badge ]   [ ● Start session now ▾ ]   [ Edit ]
                     primary   caret → Customize…
```

- **Click body (one tap):** POST `/start-now` with empty body, server
  uses defaults, on 200 navigate to
  `/courses/:slug/live/:session_id/broadcast`.
- **Click caret → "Customize…":** opens `StartNowModal`. On submit, same
  POST with body, same navigation.
- **Active-session conflict:** primary button relabels to
  **Join active session**; caret hidden. Click → existing broadcast
  route for the active session id (no second POST).

### Student view — course-detail page

When `use_active_session_poll` returns `Some(_)` for a non-admin viewer,
`LiveNowBanner` renders just below the `PageHeader`:

```
┌──────────────────────────────────────────────────────────────┐
│ ● Live now — Quick session — May 17, 2:32 PM      [ Join ]   │
└──────────────────────────────────────────────────────────────┘
```

Banner uses the existing danger / accent surface tokens (red dot for
"live"). Click → existing student join route (`/courses/:slug/live/:id`).

### Schedule tab

No special UI. The new occurrence is a normal row in the existing
schedule (it goes through the same series + occurrence tables) and the
"live" status badge surfaces via existing badge-tone logic.

## Backend

### `POST /v1/courses/:cid/sessions/start-now`

Request body, all optional:

```json
{ "title": "...", "duration_minutes": 60, "recording_enabled": true }
```

**Schema prerequisite.** Add a partial unique index in a new migration:

```sql
CREATE UNIQUE INDEX live_sessions_one_live_per_course
  ON live_sessions (course_id)
  WHERE status = 'live';
```

(The occurrence table is named `live_sessions`; rows represent individual
session occurrences. The series header is in `live_session_series`.)

This makes "at most one live session per course" a declarative
constraint instead of an application-level check, closing the race
where two `SELECT … FOR UPDATE` checks both find zero rows and both
proceed to insert. The index also protects any other code path that
might transition an occurrence to `live` (e.g., a future
auto-start-on-schedule worker).

Handler logic, all in one transaction:

1. `caller_can_admin_course` check — 403 if not allowed.
2. `SELECT id, title, starts_at FROM live_sessions
   WHERE course_id = $1 AND status = 'live' LIMIT 1` — if a row
   exists, return **409 Conflict** with body
   `{ active_session_id, title, starts_at }` before doing any
   inserts. This is the common-case fast path that gives the user a
   clean error message instead of a unique-violation surfacing.
3. Resolve defaults:
   - `title` → `format!("Quick session — {}", now_utc.format("%b %-d, %Y %-I:%M %p UTC"))`.
     Server formats in UTC; the client may re-render in the viewer's
     timezone if desired (out of scope for v1).
   - `duration_minutes` → 60.
   - `recording_enabled` → fall through to the tenant default, identical
     to the existing fallback in `create_series_inner`
     (`crates/backend/src/handlers/live_sessions.rs` around line 248).
4. Reuse `create_series_inner` with `frequency=none`, `end_kind=count`,
   `occurrence_count=1`, `starts_at=now()`. One series row, one
   occurrence row, same transaction.
5. Transition the new occurrence to `status='live'` in the same
   transaction, setting `transport_mode='webrtc'`. This is the key
   semantic difference from the scheduled-series path: an instant
   session is born live, so there is no scheduled→live race window
   between handler return and the teacher pressing Go Live.
6. Return:
   ```json
   { "session_id": "...", "series_id": "...", "title": "...",
     "starts_at": "...", "duration_minutes": 60,
     "recording_enabled": true, "transport_mode": "webrtc",
     "status": "live" }
   ```

**Race-safety summary.** Two simultaneous clicks resolve cleanly:

- If one tx commits first, the other's step-2 check returns the live
  row and the handler returns 409 normally.
- If both pass step 2 (zero rows visible), they race on the unique
  index in step 5. One commits; the other gets a unique-violation,
  which the handler maps to **409 Conflict** with the now-visible
  active session's id (a second `SELECT` after catching the
  uniqueness error).

**Coordination with `/go-live`.** The broadcast view will land on a
session that already has `status='live'`. The frontend's existing
"click Go Live" flow needs a small adjustment to recognise this state
and proceed straight to socket + WHIP attach without re-POSTing
`/go-live`. (If `/go-live` is currently idempotent — it sets status to
`live` and is safe to re-call on an already-live session — no
frontend change is needed; the writing-plans pass will verify and
adjust.)

### `GET /v1/courses/:cid/active-session`

- Auth: any enrolled member of the course (teacher, co-teacher, or
  student). Reuses the existing course-membership check used by
  `GET /v1/courses/:cid/sessions`.
- Always 200 to keep the polling loop simple:
  - `{ "active": null }` when no live session.
  - `{ "active": { "session_id": "...", "title": "...",
    "starts_at": "...", "transport_mode": "..." } }` otherwise.
- Single indexed query: `SELECT … FROM live_sessions
  WHERE course_id = $1 AND status = 'live' LIMIT 1`.

### Routing

Added inside `handlers::live_sessions::routes()` and mirrored in
`router_for_tests` (matches the existing pattern at
`crates/backend/src/handlers/live_sessions.rs:69-88`).

## Client data flow

### Teacher click — happy path

```
StartNowButton.on_click
  → api.post("/v1/courses/{cid}/sessions/start-now", body)
  → 200 { session_id, ... }
  → router.navigate("/courses/{slug}/live/{session_id}/broadcast")
  → existing live_room_broadcast mounts → LiveRoomSession::new
  → connect_socket → teacher clicks "Go Live"
```

### Teacher click — 409 conflict

```
  → 409 { active_session_id, title, starts_at }
  → toast: "A session is already live in this course."
  → Button relabels to "Join active session" using the 409 payload
  → next click navigates to /live/{active_session_id}/broadcast
    (no new POST)
```

### Student banner

```
CourseDetail mounts (non-admin viewer)
  → use_active_session_poll(course_id) spawns a poll loop
  → every 15s: GET /v1/courses/{cid}/active-session
  → on { active: Some } → render LiveNowBanner with Join CTA
  → on Join click → /courses/{slug}/live/{session_id}
  → on unmount: poll loop aborts via cancellation token
```

### Teacher polling

The hook also runs for teachers, but the result feeds the **button
state** (conflict mode) rather than a banner. If a co-teacher starts a
session in a course you're viewing, your button reflects that on the
next poll without you needing to click first.

### Poll-loop ownership

Lives in `features-courses` as `use_active_session_poll`. Implemented
with `use_future` + `gloo_timers::future::TimeoutFuture::new(15_000)`,
mirroring the existing backoff pattern in `live_room_socket.rs`. On
error (network blip, 5xx), back off to 30s and keep prior state — the
banner does not flicker. On 401, stop polling and let the existing
auth-refresh path resume on next user interaction.

## Error handling & edge cases

| Case | Behavior |
|---|---|
| Click on slow network | Button shows spinner + `disabled` until response. |
| 5xx from `start-now` | Toast: "Couldn't start the session. Try again." Button re-enables. No client-side retry. |
| 409 from `start-now` | Toast + button flips to "Join active session" using the 409 payload. No second POST. |
| 403 from `start-now` | Should be impossible (button gated on `can_admin`). Log + toast "You no longer have permission" — likely a stale role. |
| `/active-session` 5xx during poll | Back off to 30s, keep prior state. No toast (background poll). |
| `/active-session` 401 during poll | Stop the poll, defer to existing auth-refresh path. |
| Teacher starts, navigates away | Session stays `live` until `/end-class` is called or the existing reaper runs. 409 check protects the next start. |
| Teacher closes broadcast tab without End Class | Unchanged from today's behavior. Documented gap, out of scope. |
| Recording on but session never goes live | Existing `live_room_recording` code already handles "ended without going live". |
| Course slug missing from response | Response intentionally omits slug; client uses the slug already in props. |

## Testing

### Backend (`crates/backend`)

- `start_now_creates_one_off_occurrence` — happy path; verify
  `frequency=none`, one row in `live_sessions`, and that the
  row is born with `status='live'` (not `'scheduled'`).
- `start_now_returns_409_when_live_session_exists` — seed a `live`
  occurrence; second call returns 409 with the active id.
- `start_now_conflict_check_is_atomic` — fire two concurrent
  `start_now` calls against the same course via `tokio::join!`; exactly
  one succeeds, the other gets 409. Guards both the fast-path check
  and the unique-index fallback.
- `start_now_maps_unique_violation_to_409` — directly seed the
  unique-violation path (start a tx, insert a live row but don't
  commit; from another tx, run start-now → 409, not 500).
- `start_now_requires_can_admin` — student caller → 403.
- `start_now_defaults_recording_to_tenant_setting` — empty body →
  resolved value matches `tenants.recording_default`.
- `active_session_returns_none_when_no_live` and
  `active_session_returns_the_live_one`.
- `active_session_allows_enrolled_student` and
  `active_session_forbids_non_member`.

### Frontend (`crates/features-courses`)

- `start_now_button_renders_only_for_admin` — `can_admin=false` →
  button absent.
- `start_now_button_disabled_while_submitting` — covers double-click.
- `start_now_modal_prefills_title_with_local_time` — string-format
  check on the prefilled title.
- `start_now_button_relabels_on_409` — mocked api returns 409, button
  text switches to "Join active session".
- `use_active_session_poll_renders_banner_when_active` — hook + banner
  integration via the existing `dioxus_test_utils` harness used in
  `live_room_*` tests.
- `live_now_banner_hidden_for_admin` — only students see the banner;
  teachers get the button-state change.

### Playwright (`tools/playwright`)

- `instant-session.spec.ts` (new) — teacher logs in, opens a course,
  clicks **Start session now**, lands on broadcast view, ends class.
- A follow-up case in the same spec: open the same course as a student
  in a second browser context, assert the **Live now** banner appears
  within ~20s of the teacher starting.

No new test scaffolding required — runs on the existing
`cargo test backend`, `cargo test -p features-courses`, and
`npx playwright test` paths.

## Open questions

None. Forks resolved during brainstorming:

- Scope: per-course only (not dashboard / global). — A in Q1
- Friction: one-tap primary + Customize secondary. — A+B in Q2
- Concurrency: block + offer "Join active". — A in Q3
- Discovery: schedule entry + real-time banner; no external push. — D in Q4
- Backend: new dedicated endpoint. — 1A
- Realtime: polling. — 2A

## Implementation order (preview, not the plan)

The writing-plans pass will decompose this. Rough shape for context:

1. Backend: add `POST /start-now` + `GET /active-session` with tests.
2. Frontend hook + `LiveNowBanner` + integration with `CourseDetail`.
3. `StartNowButton` + `StartNowModal` + integration with the
   `PageHeader` actions slot in `CourseDetail`.
4. Playwright coverage.
