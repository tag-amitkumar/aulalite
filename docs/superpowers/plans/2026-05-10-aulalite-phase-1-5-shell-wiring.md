# Phase 1.5 — Shell Wiring Implementation Plan

> **Historical plan:** its Dioxus 0.7.4 instructions describe the version in
> use when this plan was written. They are superseded; active workspace, CLI,
> Docker, CI, and release configuration use 0.7.9.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Connect the existing `features-courses` and `features-auth` components to the existing backend so every shell route actually fetches real data, replace `placeholder_user()` with real `/v1/me`, and migrate `shell-web` to `dioxus_router` with per-route files. Land 1b-γ polish (WhipPublisher leak fix, inline WebSocket error banner, redis broker smoke binary). Build out `shell-desktop` (thin wrapper) and `shell-mobile` (student subset).

**Architecture:** Three-provider stack at the App root (auth bootstrap → ApiContext → UserContext), then a `dioxus_router::Router`. Auth uses the existing `platform_bridge::PlatformBridge::current_id_token()` for both initial bootstrap and 401-retry refresh — Firebase JS SDK already persists auth state in IndexedDB, so no Rust-side localStorage is needed. Routes split into `crates/shell-web/src/routes/<name>.rs`, one file per route. `shell_web::App` is exported from a new `lib.rs` so `shell-desktop` can import the same App.

**Tech Stack:** Dioxus 0.7.4, `dioxus-router` 0.7, `wasm-bindgen-futures`, existing `platform_bridge` for Firebase, existing `features-courses::api::fetch_json` extended with a 401-retry interceptor.

**Predecessor:** Phase 1c complete and merged to main at `cf6327e`. Spec at `docs/superpowers/specs/2026-05-10-aulalite-phase-1-5-shell-wiring-design.md` (`4842c10`).

**Spec deviation (corrected here):** The spec described a new `crates/shell-web/src/auth.rs` module with custom localStorage persistence and `wasm_bindgen::JsValue` user references. Inspection of the existing codebase shows `platform_bridge::PlatformBridge` already exposes `current_id_token()`, `sign_in_email_password()`, `sign_out()`, etc., and the Firebase JS SDK on the JS side persists auth state automatically. The plan uses `PlatformBridge` directly, which is simpler and consistent with how `features-auth` already works. The architectural intent of the spec (root-provided ApiContext + UserContext, 401-retry) is preserved.

---

## File Structure

**Workspace (modify):**
- `Cargo.toml` — add `dioxus-router = "=0.7.4"` to `[workspace.dependencies]`.

**`crates/shell-web` (restructure):**
- Modify: `crates/shell-web/Cargo.toml` — add `dioxus-router`, `platform-bridge`, `wasm-bindgen-futures`, `gloo-timers`.
- Create: `crates/shell-web/src/lib.rs` — exports `App`, the route enum, the providers.
- Replace: `crates/shell-web/src/main.rs` — shrinks to `dioxus::launch(shell_web::App)`.
- Create: `crates/shell-web/src/contexts.rs` — `UserContext` struct + provider helper.
- Create: `crates/shell-web/src/route_enum.rs` — the `dioxus_router` `Routable` enum.
- Create: `crates/shell-web/src/routes/mod.rs` — common helpers (`use_user_context`, `use_course_by_slug`).
- Create: `crates/shell-web/src/routes/login.rs`
- Create: `crates/shell-web/src/routes/signup.rs`
- Create: `crates/shell-web/src/routes/forgot.rs`
- Create: `crates/shell-web/src/routes/dashboard.rs`
- Create: `crates/shell-web/src/routes/accept_invite.rs`
- Create: `crates/shell-web/src/routes/course_list.rs`
- Create: `crates/shell-web/src/routes/course_new.rs`
- Create: `crates/shell-web/src/routes/course_detail.rs`
- Create: `crates/shell-web/src/routes/redeem.rs`
- Create: `crates/shell-web/src/routes/my_schedule.rs`
- Create: `crates/shell-web/src/routes/live_session.rs`
- Create: `crates/shell-web/src/routes/assignments_list.rs`
- Create: `crates/shell-web/src/routes/assignments_new.rs`
- Create: `crates/shell-web/src/routes/assignments_detail.rs`
- Create: `crates/shell-web/src/routes/assignments_edit.rs`
- Create: `crates/shell-web/src/routes/assignments_grade.rs`
- Create: `crates/shell-web/tests/shell_routes_smoke.rs`

**`crates/features-courses` (modify):**
- Modify: `crates/features-courses/src/api.rs` — add 401-retry interceptor inside `web_impl::fetch_json`. Add `UserDto` (matches `/v1/me`).
- Modify: `crates/features-courses/src/submission_form.rs` — replace the `<p>` placeholder with a real `file_picker.rs` invocation.
- Modify: `crates/features-courses/src/live_room_view.rs` — store `WhipPublisher` on Promoted; close on Demoted; render inline banner for `Error`/`RateLimited` events.

**`crates/backend` (add):**
- Create: `crates/backend/src/bin/redis_broker_smoke.rs` — one-shot CLI for 1b-γ §4b.

**`crates/shell-desktop` (rewrite):**
- Replace: `crates/shell-desktop/src/main.rs` — calls `dioxus::desktop::launch(shell_web::App)`.
- Modify: `crates/shell-desktop/Cargo.toml` — depend on `shell-web`, swap web feature for `desktop`.

**`crates/shell-mobile` (rewrite):**
- Replace: `crates/shell-mobile/src/main.rs` — same provider stack as shell-web but with the student-subset `Routable` enum.
- Create: `crates/shell-mobile/src/route_enum.rs` — student-subset routes.
- Modify: `crates/shell-mobile/Cargo.toml` — add `dioxus-router`, `platform-bridge`, depend on `shell-web` for shared `routes/*` modules.

**Plan output:**
- Create: `docs/superpowers/plans/2026-05-10-aulalite-phase-1-5-shell-wiring-exit-checklist.md`

---

## Task Sequencing

Tasks 1-3 set up the workspace dep + 401-retry + ApiContext/UserContext providers. Tasks 4-5 stand up the new lib.rs scaffold and a single working route (Login). Tasks 6-19 land routes in dependency order. Task 20 fixes the SubmissionForm picker. Tasks 21-23 land the 1b-γ polish items. Tasks 24-25 wire shell-desktop and shell-mobile. Task 26 adds SSR smokes. Task 27 sweeps builds. Task 28 ships the exit checklist.

---

### Task 1: Add `dioxus-router` workspace dep

**Files:**
- Modify: `Cargo.toml`
- Modify: `crates/shell-web/Cargo.toml`

- [ ] **Step 1: Add to workspace deps**

In root `Cargo.toml` `[workspace.dependencies]`, add after the existing `dioxus = "=0.7.4"` line:

```toml
dioxus-router = { version = "=0.7.4", features = ["web"] }
```

- [ ] **Step 2: Add shell-web deps**

Modify `crates/shell-web/Cargo.toml`. The current `[dependencies]` block:

```toml
[dependencies]
dioxus = { workspace = true, features = ["web"] }
features-auth = { path = "../features-auth" }
features-courses = { path = "../features-courses" }
design-system = { path = "../design-system" }
core-types = { path = "../core-types" }
serde = { workspace = true }
```

Replace with:

```toml
[dependencies]
dioxus = { workspace = true, features = ["web"] }
dioxus-router = { workspace = true }
features-auth = { path = "../features-auth" }
features-courses = { path = "../features-courses" }
design-system = { path = "../design-system" }
core-types = { path = "../core-types" }
platform-bridge = { path = "../platform-bridge" }
serde = { workspace = true }
serde_json = { workspace = true }
wasm-bindgen = "0.2"
wasm-bindgen-futures = "0.4"
gloo-timers = { version = "0.3", features = ["futures"] }
web-sys = { version = "0.3", features = ["Window", "Storage"] }
```

- [ ] **Step 3: Verify workspace compiles (nothing changed yet but the dep resolves)**

```bash
cargo build -p shell-web --target wasm32-unknown-unknown 2>&1 | tail -5
```

Expected: clean (the new deps are unused so far; warnings are OK).

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml crates/shell-web/Cargo.toml
git commit -m "chore(shell-web): add dioxus-router and platform-bridge deps"
```

---

### Task 2: 401-retry interceptor in `fetch_json`

**Files:**
- Modify: `crates/features-courses/src/api.rs`

This is the single change that makes the existing `ApiContext` actually self-healing on token expiry. We use `platform_bridge::PlatformBridge::current_id_token()` to refresh and retry once on HTTP 401.

- [ ] **Step 1: Add a `RefreshFn` callback type**

Above the `web_impl` module in `crates/features-courses/src/api.rs`, add:

```rust
/// Callback the shell registers at boot. On 401, fetch_json calls this to
/// get a fresh token. If this returns Err the wrapper signs the user out.
pub type RefreshFn = std::sync::Arc<
    dyn Fn() -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<String, String>>>,
    >,
>;

thread_local! {
    static REFRESH: std::cell::RefCell<Option<RefreshFn>> = std::cell::RefCell::new(None);
}

pub fn set_refresh_fn(f: RefreshFn) {
    REFRESH.with(|r| *r.borrow_mut() = Some(f));
}

fn try_refresh() -> Option<RefreshFn> {
    REFRESH.with(|r| r.borrow().clone())
}
```

- [ ] **Step 2: Modify `web_impl::fetch_json` to retry on 401**

Find the existing `web_impl::fetch_json` function. The current shape is:

```rust
pub async fn fetch_json<T: DeserializeOwned>(
    cx: &ApiContext,
    method: &str,
    path: &str,
    body: Option<&(impl Serialize + ?Sized)>,
) -> Result<T, ApiError> {
    // ... build request, call window.fetch, parse response
}
```

Refactor so that the request-building + send is a private helper, and `fetch_json` retries once on 401. Replace the contents of `web_impl::fetch_json` (and add the helper) so the structure is:

```rust
async fn do_fetch<T: DeserializeOwned>(
    cx: &ApiContext,
    method: &str,
    path: &str,
    body_str: Option<String>,
) -> Result<T, ApiError> {
    // <-- exact same code that was previously inline in fetch_json,
    //     but takes body_str: Option<String> instead of a generic Serialize ref
    let window = web_sys::window().ok_or_else(|| ApiError::Network("no window".into()))?;
    let opts = web_sys::RequestInit::new();
    opts.set_method(method);
    if let Some(b) = body_str {
        opts.set_body(&b.into());
    }
    let url = format!("{}{}", cx.base_url, path);
    let req = web_sys::Request::new_with_str_and_init(&url, &opts)
        .map_err(|e| ApiError::Network(format!("{e:?}")))?;
    let headers = req.headers();
    let _ = headers.set("content-type", "application/json");
    if !cx.id_token.is_empty() {
        let _ = headers.set("authorization", &format!("Bearer {}", cx.id_token));
    }
    let resp_value = JsFuture::from(window.fetch_with_request(&req))
        .await
        .map_err(|e| ApiError::Network(format!("{e:?}")))?;
    let resp: web_sys::Response = resp_value.dyn_into()
        .map_err(|_| ApiError::Network("bad response type".into()))?;
    let status = resp.status();
    let text_promise = resp.text()
        .map_err(|e| ApiError::Network(format!("{e:?}")))?;
    let text_value = JsFuture::from(text_promise)
        .await
        .map_err(|e| ApiError::Network(format!("{e:?}")))?;
    let body_text = text_value.as_string().unwrap_or_default();
    if !(200..300).contains(&status) {
        return Err(ApiError::Status(status, body_text));
    }
    serde_json::from_str::<T>(&body_text).map_err(|e| ApiError::Decode(e.to_string()))
}

pub async fn fetch_json<T: DeserializeOwned>(
    cx: &ApiContext,
    method: &str,
    path: &str,
    body: Option<&(impl Serialize + ?Sized)>,
) -> Result<T, ApiError> {
    let body_str = match body {
        Some(b) => Some(serde_json::to_string(b).map_err(|e| ApiError::Decode(e.to_string()))?),
        None => None,
    };

    // First attempt with whatever token ApiContext currently holds.
    match do_fetch::<T>(cx, method, path, body_str.clone()).await {
        Err(ApiError::Status(401, _)) => {
            // Try once to refresh the token and retry. If no refresher is
            // registered, surface the 401 as-is.
            let refresher = match super::try_refresh() {
                Some(f) => f,
                None => return Err(ApiError::Status(401, "unauthorized".into())),
            };
            let new_token = refresher().await
                .map_err(|e| ApiError::Status(401, e))?;
            let retried_cx = ApiContext { base_url: cx.base_url.clone(), id_token: new_token };
            do_fetch::<T>(&retried_cx, method, path, body_str).await
        }
        other => other,
    }
}
```

(The exact signature of the Request/Response building above must match what's already in `crates/features-courses/src/api.rs::web_impl::fetch_json` — copy that code verbatim into `do_fetch`. The only structural changes are: factor out the existing body into `do_fetch(body_str: Option<String>)`, and wrap with the 401-retry.)

- [ ] **Step 3: Build for both targets**

```bash
cargo build -p features-courses 2>&1 | tail -5
cargo build -p features-courses --target wasm32-unknown-unknown 2>&1 | tail -5
```

Expected: clean (warnings OK).

- [ ] **Step 4: Commit**

```bash
git add crates/features-courses/src/api.rs
git commit -m "feat(api): 401-retry interceptor for fetch_json with shell-registered refresh callback"
```

---

### Task 3: `UserContext` + `UserDto`

**Files:**
- Modify: `crates/features-courses/src/api.rs` (add `UserDto` + `get_me` fetcher)
- Create: `crates/shell-web/src/contexts.rs`

- [ ] **Step 1: Add `UserDto` and `get_me` to api.rs**

Append to `crates/features-courses/src/api.rs`:

```rust
// --- Phase 1.5: /v1/me ---

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct UserDto {
    pub user_id: String,
    pub display_name: Option<String>,
    pub email: String,
    pub firebase_uid: String,
    pub tenant_id: Option<String>,
    pub tenant_role: Option<String>,
    pub is_platform_admin: bool,
}

