# AulaLite Phase 1b-β — Live Class Core Design

**Date:** 2026-05-08
**Phase:** 1b-β
**Predecessor:** 1b-α (file uploads, complete at `f2cbd1c`)
**Successors:** 1b-γ (live room UX: chat, hand-raise, presence), 1b-δ (recording)

## 1. Goal

Ship the headline AulaLite feature: live online classes. Teachers broadcast camera + mic + (optional) screen-share from a browser; enrolled students view via WebRTC or HLS depending on class size. Multi-tenant, role-aware, recording-ready (hooks only).

## 2. In scope

- Two MediaMTX publish paths per session (`<path>` for camera+mic, `<path>/screen` for screen-share).
- Tiered viewer transport per series: `webrtc` (sub-second, ≤30 viewers) or `hls` (~5s latency, scales to hundreds).
- Lifecycle: `scheduled` → (lobby UX) → `live` → `ended`, with auto-end after `duration + 30min` grace.
- Hybrid auth: HTTP callback for teacher publish (real-time authz); short-lived JWT for student watch (15-min TTL with refresh).
- Web publish (Chrome/Edge/Firefox/Safari desktop). Web + Android watch.
- Backend extensions to `handlers::live_sessions`; new `services::mediamtx`.
- Frontend extensions to `features-courses` (new `live_room_*` modules).
- Mock-MediaMTX integration tests; SSR smokes for room shell + lobby.

## 3. Out of scope

| Feature | Phase |
|---|---|
| Chat, hand-raise, presence indicators | 1b-γ |
| Recording start/stop, playback UI, file_assets sidecar | 1b-δ |
| Mobile publish (camera/mic from phone) | later |
| RTMP/OBS publish | later |
| Adaptive bitrate, simulcast, SVC | later |
| Audio-only mode, push-to-talk, mute-all | 1b-γ |
| Bandwidth-driven quality downgrade | later (relies on MediaMTX defaults for now) |

## 4. Architecture

### 4.1 Code organization

Per Phase 1b-β decision (Approach 3): extend the existing `handlers::live_sessions` and `features-courses` rather than creating a new crate. Pull MediaMTX-specific helpers into a new `services::mediamtx` module so the handler stays readable.

**Backend touch points:**
- `crates/backend/src/handlers/live_sessions.rs` — adds go-live, end-class, join, refresh-token, MediaMTX-auth-publish, JWKS routes.
- `crates/backend/src/services/mediamtx.rs` — NEW. Path naming, JWT minting, MediaMTX HTTP API client + mock trait.
- `crates/backend/src/db/live_sessions.rs` — adds queries for path lookup, status transitions, auto-end sweep.
- `crates/backend/src/main.rs` — spawns 60-second auto-end interval task.
- `crates/backend/src/lib.rs` — `AppState` gains `mediamtx: Arc<dyn MediaMtxClient>` and `jwt_signer: Arc<JwtSigner>`.

**Frontend touch points (in `features-courses`):**
- `live_room_shell.rs` — state-machine wrapper (NEW).
- `live_room_lobby.rs` — pre-live waiting UI (NEW).
- `live_room_broadcast.rs` — teacher publish UI (NEW).
- `live_room_view.rs` — student watch UI, branches WebRTC vs HLS (NEW).
- `live_room_whip.rs` — WHIP client helper (NEW). web-sys-based, wasm32-only.
- `live_room_whep.rs` — WHEP client helper (NEW). Symmetric to WHIP.

**Infrastructure:**
- `ops/mediamtx/mediamtx.yml` — replace permissive Phase 0 stub with auth-enabled config (HTTP for publish, JWT for read).
- `migrations/20260508000012_live_room_columns.sql` — additive ALTER TABLE.

### 4.2 MediaMTX path scheme

```
aula/<tenant_simple>/<course_simple>/<session_simple>          # main: camera + mic
aula/<tenant_simple>/<course_simple>/<session_simple>/screen   # screen-share, when enabled
```

`<*_simple>` is the dash-stripped UUID.simple() (matches `services::file_assets::object_key` convention from 1b-α). The `aula/` prefix namespaces our paths inside MediaMTX in case the same instance is shared with other services.

