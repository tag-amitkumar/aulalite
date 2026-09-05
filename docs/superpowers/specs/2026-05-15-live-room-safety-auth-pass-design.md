# Live-Room Safety + Auth Pass — Design

**Date:** 2026-05-15
**Status:** Approved design; implementation plan pending.
**Source:** Review findings on uncommitted work for the Elite Academy refresh. Bundles three review sub-projects — live-room lifecycle correctness (A), auth surface hardening (B), and error visibility for socket commands (C) — into a single coherent pass.

## Goal

Stop the live-room from leaking RTC peer connections, media tracks, and WebSockets on every Go-Live and route exit; close the auth-surface holes (JWT leakage via URL/logs, empty-token-at-boot in `ApiContext`, broken WHEP playback auth); and make socket-command failures visible instead of swallowed.

## Out of scope

- Asset-build wiring (`design-system` → `shell-web/public` CSS mirroring). Tracked separately.
- `dev_seed` reset cascade into `recordings` / `live_room_messages` / `live_room_kicks`. Tracked separately.
- UI polish completion (skeleton animation, dashboard hero rebuild, copy revision, `--color-live` decorative misuse). Tracked separately.
- WHEP token refresh during long sessions. Deferred — documented as known limitation.
- Spotlight / mute UI for promoted students. The `StudentPublishing` wiring lands viewers in a tile strip; richer controls are a separate feature.
- Real-browser Playwright tests for the leak fix.

## Architecture

```
shell-web::lib.rs::App
└── use_context_provider::<Signal<ApiContext>>(api_signal)     # live signal, not snapshot

api-client::hooks
└── pub fn use_api() -> ApiContext                              # reads Signal under the hood

features-courses::live_room_session   (new)
├── pub struct LiveRoomSession { socket, publisher, viewer, students }
├── impl LiveRoomSession { connect_socket, go_live, attach_main,
│                          attach_student, detach_student, end_class, close }
└── impl Drop                                                   # spawn_local(self.close())

shell-web::routes::live_session::LiveSession
├── use_context_provider::<Signal<LiveRoomSession>>
└── use_on_destroy(|| session.close().await)                   # awaited cleanup
```

**Auth flow (Q1-C: bearer-only client, server accepts either)**

| Path | Client emits | Server accepts |
| --- | --- | --- |
| WebSocket join | `?access_token=<urlencoded jwt>` on the upgrade URL | `?access_token=` (upgrade requests only) + `Authorization: Bearer` |
| WHEP play | `Authorization: Bearer <jwt>` header only | bearer (case-insensitive, whitespace-tolerant) + `?jwt=` query body (for third-party players) |
| HLS play | `?jwt=<jwt>` in URL (existing) | `?jwt=` query body (existing) |

## Backend changes

### `crates/backend/src/auth/middleware.rs`

- New `fn is_websocket_upgrade(headers: &HeaderMap) -> bool` checking `Upgrade: websocket` (case-insensitive) and `Connection: upgrade`.
- `query_access_token` decodes via `percent_encoding::percent_decode_str(...).decode_utf8()`; lossy decode rejects.
- Token resolution order:
  1. `Authorization: Bearer <jwt>` — any request.
  2. `?access_token=<jwt>` — only when `is_websocket_upgrade`.
- `local_login.claims_for_token` lookup moves after the verifier attempt, so a real JWT cannot collide with a local-login token name.

### `crates/backend/src/main.rs` (or trace-wiring location)

- New `crates/backend/src/trace_scrub.rs` exporting a `MakeSpan` impl that scrubs `access_token`, `jwt`, and `token` query parameters from the recorded URI before `tower-http::trace` logs it. Replacement value: `[REDACTED]`.

### `crates/backend/src/handlers/live_sessions.rs`

- `main_url` and `screen_url` no longer append `?jwt=<jwt>` for `transport_mode = "webrtc"`. Client now sends Bearer.
- MediaMTX bearer-strip in the read callback:
  ```rust
  let token = s.get(..7)
      .map(|p| p.eq_ignore_ascii_case("bearer "))
      .unwrap_or(false)
      .then(|| s[7..].trim().to_string())
      .or_else(|| extract_jwt_from_query(&body.query));
  ```
- `messages_inner`: `let limit = q.limit.unwrap_or(50).clamp(1, 200);`
- Window calc using `to_secs`:
  ```rust
  // 4_102_444_800.0 = 2100-01-01 UTC in seconds; chosen well below i64::MAX so
  // chrono::Duration::seconds(MAX_SAFE_SECS as i64) can't overflow.
  const MAX_SAFE_SECS: f64 = 4_102_444_800.0;
  let to = if !to_secs.is_finite() || to_secs > MAX_SAFE_SECS { MAX_UTC } else { ... };
  ```