pub async fn get_me(ctx: &ApiContext) -> Result<UserDto, ApiError> {
    fetch_json(ctx, "GET", "/v1/me", None::<&()>).await
}
```

(Verify the field names match the actual `/v1/me` JSON response — quickly:
```bash
grep -A20 "fn me" crates/backend/src/handlers/me.rs | head -30
```
Adjust `UserDto` field names to match the backend's actual response. Most likely the backend already returns these exact names because the `RequestContext` struct uses them.)

- [ ] **Step 2: Create `crates/shell-web/src/contexts.rs`**

```rust
// crates/shell-web/src/contexts.rs
use core_types::TenantRole;
use dioxus::prelude::*;
use features_courses::api::UserDto;

#[derive(Clone, Debug, PartialEq)]
pub struct UserContext {
    pub user_id: String,
    pub display_name: String,
    pub email: String,
    pub tenant_role: Option<TenantRole>,
    pub is_platform_admin: bool,
}

impl UserContext {
    pub fn is_teacher(&self) -> bool {
        matches!(
            self.tenant_role,
            Some(TenantRole::Teacher) | Some(TenantRole::OrgAdmin) | Some(TenantRole::Ta)
        ) || self.is_platform_admin
    }

    pub fn from_dto(dto: UserDto) -> Self {
        let role = dto.tenant_role.as_deref().and_then(parse_role);
        Self {
            user_id: dto.user_id,
            display_name: dto.display_name.unwrap_or_else(|| dto.email.clone()),
            email: dto.email,
            tenant_role: role,
            is_platform_admin: dto.is_platform_admin,
        }
    }
}

fn parse_role(s: &str) -> Option<TenantRole> {
    match s {
        "org_admin" => Some(TenantRole::OrgAdmin),
        "teacher" => Some(TenantRole::Teacher),
        "ta" => Some(TenantRole::Ta),
        "student" => Some(TenantRole::Student),
        "parent" => Some(TenantRole::Parent),
        _ => None,
    }
}

/// Provided at the App root by `UserContextProvider`. Routes read via
/// `use_context::<Signal<Option<UserContext>>>()`.
pub type UserContextSignal = Signal<Option<UserContext>>;
```

- [ ] **Step 3: Build**

```bash
cargo build -p shell-web --target wasm32-unknown-unknown 2>&1 | tail -5
```

(`contexts.rs` won't be wired in yet — but the type compiles.)

- [ ] **Step 4: Commit**

```bash
git add crates/features-courses/src/api.rs crates/shell-web/src/contexts.rs
git commit -m "feat(shell-web): UserDto from /v1/me + UserContext type"
```

---

### Task 4: shell-web/src/lib.rs scaffold + dioxus_router enum

**Files:**
- Create: `crates/shell-web/src/lib.rs`
- Create: `crates/shell-web/src/route_enum.rs`
- Create: `crates/shell-web/src/routes/mod.rs`
- Replace: `crates/shell-web/src/main.rs`

The existing `main.rs` (286 lines) gets replaced wholesale. The new shape:
- `lib.rs` exports `App` and supporting types.
- `main.rs` becomes a 4-line wasm entrypoint.

- [ ] **Step 1: Create `crates/shell-web/src/route_enum.rs`**

```rust
// crates/shell-web/src/route_enum.rs
use dioxus::prelude::*;
use dioxus_router::prelude::*;

use crate::routes;

#[rustfmt::skip]
#[derive(Routable, Clone, PartialEq, Eq)]
pub enum Route {
    #[route("/login")]
    Login {},
    #[route("/signup")]
    Signup {},
    #[route("/forgot")]
    Forgot {},
    #[route("/")]
    Dashboard {},
    #[route("/accept-invite/:token")]
    AcceptInvite { token: String },
    #[route("/courses")]
    CourseList {},
    #[route("/courses/new")]
    CourseNew {},
    #[route("/courses/:slug")]
    CourseDetail { slug: String },
    #[route("/courses/:slug/people")]
    CoursePeople { slug: String },
    #[route("/courses/:slug/edit")]
    CourseEdit { slug: String },
    #[route("/courses/:slug/schedule")]
    CourseSchedule { slug: String },
    #[route("/courses/:slug/sessions/:session_id")]
    LiveSession { slug: String, session_id: String },
    #[route("/courses/:slug/assignments")]
    AssignmentList { slug: String },
    #[route("/courses/:slug/assignments/new")]
    AssignmentNew { slug: String },
    #[route("/courses/:slug/assignments/:id")]
    AssignmentDetail { slug: String, id: String },
    #[route("/courses/:slug/assignments/:id/edit")]
    AssignmentEdit { slug: String, id: String },
    #[route("/courses/:slug/assignments/:id/grade")]
    AssignmentGrade { slug: String, id: String },
    #[route("/redeem")]
    Redeem {},
    #[route("/schedule")]
    MySchedule {},
}
```

- [ ] **Step 2: Create `crates/shell-web/src/routes/mod.rs` (empty stub for now)**

```rust
// crates/shell-web/src/routes/mod.rs
//! Per-route components. Each file owns one `Routable` variant.

use dioxus::prelude::*;
use features_courses::api::ApiContext;

use crate::contexts::{UserContext, UserContextSignal};

pub mod login;
// Subsequent route modules (signup, forgot, dashboard, ...) are added in
// later tasks. Module declarations are appended as each task lands.

/// Shorthand to read the signed-in user from context (None until /v1/me resolves).
pub fn use_user_context() -> UserContextSignal {
    use_context::<UserContextSignal>()
}

/// Shorthand to read ApiContext.
pub fn use_api() -> ApiContext {
    use_context::<ApiContext>()
}
```

- [ ] **Step 3: Create a minimal `crates/shell-web/src/routes/login.rs`** (stub that compiles)

```rust
// crates/shell-web/src/routes/login.rs
use dioxus::prelude::*;
use dioxus_router::prelude::*;

use crate::route_enum::Route;

#[component]
pub fn Login() -> Element {
    let nav = use_navigator();
    rsx! {
        features_auth::Login {
            on_success: move |_token: String| {
                nav.push(Route::Dashboard {});
            },
        }
    }
}
```

(This stub doesn't yet update ApiContext from the token — Task 5 wires the providers around it. We just need the route enum + at least one Element-returning component to compile dioxus-router macros.)

- [ ] **Step 4: Create `crates/shell-web/src/lib.rs`**

```rust
// crates/shell-web/src/lib.rs
//! Web shell crate. The same `App` is launched by `main.rs` (wasm) and
//! re-exported for `shell-desktop`.

pub mod contexts;
pub mod route_enum;
pub mod routes;

use dioxus::prelude::*;
use dioxus_router::prelude::*;

use crate::contexts::UserContextSignal;
use features_courses::api::ApiContext;

#[component]
pub fn App() -> Element {
    // Phase 1.5 Tasks 5-7 add real auth/UserContext bootstrap here.
    // For now, provide an empty ApiContext + None UserContext so the router
    // mounts and route stubs can compile.
    use_context_provider(|| ApiContext { base_url: String::new(), id_token: String::new() });
    use_context_provider::<UserContextSignal>(|| Signal::new(None));

    rsx! {
        Router::<route_enum::Route> {}
    }
}
```

- [ ] **Step 5: Replace `crates/shell-web/src/main.rs` with the wasm entrypoint**

```rust
// crates/shell-web/src/main.rs
fn main() {
    dioxus::launch(shell_web::App);
}
```

- [ ] **Step 6: Build**

```bash
cargo build -p shell-web --target wasm32-unknown-unknown 2>&1 | tail -10
```

Expected: clean. The router will compile because every `Route` variant maps to a component (only `Login` exists; the other variants will fail at link time only if rendered. dioxus_router's `Routable` derive generates `Element`-returning fns by looking up `routes::Login`, etc. — so we need stubs for ALL variants, or use a `#[redirect]` to login).

If the build fails because dioxus_router can't find `routes::Signup`, `routes::Forgot`, etc., add temporary stubs in `routes/mod.rs`:

```rust
pub use stubs::*;

mod stubs {
    use dioxus::prelude::*;
    macro_rules! stub_route {
        ($name:ident) => {
            #[component]
            pub fn $name() -> Element { rsx! { p { "Not yet wired" } } }
        };
        ($name:ident { $($arg:ident: $ty:ty),* }) => {
            #[component]
            pub fn $name($($arg: $ty),*) -> Element { rsx! { p { "Not yet wired" } } }
        };
    }

    stub_route!(Signup);
    stub_route!(Forgot);
    stub_route!(Dashboard);
    stub_route!(AcceptInvite { token: String });
    stub_route!(CourseList);
    stub_route!(CourseNew);
    stub_route!(CourseDetail { slug: String });
    stub_route!(CoursePeople { slug: String });
    stub_route!(CourseEdit { slug: String });
    stub_route!(CourseSchedule { slug: String });
    stub_route!(LiveSession { slug: String, session_id: String });
    stub_route!(AssignmentList { slug: String });
    stub_route!(AssignmentNew { slug: String });
    stub_route!(AssignmentDetail { slug: String, id: String });
    stub_route!(AssignmentEdit { slug: String, id: String });
    stub_route!(AssignmentGrade { slug: String, id: String });
    stub_route!(Redeem);
    stub_route!(MySchedule);
}
```

These stubs are deleted incrementally as Tasks 6-19 land their real implementations.

- [ ] **Step 7: Build again and commit**

```bash
cargo build -p shell-web --target wasm32-unknown-unknown 2>&1 | tail -5
git add crates/shell-web/src/lib.rs crates/shell-web/src/main.rs \
        crates/shell-web/src/route_enum.rs crates/shell-web/src/routes/
git commit -m "feat(shell-web): migrate to dioxus_router + lib.rs scaffold"
```

---

### Task 5: Auth bootstrap + ApiContext + UserContext providers

**Files:**
- Modify: `crates/shell-web/src/lib.rs`

This is the single highest-leverage task: wire `PlatformBridge` into the App boot, populate `ApiContext` with a real token, register the `RefreshFn` so 401-retry works, fetch `/v1/me`, populate `UserContextSignal`. After this task, `Route::Dashboard` (still a stub) at least has a real authenticated context.

- [ ] **Step 1: Replace the body of `crates/shell-web/src/lib.rs::App`**

```rust
// crates/shell-web/src/lib.rs
//! Web shell crate. The same `App` is launched by `main.rs` (wasm) and
//! re-exported for `shell-desktop`.

pub mod contexts;
pub mod route_enum;
pub mod routes;

use dioxus::prelude::*;
use dioxus_router::prelude::*;
use std::sync::Arc;

use crate::contexts::{UserContext, UserContextSignal};
use features_courses::api::{self, ApiContext};
use platform_bridge::PlatformBridge;

#[component]
pub fn App() -> Element {
    // Auth + ApiContext: read the current Firebase ID token at boot.
    let mut api_ctx = use_signal(|| ApiContext { base_url: String::new(), id_token: String::new() });
    let mut user_ctx: UserContextSignal = use_signal(|| None);
    let mut bootstrapped = use_signal(|| false);

    // Register the 401-retry refresher exactly once.
    use_hook(|| {
        let refresh: api::RefreshFn = Arc::new(|| {
            Box::pin(async move {
                #[cfg(target_arch = "wasm32")]
                {
                    let bridge = platform_bridge::web::WebBridge;
                    bridge.current_id_token().await.map_err(|e| e.to_string())
                }
                #[cfg(not(target_arch = "wasm32"))]
                {
                    Err("not on wasm32".to_string())
                }
            })
        });
        api::set_refresh_fn(refresh);
    });

    // On first render, bootstrap: try to fetch a token from the bridge
    // (Firebase JS SDK persists state in IndexedDB, so a returning user is
    // still signed in here). If we get one, populate ApiContext and fetch /v1/me.
    use_future(move || async move {
        if *bootstrapped.read() {
            return;
        }
        bootstrapped.set(true);

        #[cfg(target_arch = "wasm32")]
        {
            let bridge = platform_bridge::web::WebBridge;
            match bridge.current_id_token().await {
                Ok(token) => {
                    api_ctx.set(ApiContext { base_url: String::new(), id_token: token });
                    let cx_for_me = api_ctx.read().clone();
                    match api::get_me(&cx_for_me).await {
                        Ok(dto) => user_ctx.set(Some(UserContext::from_dto(dto))),
                        Err(_) => {
                            // Token was probably stale or /v1/me failed.
                            // Leave api_ctx populated; let the router send to /login.
                            user_ctx.set(None);
                        }
                    }
                }
                Err(_) => {
                    // Not signed in. ApiContext stays empty; router shows /login.
                    user_ctx.set(None);
                }
            }
        }
    });

    // Provide both contexts to the rest of the tree. Note we provide the
    // CURRENT VALUE of api_ctx — when Login completes, the on_success handler
    // updates `api_ctx` and components re-render with the new token.
    let cx = api_ctx.read().clone();
    use_context_provider(|| cx);
    use_context_provider::<UserContextSignal>(|| user_ctx);
    use_context_provider::<Signal<ApiContext>>(|| api_ctx);

    rsx! {
        Router::<route_enum::Route> {}
    }
}
```

