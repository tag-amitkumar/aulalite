# Phase 1.5 — Shell Wiring Design

**Status:** Approved 2026-05-10
**Predecessor:** Phase 1c complete and merged at `cf6327e`.
**Successor unlocks:** Phase 1d (notifications, parent role, TA scoping).

## Overview

The audit at the close of Phase 1c found that while the AulaLite backend is
fully built (248 tests passing across 47 binaries), `shell-web` is mostly a
navigation skeleton: `ApiContext` is never provided in production, every
route below Login uses `placeholder_user()`, and most routes render hardcoded
`vec![]` or "not yet wired" strings. The Phase 1c assignment routes pass
empty UUIDs.

Phase 1.5 is corrective, not additive: it connects the existing components
to the existing backend so the app is actually usable end-to-end. No new
features. The phase also rolls in two carry-over polish items from Phase
1b-γ that are quick wins, and wires `shell-desktop` and a student-focused
`shell-mobile` so the same code reaches all three deployment targets.

## Decisions captured during brainstorm

| # | Decision | Choice |
|---|---|---|
| 1 | Scope | Full critical loop + 1b-γ polish + mobile/desktop wiring. |
| 2 | Routing model | Migrate to `dioxus_router` (real URLs, browser back/forward, deep-linkable). |
| 3 | Auth lifetime | localStorage persistence + Firebase SDK auto-refresh via `getIdToken(true)` on 401. |
| 4 | Mobile/desktop depth | Desktop = thin wrapper around web (full feature set). Mobile = student-focused subset (Login, Dashboard, schedule, assignments view + submit, live session watch). |
| 5 | User identity propagation | Fetch `/v1/me` once at app boot, cache in `Signal<Option<UserContext>>` provided at root. |
| 6 | Error UX | Inline only via existing `error_messages.rs`. 401 from any API call → wrapper signs out, redirects to Login. No toast queue, no global error boundary. |
| 7 | File picker | Wire `file_picker.rs` into `SubmissionForm` only. `AssignmentEditor` reference-attachment picker deferred. |
| 8 | Codebase organization | Split `shell-web/src/main.rs` into `routes/{name}.rs` per route. `main.rs` shrinks to providers + `Router` config. |

## Section A — App-root architecture

Three concerns layered top-to-bottom at the `App` root.

### 1. Auth bootstrap

A new `crates/shell-web/src/auth.rs` module encapsulates Firebase + localStorage:

```rust
pub struct AuthState {
    /// Firebase auth user JS reference (kept opaque via wasm-bindgen).
    pub user: Option<wasm_bindgen::JsValue>,
    /// Currently cached ID token (refreshed lazily).
    pub id_token: String,
}

pub async fn get_fresh_token(user: &wasm_bindgen::JsValue) -> Result<String, JsValue>;
pub fn restore_from_local_storage() -> Option<AuthState>;
pub fn persist_to_local_storage(state: &AuthState);
pub fn sign_out_and_clear();
```

`Login` and `Signup` callbacks are extended to return both the Firebase user
JS object AND the initial ID token (today they return only the token
string). The shell stores the user object so it can call `getIdToken(true)`
at any later point.

### 2. ApiContext provider

`fetch_json` in `crates/features-courses/src/api.rs` gains an interceptor:

- Reads `id_token` from the `ApiContext` provided at root.
- On HTTP 401: calls back into the shell's `get_fresh_token(user)` (via a
  callback registered at boot) to refresh; retries the request once. If the
  refresh itself fails or returns 401 again → calls `sign_out_and_clear()`
  and the router navigates to `/login`.

`ApiContext` lives at the root via `provide_context(...)` so every component
that calls `use_context::<ApiContext>()` sees it.

### 3. UserContext provider

After auth bootstraps, an `App`-level `use_resource` calls `GET /v1/me`. The
result populates a `Signal<Option<UserContext>>` provided at root:

```rust
pub struct UserContext {
    pub user_id: String,
    pub display_name: String,
    pub email: String,
    pub tenant_role: Option<core_types::TenantRole>,
    pub is_platform_admin: bool,
}

impl UserContext {
    pub fn is_teacher(&self) -> bool {
        matches!(self.tenant_role, Some(TenantRole::Teacher | TenantRole::OrgAdmin | TenantRole::Ta))
            || self.is_platform_admin
    }
}
```

Routes read from this context for `is_teacher` flags, `current_user_id`,
header display.

### App tree shape

```
App
└─ <AuthBootstrap>           // restores from localStorage, mounts Login if absent
   └─ <ApiContextProvider>   // exposes ApiContext from auth state
      └─ <UserContextProvider>  // fetches /v1/me, exposes UserContext
         └─ <Router>         // dioxus_router with all 15+ routes
```