- The `_ => format!("{public_webrtc_url}/{sp}/whep")` fallback arm in `screen_url` construction is now pure dead code (transport_mode is validated upstream and the only live arm is `"webrtc"`). Remove the arm.
- New `ServerEvent::CommandFailed { command: String, reason: String }`. Every command handler that today does `let _ = …; return;` becomes:
  ```rust
  if let Err(e) = res {
      tracing::warn!(error = %e, %command, "command failed");
      send_to_sender(ServerEvent::CommandFailed { command, reason: e.user_facing() });
      return;
  }
  ```
  Applies to `AcceptHand`, `Chat`, `DeleteMessage`, `Kick`, `DemoteHand`. The `clear_student_publish_nonce` failure must occur *before* the broker emits `Demoted` — if clear fails, emit `CommandFailed`, suppress `Demoted`.
- `HandRaiseChanged`, `StudentPublishing`, `StudentDemoted` events gain `display_name: String` via the existing user-side join in `db::live_room::*`.

### `crates/backend/src/handlers/dev_seed.rs`

- Compute `starts_at` once in `seed()`, thread into both `get_or_create_live_series` and `get_or_create_live_session`. (Full reset cascade is out of scope.)

### `crates/backend/Cargo.toml`

- Add `percent-encoding = "2"`.

### `crates/core-types/src/live_room.rs`

```rust
pub enum ServerEvent {
    // existing variants…
    CommandFailed { command: String, reason: String },
}

pub struct HandRaiseChanged { user_id: Uuid, raised: bool, display_name: String }
pub struct StudentPublishing { user_id: Uuid, publish_path: String, display_name: String }
pub struct StudentDemoted    { user_id: Uuid, display_name: String }

pub mod close_codes {
    pub const AUTH_EXPIRED: u16 = 4001;
    pub const AUTH_INVALID: u16 = 4003;
}
```

Verify `ServerEvent` is not annotated `#[serde(deny_unknown_fields)]`. New variant + new fields are additive.

## Frontend changes

### `crates/api-client/src/lib.rs`

```rust
pub fn use_api() -> ApiContext {
    use_context::<Signal<ApiContext>>().read().clone()
}
```

Hides `Signal<ApiContext>` from call sites.

### `crates/shell-web/src/lib.rs`

- `use_context_provider(|| api_ctx_signal)` (Signal itself, not a snapshot).
- Drop dead guards at lines 69-71 and 94-96 (verified no-op in review).
- Bootstrap logic flattens to a single `match` on `bridge.current_id_token().await`; on `Err`, leave token empty and let `/login` redirect handle it.

### `crates/shell-web/src/routes/live_session.rs`

- Remove the redundant `use_context_provider(|| api.clone())`.
- Wrap `<LiveRoomView>` / `<LiveRoomBroadcast>` inside a `LiveSessionShell` component that:
  - `use_context_provider::<Signal<LiveRoomSession>>(|| Signal::new(LiveRoomSession::new(config, use_api())))`
  - `use_on_destroy(move || spawn_local(async move { session.write().close().await }))`

### `crates/features-courses/src/live_room_session.rs` *(new)*

```rust
pub struct LiveRoomSession {
    config: SessionConfig,
    api: ApiContext,
    socket: Option<LiveRoomSocket>,
    publisher: Option<WhipPublisher>,
    viewer: Option<WhepViewer>,                // student-side view of teacher
    students: HashMap<Uuid, WhepViewer>,       // promoted students
    closed: bool,
}

impl LiveRoomSession {
    pub fn new(config: SessionConfig, api: ApiContext) -> Self;
    pub async fn connect_socket(&mut self, on_event: impl FnMut(ServerEvent)) -> Result<()>;
    pub async fn go_live(&mut self, transport: TransportMode) -> Result<()>;
    pub async fn attach_main(&mut self, url: &str) -> Result<()>;
    pub async fn attach_student(&mut self, user_id: Uuid, path: &str) -> Result<()>;
    pub async fn detach_student(&mut self, user_id: Uuid);
    pub async fn end_class(&mut self) -> Result<()> {
        self.close().await;
        self.api.post(&format!("/v1/sessions/{}/end-class", self.config.session_id)).await?;
        Ok(())
    }
    pub async fn close(&mut self); // idempotent
    pub async fn reconnect_with_fresh_token(&mut self); // Q5 path
}

impl Drop for LiveRoomSession {
    fn drop(&mut self) {
        if !self.closed { /* spawn_local best-effort close */ }
    }
}
```