- [ ] **Step 2: Update `routes/mod.rs::use_api`** to read from the live `Signal<ApiContext>`

Replace the existing helpers in `crates/shell-web/src/routes/mod.rs`:

```rust
pub fn use_api() -> ApiContext {
    use_context::<Signal<ApiContext>>().read().clone()
}
```

- [ ] **Step 3: Build**

```bash
cargo build -p shell-web --target wasm32-unknown-unknown 2>&1 | tail -10
```

Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add crates/shell-web/src/lib.rs crates/shell-web/src/routes/mod.rs
git commit -m "feat(shell-web): App-root auth bootstrap + ApiContext/UserContext providers + 401-retry"
```

---

### Task 6: routes/login.rs (real)

**Files:**
- Modify: `crates/shell-web/src/routes/login.rs`

`features_auth::Login::on_success` returns the freshly-minted ID token as a String. We push it into the live `Signal<ApiContext>` and trigger a `/v1/me` fetch by cloning the signal in the App.

- [ ] **Step 1: Replace `crates/shell-web/src/routes/login.rs`**

```rust
// crates/shell-web/src/routes/login.rs
use dioxus::prelude::*;
use dioxus_router::prelude::*;
use features_courses::api::{self, ApiContext};

use crate::contexts::{UserContext, UserContextSignal};
use crate::route_enum::Route;

#[component]
pub fn Login() -> Element {
    let nav = use_navigator();
    let mut api_signal = use_context::<Signal<ApiContext>>();
    let mut user_signal = use_context::<UserContextSignal>();

    let on_success = move |token: String| {
        api_signal.set(ApiContext {
            base_url: String::new(),
            id_token: token,
        });
        let api_ctx = api_signal.read().clone();
        spawn(async move {
            if let Ok(dto) = api::get_me(&api_ctx).await {
                user_signal.set(Some(UserContext::from_dto(dto)));
            }
            nav.push(Route::Dashboard {});
        });
    };

    rsx! {
        features_auth::Login { on_success: on_success }
    }
}
```

- [ ] **Step 2: Remove the `Login` stub from `routes/mod.rs::stubs`** (keep the others until their tasks land):

In `crates/shell-web/src/routes/mod.rs::stubs`, remove the `stub_route!(Login)` line. The real `Login` component now lives in `routes/login.rs` (which is `pub mod login;` already declared).

- [ ] **Step 3: Build**

```bash
cargo build -p shell-web --target wasm32-unknown-unknown 2>&1 | tail -5
```

Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add crates/shell-web/src/routes/login.rs crates/shell-web/src/routes/mod.rs
git commit -m "feat(shell-web): wire Login route — populate ApiContext + UserContext on success"
```

---

### Task 7: routes/signup.rs + routes/forgot.rs

**Files:**
- Create: `crates/shell-web/src/routes/signup.rs`
- Create: `crates/shell-web/src/routes/forgot.rs`
- Modify: `crates/shell-web/src/routes/mod.rs`

- [ ] **Step 1: Create `crates/shell-web/src/routes/signup.rs`**

```rust
// crates/shell-web/src/routes/signup.rs
use dioxus::prelude::*;
use dioxus_router::prelude::*;
use features_courses::api::{self, ApiContext};

use crate::contexts::{UserContext, UserContextSignal};
use crate::route_enum::Route;

#[component]
pub fn Signup() -> Element {
    let nav = use_navigator();
    let mut api_signal = use_context::<Signal<ApiContext>>();
    let mut user_signal = use_context::<UserContextSignal>();

    let on_success = move |token: String| {
        api_signal.set(ApiContext { base_url: String::new(), id_token: token });
        let api_ctx = api_signal.read().clone();
        spawn(async move {
            if let Ok(dto) = api::get_me(&api_ctx).await {
                user_signal.set(Some(UserContext::from_dto(dto)));
            }
            nav.push(Route::Dashboard {});
        });
    };

    rsx! {
        features_auth::Signup { on_success: on_success }
    }
}
```

- [ ] **Step 2: Create `crates/shell-web/src/routes/forgot.rs`**

```rust
// crates/shell-web/src/routes/forgot.rs
use dioxus::prelude::*;

#[component]
pub fn Forgot() -> Element {
    rsx! {
        features_auth::ForgotPassword {}
    }
}
```

- [ ] **Step 3: Wire modules in `routes/mod.rs`**

Add after `pub mod login;`:
```rust
pub mod signup;
pub mod forgot;
```

Remove `stub_route!(Signup);` and `stub_route!(Forgot);` from the stubs block.

Add to the top of `mod.rs` (so the macro generates non-conflicting fn names — the real ones live in their submodules and the dioxus_router derive resolves them via `routes::Signup`, `routes::Forgot`, etc.):

```rust
pub use signup::Signup;
pub use forgot::Forgot;
pub use login::Login;
```

(Add `pub use login::Login;` from Task 6 if not already present.)

- [ ] **Step 4: Build + commit**

```bash
cargo build -p shell-web --target wasm32-unknown-unknown 2>&1 | tail -5
git add crates/shell-web/src/routes/signup.rs crates/shell-web/src/routes/forgot.rs \
        crates/shell-web/src/routes/mod.rs
git commit -m "feat(shell-web): wire Signup + Forgot routes"
```

---

### Task 8: routes/dashboard.rs

**Files:**
- Create: `crates/shell-web/src/routes/dashboard.rs`
- Modify: `crates/shell-web/src/routes/mod.rs`

The Dashboard fetches `/v1/me/courses` and `/v1/me/schedule` and renders the existing `Dashboard` component with real data wrapped in `AppShell`.

- [ ] **Step 1: Add lookup helpers to api.rs** (if missing)

Verify `crates/features-courses/src/api.rs` has `list_my_courses` and `list_my_schedule`:
```bash
grep -n "list_my_\|my_courses\|my_schedule" crates/features-courses/src/api.rs
```

If not, append:
```rust
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct MyCourseDto {
    pub id: String,
    pub slug: String,
    pub title: String,
    pub status: String,
    pub description: Option<String>,
    pub owner_user_id: String,
    pub cover_asset_id: Option<String>,
}

pub async fn list_my_courses(ctx: &ApiContext) -> Result<Vec<MyCourseDto>, ApiError> {
    fetch_json(ctx, "GET", "/v1/me/courses", None::<&()>).await
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct MyScheduleEntry {
    pub session_id: String,
    pub course_slug: String,
    pub course_title: String,
    pub starts_at: String,
    pub ends_at: String,
    pub state: String,
}

pub async fn list_my_schedule(ctx: &ApiContext) -> Result<Vec<MyScheduleEntry>, ApiError> {
    fetch_json(ctx, "GET", "/v1/me/schedule", None::<&()>).await
}
```

- [ ] **Step 2: Create `crates/shell-web/src/routes/dashboard.rs`**

```rust
// crates/shell-web/src/routes/dashboard.rs
use dioxus::prelude::*;
use dioxus_router::prelude::*;
use features_courses::api::{self};
use features_courses::app_shell::{AppShell, ShellUser};
use features_courses::dashboard::Dashboard as DashboardView;

use crate::route_enum::Route;
use crate::routes::{use_api, use_user_context};

#[component]
pub fn Dashboard() -> Element {
    let nav = use_navigator();
    let api = use_api();
    let user_ctx = use_user_context();

    if user_ctx.read().is_none() {
        // Not signed in (or still bootstrapping). Send to /login.
        nav.push(Route::Login {});
        return rsx! { p { "Redirecting…" } };
    }

    let courses_resource = use_resource({
        let api = api.clone();
        move || {
            let api = api.clone();
            async move { api::list_my_courses(&api).await }
        }
    });
    let schedule_resource = use_resource({
        let api = api.clone();
        move || {
            let api = api.clone();
            async move { api::list_my_schedule(&api).await }
        }
    });

    let user = match user_ctx.read().as_ref() {
        Some(u) => ShellUser {
            display_name: u.display_name.clone(),
            email: u.email.clone(),
            tenant_role: u.tenant_role,
            is_platform_admin: u.is_platform_admin,
        },
        None => return rsx! { p { "Loading…" } },
    };

    let display_name = user.display_name.clone();
    let courses = courses_resource.read_unchecked().clone();
    let schedule = schedule_resource.read_unchecked().clone();

    rsx! {
        AppShell {
            user: user,
            on_signout: move |_| {
                #[cfg(target_arch = "wasm32")]
                spawn(async move {
                    let _ = platform_bridge::web::WebBridge.sign_out().await;
                });
                nav.push(Route::Login {});
            },
            DashboardView {
                display_name: display_name,
                courses: match &courses {
                    Some(Ok(cs)) => cs.iter().map(|c| features_courses::dashboard::CourseCard {
                        id: c.id.clone(),
                        slug: c.slug.clone(),
                        title: c.title.clone(),
                        status: c.status.clone(),
                    }).collect(),
                    _ => vec![],
                },
                upcoming_count: match &schedule {
                    Some(Ok(s)) => s.len(),
                    _ => 0,
                },
            }
        }
    }
}
```

(Note: `Dashboard` from `features_courses::dashboard` may have a different prop shape than what's shown above. Open it (`grep -A20 "pub fn Dashboard\|DashboardProps" crates/features-courses/src/dashboard.rs`) and adapt the prop names. The point is: real `use_resource` calls populate the props with real data, replacing the previous hardcoded `vec![]` and `0usize`.)

- [ ] **Step 3: Wire and build**

In `crates/shell-web/src/routes/mod.rs` add:
```rust
pub mod dashboard;
pub use dashboard::Dashboard;
```
And remove `stub_route!(Dashboard);` from the stubs block.

```bash
cargo build -p shell-web --target wasm32-unknown-unknown 2>&1 | tail -10
```

Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add crates/shell-web/src/routes/dashboard.rs \
        crates/shell-web/src/routes/mod.rs \
        crates/features-courses/src/api.rs
git commit -m "feat(shell-web): wire Dashboard — real /v1/me/courses + /v1/me/schedule + signout"
```

---

### Task 9: routes/course_list.rs + routes/course_new.rs

**Files:**
- Create: `crates/shell-web/src/routes/course_list.rs`
- Create: `crates/shell-web/src/routes/course_new.rs`
- Modify: `crates/shell-web/src/routes/mod.rs`

- [ ] **Step 1: Verify api.rs has `list_courses` and `create_course`**

```bash
grep -n "list_courses\|create_course\|fn courses" crates/features-courses/src/api.rs
```

If missing, append:
```rust
pub async fn list_courses(ctx: &ApiContext) -> Result<Vec<MyCourseDto>, ApiError> {
    fetch_json(ctx, "GET", "/v1/courses", None::<&()>).await
}

#[derive(serde::Serialize)]
pub struct CreateCourseBody<'a> {
    pub title: &'a str,
    pub description: Option<&'a str>,
}

pub async fn create_course(
    ctx: &ApiContext, body: &CreateCourseBody<'_>,
) -> Result<MyCourseDto, ApiError> {
    fetch_json(ctx, "POST", "/v1/courses", Some(body)).await
}
```

- [ ] **Step 2: Create `crates/shell-web/src/routes/course_list.rs`**

```rust
// crates/shell-web/src/routes/course_list.rs
use dioxus::prelude::*;
use dioxus_router::prelude::*;
use features_courses::api;
use features_courses::app_shell::{AppShell, ShellUser};
use features_courses::course_list::{CourseList as CourseListView, CourseListItem};

use crate::route_enum::Route;
use crate::routes::{use_api, use_user_context};

#[component]
pub fn CourseList() -> Element {
    let nav = use_navigator();
    let api = use_api();
    let user_ctx = use_user_context();

    let user_snapshot = user_ctx.read().clone();
    let user = match user_snapshot {
        Some(u) => u,
        None => { nav.push(Route::Login {}); return rsx! { p { "Redirecting…" } }; }
    };
    let can_create = user.is_teacher();

    let courses = use_resource({
        let api = api.clone();
        move || {
            let api = api.clone();
            async move { api::list_courses(&api).await }
        }
    });

    let items: Vec<CourseListItem> = match &*courses.read_unchecked() {
        Some(Ok(rows)) => rows.iter().map(|c| CourseListItem {
            id: c.id.clone(),
            slug: c.slug.clone(),
            title: c.title.clone(),
            status: c.status.clone(),
            description: c.description.clone(),
            owner_user_id: c.owner_user_id.clone(),
            cover_asset_id: c.cover_asset_id.clone(),
        }).collect(),
        _ => vec![],
    };

    let shell_user = ShellUser {
        display_name: user.display_name.clone(),
        email: user.email.clone(),
        tenant_role: user.tenant_role,
        is_platform_admin: user.is_platform_admin,
    };

    rsx! {
        AppShell {
            user: shell_user,
            on_signout: move |_| {
                #[cfg(target_arch = "wasm32")]
                spawn(async move { let _ = platform_bridge::web::WebBridge.sign_out().await; });
                nav.push(Route::Login {});
            },
            CourseListView {
                courses: items,
                can_create: can_create,
                on_create_clicked: move |_| { nav.push(Route::CourseNew {}); },
            }
        }
    }
}
```

- [ ] **Step 3: Create `crates/shell-web/src/routes/course_new.rs`**

```rust
// crates/shell-web/src/routes/course_new.rs
use dioxus::prelude::*;
use dioxus_router::prelude::*;
use features_courses::api::{self, CreateCourseBody};
use features_courses::app_shell::{AppShell, ShellUser};
use features_courses::course_create::CourseCreate;