### 4.3 Auth

**Publish path (teachers):**
- MediaMTX `authMethod: http`, `authHTTPAddress: http://backend:8080/v1/mediamtx/auth/publish`.
- On every WHIP connect, MediaMTX POSTs `{action, path, ip, user, password, query, protocol}` to backend.
- Backend parses path → `(tenant_id, course_id, session_id)`, looks up session, verifies caller is course-teacher / org_admin / TA, status ∈ {scheduled, live}, time within `[starts_at - 30min, starts_at + 4hr]`. Returns 200/403.
- Caller proves identity via `password` field, set to a one-time publish token minted by `/v1/sessions/:id/go-live` (TTL 4hr, single-use enforced via `live_sessions.publish_nonce` column). Stored hashed.

**Read path (students):**
- MediaMTX `authMethod: jwt`, `authJWTJWKSURL: http://backend:8080/v1/mediamtx/jwks`.
- Backend mints RS256 JWTs (15-min TTL). Asymmetric algorithm chosen because MediaMTX `authJWTJWKSURL` expects to fetch a public key; symmetric HS256 doesn't fit the JWKS-fetch pattern. Claims:
  - `iss = "aulalite"`
  - `sub = user_id.to_string()`
  - `tnt = tenant_id.to_string()`
  - `mediamtx_permissions = [{"action":"read","path":"aula/<t>/<c>/<s>"}, {"action":"read","path":"aula/<t>/<c>/<s>/screen"}]`
  - `exp` = now + 900s
- Browser includes JWT in MediaMTX connect (header for WebRTC; query param `?jwt=...` for HLS m3u8 URL).
- Refresh: client calls `POST /v1/sessions/:id/refresh-token` at TTL-1min → new JWT.

**JWKS endpoint:**
- `GET /v1/mediamtx/jwks` returns the public half of the RS256 keypair as a standard JWK set.
- Backend reads private key from `JWT_RS256_PRIVATE_KEY_PEM` env var. If unset at startup, backend generates a fresh keypair in-process and logs a warning ("ephemeral keypair; backend restart invalidates outstanding viewer JWTs"). Acceptable because viewer JWT TTL is 15 min — a restart is no worse than 15 min of forced reconnects, and viewers' poll/refresh handles it.
- For production, the key must be set via secret manager so JWTs survive restarts.

## 5. Schema changes

Migration `20260508000012_live_room_columns.sql`:

```sql
ALTER TABLE live_session_series
    ADD COLUMN transport_mode TEXT NOT NULL DEFAULT 'webrtc'
    CHECK (transport_mode IN ('webrtc', 'hls'));

ALTER TABLE live_sessions
    ADD COLUMN screen_path TEXT,
    ADD COLUMN transport_mode TEXT NOT NULL DEFAULT 'webrtc'
    CHECK (transport_mode IN ('webrtc', 'hls')),
    ADD COLUMN publish_nonce TEXT,
    ADD COLUMN publish_nonce_expires_at TIMESTAMPTZ;

CREATE INDEX live_sessions_status_started_idx
    ON live_sessions(status, actual_started_at)
    WHERE status = 'live';   -- supports auto-end sweep
```

**Pre-existing columns reused:** `actual_started_at`, `actual_ended_at`, `main_path`, `recording_enabled`.

**Note:** the existing series-creation handler must thread `transport_mode` through to inserted occurrences (default to series value).

## 6. API surface

| Method | Path | Caller | Description |
|---|---|---|---|
| `POST` | `/v1/sessions/:id/go-live` | course teacher / org_admin | Start session: set status=live, populate paths, mint publish nonce. Returns `{publish_url, publish_password, screen_url}`. |
| `POST` | `/v1/sessions/:id/end-class` | course teacher / org_admin | End session: status=ended, `actual_ended_at=now()`. Idempotent. |
| `POST` | `/v1/sessions/:id/join` | enrolled course member | Returns `{state: "lobby"\|"live"\|"ended", transport_mode, viewer_jwt, main_url, screen_url}`. |
| `POST` | `/v1/sessions/:id/refresh-token` | already-joined viewer | Mints fresh JWT; rate-limited to once per 5 min per user. |
| `POST` | `/v1/mediamtx/auth/publish` | MediaMTX (server-to-server, IP allowlist or shared header) | Authorize publish per-connection. |
| `GET`  | `/v1/mediamtx/jwks` | MediaMTX | RS256 JWKS for read-token validation. |
| `GET`  | `/v1/mediamtx/healthz` | UI / ops | Proxies MediaMTX `/v3/config/get`; returns 200/503. |