### `crates/features-courses/src/live_room_socket.rs`

- New `pub fn close(&mut self)` — synchronous `ws.close()`, clears closures.
- `set_onclose(|code|)` wired in `connect()`:
  - Code `4001` → call `LiveRoomSession::reconnect_with_fresh_token()`.
  - Code `4003` → no reconnect; surface to UI.
  - Other codes → `backoff_delay_ms`-driven reconnect, max 5 attempts, cap 30 s.
- URL builder: `format!("?access_token={}", urlencoding::encode(token))`.
- `LiveRoomSocket` implements `Drop` (calls `close`).
- `parse_event` errors → `tracing::debug!` instead of silent swallow.

### `crates/features-courses/src/live_room_whip.rs`

- `WhipPublisher` gains `pub async fn close(&mut self) -> Result<()>`:
  - `pc.close()`.
  - `DELETE resource_url` if present.
  - Stop all `MediaStreamTrack`s.
- Implements `Drop` (best-effort `spawn_local`).
- Replace `Reflect::get(&offer, "sdp")` with typed `offer.dyn_into::<RtcSessionDescription>().sdp()`.

### `crates/features-courses/src/live_room_whep.rs`

- `WhepViewer::close()` mirrors `WhipPublisher::close()` (DELETE on `resource_url`).
- Implements `Drop`.
- Drop the URL-substring `?jwt=` guard; always attach `Authorization: Bearer <jwt>`.

### `crates/features-courses/src/live_room_view.rs`

- `render_webrtc` / `render_hls` become `#[component] WebRtcStage` / `HlsStage`. They consume `Signal<LiveRoomSession>` via context, not their own `use_effect` from plain `fn`.
- Hand-raise handler reads `display_name` from event payload (no more `format!("user-{short}")`).
- Subscribes to `ServerEvent::StudentPublishing` and calls `session.attach_student(user_id, publish_path)`.
- Subscribes to `ServerEvent::CommandFailed` and renders into a small `.system-state--error` toast in the existing right rail.

### `crates/features-courses/src/live_room_broadcast.rs`

- `go_live_flow(&mut LiveRoomSession, transport)` — stores the publisher inside the session, not a local.
- `end_class_flow(&mut LiveRoomSession)` — calls `session.end_class().await` (which closes locally first, then POSTs `/end-class`). Errors surface to the UI.

### Consumers migrating from `use_context::<ApiContext>()` to `use_api()`

- `crates/features-courses/src/lesson_outline_view.rs`
- `crates/features-courses/src/lesson_files_editor.rs`
- `crates/features-courses/src/lesson_video_editor.rs`
- `crates/features-courses/src/file_picker.rs`
- `crates/features-courses/src/file_asset_image.rs`
- `crates/features-courses/src/live_room_replay.rs`
- Any other call site found via `grep -r "use_context::<ApiContext>"`.

A `#[deprecated]` attribute on the bare `ApiContext` context provider makes accidental future use a compile-time warning.

## Protocol additions (summary)

| Change | Direction | Compat |
| --- | --- | --- |
| `ServerEvent::CommandFailed` | server → client | additive variant |
| `display_name` field on hand-raise / publishing / demoted events | server → client | additive field |
| WS close codes 4001 / 4003 | server → client | new convention |

## Test plan

### Backend integration (`crates/backend/tests/`)

- `live_room::whep_bearer_e2e_real_jwt` — drives `join_inner` → uses returned JWT as `Bearer <jwt>` password in `mediamtx_auth_publish_inner`. Asserts 200.
- `live_room::whep_main_url_has_no_jwt_in_query` — join response URLs do not contain `jwt=` or `access_token=`.
- `live_room::mediamtx_read_accepts_lowercase_bearer`.
- `live_room::mediamtx_read_accepts_bearer_with_whitespace`.
- `local_login_bypass::access_token_query_rejected_on_non_upgrade` — `/v1/me?access_token=…` → 401.
- `local_login_bypass::access_token_query_accepted_on_ws_upgrade` — real `tokio_tungstenite` connect with `?access_token=` succeeds (real middleware, not `StubAuth`).
- `local_login_bypass::access_token_url_decoded` — token with `%2B`, `%3D` decodes and authenticates.
- `live_room::command_failure_emits_event` — failed `DemoteHand` emits `CommandFailed` and suppresses `Demoted`.
- `live_room::messages_limit_clamped_to_200`.
- `live_room::messages_inner_rejects_non_finite_to`.
- `live_room::hand_raise_event_includes_display_name`.
- `audit_seed::starts_at_consistent_between_series_and_session`.