use crate::route_enum::Route;
use crate::routes::{use_api, use_user_context};

#[component]
pub fn CourseNew() -> Element {
    let nav = use_navigator();
    let api = use_api();
    let user_ctx = use_user_context();

    let user = match user_ctx.read().clone() {
        Some(u) => u,
        None => { nav.push(Route::Login {}); return rsx! { p { "Redirecting…" } }; }
    };

    let create_fn = move |(title, description, cb): (
        String,
        String,
        EventHandler<Result<String, String>>,
    )| {
        let api = api.clone();
        spawn(async move {
            let body = CreateCourseBody {
                title: &title,
                description: if description.is_empty() { None } else { Some(&description) },
            };
            match api::create_course(&api, &body).await {
                Ok(c) => cb.call(Ok(c.slug)),
                Err(e) => cb.call(Err(format!("{e}"))),
            }
        });
    };

    let shell_user = ShellUser {
        display_name: user.display_name.clone(),
        email: user.email.clone(),
        tenant_role: user.tenant_role,
        is_platform_admin: user.is_platform_admin,
    };

    rsx! {
        AppShell {
            user: shell_user,
            on_signout: move |_| {
                #[cfg(target_arch = "wasm32")]
                spawn(async move { let _ = platform_bridge::web::WebBridge.sign_out().await; });
                nav.push(Route::Login {});
            },
            CourseCreate {
                on_created: move |slug: String| { nav.push(Route::CourseDetail { slug }); },
                create_fn: create_fn,
            }
        }
    }
}
```

(Note: the `CourseCreate` prop signature might use `EventHandler<(String, String, EventHandler<Result<String, String>>)>` or a slightly different shape — inspect `crates/features-courses/src/course_create.rs` first and match the actual signature.)

- [ ] **Step 4: Wire in routes/mod.rs and build**

```rust
pub mod course_list;
pub mod course_new;
pub use course_list::CourseList;
pub use course_new::CourseNew;
```

Remove `stub_route!(CourseList);` and `stub_route!(CourseNew);`.

```bash
cargo build -p shell-web --target wasm32-unknown-unknown 2>&1 | tail -5
```

- [ ] **Step 5: Commit**

```bash
git add crates/shell-web/src/routes/course_list.rs \
        crates/shell-web/src/routes/course_new.rs \
        crates/shell-web/src/routes/mod.rs \
        crates/features-courses/src/api.rs
git commit -m "feat(shell-web): wire CourseList + CourseNew (real list + real POST /v1/courses)"
```

---

### Task 10: routes/course_detail.rs (with tab dispatch)

**Files:**
- Create: `crates/shell-web/src/routes/course_detail.rs`
- Modify: `crates/shell-web/src/routes/mod.rs`

The 4 sub-routes (`/courses/:slug`, `/.../people`, `/.../edit`, `/.../schedule`) all dispatch to one `CourseDetail` component, varying only the `active_tab` prop. Each sub-route file is a 5-liner that delegates.

- [ ] **Step 1: Verify api.rs has `get_course_by_slug` (or add it)**

```bash
grep -n "get_course_by_slug\|fn get_course" crates/features-courses/src/api.rs
```

If missing, append:
```rust
pub async fn get_course_by_slug(ctx: &ApiContext, slug: &str) -> Result<MyCourseDto, ApiError> {
    fetch_json(ctx, "GET", &format!("/v1/courses/by-slug/{slug}"), None::<&()>).await
}
```

If the backend doesn't have `/v1/courses/by-slug/:slug`, use `list_courses` and filter by slug in the route. Verify with:
```bash
grep -n "by-slug\|by_slug" crates/backend/src/handlers/courses.rs
```

If not present, **filter client-side** for now: list all courses, find by slug. Adapt the route accordingly.

- [ ] **Step 2: Create `crates/shell-web/src/routes/course_detail.rs`**

```rust
// crates/shell-web/src/routes/course_detail.rs
use dioxus::prelude::*;
use dioxus_router::prelude::*;
use features_courses::api;
use features_courses::app_shell::{AppShell, ShellUser};
use features_courses::course_detail::CourseDetail as CourseDetailView;

use crate::route_enum::Route;
use crate::routes::{use_api, use_user_context};

fn render_with_tab(slug: String, active_tab: &'static str) -> Element {
    let nav = use_navigator();
    let api = use_api();
    let user_ctx = use_user_context();

    let user = match user_ctx.read().clone() {
        Some(u) => u,
        None => { nav.push(Route::Login {}); return rsx! { p { "Redirecting…" } }; }
    };

    let course = use_resource({
        let api = api.clone();
        let slug = slug.clone();
        move || {
            let api = api.clone();
            let slug = slug.clone();
            async move {
                // If the backend has /v1/courses/by-slug/:slug, use get_course_by_slug.
                // Otherwise fall back to list + filter:
                let all = api::list_courses(&api).await?;
                all.into_iter().find(|c| c.slug == slug)
                    .ok_or(api::ApiError::Status(404, "course not found".into()))
            }
        }
    });

    let shell_user = ShellUser {
        display_name: user.display_name.clone(),
        email: user.email.clone(),
        tenant_role: user.tenant_role,
        is_platform_admin: user.is_platform_admin,
    };

    let slug_for_tab_change = slug.clone();
    rsx! {
        AppShell {
            user: shell_user,
            on_signout: move |_| {
                #[cfg(target_arch = "wasm32")]
                spawn(async move { let _ = platform_bridge::web::WebBridge.sign_out().await; });
                nav.push(Route::Login {});
            },
            match &*course.read_unchecked() {
                Some(Ok(c)) => {
                    let title = c.title.clone();
                    let status = c.status.clone();
                    let cover = c.cover_asset_id.clone();
                    let can_admin = user.is_teacher();
                    rsx! {
                        CourseDetailView {
                            course_title: title,
                            course_status: status,
                            course_cover_asset_id: cover,
                            can_admin: can_admin,
                            active_tab: active_tab.to_string(),
                            on_tab_change: move |tab: String| {
                                let slug = slug_for_tab_change.clone();
                                match tab.as_str() {
                                    "outline" => nav.push(Route::CourseDetail { slug }),
                                    "people" => nav.push(Route::CoursePeople { slug }),
                                    "edit" => nav.push(Route::CourseEdit { slug }),
                                    "schedule" => nav.push(Route::CourseSchedule { slug }),
                                    "assignments" => nav.push(Route::AssignmentList { slug }),
                                    _ => {}
                                };
                            },
                            div { "Tab content for {active_tab} — wired components land in subsequent route files." }
                        }
                    }
                },
                Some(Err(e)) => rsx! { p { class: "error", "Course not found: {e}" } },
                None => rsx! { p { "Loading…" } },
            }
        }
    }
}

#[component]
pub fn CourseDetail(slug: String) -> Element { render_with_tab(slug, "outline") }

#[component]
pub fn CoursePeople(slug: String) -> Element { render_with_tab(slug, "people") }

#[component]
pub fn CourseEdit(slug: String) -> Element { render_with_tab(slug, "edit") }

#[component]
pub fn CourseSchedule(slug: String) -> Element { render_with_tab(slug, "schedule") }
```

(The `div { "Tab content for {active_tab}..." }` placeholder for now — Task 11 wires real lessons/people/etc components. Each tab's real body lands incrementally.)

- [ ] **Step 3: Wire in routes/mod.rs**

```rust
pub mod course_detail;
pub use course_detail::{CourseDetail, CoursePeople, CourseEdit, CourseSchedule};
```

Remove the four corresponding `stub_route!` lines.

- [ ] **Step 4: Build + commit**

```bash
cargo build -p shell-web --target wasm32-unknown-unknown 2>&1 | tail -5
git add crates/shell-web/src/routes/course_detail.rs \
        crates/shell-web/src/routes/mod.rs \
        crates/features-courses/src/api.rs
git commit -m "feat(shell-web): wire CourseDetail with 4-tab dispatch (outline/people/edit/schedule)"
```

---

### Task 11: routes/accept_invite.rs

**Files:**
- Create: `crates/shell-web/src/routes/accept_invite.rs`
- Modify: `crates/shell-web/src/routes/mod.rs`

- [ ] **Step 1: Verify api.rs `accept_invitation`**

```bash
grep -n "accept_invitation\|invite" crates/features-courses/src/api.rs
```

If missing:
```rust
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct AcceptInviteResponse {
    pub course_slug: String,
    pub course_title: String,
}

pub async fn accept_invitation(
    ctx: &ApiContext, token: &str,
) -> Result<AcceptInviteResponse, ApiError> {
    fetch_json(ctx, "POST", &format!("/v1/invitations/{token}/accept"), None::<&()>).await
}
```

(Verify the actual route path with `grep -n "invitations.*accept\|accept.*invitation" crates/backend/src/handlers/enrollments.rs`.)

- [ ] **Step 2: Create `crates/shell-web/src/routes/accept_invite.rs`**

```rust
// crates/shell-web/src/routes/accept_invite.rs
use dioxus::prelude::*;
use dioxus_router::prelude::*;
use features_courses::accept_invite::{AcceptInvite as AcceptInviteView, AcceptState};
use features_courses::api;

use crate::route_enum::Route;
use crate::routes::use_api;

#[component]
pub fn AcceptInvite(token: String) -> Element {
    let nav = use_navigator();
    let api = use_api();
    let mut state = use_signal(|| AcceptState::Pending);

    let token_for_effect = token.clone();
    use_future(move || {
        let api = api.clone();
        let token = token_for_effect.clone();
        async move {
            match api::accept_invitation(&api, &token).await {
                Ok(resp) => state.set(AcceptState::Success {
                    course_title: resp.course_title,
                    course_slug: resp.course_slug,
                }),
                Err(e) => state.set(AcceptState::Failure { reason: format!("{e}") }),
            }
        }
    });

    rsx! {
        AcceptInviteView {
            state: state.read().clone(),
            on_open_course: move |slug: String| nav.push(Route::CourseDetail { slug }),
        }
    }
}
```

(`AcceptState` and props signature: inspect `crates/features-courses/src/accept_invite.rs` and match exactly. The `Pending`, `Success`, `Failure` variants likely exist; if `Success` only takes one field instead of two, adapt.)

- [ ] **Step 3: Wire + build + commit**

```rust
pub mod accept_invite;
pub use accept_invite::AcceptInvite;
```

Remove `stub_route!(AcceptInvite { token: String });`.

```bash
cargo build -p shell-web --target wasm32-unknown-unknown 2>&1 | tail -5
git add crates/shell-web/src/routes/accept_invite.rs \
        crates/shell-web/src/routes/mod.rs \
        crates/features-courses/src/api.rs
git commit -m "feat(shell-web): wire AcceptInvite — real POST /v1/invitations/:token/accept"
```

---

### Task 12: routes/redeem.rs + routes/my_schedule.rs

**Files:**
- Create: `crates/shell-web/src/routes/redeem.rs`
- Create: `crates/shell-web/src/routes/my_schedule.rs`
- Modify: `crates/shell-web/src/routes/mod.rs`

- [ ] **Step 1: Verify/add `redeem_code` in api.rs**

```rust
#[derive(serde::Serialize)]
pub struct RedeemBody<'a> { pub code: &'a str }

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct RedeemResponse {
    pub course_slug: String,
    pub course_title: String,
}

pub async fn redeem_enrollment_code(
    ctx: &ApiContext, code: &str,
) -> Result<RedeemResponse, ApiError> {
    fetch_json(ctx, "POST", "/v1/enrollments/redeem", Some(&RedeemBody { code })).await
}
```

(Verify with `grep -n "redeem" crates/backend/src/handlers/enrollments.rs`.)

- [ ] **Step 2: Create `crates/shell-web/src/routes/redeem.rs`**

```rust
// crates/shell-web/src/routes/redeem.rs
use dioxus::prelude::*;
use dioxus_router::prelude::*;
use features_courses::api;
use features_courses::app_shell::{AppShell, ShellUser};
use features_courses::redeem_code::RedeemCode;

use crate::route_enum::Route;
use crate::routes::{use_api, use_user_context};