### Failure modes

- Token-refresh fails → sign out + redirect to `/login`.
- `/v1/me` returns 401 → same handling as above.
- `/v1/me` returns other errors → render `error_messages.rs` inline; user can
  retry via a Retry button. No global error boundary.

## Section B — URL routes + per-route file structure

`crates/shell-web/src/main.rs` shrinks to ~60 lines: `App` function with the
3 nested providers + `<Router>` config. Route bodies move to per-file
modules under `crates/shell-web/src/routes/`.

### URL structure

```
/                                        → DashboardRoute
/login                                   → LoginRoute
/signup                                  → SignupRoute
/forgot                                  → ForgotRoute
/accept-invite/:token                    → AcceptInviteRoute
/courses                                 → CourseListRoute
/courses/new                             → CourseNewRoute
/courses/:slug                           → CourseDetailRoute (default tab=outline)
/courses/:slug/people                    → CourseDetailRoute (tab=people)
/courses/:slug/edit                      → CourseDetailRoute (tab=edit)
/courses/:slug/schedule                  → CourseDetailRoute (tab=schedule)
/courses/:slug/sessions/:session_id      → LiveSessionRoute
/courses/:slug/assignments               → AssignmentListRoute
/courses/:slug/assignments/new           → AssignmentNewRoute
/courses/:slug/assignments/:id           → AssignmentDetailRoute
/courses/:slug/assignments/:id/edit      → AssignmentEditRoute
/courses/:slug/assignments/:id/grade     → AssignmentGradeRoute
/redeem                                  → RedeemRoute
/schedule                                → MyScheduleRoute
```

### Per-route files

| File | Responsibility |
|---|---|
| `routes/login.rs` | Wraps `features_auth::Login`; on success populates `AuthState` + navigates to `/`. |
| `routes/signup.rs` | Same pattern via `features_auth::Signup`. |
| `routes/forgot.rs` | `features_auth::ForgotPassword` passthrough. |
| `routes/dashboard.rs` | Reads `UserContext`; `use_resource` for `/v1/me/courses` + `/v1/me/schedule`; renders `Dashboard` with real data. |
| `routes/accept_invite.rs` | Calls `POST /v1/invitations/accept` with `:token`; renders `AcceptInvite` with real success/failure. |
| `routes/course_list.rs` | `use_resource` `/v1/courses`; renders `CourseList`. `can_create` derived from `UserContext.is_teacher()`. |
| `routes/course_new.rs` | Real `create_fn` calling `POST /v1/courses`; on success navigates to `/courses/:slug`. |
| `routes/course_detail.rs` | Resolves `:slug` → `CourseRow`; sets `course_id`; tab dispatch nests outline/people/edit/schedule/assignments components. |
| `routes/redeem.rs` | Calls `POST /v1/enrollments/redeem`. |
| `routes/my_schedule.rs` | `use_resource` `/v1/me/schedule`; real entries. |
| `routes/live_session.rs` | The existing `LiveSessionPage` extracted; uses `UserContext` for `caller_role` + `display_name`. |
| `routes/assignments_list.rs` | `course_id` resolved via slug; `is_teacher` from `UserContext`; renders `AssignmentList`. |
| `routes/assignments_new.rs` | `AssignmentEditor { initial: None }`. |
| `routes/assignments_detail.rs` | Fetches assignment; `current_user_id` from `UserContext`. |
| `routes/assignments_edit.rs` | `use_resource(get_assignment)` → `AssignmentEditor { initial: Some(...) }`. |
| `routes/assignments_grade.rs` | `use_resource(get_assignment)` → `SubmissionsGradingTable`. |

Common helpers in `routes/mod.rs`:
- `pub fn use_user_context() -> Signal<Option<UserContext>>` — accessor.
- `pub fn use_course_by_slug(slug: &str) -> Resource<Result<CourseDto, ApiError>>` — resolves slug to row, used by every route under `/courses/:slug`.

### `App` lib export

`crates/shell-web/src/lib.rs` is added so `shell-desktop` can import the
same `App` function. `main.rs` becomes the wasm entrypoint that calls
`dioxus::launch(shell_web::App)`.

## Section C — Mobile shell (student subset)

`shell-mobile/src/main.rs` mirrors the same three-provider stack as web
(auth bootstrap → ApiContext → UserContext) but registers a smaller route
set focused on the student-attending-class flow.

### Mobile routes