### Trace-scrub unit (`crates/backend/src/trace_scrub.rs`)

- `scrubs_access_token_jwt_token_from_uri` — `/v1/x?access_token=secret&foo=1` → `/v1/x?access_token=[REDACTED]&foo=1`.

### Frontend SSR (`crates/features-courses/tests/`)

- `live_room_smoke::renders_command_failed_toast`.
- `live_room_smoke::hand_raise_shows_real_display_name`.

### Frontend logic (`#[cfg(test)]`)

- `live_room_session::close_is_idempotent`.
- `live_room_session::end_class_calls_close_before_post`.
- `live_room_session::build_ws_url_urlencodes_token`.
- `live_room_socket::onclose_4001_triggers_reconnect_with_fresh_token`.
- `live_room_socket::onclose_4003_does_not_reconnect`.

### Test-quality fixes from the review

- `shell-web::tests::dashboard_smoke` — replace the tautological assertion with the strict form.
- `features-courses::tests::assignments_ssr` — drop the OR-form selector check.

## Risk and rollback

| Risk | Mitigation |
| --- | --- |
| Third-party WHEP clients depending on `?jwt=` break when our server-side support is reduced. | Server keeps the query-body `jwt=` path. Only our own client stops emitting it. |
| `CommandFailed` event breaks old clients parsing `ServerEvent`. | Variant is additive. Verify `#[serde(deny_unknown_fields)]` is absent. |
| `use_api()` migration misses a consumer → that consumer still ships an empty token. | Repo-wide grep `use_context::<ApiContext>` must return zero hits after migration. `#[deprecated]` on the bare context surfaces accidents at compile time. |
| `LiveRoomSession::Drop` `spawn_local` doesn't run during page-close (browser kills the task). | `use_on_destroy` awaits cleanup on normal route exit; Drop is the fallback for abnormal-exit, accepted as best-effort. |
| Token refresh reconnect loops if server keeps returning 4001. | Backoff caps at 30 s, gives up after 5 attempts; UI shows a manual "Reconnect" button. |

## Rollout order

1. Backend: middleware + scrub + bearer/case + limit-clamp + f64 guard. Self-contained; existing client still works.
2. Protocol: new event + display_name fields. Additive.
3. Frontend: `use_api()` hook + `Signal<ApiContext>` provider + consumer migration.
4. Frontend: `LiveRoomSession` aggregate + Drop + `close()` on Whip/Whep + Socket Drop.
5. Frontend: route wiring + `use_on_destroy` + `onclose` token-refresh.
6. Frontend: WHIP/WHEP client switches to Bearer-only emit.
7. Frontend: `StudentPublishing` → `attach_student`. Hand-raise reads `display_name`.

Each step independently testable.

## Files created / modified (canonical inventory)

**Created**

- `crates/features-courses/src/live_room_session.rs`
- `crates/backend/src/trace_scrub.rs` (or inline in `main.rs`)
- `crates/api-client/src/hooks.rs` (or extension to `lib.rs`) for `use_api()`
- Test files per the test plan.

**Modified — backend**

- `crates/backend/src/auth/middleware.rs`
- `crates/backend/src/handlers/live_sessions.rs`
- `crates/backend/src/handlers/dev_seed.rs`
- `crates/backend/src/main.rs`
- `crates/backend/Cargo.toml`
- `crates/core-types/src/live_room.rs`

**Modified — frontend**

- `crates/shell-web/src/lib.rs`
- `crates/shell-web/src/routes/live_session.rs`
- `crates/api-client/src/lib.rs`
- `crates/features-courses/src/lib.rs` (module declaration)
- `crates/features-courses/src/live_room_view.rs`
- `crates/features-courses/src/live_room_broadcast.rs`
- `crates/features-courses/src/live_room_socket.rs`
- `crates/features-courses/src/live_room_whip.rs`
- `crates/features-courses/src/live_room_whep.rs`
- `crates/features-courses/src/lesson_outline_view.rs`
- `crates/features-courses/src/lesson_files_editor.rs`
- `crates/features-courses/src/lesson_video_editor.rs`
- `crates/features-courses/src/file_picker.rs`
- `crates/features-courses/src/file_asset_image.rs`
- `crates/features-courses/src/live_room_replay.rs`

**Modified — tests**

- `crates/shell-web/tests/dashboard_smoke.rs`
- `crates/features-courses/tests/assignments_ssr.rs`
- New test files per the plan.