#[component]
pub fn Redeem() -> Element {
    let nav = use_navigator();
    let api = use_api();
    let user_ctx = use_user_context();
    let mut error: Signal<Option<String>> = use_signal(|| None);
    let mut submitting = use_signal(|| false);

    let user = match user_ctx.read().clone() {
        Some(u) => u,
        None => { nav.push(Route::Login {}); return rsx! { p { "Redirecting…" } }; }
    };

    let on_submit = move |code: String| {
        let api = api.clone();
        submitting.set(true);
        spawn(async move {
            match api::redeem_enrollment_code(&api, &code).await {
                Ok(resp) => {
                    submitting.set(false);
                    nav.push(Route::CourseDetail { slug: resp.course_slug });
                }
                Err(e) => {
                    submitting.set(false);
                    error.set(Some(format!("{e}")));
                }
            }
        });
    };

    let shell_user = ShellUser {
        display_name: user.display_name.clone(),
        email: user.email.clone(),
        tenant_role: user.tenant_role,
        is_platform_admin: user.is_platform_admin,
    };

    rsx! {
        AppShell {
            user: shell_user,
            on_signout: move |_| {
                #[cfg(target_arch = "wasm32")]
                spawn(async move { let _ = platform_bridge::web::WebBridge.sign_out().await; });
                nav.push(Route::Login {});
            },
            RedeemCode {
                on_submit: on_submit,
                submitting: *submitting.read(),
                error: error.read().clone(),
            }
        }
    }
}
```

- [ ] **Step 3: Create `crates/shell-web/src/routes/my_schedule.rs`**

```rust
// crates/shell-web/src/routes/my_schedule.rs
use dioxus::prelude::*;
use dioxus_router::prelude::*;
use features_courses::api;
use features_courses::app_shell::{AppShell, ShellUser};
use features_courses::schedule_view::{ScheduleEntry, ScheduleView};

use crate::route_enum::Route;
use crate::routes::{use_api, use_user_context};

#[component]
pub fn MySchedule() -> Element {
    let nav = use_navigator();
    let api = use_api();
    let user_ctx = use_user_context();

    let user = match user_ctx.read().clone() {
        Some(u) => u,
        None => { nav.push(Route::Login {}); return rsx! { p { "Redirecting…" } }; }
    };

    let schedule = use_resource({
        let api = api.clone();
        move || {
            let api = api.clone();
            async move { api::list_my_schedule(&api).await }
        }
    });

    let entries: Vec<ScheduleEntry> = match &*schedule.read_unchecked() {
        Some(Ok(rows)) => rows.iter().map(|s| ScheduleEntry {
            session_id: s.session_id.clone(),
            course_slug: s.course_slug.clone(),
            course_title: s.course_title.clone(),
            starts_at: s.starts_at.clone(),
            ends_at: s.ends_at.clone(),
            state: s.state.clone(),
        }).collect(),
        _ => vec![],
    };

    let shell_user = ShellUser {
        display_name: user.display_name.clone(),
        email: user.email.clone(),
        tenant_role: user.tenant_role,
        is_platform_admin: user.is_platform_admin,
    };

    rsx! {
        AppShell {
            user: shell_user,
            on_signout: move |_| {
                #[cfg(target_arch = "wasm32")]
                spawn(async move { let _ = platform_bridge::web::WebBridge.sign_out().await; });
                nav.push(Route::Login {});
            },
            ScheduleView {
                entries: entries,
                on_cancel: move |_id: String| {},
                on_reschedule: move |_id: String| {},
            }
        }
    }
}
```

(`ScheduleEntry` may have a different field shape — open `crates/features-courses/src/schedule_view.rs` and adapt.)

- [ ] **Step 4: Wire + build + commit**

```rust
pub mod my_schedule;
pub mod redeem;
pub use my_schedule::MySchedule;
pub use redeem::Redeem;
```

Remove `stub_route!(Redeem);` and `stub_route!(MySchedule);`.

```bash
cargo build -p shell-web --target wasm32-unknown-unknown 2>&1 | tail -5
git add crates/shell-web/src/routes/ crates/features-courses/src/api.rs
git commit -m "feat(shell-web): wire Redeem + MySchedule routes"
```

---

### Task 13: routes/live_session.rs

**Files:**
- Create: `crates/shell-web/src/routes/live_session.rs`
- Modify: `crates/shell-web/src/routes/mod.rs`

The existing `LiveSessionPage` (in `main.rs` of the old shell, ~80 lines) just needs to move into a route file with `use_user_context` propagating `caller_role` and `display_name`.

- [ ] **Step 1: Move + adapt the existing LiveSessionPage**

Take the existing `LiveSessionPage` body from the (deleted) `main.rs` and place it in `crates/shell-web/src/routes/live_session.rs`. Replace its hardcoded `caller_role` derivation with `UserContext::is_teacher()`:

```rust
// crates/shell-web/src/routes/live_session.rs
use dioxus::prelude::*;
use dioxus_router::prelude::*;
use features_courses::api::fetch_json;
use features_courses::{LiveRoomShell, CallerRole, SessionStatus};

use crate::route_enum::Route;
use crate::routes::{use_api, use_user_context};

#[derive(serde::Deserialize, Clone, Default, PartialEq)]
struct JoinResp {
    state: String,
    transport_mode: String,
    viewer_jwt: Option<String>,
    main_url: Option<String>,
    screen_url: Option<String>,
    instructor_user_id: Option<String>,
    course_title: String,
    scheduled_starts_at: String,
    #[serde(default)]
    has_recording: bool,
}

#[component]
pub fn LiveSession(slug: String, session_id: String) -> Element {
    let nav = use_navigator();
    let api = use_api();
    let user_ctx = use_user_context();
    let user_snap = user_ctx.read().clone();

    if user_snap.is_none() {
        nav.push(Route::Login {});
        return rsx! { p { "Redirecting…" } };
    }

    let session_id_clone = session_id.clone();
    let join = use_resource(move || {
        let api = api.clone();
        let session_id = session_id_clone.clone();
        async move {
            fetch_json::<JoinResp>(
                &api, "POST", &format!("/v1/sessions/{session_id}/join"),
                None::<&()>,
            ).await
        }
    });

    let user = user_snap.unwrap();
    let caller_role = if user.is_teacher() { CallerRole::Teacher } else { CallerRole::Student };

    rsx! {
        match &*join.read_unchecked() {
            Some(Ok(j)) => rsx! {
                LiveRoomShell {
                    session_id: session_id.clone(),
                    course_title: j.course_title.clone(),
                    caller_role: caller_role,
                    is_teacher: user.is_teacher(),
                    has_recording: j.has_recording,
                    status: SessionStatus::from_state(&j.state),
                    transport_mode: j.transport_mode.clone(),
                    viewer_jwt: j.viewer_jwt.clone(),
                    main_url: j.main_url.clone(),
                    screen_url: j.screen_url.clone(),
                    instructor_user_id: j.instructor_user_id.clone(),
                    scheduled_starts_at: j.scheduled_starts_at.clone(),
                    current_user_id: user.user_id.clone(),
                    current_user_display_name: user.display_name.clone(),
                }
            },
            Some(Err(e)) => rsx! { p { class: "error", "Failed to join session: {e}" } },
            None => rsx! { p { "Joining…" } },
        }
    }
}
```

(Inspect the actual `LiveRoomShell` props in `crates/features-courses/src/live_room_shell.rs` and match exactly. The `_ = slug;` on the unused `slug` arg is fine — slug is the URL identifier; the backend resolves the session by `session_id`.)

- [ ] **Step 2: Wire + build + commit**

```rust
pub mod live_session;
pub use live_session::LiveSession;
```

Remove `stub_route!(LiveSession { slug: String, session_id: String });`.

```bash
cargo build -p shell-web --target wasm32-unknown-unknown 2>&1 | tail -5
git add crates/shell-web/src/routes/live_session.rs crates/shell-web/src/routes/mod.rs
git commit -m "feat(shell-web): wire LiveSession with real UserContext-driven role"
```

---

### Task 14: routes/assignments_list.rs

**Files:**
- Create: `crates/shell-web/src/routes/assignments_list.rs`
- Modify: `crates/shell-web/src/routes/mod.rs`

- [ ] **Step 1: Create the route**

```rust
// crates/shell-web/src/routes/assignments_list.rs
use dioxus::prelude::*;
use dioxus_router::prelude::*;
use features_courses::api;
use features_courses::app_shell::{AppShell, ShellUser};
use features_courses::assignment_list::AssignmentList as AssignmentListView;

use crate::route_enum::Route;
use crate::routes::{use_api, use_user_context};

#[component]
pub fn AssignmentList(slug: String) -> Element {
    let nav = use_navigator();
    let api = use_api();
    let user_ctx = use_user_context();

    let user = match user_ctx.read().clone() {
        Some(u) => u,
        None => { nav.push(Route::Login {}); return rsx! { p { "Redirecting…" } }; }
    };

    let course = use_resource({
        let api = api.clone();
        let slug = slug.clone();
        move || {
            let api = api.clone();
            let slug = slug.clone();
            async move {
                let all = api::list_courses(&api).await?;
                all.into_iter().find(|c| c.slug == slug)
                    .ok_or(api::ApiError::Status(404, "course not found".into()))
            }
        }
    });

    let shell_user = ShellUser {
        display_name: user.display_name.clone(),
        email: user.email.clone(),
        tenant_role: user.tenant_role,
        is_platform_admin: user.is_platform_admin,
    };

    rsx! {
        AppShell {
            user: shell_user,
            on_signout: move |_| {
                #[cfg(target_arch = "wasm32")]
                spawn(async move { let _ = platform_bridge::web::WebBridge.sign_out().await; });
                nav.push(Route::Login {});
            },
            match &*course.read_unchecked() {
                Some(Ok(c)) => rsx! {
                    AssignmentListView {
                        api: api.clone(),
                        course_slug: slug.clone(),
                        course_id: c.id.clone(),
                        is_teacher: user.is_teacher(),
                    }
                },
                Some(Err(e)) => rsx! { p { class: "error", "{e}" } },
                None => rsx! { p { "Loading course…" } },
            }
        }
    }
}
```

- [ ] **Step 2: Wire + build + commit**

```rust
pub mod assignments_list;
pub use assignments_list::AssignmentList;
```

Remove `stub_route!(AssignmentList { slug: String });`.

```bash
cargo build -p shell-web --target wasm32-unknown-unknown 2>&1 | tail -5
git add crates/shell-web/src/routes/assignments_list.rs crates/shell-web/src/routes/mod.rs
git commit -m "feat(shell-web): wire AssignmentList — real course resolution + is_teacher"
```

---

### Task 15: routes/assignments_new.rs + routes/assignments_edit.rs

**Files:**
- Create: `crates/shell-web/src/routes/assignments_new.rs`
- Create: `crates/shell-web/src/routes/assignments_edit.rs`
- Modify: `crates/shell-web/src/routes/mod.rs`

- [ ] **Step 1: Create assignments_new.rs**

```rust
// crates/shell-web/src/routes/assignments_new.rs
use dioxus::prelude::*;
use dioxus_router::prelude::*;
use features_courses::api;
use features_courses::app_shell::{AppShell, ShellUser};
use features_courses::assignment_editor::AssignmentEditor;

use crate::route_enum::Route;
use crate::routes::{use_api, use_user_context};

#[component]
pub fn AssignmentNew(slug: String) -> Element {
    let nav = use_navigator();
    let api = use_api();
    let user_ctx = use_user_context();

    let user = match user_ctx.read().clone() {
        Some(u) => u,
        None => { nav.push(Route::Login {}); return rsx! { p { "Redirecting…" } }; }
    };

    let course = use_resource({
        let api = api.clone();
        let slug = slug.clone();
        move || {
            let api = api.clone();
            let slug = slug.clone();
            async move {
                let all = api::list_courses(&api).await?;
                all.into_iter().find(|c| c.slug == slug)
                    .ok_or(api::ApiError::Status(404, "course not found".into()))
            }
        }
    });

    let shell_user = ShellUser {
        display_name: user.display_name.clone(),
        email: user.email.clone(),
        tenant_role: user.tenant_role,
        is_platform_admin: user.is_platform_admin,
    };

    rsx! {
        AppShell {
            user: shell_user,
            on_signout: move |_| {
                #[cfg(target_arch = "wasm32")]
                spawn(async move { let _ = platform_bridge::web::WebBridge.sign_out().await; });
                nav.push(Route::Login {});
            },
            match &*course.read_unchecked() {
                Some(Ok(c)) => rsx! {
                    AssignmentEditor {
                        api: api.clone(),
                        course_slug: slug.clone(),
                        course_id: c.id.clone(),
                        initial: None,
                    }
                },
                Some(Err(e)) => rsx! { p { class: "error", "{e}" } },
                None => rsx! { p { "Loading course…" } },
            }
        }
    }
}
```

- [ ] **Step 2: Create assignments_edit.rs**

```rust
// crates/shell-web/src/routes/assignments_edit.rs
use dioxus::prelude::*;
use dioxus_router::prelude::*;
use features_courses::api;
use features_courses::app_shell::{AppShell, ShellUser};
use features_courses::assignment_editor::AssignmentEditor;

use crate::route_enum::Route;
use crate::routes::{use_api, use_user_context};