```
/                                        → MobileDashboardRoute   (today's classes + recent assignments)
/login                                   → LoginRoute             (reused from shell-web)
/courses/:slug                           → MobileCourseRoute      (read-only, student view)
/courses/:slug/sessions/:session_id      → LiveSessionRoute       (reused — watch class)
/courses/:slug/assignments               → AssignmentListRoute    (reused, is_teacher=false)
/courses/:slug/assignments/:id           → AssignmentDetailRoute  (reused, student-only path)
/schedule                                → MyScheduleRoute        (reused)
```

### Excluded from mobile (teacher-side)

- `/courses/new`, `/courses/:slug/edit`, `/courses/:slug/people`
- `/courses/:slug/assignments/new`, `/.../edit`, `/.../grade`
- `/redeem` (would deep-link from web on tablet; mobile lacks the
  keyboard-friendly UX for it)

### `shell-desktop`

Thin wrapper around `shell_web::App` rendered via Dioxus's native desktop
renderer (`dioxus-desktop`, no separate Tauri integration). Single file
(~30 lines): import `shell_web::App`, launch via `dioxus::desktop::launch`.
No route divergence; teachers running desktop see the full surface.

This requires moving `App` and `routes` out of `shell_web::main` into a new
`shell_web::lib` so desktop can import them. `main.rs` becomes the wasm
entrypoint only.

## Section D — 1b-γ polish, testing, scope cuts

### 1b-γ polish (in-scope)

1. **WhipPublisher leak on Demoted.** In
   `crates/features-courses/src/live_room_view.rs`, store the
   `WhipPublisher` returned by `publish_audio` into a
   `Signal<Option<WhipPublisher>>`. On `ServerEvent::Demoted` for the
   current user, call `publisher.close()` and clear the signal.

2. **WebSocket Error / RateLimited surfacing.** Per the inline-only error
   UX (decision 6), render `Error { code, message }` and
   `RateLimited { retry_after_ms }` events as a small banner at the top of
   the live room view (using `error_messages.rs`), auto-dismissed after 5
   seconds. No separate toast queue.

3. **Redis broker production smoke.** Add a one-shot
   `cargo run -p backend --bin redis_broker_smoke` (small CLI in
   `crates/backend/src/bin/redis_broker_smoke.rs`) that connects to
   `REDIS_URL`, publishes a synthetic `chat` event, subscribes, and asserts
   the message round-trips. Documented in the exit checklist as the
   manual-verification step that closes out 1b-γ §4b.

### Testing strategy

- **shell-web SSR smokes:** for each new route file under `routes/`, render
  with a mocked `ApiContext` and a `UserContext` populated to a known role.
  Assert the route mounts without panic and the loading state renders. The
  pattern mirrors `assignments_ssr.rs` from Phase 1c.
- **shell-web auth-bootstrap unit tests:** test `restore_from_local_storage`
  round-trip, `sign_out_and_clear`, and the 401-retry branch in
  `fetch_json`. Pure helpers where possible.
- **End-to-end manual checklist** in the exit doc covers each route's
  golden path:
  - Sign in → Dashboard shows real courses
  - Click course → outline tab renders real lessons
  - Click Assignments tab → list renders, click one → detail renders
  - Student submits → teacher grades → student sees grade (instant + manual
    release)
  - AcceptInvite link from email actually accepts the invitation
  - Redeem code actually enrolls
- **Backend:** no new tests; backend is already covered.
- **Mobile/desktop:**
  - shell-mobile: unit-test the route enum compiles to the student subset;
    SSR smoke for `MobileDashboardRoute` and `MobileCourseRoute`.
  - shell-desktop: build target check (`cargo build -p shell-desktop`)
    verifies the import-`shell_web::App` path compiles.

### Explicit scope cuts (deferred)

- **Toast queue / global error boundary** — decision 6 chose A
  (inline-only). Toasts can come if a Phase 1d UX pass demands them.
- **Push notifications, parent role, TA scoping** — original Phase 1d.
- **Mobile native publishing** (camera/mic from device) — P1 per scope doc.
- **Internationalization, accessibility audit, design polish** — separate
  cycle.
- **Bulk submission ZIP download** — Phase 1d follow-up.
- **AssignmentEditor reference-attachment picker** — decision 7 deferred
  this; teachers can attach reference materials in a small follow-up.

## Open follow-ups carried into next phase

After 1.5 ships, the only items still deferred from prior phases are
genuinely new features:

- Notifications (in-app/email/push when graded/returned/due-soon).
- Parent role + parent dashboard.
- TA per-course scoping.
- Mobile native publishing.

These remain Phase 1d work.