### 6.1 DTOs

**`GoLiveResponse`:**
```json
{
  "session_id": "...",
  "main_publish_url": "http://localhost:8889/aula/<t>/<c>/<s>/whip",
  "screen_publish_url": "http://localhost:8889/aula/<t>/<c>/<s>/screen/whip",
  "publish_password": "<one-time nonce, 4hr TTL>",
  "transport_mode": "webrtc"
}
```

**`JoinResponse`:**
```json
{
  "state": "lobby" | "live" | "ended" | "cancelled",
  "session_id": "...",
  "transport_mode": "webrtc" | "hls",
  "viewer_jwt": "<JWT, 15-min TTL>" | null,
  "main_url": "http://localhost:8889/aula/<t>/<c>/<s>/whep" | "http://localhost:8888/aula/<t>/<c>/<s>/index.m3u8" | null,
  "screen_url": <same shape, or null>,
  "instructor_name": "...",
  "scheduled_starts_at": "..."
}
```

`viewer_jwt` and `*_url` are null in lobby and ended/cancelled states.

## 7. Lifecycle state machine

```
scheduled  ──[teacher: POST go-live]──▶ live  ──[teacher: POST end-class]──▶ ended
    │                                    │
    │                                    └──[auto-end sweep, dur+30min]──▶ ended
    │
    ├──[per-occurrence cancel API]──▶ cancelled
    └──[reaches starts_at-5min]──▶ still scheduled, but lobby is joinable
```

**Lobby is a UX state, not a DB state.** Students get `state: "lobby"` from `/join` when:
- `now ∈ [starts_at - 5min, starts_at + 4hr]`
- session.status = scheduled
- No publish-active signal yet

When teacher hits Go Live, status → live. Lobby clients poll `GET /v1/sessions/:id` every 5s, detect transition, request `/join` again, get full live join payload (JWT + URLs).

**Auto-end sweep:** Tokio interval task (60s cadence) running in `main.rs`:
```rust
UPDATE live_sessions
   SET status='ended', actual_ended_at=now()
 WHERE status='live'
   AND actual_started_at + (duration_minutes + 30) * interval '1 minute' < now()
RETURNING id, tenant_id;
```
Audit event `live_session.auto_ended` per row. Publicly-callable `db::live_sessions::sweep_auto_end()` lets tests trigger it deterministically.

## 8. Frontend component design

### 8.1 LiveRoomShell

Mounted at `/courses/:slug/sessions/:id`. Polls `GET /v1/sessions/:id` every 5s while status=scheduled. Decides which child to render based on `(role, status)`:

| Role | Status | Renders |
|---|---|---|
| teacher | scheduled | LiveRoomBroadcast (preview UI, "Go Live" button) |
| teacher | live | LiveRoomBroadcast (active publish UI) |
| student | scheduled | LiveRoomLobby |
| student | live | LiveRoomView (WebRTC or HLS based on transport_mode) |
| any | ended | "Class has ended" + (later) recording link |
| any | cancelled | "Class was cancelled" |

### 8.2 LiveRoomBroadcast (teacher)

Two simultaneous WHIP publishers managed by `live_room_whip::WhipPublisher`:
- **Camera+mic** stream from `getUserMedia({video: true, audio: true})`. Always present once teacher hits Go Live.
- **Screen** stream from `getDisplayMedia({video: true})`. Toggle on/off; when on, second publisher pushes to `<path>/screen`.

UI: large local-camera preview, screen preview (when active), Camera/Mic/Screen toggles, "End Class" button, network status indicator (good/degraded/disconnected from `RtcPeerConnection.connectionState`).

### 8.3 LiveRoomView (student)

Branches on `transport_mode`:

**WebRTC branch** (`live_room_whep::WhepViewer`):
- POSTs SDP offer to `<main_url>` (which ends in `/whep`) with `Authorization: Bearer <jwt>`.
- Receives SDP answer; attaches inbound MediaStream tracks to `<video>` elements.
- For two-stream (camera + screen) layout: side-by-side or picture-in-picture; teacher's screen-share is shown larger when active.

**HLS branch** (uses external `hls.js` via `<script>` tag):
- Loads m3u8 URL into `<video>` element.
- Native HLS on Safari/iOS; hls.js on others.
- JWT is appended as `?jwt=...` query param to the m3u8 URL (MediaMTX supports this for HLS auth).
- hls.js bundle (~70KB gzip) loaded only on this branch via dynamic `<script defer src="/static/hls.js">` — no penalty for WebRTC viewers.

### 8.4 LiveRoomLobby

Static UI: course title, instructor name, scheduled start time, "Class will begin shortly…" placeholder, current local time vs scheduled start. Polls `GET /v1/sessions/:id` every 5s. On transition to live, swaps to LiveRoomView automatically.

### 8.5 Mobile (shell-mobile, Android)

LiveRoomShell, LiveRoomLobby, LiveRoomView all reused on Android via Dioxus mobile (WebView under the hood). HLS view is the safer mobile path; WebRTC view is best-effort. Mobile *publish* is out of scope for this phase; LiveRoomBroadcast is gated behind `cfg(target_arch = "wasm32")` (web-only).

### 8.6 Routing

`shell-web` adds the route `/courses/:slug/sessions/:id` → LiveRoomShell. The existing schedule tab navigates here on click.

## 9. Failure & recovery

**Teacher disconnects (closes tab, network drop):**
- WHIP connection closes; MediaMTX path goes idle.
- Backend has no real-time signal — session stays `live` until (a) teacher returns and re-publishes, ICE restart resumes the stream, or (b) auto-end sweep fires after duration+30min.
- Students see "stream interrupted, reconnecting…" overlay if `RtcPeerConnection.iceConnectionState` ∈ {disconnected, failed}. Component retries every 5s for 60s, then shows "instructor disconnected; class may resume shortly".

**MediaMTX is down:**
- Go-live POST succeeds (DB-only). Browser's WHIP POST gets network error → broadcast UI shows "media server unreachable, please refresh".
- Students get JWT but WHEP/HLS connect fails → view shows same error with retry.
- `GET /v1/mediamtx/healthz` proxies MediaMTX `/v3/config/get` so UI can detect this proactively.

**JWT expiry mid-stream:**
- Frontend timer fires at TTL-1min → `POST /v1/sessions/:id/refresh-token` → new JWT.
- WebRTC: not strictly needed mid-connection (MediaMTX validated at connect time).
- HLS: segments are independent; new JWT used for next m3u8 fetch.

**Cancelled session while live:**
- Per-occurrence cancel handler (already exists from Phase 1a) detects status='live' and atomically also calls end-class flow. Audit events for both transitions.

**Publish nonce reuse / theft:**
- `publish_nonce` column stores the **hash** (argon2 or sha256) of the nonce. Plaintext is returned to the teacher exactly once in the `GoLiveResponse.publish_password` field.
- On MediaMTX auth callback, backend hashes the candidate `password`, compares constant-time against stored hash, and atomically NULLs the column on success: `UPDATE live_sessions SET publish_nonce = NULL WHERE id = $1 AND publish_nonce = $2 RETURNING id`. If `0 rows` returned, callback returns 403.
- 4-hour TTL via `publish_nonce_expires_at` column.
- If teacher's browser fails to establish WHIP after go-live (e.g., MediaMTX 5xx, network blip), they call go-live again. Backend mints a fresh nonce (overwrites the stored hash + extends expiry); previous nonce is invalidated.

## 10. Testing strategy

### 10.1 Backend unit tests (pure)