#[component]
pub fn AssignmentEdit(slug: String, id: String) -> Element {
    let nav = use_navigator();
    let api = use_api();
    let user_ctx = use_user_context();

    let user = match user_ctx.read().clone() {
        Some(u) => u,
        None => { nav.push(Route::Login {}); return rsx! { p { "Redirecting…" } }; }
    };

    let course = use_resource({
        let api = api.clone();
        let slug = slug.clone();
        move || {
            let api = api.clone();
            let slug = slug.clone();
            async move {
                let all = api::list_courses(&api).await?;
                all.into_iter().find(|c| c.slug == slug)
                    .ok_or(api::ApiError::Status(404, "course not found".into()))
            }
        }
    });

    let assignment = use_resource({
        let api = api.clone();
        let id = id.clone();
        move || {
            let api = api.clone();
            let id = id.clone();
            async move { api::get_assignment(&api, &id).await }
        }
    });

    let shell_user = ShellUser {
        display_name: user.display_name.clone(),
        email: user.email.clone(),
        tenant_role: user.tenant_role,
        is_platform_admin: user.is_platform_admin,
    };

    rsx! {
        AppShell {
            user: shell_user,
            on_signout: move |_| {
                #[cfg(target_arch = "wasm32")]
                spawn(async move { let _ = platform_bridge::web::WebBridge.sign_out().await; });
                nav.push(Route::Login {});
            },
            match (&*course.read_unchecked(), &*assignment.read_unchecked()) {
                (Some(Ok(c)), Some(Ok(a))) => rsx! {
                    AssignmentEditor {
                        api: api.clone(),
                        course_slug: slug.clone(),
                        course_id: c.id.clone(),
                        initial: Some(a.clone()),
                    }
                },
                (Some(Err(e)), _) | (_, Some(Err(e))) => rsx! { p { class: "error", "{e}" } },
                _ => rsx! { p { "Loading…" } },
            }
        }
    }
}
```

- [ ] **Step 3: Wire + build + commit**

```rust
pub mod assignments_edit;
pub mod assignments_new;
pub use assignments_edit::AssignmentEdit;
pub use assignments_new::AssignmentNew;
```

Remove the two corresponding `stub_route!` lines.

```bash
cargo build -p shell-web --target wasm32-unknown-unknown 2>&1 | tail -5
git add crates/shell-web/src/routes/ crates/shell-web/src/routes/mod.rs
git commit -m "feat(shell-web): wire AssignmentNew + AssignmentEdit (real course + assignment fetch)"
```

---

### Task 16: routes/assignments_detail.rs

**Files:**
- Create: `crates/shell-web/src/routes/assignments_detail.rs`
- Modify: `crates/shell-web/src/routes/mod.rs`

```rust
// crates/shell-web/src/routes/assignments_detail.rs
use dioxus::prelude::*;
use dioxus_router::prelude::*;
use features_courses::app_shell::{AppShell, ShellUser};
use features_courses::assignment_detail::AssignmentDetail as AssignmentDetailView;

use crate::route_enum::Route;
use crate::routes::{use_api, use_user_context};

#[component]
pub fn AssignmentDetail(slug: String, id: String) -> Element {
    let nav = use_navigator();
    let api = use_api();
    let user_ctx = use_user_context();

    let user = match user_ctx.read().clone() {
        Some(u) => u,
        None => { nav.push(Route::Login {}); return rsx! { p { "Redirecting…" } }; }
    };

    let shell_user = ShellUser {
        display_name: user.display_name.clone(),
        email: user.email.clone(),
        tenant_role: user.tenant_role,
        is_platform_admin: user.is_platform_admin,
    };

    rsx! {
        AppShell {
            user: shell_user,
            on_signout: move |_| {
                #[cfg(target_arch = "wasm32")]
                spawn(async move { let _ = platform_bridge::web::WebBridge.sign_out().await; });
                nav.push(Route::Login {});
            },
            AssignmentDetailView {
                api: api.clone(),
                assignment_id: id.clone(),
                course_slug: slug.clone(),
                current_user_id: user.user_id.clone(),
                is_teacher: user.is_teacher(),
            }
        }
    }
}
```

Wire + build + commit:
```rust
pub mod assignments_detail;
pub use assignments_detail::AssignmentDetail;
```

Remove `stub_route!(AssignmentDetail { slug: String, id: String });`.

```bash
cargo build -p shell-web --target wasm32-unknown-unknown 2>&1 | tail -5
git add crates/shell-web/src/routes/assignments_detail.rs crates/shell-web/src/routes/mod.rs
git commit -m "feat(shell-web): wire AssignmentDetail with real UserContext"
```

---

### Task 17: routes/assignments_grade.rs

**Files:**
- Create: `crates/shell-web/src/routes/assignments_grade.rs`
- Modify: `crates/shell-web/src/routes/mod.rs`

```rust
// crates/shell-web/src/routes/assignments_grade.rs
use dioxus::prelude::*;
use dioxus_router::prelude::*;
use features_courses::api;
use features_courses::app_shell::{AppShell, ShellUser};
use features_courses::submissions_grading_table::SubmissionsGradingTable;

use crate::route_enum::Route;
use crate::routes::{use_api, use_user_context};

#[component]
pub fn AssignmentGrade(slug: String, id: String) -> Element {
    let nav = use_navigator();
    let api = use_api();
    let user_ctx = use_user_context();

    let user = match user_ctx.read().clone() {
        Some(u) => u,
        None => { nav.push(Route::Login {}); return rsx! { p { "Redirecting…" } }; }
    };
    if !user.is_teacher() {
        // Students don't have access to the grading view.
        nav.push(Route::AssignmentDetail { slug: slug.clone(), id: id.clone() });
        return rsx! { p { "Redirecting…" } };
    }

    let assignment = use_resource({
        let api = api.clone();
        let id = id.clone();
        move || {
            let api = api.clone();
            let id = id.clone();
            async move { api::get_assignment(&api, &id).await }
        }
    });

    let shell_user = ShellUser {
        display_name: user.display_name.clone(),
        email: user.email.clone(),
        tenant_role: user.tenant_role,
        is_platform_admin: user.is_platform_admin,
    };

    rsx! {
        AppShell {
            user: shell_user,
            on_signout: move |_| {
                #[cfg(target_arch = "wasm32")]
                spawn(async move { let _ = platform_bridge::web::WebBridge.sign_out().await; });
                nav.push(Route::Login {});
            },
            match &*assignment.read_unchecked() {
                Some(Ok(a)) => rsx! {
                    SubmissionsGradingTable {
                        api: api.clone(),
                        assignment: a.clone(),
                    }
                },
                Some(Err(e)) => rsx! { p { class: "error", "Assignment not found: {e}" } },
                None => rsx! { p { "Loading assignment…" } },
            }
        }
    }
}
```

Wire + build + commit:
```rust
pub mod assignments_grade;
pub use assignments_grade::AssignmentGrade;
```

Remove `stub_route!(AssignmentGrade { slug: String, id: String });`.

```bash
cargo build -p shell-web --target wasm32-unknown-unknown 2>&1 | tail -5
git add crates/shell-web/src/routes/assignments_grade.rs crates/shell-web/src/routes/mod.rs
git commit -m "feat(shell-web): wire AssignmentGrade — real assignment fetch + role gate"
```

---

### Task 18: SubmissionForm file picker wiring

**Files:**
- Modify: `crates/features-courses/src/submission_form.rs`

Replace the `<p class="files-placeholder">` block with a real `file_picker::FilePicker` invocation that uploads files tied to the submission via `linked_entity_type='submission_attachment'`.

- [ ] **Step 1: Inspect file_picker.rs**

```bash
grep -n "FilePicker\|FilePickerProps\|on_uploaded\|pub fn" crates/features-courses/src/file_picker.rs | head -20
```

Note the exact prop signature (component name, expected props like `linked_entity_type`, `linked_entity_id`, `on_uploaded` callback returning the new asset_id, allowed types, max size, etc.).

- [ ] **Step 2: Replace the placeholder block in `submission_form.rs`**

Find the existing block (around line 107):
```rust
                        if accepts_files {
                            p { class: "files-placeholder",
                                "(File upload reuses /v1/uploads/begin → MinIO PUT → /v1/uploads/complete then PATCH submission with new asset_id; mirror file_picker.rs.)" }
                        }
```

Replace with (adapt prop names to whatever `file_picker.rs` actually exposes):
```rust
                        if accepts_files && edit {
                            crate::file_picker::FilePicker {
                                api: props.api.clone(),
                                linked_entity_type: "submission_attachment".to_string(),
                                linked_entity_id: s.id.clone(),
                                purpose: "attachment".to_string(),
                                allowed_content_types: vec![
                                    "application/pdf".into(),
                                    "image/png".into(),
                                    "image/jpeg".into(),
                                    "text/plain".into(),
                                ],
                                max_size_bytes: 50 * 1024 * 1024,
                                on_uploaded: {
                                    let api = props.api.clone();
                                    let sid = s.id.clone();
                                    let mut submission = submission;
                                    let mut error = error;
                                    move |asset_id: String| {
                                        let api = api.clone();
                                        let sid = sid.clone();
                                        let current = submission.read().clone();
                                        let mut existing_ids: Vec<String> = current
                                            .as_ref()
                                            .map(|c| c.attachment_asset_ids.clone())
                                            .unwrap_or_default();
                                        existing_ids.push(asset_id);
                                        spawn(async move {
                                            let body = api::PatchSubmissionBody {
                                                text_answer: None,
                                                attachment_asset_ids: Some(
                                                    existing_ids.iter().map(|s| s.as_str()).collect(),
                                                ),
                                            };
                                            match api::patch_submission(&api, &sid, &body).await {
                                                Ok(s) => submission.set(Some(s)),
                                                Err(e) => error.set(Some(format!("{e}"))),
                                            }
                                        });
                                    }
                                },
                            }
                            ul { class: "submission-form__attachments",
                                for id in s.attachment_asset_ids.iter() {
                                    li { key: "{id}", "{id}" }
                                }
                            }
                        }
```

(The exact prop set above must match `FilePicker`'s actual signature. If it differs — for example, `FilePicker` takes a single `purpose` string and infers `allowed_content_types` from a backend matrix — adapt. Do not invent props.)

- [ ] **Step 3: Build and commit**

```bash
cargo build -p features-courses --target wasm32-unknown-unknown 2>&1 | tail -10
git add crates/features-courses/src/submission_form.rs
git commit -m "feat(submissions): wire SubmissionForm file picker (linked_entity_type=submission_attachment)"
```

---

### Task 19: WhipPublisher leak fix

**Files:**
- Modify: `crates/features-courses/src/live_room_view.rs`

Replace the existing `Promoted`/`Demoted` arms to store and close the publisher.

- [ ] **Step 1: Locate the existing arms**

```bash
grep -n "Promoted\|Demoted\|WhipPublisher\|publish_audio" crates/features-courses/src/live_room_view.rs | head -10
```

Open the file at the relevant lines. The existing arms (around lines 263–281) look like:

```rust
        ServerEvent::Promoted {
            publish_url,
            publish_password,
            ..
        } => {
            // TODO: store the returned WhipPublisher so Demoted can close() it.
            wasm_bindgen_futures::spawn_local(async move {
                let _ =
                    crate::live_room_audio_publisher::publish_audio(
                        &publish_url,
                        &publish_password,
                    )
                    .await;
            });
        }
        ServerEvent::Demoted { .. } => {
            // TODO: close stored WhipPublisher.
        }
```

- [ ] **Step 2: Add a publisher signal at the top of the component**

Find the `LiveRoomView` function. Near the other `use_signal` calls (e.g. the `state` signal that holds `LiveRoomState`), add:

```rust
    let mut publisher: Signal<Option<crate::live_room_audio_publisher::WhipPublisher>> =
        use_signal(|| None);
```

(If `WhipPublisher` is not currently `pub`, also export it from `crates/features-courses/src/live_room_audio_publisher.rs` — change `pub(crate) struct WhipPublisher` to `pub struct WhipPublisher`, or add `pub use WhipPublisher` from the module. Check first with `grep -n "WhipPublisher\|pub struct\|pub fn publish_audio" crates/features-courses/src/live_room_audio_publisher.rs`.)

- [ ] **Step 3: Update the Promoted arm**

Replace the `ServerEvent::Promoted { ... }` arm with:

```rust
        ServerEvent::Promoted {
            publish_url,
            publish_password,
            ..
        } => {
            wasm_bindgen_futures::spawn_local(async move {
                match crate::live_room_audio_publisher::publish_audio(
                    &publish_url,
                    &publish_password,
                )
                .await
                {
                    Ok(p) => publisher.set(Some(p)),
                    Err(e) => {
                        web_sys::console::warn_1(&format!("publish_audio failed: {e}").into());
                    }
                }
            });
        }
```

- [ ] **Step 4: Update the Demoted arm**

Replace `ServerEvent::Demoted { .. } => { ... }` with:

```rust
        ServerEvent::Demoted { .. } => {
            if let Some(p) = publisher.read().clone() {
                p.close();
            }
            publisher.set(None);
        }
```

(If `WhipPublisher::close` takes `&self` vs `&mut self` — check via `grep -n "fn close" crates/features-courses/src/live_room_audio_publisher.rs`. If it requires `&mut`, adapt by `take()`-ing from the signal: `if let Some(mut p) = publisher.write().take() { p.close(); }`.)

- [ ] **Step 5: Build + commit**

```bash
cargo build -p features-courses --target wasm32-unknown-unknown 2>&1 | tail -5
git add crates/features-courses/src/live_room_view.rs \
        crates/features-courses/src/live_room_audio_publisher.rs
