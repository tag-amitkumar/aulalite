# Local-Login UX Fix + Auth Pass + Real-Stack UI Test — Design

**Date:** 2026-05-16
**Status:** Approved design; implementation plan pending.
**Source:** User-reported regression — clicking any course/route as a Local Teacher or Local Student in the current dev login flow bounces back to `/login`. Bundles three changes into one cohesive pass: (1) the auth-context fix that stops the bounce; (2) a UX change that replaces the "Choose a test profile" panel with an email/password flow driven by `.env` creds; (3) a real-stack Playwright spec that verifies the fix end-to-end.

The deferred SaaS expansion backlog (billing, tenant settings, admin console, analytics, onboarding, notifications, support, audit/compliance) — recorded in `docs/superpowers/specs/2026-05-15-aulalite-saas-expansion-backlog.md` — is **not** in scope here and will be brainstormed separately.

## Goal

1. Make local-dev sign-in work the same way real sign-in works: type an email and password into the existing form, get a session, navigate without being kicked back to `/login`.
2. Land the full auth-surface and live-room-lifecycle pass already designed in `docs/superpowers/specs/2026-05-15-live-room-safety-auth-pass-design.md` so the bounce stops at the root cause, not at the symptom.
3. Replace the existing two Playwright specs (one Firebase-mocked, one compose-driven, both clicking the profile-button UI that goes away) with a single real-stack spec that exercises the new login flow as both Local Teacher and Local Student.

## Out of scope

- The deferred SaaS expansion backlog. Tracked separately.
- Real WebRTC media playback assertions (mediamtx is in the stack but the spec only validates the WebRTC stage renders and the WS handshake completes).
- Mobile gestures, cross-browser runs (chromium-only).
- CI wiring for the Playwright spec. Local-only via script.
- Any change to Firebase email/password flow. The existing path stays as the fallback.

## Architecture

```
features-auth::Login (email/password form)
  │
  ├── (1) POST /v1/auth/local-login { email, password }
  │       └── 200 { id_token } → on_success
  │       └── 4xx → fall through
  │
  └── (2) Firebase sign_in_with_email_password
          └── on_success → on_success(id_token)
          └── error → render Firebase error

on_success(id_token):
  shell-web::routes::login::Login
    └── api_signal.set(ApiContext { id_token })  // Signal, not snapshot
        └── consumers everywhere read via use_api()
            └── token is live; no stale snapshot kicks them to /login
```

The two existing routes `/v1/dev/login` and `/v1/dev/login/config` are removed entirely. Nothing else in the codebase consumes them after the profile-button panel is gone. `/v1/dev/audit-seed` stays — it computes profiles server-side without calling the dev-login HTTP endpoint.

The full live-room safety + auth pass from the 2026-05-15 design doc lands in the same project because the auth-context fix (`Signal<ApiContext>` + `use_api()`) is step 3 of that design's rollout. We follow that design verbatim with two small adjustments noted below.

## Backend changes

### `crates/backend/src/auth/local_login.rs`

- `LocalLoginProfile` gains `pub password: String`.
- `LocalLoginConfig::from_env` reads:
  - `LOCAL_LOGIN_TEACHER_PASSWORD` into the teacher profile
  - `LOCAL_LOGIN_STUDENT_PASSWORD` into the student profile
- `is_complete_profile` requires `!profile.password.trim().is_empty()`. A profile without a password is not a valid login target.
- New method:

  ```rust
  pub fn profile_for_email_password(&self, email: &str, password: &str)
      -> Option<&LocalLoginProfile>;
  ```

  Returns the matching complete profile when both fields match (case-insensitive email compare, exact password compare). `None` otherwise. Returns `None` when `is_enabled()` is false.

### `crates/backend/src/handlers/dev_login.rs`

Replace the entire module. The new shape:

- Route: `POST /v1/auth/local-login`
- Request body: `{ "email": string, "password": string }`
- Response 200: `{ "id_token": string }` — the per-profile token already accepted by the auth middleware via `local_login.claims_for_token`.
- Errors:
  - `400` — missing or empty email/password field
  - `401` — no matching profile (same status for wrong email and wrong password, no information leak)
  - `403` — local login disabled (`APP_ENV=production` or `LOCAL_LOGIN_BYPASS_ENABLED` not truthy)