In `crates/backend/src/services/mediamtx.rs`:
- `path_for_session(tenant, course, session)` — string formatting, prefix correct, dashes stripped.
- `screen_path_for_session(...)` — appends `/screen`.
- `parse_path("aula/<t>/<c>/<s>")` — returns `(tenant_id, course_id, session_id)` parsed from simple-form UUIDs.
- `build_viewer_jwt(claims, secret, ttl)` — JWT structure, claim shape, expiry.
- `validate_publish_nonce(stored_hash, candidate)` — constant-time compare.

### 10.2 Backend integration tests (live Postgres + MockMediaMtxClient)

`MediaMtxClient` trait in `services::mediamtx`:
```rust
pub trait MediaMtxClient: Send + Sync {
    async fn publish_started(&self, path: &str) -> Result<(), MediaMtxError>;
    async fn publish_ended(&self, path: &str) -> Result<(), MediaMtxError>;
    async fn path_status(&self, path: &str) -> Result<PathStatus, MediaMtxError>;
    async fn healthz(&self) -> Result<(), MediaMtxError>;
}
```
Production impl wraps MediaMTX HTTP API. `MockMediaMtxClient` records calls for assertions. Pattern mirrors `S3Client` from 1b-α.

Tests:
- `tests/live_room.rs`:
  - `go_live_happy_path` — teacher hits go-live, gets publish creds, status=live, audit event recorded.
  - `go_live_outside_window` — go-live attempted >4hr after starts_at → 400.
  - `go_live_by_non_teacher` — student tries go-live → 403.
  - `go_live_idempotency` — second go-live on already-live session → 409 (or returns same nonce).
  - `end_class_happy_path` + `end_class_idempotency`.
  - `join_in_lobby_returns_lobby_state`.
  - `join_in_live_returns_jwt_and_urls`.
  - `join_after_end_returns_ended_state`.
  - `mediamtx_auth_publish_accepts_valid_nonce` — verifies the callback flow.
  - `mediamtx_auth_publish_rejects_used_nonce`.
  - `mediamtx_auth_publish_rejects_wrong_path`.
  - `refresh_token_rate_limit`.
  - `auto_end_sweep_ends_overdue_sessions` — drives `db::live_sessions::sweep_auto_end()` directly.
- `tests/rls_tenant_isolation.rs` — append: cross-tenant probe for `live_sessions.publish_nonce` and `screen_path` columns under RLS test role.

### 10.3 Frontend unit tests (no real WebRTC)

In `crates/features-courses/src/live_room_shell.rs`:
- `route_for(role, status, time_in_window)` — pure function returning which child to render. Cover all matrix cells.
- SSR tests:
  - `lobby_renders_class_will_begin_placeholder` — mount LiveRoomLobby with stub session data, assert SSR contains "will begin".
  - `broadcast_renders_go_live_button_when_scheduled` — mount LiveRoomBroadcast in scheduled state.
  - `view_renders_video_tag_for_webrtc` and `_for_hls` — mount LiveRoomView with appropriate transport_mode.
  - `room_shell_renders_lobby_for_student_during_scheduled`.
  - `room_shell_renders_ended_for_any_after_end`.

### 10.4 Workspace test sweep

`cargo test --workspace -j 2` (the `-j 2` is required on this Windows host to avoid pagefile pressure — established during Phase 1b-α exit). Wasm and native builds for `features-courses` and `shell-web`.

### 10.5 Manual exit-checklist (deferred per established pattern)

- Real Firebase teacher publishes via Chrome desktop with camera + mic.
- Same teacher toggles screen-share; students see screen replace/supplement camera.
- Real student joins via Chrome on `webrtc` series — sub-second latency.
- Real student joins via Chrome on `hls` series — ~5s latency.
- Real student joins via Android shell — HLS plays cleanly.
- JWT refresh fires after 14 min, no interruption.
- Teacher closes tab; auto-end sweep ends session after duration+30min.
- Cross-tenant probe: tenant B requests join on tenant A's session → 404.

## 11. Recording handoff hooks for 1b-δ

1b-β leaves clean seams. **What 1b-β deliberately does NOT do** (1b-δ owns):
- No recording start/stop logic. MediaMTX `record: yes` flag stays off.
- No file_assets row created on session end.
- No playback UI.