git commit -m "fix(live_room): close WhipPublisher on Demoted (audio cleanup)"
```

---

### Task 20: Inline banner for WebSocket Error / RateLimited

**Files:**
- Modify: `crates/features-courses/src/live_room_view.rs`

Per Q6 (inline-only error UX), surface `Error` and `RateLimited` server events as a small inline banner that auto-dismisses after 5 seconds.

- [ ] **Step 1: Add a banner signal**

In `LiveRoomView`, alongside the publisher signal:

```rust
    let mut banner: Signal<Option<String>> = use_signal(|| None);
```

- [ ] **Step 2: Update the Error / RateLimited arms**

Replace:
```rust
        ServerEvent::RateLimited { .. } | ServerEvent::Error { .. } => {
            // TODO: surface as toast notification.
        }
```

with:

```rust
        ServerEvent::RateLimited { retry_after_ms } => {
            banner.set(Some(format!(
                "Rate limited — try again in {} ms",
                retry_after_ms
            )));
            let mut banner = banner;
            wasm_bindgen_futures::spawn_local(async move {
                gloo_timers::future::TimeoutFuture::new(5_000).await;
                banner.set(None);
            });
        }
        ServerEvent::Error { code, message } => {
            banner.set(Some(format!("[{code}] {message}")));
            let mut banner = banner;
            wasm_bindgen_futures::spawn_local(async move {
                gloo_timers::future::TimeoutFuture::new(5_000).await;
                banner.set(None);
            });
        }
```

- [ ] **Step 3: Render the banner in the rsx**

Find the top of the `rsx! { div { class: "live-room-view", ... } }` returned by `LiveRoomView`. Add immediately after the opening `div`:

```rust
            if let Some(msg) = banner.read().as_ref() {
                div { class: "live-room-banner live-room-banner--warn",
                    "{msg}"
                }
            }
```

- [ ] **Step 4: Verify gloo-timers is already a dep of features-courses**

```bash
grep -n "gloo-timers" crates/features-courses/Cargo.toml
```

If not, add to `[target.'cfg(target_arch = "wasm32")'.dependencies]`:
```toml
gloo-timers = { version = "0.3", features = ["futures"] }
```

(Already added during Phase 1b-δ for `LiveRoomReplay`'s currentTime polling. Verify before adding again.)

- [ ] **Step 5: Build + commit**

```bash
cargo build -p features-courses --target wasm32-unknown-unknown 2>&1 | tail -5
git add crates/features-courses/src/live_room_view.rs crates/features-courses/Cargo.toml
git commit -m "feat(live_room): inline banner for WebSocket Error + RateLimited events"
```

---

### Task 21: redis_broker_smoke binary

**Files:**
- Create: `crates/backend/src/bin/redis_broker_smoke.rs`

A one-shot CLI that connects to `REDIS_URL`, publishes a synthetic chat event to a unique channel, subscribes, and asserts round-trip.

- [ ] **Step 1: Inspect the existing `RedisLiveRoomBroker`**

```bash
grep -n "publish\|subscribe\|RedisLiveRoomBroker\|fred::" crates/backend/src/services/live_room.rs | head -20
```

Note the exact methods exposed for publishing and subscribing; reuse them directly so the smoke test exercises the same path the production sockets use.

- [ ] **Step 2: Create the binary**

```rust
// crates/backend/src/bin/redis_broker_smoke.rs
//! One-shot smoke test for the Redis live-room broker. Closes out the
//! Phase 1b-γ §4b manual verification.
//!
//! Usage:
//!   REDIS_URL=redis://localhost:56379 cargo run -p backend --bin redis_broker_smoke
//!
//! Exit code 0 on success; non-zero with an error message on failure.

use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let url = std::env::var("REDIS_URL")
        .unwrap_or_else(|_| "redis://localhost:56379".to_string());
    eprintln!("connecting to {url}…");

    let broker = backend::services::live_room::RedisLiveRoomBroker::connect(&url).await?;

    let session_id = uuid::Uuid::new_v4();
    let channel = format!("aulalite:room:{session_id}");
    eprintln!("channel: {channel}");

    let mut subscriber = broker.subscribe(session_id).await?;

    let payload = serde_json::json!({
        "type": "chat",
        "id": uuid::Uuid::new_v4().to_string(),
        "sender_user_id": uuid::Uuid::new_v4().to_string(),
        "sender_display_name": "smoke-test",
        "body": "hello from smoke test",
        "created_at": chrono::Utc::now().to_rfc3339(),
    });
    broker.publish(session_id, payload.to_string()).await?;
    eprintln!("published");

    let received = tokio::time::timeout(Duration::from_secs(5), subscriber.recv()).await
        .map_err(|_| anyhow::anyhow!("timed out waiting for round-trip"))?
        .ok_or_else(|| anyhow::anyhow!("subscriber closed before message arrived"))?;

    eprintln!("received: {received}");

    let parsed: serde_json::Value = serde_json::from_str(&received)?;
    if parsed["body"] != "hello from smoke test" {
        anyhow::bail!("payload mismatch: {parsed}");
    }

    eprintln!("OK — round-trip successful");
    Ok(())
}
```

(The `RedisLiveRoomBroker::connect`, `subscribe`, and `publish` method names above must match what `crates/backend/src/services/live_room.rs` actually exposes. Adapt as needed — the goal is to prove that the production broker code can publish + subscribe end-to-end against a real Redis.)

- [ ] **Step 3: Build the binary**

```bash
cargo build -p backend --bin redis_broker_smoke 2>&1 | tail -5
```

Expected: clean.

- [ ] **Step 4: Manual smoke test (optional — documented in exit checklist)**

```bash
docker compose up -d redis
REDIS_URL=redis://localhost:56379 cargo run -p backend --bin redis_broker_smoke
```

Expected: exit 0, "OK — round-trip successful".

- [ ] **Step 5: Commit**

```bash
git add crates/backend/src/bin/redis_broker_smoke.rs
git commit -m "feat(backend): redis_broker_smoke binary for 1b-gamma §4b verification"
```

---

### Task 22: shell-desktop wrapper

**Files:**
- Modify: `crates/shell-desktop/Cargo.toml`
- Replace: `crates/shell-desktop/src/main.rs`

- [ ] **Step 1: Inspect current shell-desktop**

```bash
cat crates/shell-desktop/Cargo.toml
cat crates/shell-desktop/src/main.rs
```

- [ ] **Step 2: Update `crates/shell-desktop/Cargo.toml`**

```toml
[package]
name = "shell-desktop"
version.workspace = true
edition.workspace = true
license.workspace = true
publish.workspace = true

[dependencies]
dioxus = { workspace = true, features = ["desktop"] }
shell-web = { path = "../shell-web" }
```

(If the workspace's `dioxus` doesn't enable `desktop` cleanly because of `web` already being there, add an explicit `dioxus-desktop = "=0.7.4"` instead.)

- [ ] **Step 3: Replace `crates/shell-desktop/src/main.rs`**

```rust
// crates/shell-desktop/src/main.rs
fn main() {
    dioxus::launch(shell_web::App);
}
```

- [ ] **Step 4: Build (native, NOT wasm)**

```bash
cargo build -p shell-desktop 2>&1 | tail -10
```

Expected: clean. The native build is what desktop uses; wasm is irrelevant.

If the build fails because `shell-web` has wasm-only code in its `lib.rs` (e.g. `web_sys::window()` calls in the auth bootstrap), the desktop target will need to either gate those calls behind `cfg(target_arch = "wasm32")` (preferred — auth on desktop isn't part of this phase, so a no-op fallback is fine) OR push them into a separate wasm-only module.

The auth bootstrap in `lib.rs` already has `cfg(target_arch = "wasm32")` gates around the `WebBridge` calls (Task 5 step 1) — so on desktop, `bootstrapped` flips to true but no token is fetched. That leaves the user signed-out and the router lands on `/login`. Login on desktop won't work without a desktop bridge implementation, but the build compiles. That's the deliberate scope cut — desktop auth is a follow-up.

- [ ] **Step 5: Commit**

```bash
git add crates/shell-desktop/Cargo.toml crates/shell-desktop/src/main.rs
git commit -m "feat(shell-desktop): launch shell_web::App via dioxus-desktop"
```

---

### Task 23: shell-mobile student subset

**Files:**
- Modify: `crates/shell-mobile/Cargo.toml`
- Replace: `crates/shell-mobile/src/main.rs`
- Create: `crates/shell-mobile/src/route_enum.rs`

- [ ] **Step 1: Update `crates/shell-mobile/Cargo.toml`**

```toml
[package]
name = "shell-mobile"
version.workspace = true
edition.workspace = true
license.workspace = true
publish.workspace = true

[dependencies]
dioxus = { workspace = true, features = ["mobile"] }
dioxus-router = { workspace = true }
shell-web = { path = "../shell-web" }
features-courses = { path = "../features-courses" }
features-auth = { path = "../features-auth" }
core-types = { path = "../core-types" }
```

- [ ] **Step 2: Create `crates/shell-mobile/src/route_enum.rs`**

```rust
// crates/shell-mobile/src/route_enum.rs
use dioxus::prelude::*;
use dioxus_router::prelude::*;

// Reuse shell-web's route components for the routes mobile shares.
use shell_web::routes;

#[rustfmt::skip]
#[derive(Routable, Clone, PartialEq, Eq)]
pub enum MobileRoute {
    #[route("/login")]
    Login {},
    #[route("/")]
    Dashboard {},
    #[route("/courses/:slug")]
    CourseDetail { slug: String },
    #[route("/courses/:slug/sessions/:session_id")]
    LiveSession { slug: String, session_id: String },
    #[route("/courses/:slug/assignments")]
    AssignmentList { slug: String },
    #[route("/courses/:slug/assignments/:id")]
    AssignmentDetail { slug: String, id: String },
    #[route("/schedule")]
    MySchedule {},
}
```

(For dioxus_router to resolve each variant to a component, mobile re-exports the same names from `shell_web::routes`. The student-subset just OMITS the teacher-only routes; it doesn't redefine the components.)

- [ ] **Step 3: Replace `crates/shell-mobile/src/main.rs`**

```rust
// crates/shell-mobile/src/main.rs
mod route_enum;

use dioxus::prelude::*;
use dioxus_router::prelude::*;
use shell_web::contexts::UserContextSignal;
use features_courses::api::ApiContext;

fn main() {
    dioxus::launch(MobileApp);
}

#[component]
fn MobileApp() -> Element {
    // Same auth + UserContext bootstrap as shell-web's App (could be factored
    // into a shared helper; for now duplicate).
    let mut api_ctx = use_signal(|| ApiContext { base_url: String::new(), id_token: String::new() });
    let mut user_ctx: UserContextSignal = use_signal(|| None);
    use_context_provider::<Signal<ApiContext>>(|| api_ctx);
    use_context_provider::<UserContextSignal>(|| user_ctx);
    use_context_provider(|| api_ctx.read().clone());

    // (Mobile bootstrap deferred — login on mobile requires a mobile-specific
    // platform_bridge::PlatformBridge impl. For now the router lands on /login
    // and nothing more.)
    let _ = (api_ctx.set(ApiContext::default()), user_ctx.set(None));

    rsx! {
        Router::<route_enum::MobileRoute> {}
    }
}
```

(`ApiContext::default()` requires `impl Default for ApiContext` — add a `#[derive(Default)]` to the `ApiContext` struct in `crates/features-courses/src/api.rs` if it doesn't already have one.)

- [ ] **Step 4: Build**

```bash
cargo build -p shell-mobile 2>&1 | tail -10
```