On success, `ensure_user` runs with the matched profile's claims, mirroring the current `/v1/dev/login` behavior. The token returned is the same per-profile token already in `.env`.

The two old routes (`POST /v1/dev/login`, `GET /v1/dev/login/config`) are deleted. `crates/backend/src/lib.rs` route wiring is updated; nothing else in-tree calls them.

### Backend tests — `crates/backend/tests/local_login_bypass.rs`

Additions:

- `local_login_email_password_grants_teacher_token` — POST with the teacher env email + teacher env password returns 200, the returned token equals `LOCAL_LOGIN_TEACHER_TOKEN`, and a subsequent `/v1/me` request with that token returns the teacher profile.
- `local_login_email_password_grants_student_token` — same shape for the student profile.
- `local_login_wrong_password_returns_401` — correct email, wrong password returns 401, body does not contain any token-shaped string.
- `local_login_unknown_email_returns_401` — random email returns 401.
- `local_login_missing_password_field_returns_400`.
- `local_login_when_disabled_returns_403` — with `LOCAL_LOGIN_BYPASS_ENABLED=false` (or `APP_ENV=production`) the endpoint returns 403 regardless of body content.
- `local_login_case_insensitive_email_matches` — `LOCAL.TEACHER@EXAMPLE.TEST` matches the lowercase env value.

### `.env` and `.env.example` additions

```
LOCAL_LOGIN_TEACHER_PASSWORD=local-teacher-pass
LOCAL_LOGIN_STUDENT_PASSWORD=local-student-pass
```

Both files updated. Existing `LOCAL_LOGIN_*` variables stay.

### Live-room safety pass — backend portion

Follows `docs/superpowers/specs/2026-05-15-live-room-safety-auth-pass-design.md` verbatim:

- `crates/backend/src/auth/middleware.rs` — `is_websocket_upgrade`, percent-decoded `query_access_token`, token resolution order (Bearer header → `?access_token=` only on WS upgrades), `local_login.claims_for_token` lookup moves after the JWT verifier attempt.
- `crates/backend/src/trace_scrub.rs` (new) — `MakeSpan` impl that scrubs `access_token`, `jwt`, `token` query params from the recorded URI before `tower-http::trace` logs it. Wired in `main.rs`.
- `crates/backend/src/handlers/live_sessions.rs` — drop `?jwt=` from `main_url` / `screen_url`; MediaMTX bearer-strip with case/whitespace tolerance plus query-body `jwt=` fallback; `messages_inner` clamps `limit` to `1..=200`; `f64`-finite + `MAX_SAFE_SECS = 4_102_444_800.0` guard for `to_secs`; drop the dead `_ => format!("{public_webrtc_url}/{sp}/whep")` arm; every silent `let _ = …; return;` becomes `tracing::warn!` + `ServerEvent::CommandFailed`; `clear_student_publish_nonce` failure suppresses the broker's `Demoted` emission.
- `crates/backend/src/handlers/dev_seed.rs` — `starts_at` computed once in `seed()`, threaded into both `get_or_create_live_series` and `get_or_create_live_session`.
- `crates/core-types/src/live_room.rs`:
  - `ServerEvent::CommandFailed { command: String, reason: String }` — additive variant. Verify `#[serde(deny_unknown_fields)]` is absent.
  - `display_name: String` on `HandRaiseChanged`, `StudentPublishing`, `StudentDemoted`. Server-side populated via the existing user-side join in `db::live_room::*`.
  - `pub mod close_codes { pub const AUTH_EXPIRED: u16 = 4001; pub const AUTH_INVALID: u16 = 4003; }`
- `crates/backend/Cargo.toml` — add `percent-encoding = "2"`.

**Adjustment 1 (vs. the 2026-05-15 design):** the design said `local_login.claims_for_token` lookup moves after the verifier; this still holds. The new `/v1/auth/local-login` endpoint produces tokens that the middleware accepts via that same `claims_for_token` path, so no middleware change is needed beyond what the 2026-05-15 design already specifies.