**What 1b-β DOES leave:**
- `live_session_series.recording_enabled` boolean (already present from Phase 1a).
- `actual_started_at` and `actual_ended_at` populated reliably — 1b-δ uses time range to find recording segments.
- `main_path` populated reliably — 1b-δ correlates MediaMTX recordings to sessions by path.
- Clean `MediaMtxClient` trait that 1b-δ extends with `start_recording(path)` / `stop_recording(path)` methods.
- `LiveRoomShell` "ended" branch contains a placeholder slot where the 1b-δ playback UI will mount.

## 12. Dependencies (Rust + JS)

**New Rust workspace deps** (root `Cargo.toml`):
- `jsonwebtoken = "9"` — HS256 mint + verify (publish nonce hashing + viewer JWT).

**Existing reuse:**
- `reqwest` (already present from JWKS for Firebase) for MediaMTX HTTP API client.
- `web-sys`, `wasm-bindgen`, `wasm-bindgen-futures` (already present from 1b-α) — extend `web-sys` features to add `RtcPeerConnection`, `RtcSessionDescription`, `RtcIceCandidate`, `MediaStream`, `MediaStreamTrack`, `MediaDevices`, `MediaStreamConstraints`, `Navigator`.

**JS deps** (loaded via `<script>` from shell-web index.html):
- `hls.js@1.5+` — vendored to `crates/shell-web/static/vendor/hls.js`. Loaded with `defer` only when an HLS series is opened. Not a Cargo dep.

**Env vars:**
- `JWT_RS256_PRIVATE_KEY_PEM` (RS256 private key in PEM; if unset, backend generates an ephemeral keypair at startup and logs a warning).
- `MEDIAMTX_HTTP_URL` (default `http://mediamtx:9997`) — MediaMTX HTTP API endpoint.
- `MEDIAMTX_PUBLIC_WEBRTC_URL` (default `http://localhost:8889`) — base URL the browser uses for WHIP/WHEP. Differs from the internal URL because Docker port-publishing.
- `MEDIAMTX_PUBLIC_HLS_URL` (default `http://localhost:8888`) — base URL the browser uses for m3u8.
- `MEDIAMTX_AUTH_SHARED_HEADER` — backend rejects `/v1/mediamtx/auth/publish` requests missing this header value. Set in `mediamtx.yml` as a forwarded header on the auth-callback request.

## 13. Open questions / deferred decisions

1. **TURN server.** WebRTC behind strict NATs needs a TURN relay. MediaMTX has built-in ICE servers but in adverse networks a real TURN (coturn) helps. Decision: ship without TURN in 1b-β; add as exit-checklist gate ("does WebRTC work on hotel wifi"). If yes — defer. If no — coturn becomes a 1b-γ task.
2. **MediaMTX HA.** Single MediaMTX instance is a single point of failure. For now assume one container per environment. Multi-instance + load balancing is a Phase 2 problem.
3. **Per-occurrence transport override.** Schema includes `live_sessions.transport_mode` defaulting to series value. Whether the UI lets teachers override per-occurrence is undecided; recommend leaving as a hidden API capability for now.
4. **Recording-aware watermark in viewer JWT.** When 1b-δ adds recording, the "you are being recorded" notice for joiners. Reserved for 1b-δ.
5. **Bandwidth estimation on the publisher side.** Show teacher upstream throughput and warn if degraded. Deferred — 1b-γ or later.

## 14. Acceptance criteria (Phase 1b-β complete when)

- All migrations applied; new columns visible in `\d live_sessions` and `\d live_session_series`.
- `cargo test --workspace -j 2` green (zero failures).
- Both wasm32 and native builds clean for backend + features-courses + shell-web.
- LiveRoomShell SSR smokes pass for all role × status cells.
- MediaMTX-auth-publish callback rejects bogus nonces, accepts valid ones (integration test).
- Auto-end sweep correctly ends sessions past `duration + 30min` (integration test).
- Manual exit-checklist (deferred per established pattern): teacher publishes from Chrome, student joins via Chrome and Android, screen-share toggles, JWT refreshes, end-class ends cleanly, cross-tenant probe masks foreign sessions.
- Tag `phase-1b-beta-complete` on `phase-0-foundations` after manual checks pass.