(Mobile native target may not build cleanly without an Android/iOS toolchain — but `cargo build -p shell-mobile` against the workspace's default native target should at least confirm the route enum compiles and the `MobileApp` function type-checks.)

- [ ] **Step 5: Commit**

```bash
git add crates/shell-mobile/Cargo.toml \
        crates/shell-mobile/src/main.rs \
        crates/shell-mobile/src/route_enum.rs \
        crates/features-courses/src/api.rs
git commit -m "feat(shell-mobile): student-subset MobileRoute enum + same provider stack as web"
```

---

### Task 24: SSR smokes for new shell routes

**Files:**
- Create: `crates/shell-web/tests/shell_routes_smoke.rs`

- [ ] **Step 1: Write the smoke**

```rust
//! Phase 1.5: SSR smokes for the new shell-web routes.

use dioxus::prelude::*;
use dioxus_ssr::render;
use features_courses::api::ApiContext;
use shell_web::contexts::{UserContext, UserContextSignal};

fn fake_api() -> ApiContext {
    ApiContext { base_url: "http://localhost:8080".into(), id_token: String::new() }
}

fn fake_user() -> UserContext {
    UserContext {
        user_id: "00000000-0000-0000-0000-000000000001".into(),
        display_name: "Test User".into(),
        email: "test@example.test".into(),
        tenant_role: Some(core_types::TenantRole::Teacher),
        is_platform_admin: false,
    }
}

fn provide_test_contexts() -> (Signal<ApiContext>, UserContextSignal) {
    let api_signal = Signal::new(fake_api());
    let user_signal: UserContextSignal = Signal::new(Some(fake_user()));
    (api_signal, user_signal)
}

#[test]
fn login_route_renders() {
    let mut dom = VirtualDom::new(|| {
        let (api_signal, user_signal) = provide_test_contexts();
        use_context_provider::<Signal<ApiContext>>(|| api_signal);
        use_context_provider::<UserContextSignal>(|| user_signal);
        use_context_provider(|| api_signal.read().clone());
        rsx! {
            shell_web::routes::Login {}
        }
    });
    let _ = dom.rebuild_in_place();
    let html = render(&dom);
    // Login form should at least render the email field.
    assert!(html.contains("email") || html.contains("Email"));
}

#[test]
fn dashboard_route_renders_loading() {
    let mut dom = VirtualDom::new(|| {
        let (api_signal, user_signal) = provide_test_contexts();
        use_context_provider::<Signal<ApiContext>>(|| api_signal);
        use_context_provider::<UserContextSignal>(|| user_signal);
        use_context_provider(|| api_signal.read().clone());
        rsx! {
            shell_web::routes::Dashboard {}
        }
    });
    let _ = dom.rebuild_in_place();
    let html = render(&dom);
    // Either header username is rendered or the loading state.
    assert!(html.contains("Test User") || html.contains("Loading"));
}

#[test]
fn course_list_route_renders_for_teacher() {
    let mut dom = VirtualDom::new(|| {
        let (api_signal, user_signal) = provide_test_contexts();
        use_context_provider::<Signal<ApiContext>>(|| api_signal);
        use_context_provider::<UserContextSignal>(|| user_signal);
        use_context_provider(|| api_signal.read().clone());
        rsx! {
            shell_web::routes::CourseList {}
        }
    });
    let _ = dom.rebuild_in_place();
    let html = render(&dom);
    // Teacher should see the create button OR an empty list message.
    assert!(html.len() > 100);
}

#[test]
fn assignment_list_route_renders() {
    let mut dom = VirtualDom::new(|| {
        let (api_signal, user_signal) = provide_test_contexts();
        use_context_provider::<Signal<ApiContext>>(|| api_signal);
        use_context_provider::<UserContextSignal>(|| user_signal);
        use_context_provider(|| api_signal.read().clone());
        rsx! {
            shell_web::routes::AssignmentList { slug: "math".to_string() }
        }
    });
    let _ = dom.rebuild_in_place();
    let html = render(&dom);
    assert!(html.len() > 50); // mounts without panic
}

#[test]
fn unauthenticated_dashboard_does_not_panic() {
    let mut dom = VirtualDom::new(|| {
        let api_signal = Signal::new(fake_api());
        let user_signal: UserContextSignal = Signal::new(None);
        use_context_provider::<Signal<ApiContext>>(|| api_signal);
        use_context_provider::<UserContextSignal>(|| user_signal);
        use_context_provider(|| api_signal.read().clone());
        rsx! {
            shell_web::routes::Dashboard {}
        }
    });
    let _ = dom.rebuild_in_place();
    let _ = render(&dom);
    // No panic = pass. Dashboard should render either the loading state or
    // a redirect placeholder.
}
```

- [ ] **Step 2: Verify dioxus_ssr is a dev-dep of shell-web**

```bash
grep -A3 "\[dev-dependencies\]" crates/shell-web/Cargo.toml
```

If absent, add:
```toml
[dev-dependencies]
dioxus-ssr = "=0.7.4"
core-types = { path = "../core-types" }
```

- [ ] **Step 3: Run**

```bash
cargo test -p shell-web --test shell_routes_smoke 2>&1 | tail -10
```

Expected: 5 passed.

- [ ] **Step 4: Commit**

```bash
git add crates/shell-web/tests/shell_routes_smoke.rs crates/shell-web/Cargo.toml
git commit -m "test(shell-web): SSR smokes for Login/Dashboard/CourseList/AssignmentList routes"
```

---

### Task 25: Build sweeps + workspace test sweep

**Files:** none — verification only.

- [ ] **Step 1: Wasm builds**

```bash
cargo build -p shell-web --target wasm32-unknown-unknown 2>&1 | tail -5
cargo build -p features-courses --target wasm32-unknown-unknown 2>&1 | tail -5
```

Expected: clean (warnings OK).

- [ ] **Step 2: Native builds**

```bash
cargo build -p shell-desktop 2>&1 | tail -5
cargo build -p backend 2>&1 | tail -5
```

- [ ] **Step 3: dx build (if available)**

```bash
dx build --platform web --package shell-web 2>&1 | tail -10
```

- [ ] **Step 4: Workspace tests**

```bash
DATABASE_URL=postgres://aulalite:changeme@localhost:55432/aulalite \
    cargo test --workspace -j 2 2>&1 | tail -20
```

Expected: zero failures. Tests gain ~5 from `shell_routes_smoke.rs`. Total should land around 253–255 passed.

- [ ] **Step 5: If a Windows PDB linker error (LNK1318) appears**

```bash
cargo clean -p backend
DATABASE_URL=... cargo test --workspace -j 2
```

- [ ] **Step 6: Commit any incidental fixes** (if a flaky test surfaces a real bug; otherwise nothing to commit).

---

### Task 26: Phase 1.5 exit checklist + commit + report

**Files:**
- Create: `docs/superpowers/plans/2026-05-10-aulalite-phase-1-5-shell-wiring-exit-checklist.md`

- [ ] **Step 1: Write the checklist**

```markdown
# Phase 1.5 Exit Checklist

Run these checks in order from the repository root. Phase 1.5 is complete only
when every required item passes.

## 1. Stack health
- [ ] `docker compose up -d`
- [ ] `curl http://localhost:8080/healthz` returns `ok`

## 2. Automated verification
- [ ] `cargo test --workspace -j 2` (everything green; zero failures).
- [ ] `cargo build -p shell-web --target wasm32-unknown-unknown`
- [ ] `cargo build -p features-courses --target wasm32-unknown-unknown`
- [ ] `cargo build -p shell-desktop` (native target).
- [ ] `cargo build -p shell-mobile` (workspace native target — full mobile toolchain not required).
- [ ] `dx build --platform web --package shell-web` succeeds.

## 3. End-to-end browser flow (manual, the headline acceptance)
- [ ] Sign in as teacher in Chrome.
- [ ] **URL bar shows `/`**, the dashboard renders **the actual courses owned by that teacher** (not `vec![]`).
- [ ] Header shows the **real display_name + email** (not "User <user@example.com>").
- [ ] Hard-refresh the page. **Stay signed in**, dashboard re-renders the same data.
- [ ] Wait an hour (or manually expire the token via `firebase.auth.currentUser.getIdTokenResult(true)` in devtools) and refresh.
       **Page does not kick to /login** — the 401-retry refreshes silently.
- [ ] Click a course → **/courses/:slug renders, lessons load**, browser back button returns to `/`.
- [ ] Click Assignments tab → URL bar updates to `/courses/:slug/assignments`, real assignment list loads.
- [ ] Click an assignment → URL `/courses/:slug/assignments/:id` opens detail with role-aware UI.
- [ ] Sign out from header. URL goes to `/login`. Reload → stays on `/login`.

## 4. Student flow
- [ ] Sign in as student. Dashboard shows enrolled courses.
- [ ] Open an assignment with `accepts_files=true`.
       The **file picker renders** (not the placeholder `<p>`).
       Select a PDF → upload completes → asset_id appears in `attachment_asset_ids`.
- [ ] Submit the assignment. Status flips to `submitted`. Receipt visible.

## 5. Invite + redeem
- [ ] Trigger an invite email from teacher → student.
- [ ] Click the email link. URL `/accept-invite/:token` opens.
       **Accept actually works** (POST /v1/invitations/:token/accept). Browser redirects to the course detail.
       (Pre-1.5 this hardcoded "Invite flow not yet wired".)
- [ ] As a student with an enrollment code, navigate to `/redeem`. Enter the code.
       **Redeem actually enrolls** and routes to the course.

## 6. Live class polish (1b-γ §4a follow-ups)
- [ ] Teacher promotes a student. Student's mic activates.
- [ ] Teacher demotes the same student. **Mic stops** (WhipPublisher closed). Verify no audio bleed
       in a third Chrome profile that was watching.
- [ ] Trigger rate-limit (5 chats back-to-back as student). **Inline banner** appears at top of live room
       reading "Rate limited — try again in N ms"; auto-dismisses after 5s.
- [ ] Server emits an Error event (e.g. send malformed JSON). **Inline banner** shows code + message; auto-dismisses.

## 7. Redis broker production smoke (1b-γ §4b follow-up)
- [ ] `REDIS_URL=redis://localhost:56379 cargo run -p backend --bin redis_broker_smoke`
       returns exit 0 with "OK — round-trip successful".

## 8. Cross-tenant probe
- [ ] Tenant B's user fetches `/v1/me` → returns tenant B's user only.
- [ ] Tenant B navigates to `/courses/<tenant-A-course-slug>` → 404 inline error rendered.
- [ ] Existing `cross_tenant_assignments_masked` and `cross_tenant_submissions_masked` tests still pass.

## 9. shell-desktop sanity
- [ ] `cargo run -p shell-desktop` opens a desktop window with the same UI.
- [ ] On desktop, login is non-functional (deliberate — desktop bridge is a follow-up); the window mounts on /login and the form renders without panic.

## 10. shell-mobile sanity
- [ ] `cargo build -p shell-mobile` against the workspace default target completes.
       (Full mobile-target build requires Android/iOS toolchain — out of scope.)

## 11. Open follow-ups carried into next phase

These are NEW deferrals after 1.5; they were not in scope:
- [ ] Desktop platform-bridge implementation (file-based token persistence, e.g. via `directories` crate + Firebase REST).
- [ ] Mobile platform-bridge implementation (Android Keystore / iOS Keychain + Firebase REST).
- [ ] AssignmentEditor reference-attachment file picker (teacher attaches PDFs to assignments).
- [ ] CourseDetail outline/people/edit/schedule tab content — currently a placeholder div per tab.
       The tab dispatch works (URL changes); the tab body content (lesson list, people list, edit form,
       schedule view) is not yet rendered.

These do NOT block tagging `phase-1-5-complete`.

## Completion tag

```bash
git tag phase-1-5-complete
git push origin phase-1-5-complete
```

Tagging `phase-1-5-complete` clears the audit gap: the app actually works end-to-end
in a browser. The original Phase 1d (notifications, parent role, TA scoping) becomes
the next natural phase.
```

- [ ] **Step 2: Commit**

```bash
git add docs/superpowers/plans/2026-05-10-aulalite-phase-1-5-shell-wiring-exit-checklist.md
git commit -m "docs(plan): Phase 1.5 shell-wiring exit checklist"
git rev-parse HEAD
```

- [ ] **Step 3: Report**

Tell the user:
- Total commits added in 1.5.
- Test pass count (workspace).
- Final SHA on `phase-0-foundations`.
- The headline result: **the app now works end-to-end in a browser**.
- Pending manual exit-checklist items.
- Newly-deferred items (desktop bridge, mobile bridge, AssignmentEditor picker, CourseDetail tab bodies).
- That tagging `phase-1-5-complete` after the manual checklist closes the audit gap.

---

## Self-review notes (for the controller running this plan)

After all tasks complete:

1. **Spec coverage.** Section A (auth bootstrap + ApiContext + UserContext) → Tasks 2-5. Section B (URL routes per file) → Tasks 4 + 6-17. Section C (mobile/desktop) → Tasks 22-23. Section D (1b-γ polish + testing) → Tasks 18-21 + Task 24.

2. **Type consistency.** `ApiContext` from `features_courses::api` is provided as `Signal<ApiContext>` AND its current value is provided as bare `ApiContext` (so `use_context::<ApiContext>()` works for components that don't need to mutate). `UserContextSignal = Signal<Option<UserContext>>` — used everywhere via `use_context::<UserContextSignal>()`. `RefreshFn` registered once via `set_refresh_fn` at boot.

3. **No placeholders.** Every code-bearing step has actual code. Three places where the engineer must adapt to existing components (Tasks 8, 11, 12, 13: `Dashboard`/`AcceptInvite`/`ScheduleView`/`LiveRoomShell` prop signatures; Task 18: `FilePicker` props; Task 19: `WhipPublisher::close` signature) are explicit about WHAT to inspect and WHY — not "TODO add error handling".

4. **Frontend convention.** All new route components use `#[component] pub fn ComponentName(...)` because they don't have custom Props structs (single-arg routes like `slug: String` are declared as fn params, which Dioxus 0.7's `#[component]` macro accepts cleanly). The `pub fn ComponentName(props: ComponentProps)` convention only applies to components with custom Props structs (the existing `LiveRoomShell`, `LiveRoomReplay`, etc.); new routes don't add new Props structs.

5. **Backwards-compat with prior phases.** No backend changes except adding `redis_broker_smoke` binary. The 401-retry interceptor in `fetch_json` is additive (returns the same `ApiError::Status(401, ...)` if no refresher is registered). All existing 248 tests should still pass.

6. **The deliberate scope cut on CourseDetail tab bodies.** Task 10 wires the tab dispatch (URL routing per tab) but leaves each tab's body as a placeholder `div`. Real lesson list, people list, schedule view, and edit form rendering inside the tabs is a Phase 1d follow-up. The audit will note this as a remaining gap, but the routes themselves work and deep-link.