## Frontend changes

### `crates/api-client/src/lib.rs`

- New `api::local_login(ctx: &ApiContext, email: &str, password: &str) -> Result<TokenResponse>` — POST `/v1/auth/local-login`. `Result::Err` on any non-200 status (with the HTTP status preserved so the caller can distinguish 400 from 401 if useful).
- Remove `api::dev_login` and `api::get_dev_login_config`.
- Add `pub fn use_api() -> ApiContext { use_context::<Signal<ApiContext>>().read().clone() }` (per the 2026-05-15 design).

### `crates/features-auth/src/login.rs`

Form markup unchanged (same email field, password field, "Sign in" button, "auth-heading-block" structure).

Submit handler changes to a two-step attempt:

```text
1. let local_attempt = api::local_login(&api_ctx, &email, &password).await
2. match local_attempt:
     Ok(token) → on_success(token.id_token); return.
     Err(_)    → fall through to step 3.
3. firebase::sign_in_with_email_password(&email, &password).await
     Ok(id_token) → on_success(id_token)
     Err(e)       → render Firebase error message in the form
```

The local-login attempt is silent. No UI indicator. A Firebase user whose email happens to match a local-login profile will fail the password check and fall through to Firebase normally (this is acceptable — local-login is only enabled in dev and uses fixed `.env` passwords, not user passwords).

### `crates/shell-web/src/routes/login.rs`

Drop the entire `dev_login_config` resource, the `auth-local-panel` `<section>`, the profile button loop, and the `api::dev_login` call. The route becomes a thin shell:

```rust
rsx! {
    div { class: "auth-composite motion-page",
        section { class: "auth-hero-visual",
            div { class: "auth-hero-copy",
                AulaLogo { class: "auth-hero-logo".to_string(), compact: false }
                p { class: "auth-hero-kicker", "Elite live learning" }
                h2 { "A workspace built for modern academies." }
                p { "Run courses, live rooms, assignments, and schedules with the polish students expect." }
            }
        }
        div { class: "auth-panel-stack",
            features_auth::Login { on_success: on_success }
        }
    }
}
```

`on_success` still calls `api_signal.set(…)` and routes to `Dashboard`. With `api_signal` now provided as a `Signal<ApiContext>` (per the 2026-05-15 design's frontend section), every downstream consumer reads the live token instead of the snapshot it captured at first render. This is the fix for the reported bounce.

### Live-room safety pass — frontend portion

Follows `docs/superpowers/specs/2026-05-15-live-room-safety-auth-pass-design.md` verbatim:

- `crates/shell-web/src/lib.rs` — `use_context_provider(|| api_ctx_signal)` (the Signal itself, not a snapshot). Drop dead guards at lines 69-71 and 94-96. Flatten bootstrap to a single match on `bridge.current_id_token().await`; on `Err`, leave token empty and let `/login` redirect handle it.
- `crates/shell-web/src/routes/live_session.rs` — wrap `LiveRoomView` / `LiveRoomBroadcast` inside a `LiveSessionShell` that provides `Signal<LiveRoomSession>` and registers `use_on_destroy` to await `session.close()`.
- `crates/features-courses/src/live_room_session.rs` (new) — aggregate of socket + publisher + viewer + students; `connect_socket`, `go_live`, `attach_main`, `attach_student`, `detach_student`, `end_class`, `close` (idempotent), `reconnect_with_fresh_token`; `impl Drop` does `spawn_local` best-effort close.
- `crates/features-courses/src/live_room_socket.rs` — synchronous `close()`, `Drop`, `onclose` handler dispatching on close code (4001 → `reconnect_with_fresh_token`, 4003 → surface to UI without retry, other → backoff-driven reconnect capped at 5 attempts and 30s), URL-encoded `access_token` query param, `parse_event` errors `tracing::debug!` instead of silent swallow.
- `crates/features-courses/src/live_room_whip.rs` — `close()` does `pc.close()` + `DELETE resource_url` + stop all tracks; `Drop` spawns best-effort close. Replace `Reflect::get(&offer, "sdp")` with typed `offer.dyn_into::<RtcSessionDescription>().sdp()`.
- `crates/features-courses/src/live_room_whep.rs` — mirror `close()` and `Drop`; drop the `?jwt=` URL-substring guard; always attach `Authorization: Bearer`.
- `crates/features-courses/src/live_room_view.rs` — `WebRtcStage` / `HlsStage` components reading `Signal<LiveRoomSession>` from context; hand-raise reads `display_name` from event payload; subscribes to `StudentPublishing` → `session.attach_student(user_id, publish_path)` and to `CommandFailed` → small `.system-state--error` toast in the existing right rail.
- `crates/features-courses/src/live_room_broadcast.rs` — `go_live_flow(&mut LiveRoomSession, transport)` stores the publisher inside the session, not a local; `end_class_flow(&mut LiveRoomSession)` calls `session.end_class().await` which closes locally before POSTing `/end-class`. Errors surface to the UI.

**Adjustment 2 (vs. the 2026-05-15 design):** `crates/shell-web/src/routes/login.rs` is in *both* this design (Section 3 — drop the profile panel) and the 2026-05-15 design (Signal<ApiContext> migration of its `use_context` site). One file, two unrelated diffs. Sequencing: the panel-drop happens with Section 2's endpoint; the `use_api()` migration happens with the live-room frontend pass. Either order works on this file.

Consumer migration from `use_context::<ApiContext>()` to `use_api()` across `features-courses` (lesson_outline_view, lesson_files_editor, lesson_video_editor, file_picker, file_asset_image, live_room_replay, and any other call site found via repo-wide grep) lands as part of the live-room frontend pass. `#[deprecated]` attribute on the bare `ApiContext` provider ensures accidental future use surfaces as a compile-time warning.

### Frontend tests — `crates/features-auth/tests/login.rs`

Additions:

- `submit_calls_local_login_first` — mock API client returning 200 from `/v1/auth/local-login`; assert `on_success` fires with the returned token and the Firebase bridge is never invoked.
- `submit_falls_back_to_firebase_on_local_login_401` — mock 401 from local-login, mock Firebase success; assert `on_success` fires with the Firebase token.
- `submit_surfaces_firebase_error_when_both_fail` — mock 401 from local-login, mock Firebase error; assert the error message renders.

The existing branded-heading assertions (`auth-screen`, `auth-heading-block`, `AulaLite`) stay.

### Frontend tests — `crates/shell-web/tests/dashboard_smoke.rs` (existing)

The 2026-05-15 design's tightening of the tautological assertion still applies.

### Frontend tests — `crates/features-courses/tests/`

Per the 2026-05-15 design: `live_room_smoke::renders_command_failed_toast`, `live_room_smoke::hand_raise_shows_real_display_name`, drop OR-form selector in `assignments_ssr`.

## Playwright spec design

### Files

Delete:

- `tools/playwright-ui-check.spec.js`
- `tools/playwright-compose-ui-check.spec.js`

Create:

- `tools/ui-real-stack.spec.js` — the single spec.
- `tools/run-ui-check.ps1` — orchestration script.
- `tools/README.md` — ~20-line operator doc (prereqs, how to run, output location, env vars).

### Script — `tools/run-ui-check.ps1`

Sequence:

1. `docker compose up -d` — backend, db, redis, minio, mediamtx.
2. Poll `http://localhost:8080/healthz` until 200 (fail after 60s).
3. `pushd crates/shell-web; Start-Process dx -ArgumentList 'serve','--platform','web','--port','3000'; popd` — background process.
4. Poll `http://localhost:3000` until it responds.
5. `npx playwright test tools/ui-real-stack.spec.js`
6. On failure: leave compose + dx serve running (so the user can poke); exit non-zero.
7. On success: leave compose + dx serve running unless `-Clean` flag passed.

The script reads `LOCAL_LOGIN_EMAIL`, `LOCAL_LOGIN_TEACHER_PASSWORD`, `LOCAL_LOGIN_STUDENT_EMAIL`, `LOCAL_LOGIN_STUDENT_PASSWORD` from `.env` (using a small parser) and forwards them to the spec via `$env:`.

### Spec — `tools/ui-real-stack.spec.js`

```text
describe.configure({ mode: "serial" })

beforeAll:
  - POST /v1/dev/audit-seed → { course_slug, ... }
  - GET /v1/courses/<course_id>/sessions (as teacher via direct local-login API call)
  - PATCH the first session's starts_at to now-60s, status "scheduled" so the live-room is hot
  - Read env vars into module-scope constants

test "teacher walks the workspace":
  - page.goto('/login')
  - fill[name=email] with LOCAL_LOGIN_EMAIL
  - fill[name=password] with LOCAL_LOGIN_TEACHER_PASSWORD
  - click "Sign in"
  - expect '.dashboard-hero' visible within 15s
  - for each route in TEACHER_ROUTES:
      - navigate
      - expect selector visible
      - expect body to contain text
      - assertStyleHealth (overflow, bad bounds, motion-page animation)
      - screenshot
  - probe: visit /courses/<slug>, then /, then /courses/<slug> again
      - assert no /login redirect occurred at any point
  - probe live-room WS: open /courses/<slug>/sessions/<id>, click "Go Live",
      assert a WebSocket request to /v1/sessions/<id>/socket?access_token=… is logged
  - probe WS cleanup: navigate away from the live-room route,
      assert the WebSocket is closed within 2s
  - sign out via the app shell control
  - expect '/login' visible

test "student walks the visible subset":
  - same login flow with student creds
  - for each route in STUDENT_ROUTES (subset of teacher routes; no editor surfaces):
      - same per-route assertions
  - assert teacher-only controls are absent (no "Go Live", no "New Course", no
      "New Assignment" buttons on the visible pages)

afterAll: nothing — script controls teardown.
```

TEACHER_ROUTES: `/`, `/courses`, `/courses/<slug>`, `/courses/<slug>/people`, `/courses/<slug>/schedule`, `/courses/<slug>/assignments`, `/courses/<slug>/assignments/<id>`, `/redeem`, `/schedule`, `/courses/<slug>/sessions/<id>`.

STUDENT_ROUTES: `/`, `/courses`, `/courses/<slug>`, `/courses/<slug>/assignments`, `/courses/<slug>/assignments/<id>`, `/redeem`, `/schedule`, `/courses/<slug>/sessions/<id>`.

### Per-route assertions

For every route in both flows:

- `expect(locator(selector).first()).toBeVisible({ timeout: 15000 })`
- `expect(locator('body')).toContainText(expectedText)`
- Horizontal overflow ≤ 2px
- No visible UI elements with `left < -2` or `right > viewportWidth + 2` (existing style-health helper).
- `.motion-page` (when present) has `animationName` containing `page-in`.
- Screenshot written to `target/playwright-ui/<role>-<route-safe-name>.png`.
- Unfiltered console errors empty (favicon, ResizeObserver loop, Firebase init noise filtered).

### Served-CSS contract check

Once per run: GET `/assets/components.css` and assert it contains `.dashboard-hero`, `.course-people`, `.workflow-page`, `.system-state`, `.schedule-agenda`, `.live-room-stage`. Catches CSS-not-mirrored regressions.

### Things this spec explicitly does not cover

- Real WebRTC media. The stage element rendering and the WS handshake are validated; actual frames are not.
- Mobile gestures.
- Cross-browser (chromium only).
- The `auth-local-panel` / profile-button flow (removed in this design).

## File inventory

### Created

- `crates/backend/src/trace_scrub.rs` (per 2026-05-15 design).
- `crates/features-courses/src/live_room_session.rs` (per 2026-05-15 design).
- `crates/api-client/src/hooks.rs` *or* extension to `lib.rs` for `use_api()` (per 2026-05-15 design).
- `tools/ui-real-stack.spec.js`
- `tools/run-ui-check.ps1`
- `tools/README.md`

### Modified — backend

- `crates/backend/src/auth/local_login.rs` — `password` field + `profile_for_email_password` method.
- `crates/backend/src/handlers/dev_login.rs` — module replaced; new `POST /v1/auth/local-login` only.
- `crates/backend/src/lib.rs` — route wiring updated (old dev_login routes removed, new local-login route added).
- `crates/backend/src/main.rs` — trace-scrub wiring (per 2026-05-15 design).
- `crates/backend/src/auth/middleware.rs` — per 2026-05-15 design.
- `crates/backend/src/handlers/live_sessions.rs` — per 2026-05-15 design.
- `crates/backend/src/handlers/dev_seed.rs` — per 2026-05-15 design.
- `crates/backend/Cargo.toml` — `percent-encoding = "2"`.
- `crates/core-types/src/live_room.rs` — per 2026-05-15 design.

### Modified — frontend

- `crates/api-client/src/lib.rs` — `local_login` added, `dev_login` + `get_dev_login_config` removed, `use_api()` exposed.
- `crates/features-auth/src/login.rs` — submit handler tries local-login first, falls back to Firebase.
- `crates/shell-web/src/routes/login.rs` — `auth-local-panel` removed; thin shell remains.
- `crates/shell-web/src/lib.rs` — `Signal<ApiContext>` provider; dead guards dropped.
- `crates/shell-web/src/routes/live_session.rs` — `LiveSessionShell` wrapper.
- `crates/features-courses/src/lib.rs` — module declaration for `live_room_session`.
- `crates/features-courses/src/live_room_view.rs` — per 2026-05-15 design.
- `crates/features-courses/src/live_room_broadcast.rs` — per 2026-05-15 design.
- `crates/features-courses/src/live_room_socket.rs` — per 2026-05-15 design.
- `crates/features-courses/src/live_room_whip.rs` — per 2026-05-15 design.
- `crates/features-courses/src/live_room_whep.rs` — per 2026-05-15 design.
- Consumer migration: `lesson_outline_view.rs`, `lesson_files_editor.rs`, `lesson_video_editor.rs`, `file_picker.rs`, `file_asset_image.rs`, `live_room_replay.rs`, plus any other site found via repo-wide grep for `use_context::<ApiContext>`.

### Modified — tests

- `crates/backend/tests/local_login_bypass.rs` — six new tests.
- `crates/features-auth/tests/login.rs` — three new tests.
- `crates/shell-web/tests/dashboard_smoke.rs` — tightened assertion (per 2026-05-15 design).
- `crates/features-courses/tests/assignments_ssr.rs` — OR-form selector dropped (per 2026-05-15 design).
- `crates/features-courses/tests/live_room_smoke.rs` — two new tests (per 2026-05-15 design).

### Modified — config

- `.env` — two new variables.
- `.env.example` — two new variables.

### Deleted

- `tools/playwright-ui-check.spec.js`
- `tools/playwright-compose-ui-check.spec.js`

## Rollout order

Each step is independently shippable.

1. **Backend local-login endpoint.** Add `password` to `LocalLoginProfile`, ship `POST /v1/auth/local-login`, write backend tests. The old `/v1/dev/login` routes stay for now so the existing UI keeps working. Existing tests pass.
2. **Frontend form change.** Update `features_auth::Login` submit handler to try local-login first, fall back to Firebase. Existing UI panel still renders the profile buttons — both paths work.
3. **Drop the profile panel.** Remove `auth-local-panel` from `shell-web::routes::login::Login`. Remove `api::dev_login` / `api::get_dev_login_config`. Remove the old backend routes. Only the new email/password path remains.
4. **Live-room safety pass — backend.** Middleware + trace-scrub + bearer/case + limit-clamp + f64 guard. Self-contained.
5. **Live-room safety pass — protocol.** `CommandFailed` event + `display_name` fields. Additive.
6. **Live-room safety pass — frontend Signal/use_api migration.** `Signal<ApiContext>` provider + `use_api()` hook + consumer migration. This is the step that stops the `/login` bounce at the root cause.
7. **Live-room safety pass — session aggregate + lifecycle.** `LiveRoomSession` + `Drop` + `close()` on Whip/Whep + Socket Drop. Wires `use_on_destroy` cleanup.
8. **Live-room safety pass — client auth switch.** WHIP/WHEP emit Bearer only. WS uses URL-encoded `access_token` query param. Onclose handles 4001/4003.
9. **Live-room safety pass — event subscription.** `StudentPublishing` → `attach_student`. Hand-raise reads `display_name`. `CommandFailed` toast.
10. **Real-stack Playwright spec.** Delete the two existing specs. Create `tools/ui-real-stack.spec.js`, `tools/run-ui-check.ps1`, `tools/README.md`. Run locally end-to-end. Both teacher and student flows pass.

## Risk and rollback

| Risk | Mitigation |
| --- | --- |
| A Firebase user happens to have an email that matches a `.env` local profile. | Local-login is gated to non-production (`APP_ENV != "production"`); production never serves this endpoint. In dev, the password check will fail for that user and the form falls through to Firebase, which is the intended behavior. |
| Removing `/v1/dev/login` breaks an external script or operator workflow. | Repo-wide grep before removal confirms zero in-tree callers. External callers (if any exist in operator scripts) get a one-line migration note in `tools/README.md`. |
| The `Signal<ApiContext>` migration misses a consumer; that consumer ships an empty token. | Repo-wide grep `use_context::<ApiContext>` must return zero hits after step 6. `#[deprecated]` on the bare `ApiContext` provider surfaces accidents at compile time. |
| Third-party WHEP clients depend on `?jwt=` in the URL when our client stops emitting it. | Server keeps the query-body `jwt=` path. Only our own client stops emitting it. |
| `CommandFailed` event variant breaks old clients parsing `ServerEvent`. | Variant is additive. `#[serde(deny_unknown_fields)]` confirmed absent. |
| `LiveRoomSession::Drop` `spawn_local` does not run during page-close. | `use_on_destroy` awaits cleanup on normal route exit; Drop is the fallback for abnormal exit, accepted as best-effort. |
| Playwright spec is flaky on slow machines because `dx serve` takes >60s. | Script polls `http://localhost:3000` with an explicit timeout and a clear error message. Operator can raise the timeout via env var. |
| The new endpoint's 401 leaks timing information about which emails are configured. | Match check returns `None` from the same code path regardless of whether email or password mismatched. Acceptable for local-dev; not a production endpoint. |

## Test plan summary

### Backend

- `tests/local_login_bypass.rs`: six new tests (grants_teacher, grants_student, wrong_password, unknown_email, missing_password, disabled, case_insensitive_email).
- `tests/live_room.rs`: per 2026-05-15 design.
- `tests/audit_seed.rs`: `starts_at_consistent_between_series_and_session`.
- `src/trace_scrub.rs`: unit test `scrubs_access_token_jwt_token_from_uri`.

### Frontend

- `features-auth/tests/login.rs`: three new tests.
- `features-courses/tests/live_room_smoke.rs`: per 2026-05-15 design.
- `features-courses/tests/assignments_ssr.rs`: assertion tightening.
- `shell-web/tests/dashboard_smoke.rs`: assertion tightening.
- `features-courses/src/live_room_session.rs` (`#[cfg(test)]`): per 2026-05-15 design.
- `features-courses/src/live_room_socket.rs` (`#[cfg(test)]`): per 2026-05-15 design.

### End-to-end (Playwright)

- `tools/ui-real-stack.spec.js`: two tests (teacher walk, student walk), each covering every applicable route plus the bounce-probe and live-room WS lifecycle probe.

## Relation to the 2026-05-15 design

This design supersedes nothing. It adds:

- The local-login endpoint and UX change (sections "Backend changes" and "Frontend changes" above, parts not derived from the 2026-05-15 design).
- The real-stack Playwright spec.

It incorporates by reference:

- The entirety of `docs/superpowers/specs/2026-05-15-live-room-safety-auth-pass-design.md` for the auth-pass and live-room lifecycle work.

When the implementation plan is written, items from the 2026-05-15 design appear in the plan with their source noted; items in this design appear directly.
