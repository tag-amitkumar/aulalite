# Local-Login UX Fix + Auth Pass + Real-Stack UI Test Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the dev profile-button panel with an email/password flow driven by `.env` credentials, land the full auth-surface and live-room-lifecycle pass that stops the navigation bounce at root cause, and verify the whole thing with a single real-stack Playwright spec.

**Architecture:** A new `POST /v1/auth/local-login` endpoint validates `{email, password}` against env-configured pairs and returns the same per-profile id_token the auth middleware already accepts. The existing email/password form tries this endpoint first, falls back to Firebase. The full live-room safety pass from `docs/superpowers/specs/2026-05-15-live-room-safety-auth-pass-design.md` lands in the same project — its frontend Signal/use_api migration is the step that actually stops the bounce. A single Playwright spec drives the real stack (compose + dx serve) end-to-end as both teacher and student.

**Tech Stack:** Rust 2021, Axum 0.7, SQLx/Postgres, Dioxus 0.7.4, Dioxus Router, `percent-encoding`, `dioxus-free-icons`, Playwright (chromium), PowerShell scripts.

**Specs:** This plan implements `docs/superpowers/specs/2026-05-16-local-login-and-real-stack-ui-test-design.md` and incorporates `docs/superpowers/specs/2026-05-15-live-room-safety-auth-pass-design.md` by reference for the live-room safety details. Where a step says "apply per the 2026-05-15 design," the exact code shape is documented in that file.

---

## Scope Check

This plan is intentionally bundled per the user's choice in brainstorming. It is one cohesive slice: make local-dev sign-in work end-to-end and verify it. The deferred SaaS expansion backlog (billing, tenant settings, admin console, analytics, onboarding, notifications, support, audit/compliance) is captured in `docs/superpowers/specs/2026-05-15-aulalite-saas-expansion-backlog.md` and is **not** in scope here.

## File Structure

**Created:**

- `crates/backend/src/trace_scrub.rs` — `MakeSpan` impl scrubbing `access_token` / `jwt` / `token` query params from logged URIs.
- `crates/features-courses/src/live_room_session.rs` — aggregate manager for socket + publisher + viewer + students with `Drop`.
- `tools/ui-real-stack.spec.js` — single Playwright spec, real backend + real frontend, both roles.
- `tools/run-ui-check.ps1` — orchestration script.
- `tools/README.md` — operator doc.

**Modified — backend:**

- `crates/backend/src/auth/local_login.rs` — `password` field + `profile_for_email_password`.
- `crates/backend/src/handlers/dev_login.rs` — module replaced with single `POST /v1/auth/local-login`.
- `crates/backend/src/lib.rs` — route wiring + trace-scrub wiring.
- `crates/backend/src/main.rs` — trace-scrub wiring.
- `crates/backend/src/auth/middleware.rs` — WS upgrade detection + percent-decoded access_token query.
- `crates/backend/src/handlers/live_sessions.rs` — bearer-strip, limit clamp, f64 guard, CommandFailed, drop `?jwt=`.
- `crates/backend/src/handlers/dev_seed.rs` — single `starts_at` threaded through.
- `crates/backend/Cargo.toml` — `percent-encoding = "2"`.
- `crates/core-types/src/live_room.rs` — `CommandFailed` variant, `display_name` fields, `close_codes` module.

**Modified — frontend:**

- `crates/api-client/src/lib.rs` — `local_login` API, `use_api()` hook, deprecation of bare context.
- `crates/features-auth/src/login.rs` — submit handler tries local-login first.
- `crates/shell-web/src/lib.rs` — `Signal<ApiContext>` provider only; bare snapshot provider removed.
- `crates/shell-web/src/routes/login.rs` — `auth-local-panel` removed.
- `crates/shell-web/src/routes/live_session.rs` — `LiveSessionShell` wrapper with `use_on_destroy`.
- `crates/features-courses/src/lib.rs` — declare `live_room_session` module.
- `crates/features-courses/src/live_room_view.rs` — read session from context; subscribe to CommandFailed.
- `crates/features-courses/src/live_room_broadcast.rs` — session-driven go-live / end-class flows.
- `crates/features-courses/src/live_room_socket.rs` — `close()`, `Drop`, `onclose` codes 4001/4003, URL-encoded token.
- `crates/features-courses/src/live_room_whip.rs` — `close()`, `Drop`, typed SDP extraction.
- `crates/features-courses/src/live_room_whep.rs` — `close()`, `Drop`, bearer-only.
- `crates/features-courses/src/lesson_outline_view.rs`, `lesson_files_editor.rs`, `lesson_video_editor.rs`, `file_picker.rs`, `file_asset_image.rs`, `live_room_replay.rs` — migrated to `use_api()`.

**Modified — config:**

- `.env` — `LOCAL_LOGIN_TEACHER_PASSWORD`, `LOCAL_LOGIN_STUDENT_PASSWORD`.
- `.env.example` — same.

**Modified — tests:**

- `crates/backend/tests/local_login_bypass.rs` — six new tests for the new endpoint; remove tests for deleted routes.
- `crates/features-auth/tests/login.rs` — three new tests for the form fallback behavior.
- `crates/shell-web/tests/dashboard_smoke.rs` — tighten tautological assertion.
- `crates/features-courses/tests/assignments_ssr.rs` — drop OR-form selector.
- `crates/features-courses/tests/live_room_smoke.rs` — `renders_command_failed_toast`, `hand_raise_shows_real_display_name`.
- `crates/backend/tests/live_room.rs`, `audit_seed.rs` — additions per the 2026-05-15 design.

**Deleted:**

- `tools/playwright-ui-check.spec.js`
- `tools/playwright-compose-ui-check.spec.js`

## Commit Discipline

After each task: `git status --short`, then `git add` the files this task touched, then `git commit -m "<type>: <task summary>"`. Do not revert unrelated uncommitted changes from earlier sessions.

---

## Task 1: Baseline Worktree Guard

**Files:**
- Inspect only: repository root.

- [ ] **Step 1: Record the starting state**

Run:

```powershell
git status --short
git log --oneline -5
```

Expected: existing uncommitted changes in live-room/auth files are present. Do not revert them. The `docs/superpowers/specs/2026-05-16-*` design doc is present (committed or uncommitted).

- [ ] **Step 2: Verify a fast baseline**

Run:

```powershell
cargo check -p backend
cargo check -p features-auth
cargo check -p features-courses
cargo check -p shell-web
```

Expected: PASS. If any fail with pre-existing errors, record them before changing code so later commits do not attribute the failures.

- [ ] **Step 3: Commit the design and plan docs (optional — only if user confirms)**

Run:

```powershell
git status --short
git add -- docs/superpowers/specs/2026-05-16-local-login-and-real-stack-ui-test-design.md docs/superpowers/plans/2026-05-16-local-login-and-real-stack-ui-test.md
git commit -m "docs: design local-login + auth-pass + real-stack ui test"
```

Expected: commit succeeds. Skip if the user prefers to commit docs at the end.

---

## Task 2: Backend — LocalLoginProfile password field

**Files:**
- Modify: `crates/backend/src/auth/local_login.rs`
- Test: `crates/backend/src/auth/local_login.rs` (`#[cfg(test)]`)

- [ ] **Step 1: Write the failing unit test**

Add at the bottom of `crates/backend/src/auth/local_login.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn profile(name: &str, email: &str, password: &str, token: &str) -> LocalLoginProfile {
        LocalLoginProfile {
            name: name.to_string(),
            token: token.to_string(),
            password: password.to_string(),
            email: email.to_string(),
            display_name: format!("Display {name}"),
            firebase_uid: format!("uid-{name}"),
        }
    }

    fn config_with(profiles: Vec<LocalLoginProfile>) -> LocalLoginConfig {
        LocalLoginConfig::from_profiles("local", true, profiles)
    }

    #[test]
    fn profile_for_email_password_matches_exact_email_and_password() {
        let cfg = config_with(vec![profile(
            "teacher",
            "t@example.test",
            "teacher-pass",
            "t-token",
        )]);
        let hit = cfg.profile_for_email_password("t@example.test", "teacher-pass");
        assert!(hit.is_some());
        assert_eq!(hit.unwrap().name, "teacher");
    }

    #[test]
    fn profile_for_email_password_is_email_case_insensitive() {
        let cfg = config_with(vec![profile(
            "teacher",
            "t@example.test",
            "teacher-pass",
            "t-token",
        )]);
        assert!(cfg
            .profile_for_email_password("T@Example.TEST", "teacher-pass")
            .is_some());
    }

    #[test]
    fn profile_for_email_password_rejects_wrong_password() {
        let cfg = config_with(vec![profile(
            "teacher",
            "t@example.test",
            "teacher-pass",
            "t-token",
        )]);
        assert!(cfg
            .profile_for_email_password("t@example.test", "wrong")
            .is_none());
    }

    #[test]
    fn profile_for_email_password_returns_none_when_disabled() {
        let cfg = LocalLoginConfig::from_profiles(
            "production",
            true,
            vec![profile(
                "teacher",
                "t@example.test",
                "teacher-pass",
                "t-token",
            )],
        );
        assert!(cfg
            .profile_for_email_password("t@example.test", "teacher-pass")
            .is_none());
    }

    #[test]
    fn profile_without_password_is_incomplete() {
        let cfg = config_with(vec![profile("teacher", "t@example.test", "", "t-token")]);
        assert!(cfg
            .profile_for_email_password("t@example.test", "")
            .is_none());
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run:

```powershell
cargo test -p backend auth::local_login::tests
```

Expected: FAIL — `password` field does not exist on `LocalLoginProfile`, `profile_for_email_password` is not a method on `LocalLoginConfig`.

- [ ] **Step 3: Add the password field**

In `crates/backend/src/auth/local_login.rs`, change `LocalLoginProfile`:

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalLoginProfile {
    pub name: String,
    pub token: String,
    pub password: String,
    pub email: String,
    pub display_name: String,
    pub firebase_uid: String,
}
```

- [ ] **Step 4: Read passwords from env in `from_env`**

Replace `from_env` in the same file:

```rust
pub fn from_env() -> Self {
    let teacher_email = std::env::var("LOCAL_LOGIN_EMAIL")
        .unwrap_or_else(|_| "local.teacher@example.test".into());
    let student_email = std::env::var("LOCAL_LOGIN_STUDENT_EMAIL")
        .unwrap_or_else(|_| "local.student@example.test".into());
    let teacher = LocalLoginProfile {
        name: "teacher".into(),
        token: env_first_non_empty(&["LOCAL_LOGIN_TEACHER_TOKEN", "LOCAL_LOGIN_TOKEN"]),
        password: std::env::var("LOCAL_LOGIN_TEACHER_PASSWORD").unwrap_or_default(),
        email: teacher_email.clone(),
        display_name: std::env::var("LOCAL_LOGIN_DISPLAY_NAME")
            .unwrap_or_else(|_| "Local Teacher".into()),
        firebase_uid: std::env::var("LOCAL_LOGIN_FIREBASE_UID")
            .unwrap_or_else(|_| firebase_uid_for_email(&teacher_email)),
    };
    let student = LocalLoginProfile {
        name: "student".into(),
        token: std::env::var("LOCAL_LOGIN_STUDENT_TOKEN").unwrap_or_default(),
        password: std::env::var("LOCAL_LOGIN_STUDENT_PASSWORD").unwrap_or_default(),
        email: student_email.clone(),
        display_name: std::env::var("LOCAL_LOGIN_STUDENT_DISPLAY_NAME")
            .unwrap_or_else(|_| "Local Student".into()),
        firebase_uid: std::env::var("LOCAL_LOGIN_STUDENT_FIREBASE_UID")
            .unwrap_or_else(|_| firebase_uid_for_email(&student_email)),
    };
    Self::from_profiles(
        &std::env::var("APP_ENV").unwrap_or_else(|_| "production".into()),
        env_bool("LOCAL_LOGIN_BYPASS_ENABLED"),
        vec![teacher, student],
    )
}
```

- [ ] **Step 5: Tighten `is_complete_profile` to require a password**

Replace `is_complete_profile`:

```rust
fn is_complete_profile(profile: &LocalLoginProfile) -> bool {
    !profile.name.trim().is_empty()
        && !profile.token.trim().is_empty()
        && !profile.password.trim().is_empty()
        && !profile.email.trim().is_empty()
        && !profile.firebase_uid.trim().is_empty()
}
```

- [ ] **Step 6: Add the lookup method**

Add to `impl LocalLoginConfig`:

```rust
pub fn profile_for_email_password(&self, email: &str, password: &str) -> Option<&LocalLoginProfile> {
    if !self.is_enabled() {
        return None;
    }
    let email_lc = email.trim().to_ascii_lowercase();
    self.profiles.iter().find(|p| {
        is_complete_profile(p)
            && p.email.trim().to_ascii_lowercase() == email_lc
            && p.password == password
    })
}
```

- [ ] **Step 7: Verify all unit tests pass**

Run:

```powershell
cargo test -p backend auth::local_login::tests
```

Expected: PASS (all five tests).

- [ ] **Step 8: Commit**

```powershell
git add -- crates/backend/src/auth/local_login.rs
git commit -m "feat: add password field and email+password lookup to LocalLoginConfig"
```

---

## Task 3: Backend — fixture profiles in tests acquire password

**Files:**
- Modify: `crates/backend/tests/local_login_bypass.rs`
- Modify: `crates/backend/tests/fixtures/*` (only if `LocalLoginProfile` is constructed there)

- [ ] **Step 1: Search for `LocalLoginProfile {` construction sites in tests**

Run:

```powershell
rg -n "LocalLoginProfile \{" crates\backend\tests
```

Expected: every construction site listed. Each one needs `password: "...".to_string(),`.

- [ ] **Step 2: Update test fixture constructions to include password**

In `crates/backend/tests/local_login_bypass.rs`, change the `local_config` helper:

```rust
fn local_config(app_env: &str, enabled: bool) -> LocalLoginConfig {
    LocalLoginConfig::from_profiles(
        app_env,
        enabled,
        vec![
            LocalLoginProfile {
                name: "teacher".to_string(),
                token: "local-teacher-token".to_string(),
                password: "teacher-pass".to_string(),
                email: "local.teacher@example.test".to_string(),
                display_name: "Local Teacher".to_string(),
                firebase_uid: "local-login-local-teacher".to_string(),
            },
            LocalLoginProfile {
                name: "student".to_string(),
                token: "local-student-token".to_string(),
                password: "student-pass".to_string(),
                email: "local.student@example.test".to_string(),
                display_name: "Local Student".to_string(),
                firebase_uid: "local-login-local-student".to_string(),
            },
        ],
    )
}
```

Repeat for any other `LocalLoginProfile { ... }` construction found in step 1, supplying a `password` field. Anywhere a test relies on a profile being recognized as complete, set a non-empty password.

- [ ] **Step 3: Compile-check tests**

Run:

```powershell
cargo test -p backend --no-run
```

Expected: PASS (compiles). No test execution yet.

- [ ] **Step 4: Commit**

```powershell
git add -- crates/backend/tests
git commit -m "test: supply password in LocalLoginProfile fixture constructions"
```

---

## Task 4: Backend — new `/v1/auth/local-login` endpoint

**Files:**
- Modify: `crates/backend/src/handlers/dev_login.rs` (replace module)
- Modify: `crates/backend/src/lib.rs` (route wiring)
- Test: `crates/backend/tests/local_login_bypass.rs`

- [ ] **Step 1: Write the failing integration tests**

Replace the contents of `crates/backend/tests/local_login_bypass.rs` (keep the helper from Task 3, replace the route-specific tests). The full file becomes:

```rust
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::middleware;
use axum::routing::get;
use axum::Router;
use http_body_util::BodyExt;
use tower::ServiceExt;

mod fixtures;

use backend::auth::jwks::JwksCache;
use backend::auth::local_login::{LocalLoginConfig, LocalLoginProfile};
use backend::auth::middleware::{require_auth, AuthState};
use backend::auth::verify::Verifier;
use backend::handlers::dev_login::{routes as dev_login_routes, DevLoginState};

fn local_config(app_env: &str, enabled: bool) -> LocalLoginConfig {
    LocalLoginConfig::from_profiles(
        app_env,
        enabled,
        vec![
            LocalLoginProfile {
                name: "teacher".to_string(),
                token: "local-teacher-token".to_string(),
                password: "teacher-pass".to_string(),
                email: "local.teacher@example.test".to_string(),
                display_name: "Local Teacher".to_string(),
                firebase_uid: "local-login-local-teacher".to_string(),
            },
            LocalLoginProfile {
                name: "student".to_string(),
                token: "local-student-token".to_string(),
                password: "student-pass".to_string(),
                email: "local.student@example.test".to_string(),
                display_name: "Local Student".to_string(),
                firebase_uid: "local-login-local-student".to_string(),
            },
        ],
    )
}

async fn json_response(response: axum::response::Response) -> serde_json::Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

async fn body_text(response: axum::response::Response) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8_lossy(&bytes).into_owned()
}

fn post_local_login(body: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/v1/auth/local-login")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

#[tokio::test]
async fn local_login_email_password_grants_teacher_token() {
    let pool = fixtures::pool().await;
    let app = dev_login_routes(DevLoginState {
        pool,
        config: local_config("local", true),
    });
    let response = app
        .oneshot(post_local_login(
            r#"{"email":"local.teacher@example.test","password":"teacher-pass"}"#,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json = json_response(response).await;
    assert_eq!(json["id_token"], "local-teacher-token");
}

#[tokio::test]
async fn local_login_email_password_grants_student_token() {
    let pool = fixtures::pool().await;
    let app = dev_login_routes(DevLoginState {
        pool,
        config: local_config("local", true),
    });
    let response = app
        .oneshot(post_local_login(
            r#"{"email":"local.student@example.test","password":"student-pass"}"#,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json = json_response(response).await;
    assert_eq!(json["id_token"], "local-student-token");
}

#[tokio::test]
async fn local_login_case_insensitive_email_matches() {
    let pool = fixtures::pool().await;
    let app = dev_login_routes(DevLoginState {
        pool,
        config: local_config("local", true),
    });
    let response = app
        .oneshot(post_local_login(
            r#"{"email":"LOCAL.TEACHER@EXAMPLE.TEST","password":"teacher-pass"}"#,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json = json_response(response).await;
    assert_eq!(json["id_token"], "local-teacher-token");
}

#[tokio::test]
async fn local_login_wrong_password_returns_401() {
    let pool = fixtures::pool().await;
    let app = dev_login_routes(DevLoginState {
        pool,
        config: local_config("local", true),
    });
    let response = app
        .oneshot(post_local_login(
            r#"{"email":"local.teacher@example.test","password":"nope"}"#,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body = body_text(response).await;
    assert!(
        !body.contains("local-teacher-token"),
        "401 body must not leak the token: {body}"
    );
}

#[tokio::test]
async fn local_login_unknown_email_returns_401() {
    let pool = fixtures::pool().await;
    let app = dev_login_routes(DevLoginState {
        pool,
        config: local_config("local", true),
    });
    let response = app
        .oneshot(post_local_login(
            r#"{"email":"who@example.test","password":"teacher-pass"}"#,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn local_login_missing_password_field_returns_400() {
    let pool = fixtures::pool().await;
    let app = dev_login_routes(DevLoginState {
        pool,
        config: local_config("local", true),
    });
    let response = app
        .oneshot(post_local_login(
            r#"{"email":"local.teacher@example.test"}"#,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn local_login_when_disabled_returns_403() {
    let pool = fixtures::pool().await;
    let app = dev_login_routes(DevLoginState {
        pool,
        config: local_config("production", true),
    });
    let response = app
        .oneshot(post_local_login(
            r#"{"email":"local.teacher@example.test","password":"teacher-pass"}"#,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn local_token_authenticates_me_through_real_middleware() {
    let pool = fixtures::pool().await;
    let config = local_config("local", true);
    let jwks = JwksCache::new("http://127.0.0.1:1/jwks", Duration::from_secs(3600));
    let verifier = Arc::new(Verifier::new(
        jwks,
        "elementors-206db",
        "https://securetoken.google.com/elementors-206db",
    ));
    let auth_state = AuthState {
        pool,
        verifier,
        local_login: config,
    };
    let app = Router::new()
        .route("/v1/me", get(backend::handlers::me::me))
        .layer(middleware::from_fn_with_state(auth_state, require_auth));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/me")
                .header("authorization", "Bearer local-teacher-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = json_response(response).await;
    assert_eq!(json["firebase_uid"], "local-login-local-teacher");
    assert_eq!(json["email"], "local.teacher@example.test");
    assert_eq!(json["display_name"], "Local Teacher");
}
```

The two removed tests (`dev_login_config_is_disabled_in_production_even_if_flag_is_true`, `local_dev_login_returns_token_when_enabled`, `local_dev_login_returns_selected_student_profile_token_when_enabled`, `local_token_query_param_authenticates_through_real_middleware`) cover the old endpoints we're removing. The query-param middleware test moves to the live-room safety pass as `access_token_query_accepted_on_ws_upgrade`.

- [ ] **Step 2: Run tests to verify they fail**

Run:

```powershell
cargo test -p backend --test local_login_bypass
```

Expected: FAIL — `/v1/auth/local-login` does not exist; tests get 404.

- [ ] **Step 3: Replace the handler module**

Replace `crates/backend/src/handlers/dev_login.rs` with:

```rust
use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::auth::jit_provision::ensure_user;
use crate::auth::local_login::LocalLoginConfig;
use crate::error::ApiError;

#[derive(Clone)]
pub struct DevLoginState {
    pub pool: PgPool,
    pub config: LocalLoginConfig,
}

#[derive(Deserialize)]
struct LocalLoginRequest {
    email: Option<String>,
    password: Option<String>,
}

#[derive(Serialize)]
struct LocalLoginResponse {
    id_token: String,
}

pub fn routes(state: DevLoginState) -> Router {
    Router::new()
        .route("/v1/auth/local-login", post(local_login))
        .with_state(state)
}

async fn local_login(
    State(state): State<DevLoginState>,
    Json(body): Json<LocalLoginRequest>,
) -> Result<Json<LocalLoginResponse>, ApiError> {
    if !state.config.is_enabled() {
        return Err(ApiError::Forbidden);
    }
    let email = body
        .email
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ApiError::BadRequest("email is required".into()))?;
    let password = body
        .password
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ApiError::BadRequest("password is required".into()))?;

    let profile = state
        .config
        .profile_for_email_password(email, password)
        .ok_or(ApiError::Unauthorized)?;

    ensure_user(&state.pool, &profile.claims())
        .await
        .map_err(|err| ApiError::Internal(format!("local user provisioning failed: {err}")))?;

    Ok(Json(LocalLoginResponse {
        id_token: profile.token.clone(),
    }))
}
```

If `ApiError::Unauthorized` does not exist, search `crates/backend/src/error.rs` and add a `Unauthorized` variant that maps to 401. Same for `BadRequest` (likely already exists, used by `me::normalize_schedule_days`).

- [ ] **Step 4: Add Unauthorized variant if needed**

Run:

```powershell
rg -n "ApiError::Unauthorized|ApiError::BadRequest" crates\backend\src
```

If `Unauthorized` is missing, add to `crates/backend/src/error.rs`:

```rust
pub enum ApiError {
    // existing variants...
    Unauthorized,
}

// in IntoResponse / status mapping:
ApiError::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized").into_response(),
```

Match the existing style in the file. If `Unauthorized` already exists, no change.

- [ ] **Step 5: Run tests to verify they pass**

Run:

```powershell
cargo test -p backend --test local_login_bypass
```

Expected: PASS (all eight tests).

- [ ] **Step 6: Commit**

```powershell
git add -- crates/backend/src/handlers/dev_login.rs crates/backend/src/error.rs crates/backend/tests/local_login_bypass.rs
git commit -m "feat: add /v1/auth/local-login endpoint"
```

---

## Task 5: Backend — env additions

**Files:**
- Modify: `.env`
- Modify: `.env.example`

- [ ] **Step 1: Add password vars to .env**

Append to `.env` (after the existing `LOCAL_LOGIN_*` block):

```
LOCAL_LOGIN_TEACHER_PASSWORD=local-teacher-pass
LOCAL_LOGIN_STUDENT_PASSWORD=local-student-pass
```

- [ ] **Step 2: Add password vars to .env.example**

Append to `.env.example` (same location):

```
LOCAL_LOGIN_TEACHER_PASSWORD=local-teacher-pass
LOCAL_LOGIN_STUDENT_PASSWORD=local-student-pass
```

- [ ] **Step 3: Commit**

```powershell
git add -- .env .env.example
git commit -m "chore: add LOCAL_LOGIN_*_PASSWORD env vars"
```

---

## Task 6: Frontend — api-client `local_login` + `use_api()` hook

**Files:**
- Modify: `crates/api-client/src/lib.rs`
- Modify: `crates/features-courses/src/api.rs` (if `api::dev_login` lives there — verify)

The current `api::dev_login` and `api::get_dev_login_config` are referenced from `crates/shell-web/src/routes/login.rs` via `features_courses::api`. The `features-courses` crate currently exposes the API surface; `api-client` is a stub. We add `local_login` next to `dev_login` in whichever file currently holds `dev_login`.

- [ ] **Step 1: Find where `dev_login` is currently implemented**

Run:

```powershell
rg -n "pub (async )?fn dev_login|pub (async )?fn get_dev_login_config" crates
```

Expected: locations identified (likely `crates/features-courses/src/api.rs` and/or `crates/api-client/src/lib.rs`).

- [ ] **Step 2: Add `local_login` next to `dev_login`**

In the file identified above, add (next to `dev_login`):

```rust
#[derive(serde::Deserialize)]
pub struct LocalLoginResponse {
    pub id_token: String,
}

pub async fn local_login(
    ctx: &ApiContext,
    email: &str,
    password: &str,
) -> Result<LocalLoginResponse, String> {
    let url = format!("{}/v1/auth/local-login", ctx.base_url);
    let body = serde_json::json!({ "email": email, "password": password });
    let response = http_client()
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("{e}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("local-login HTTP {status}"));
    }
    response.json().await.map_err(|e| format!("{e}"))
}
```

Match the existing helper conventions in the file (how it constructs the client, how it stringifies errors, how it gates `#[cfg(target_arch = "wasm32")]` if applicable). If the file uses `gloo_net::http::Request` instead of `reqwest`, mirror that pattern.

- [ ] **Step 3: Add `use_api()` hook**

In `crates/features-courses/src/api.rs` (or wherever `ApiContext` is defined and re-exported), add:

```rust
use dioxus::prelude::*;

pub fn use_api() -> ApiContext {
    use_context::<Signal<ApiContext>>().read().clone()
}
```

Mark the existing direct `use_context::<ApiContext>()` consumers (the bare snapshot) with a `#[deprecated]` note on whatever helper they currently call, OR mark the bare provider in `shell-web/src/lib.rs` for removal in Task 10. Use whichever surface change makes the compile-time warning visible.

- [ ] **Step 4: Compile-check**

Run:

```powershell
cargo check -p features-courses
cargo check -p shell-web
```

Expected: PASS.

- [ ] **Step 5: Commit**

```powershell
git add -- crates/features-courses/src/api.rs crates/api-client/src/lib.rs
git commit -m "feat: add api::local_login and api::use_api hook"
```

---

## Task 7: Frontend — features-auth submit handler tries local-login first

**Files:**
- Modify: `crates/features-auth/src/login.rs`
- Test: `crates/features-auth/tests/login.rs`

- [ ] **Step 1: Write the failing SSR tests**

In `crates/features-auth/tests/login.rs`, the existing tests render the form for heading-structure assertions. The new tests must cover submit-handler behavior — since the form's submit path is wasm-only (`#[cfg(target_arch = "wasm32")]`), we factor the decision logic into a pure helper that can be unit-tested. Add this unit-style test pattern by exposing an internal `LoginOutcome` enum.

At the top of `crates/features-auth/tests/login.rs`, add:

```rust
use features_auth::login_internals::{decide_outcome, LocalAttempt, LoginOutcome};

#[test]
fn local_login_success_yields_local_token() {
    let outcome = decide_outcome(LocalAttempt::Ok("local-token".into()), None);
    assert!(matches!(outcome, LoginOutcome::Token(ref t) if t == "local-token"));
}

#[test]
fn local_login_failure_falls_back_to_firebase_success() {
    let outcome = decide_outcome(
        LocalAttempt::Err("401".into()),
        Some(Ok("firebase-token".into())),
    );
    assert!(matches!(outcome, LoginOutcome::Token(ref t) if t == "firebase-token"));
}

#[test]
fn both_failing_yields_error_with_firebase_message() {
    let outcome = decide_outcome(
        LocalAttempt::Err("401".into()),
        Some(Err("invalid password".into())),
    );
    assert!(matches!(outcome, LoginOutcome::Err(ref m) if m.contains("invalid password")));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```powershell
cargo test -p features-auth --test login decide_outcome
```

Expected: FAIL — `features_auth::login_internals` does not exist.

- [ ] **Step 3: Extract `login_internals` and rewrite submit logic**

Replace `crates/features-auth/src/login.rs`:

```rust
use design_system::{Button, ButtonVariant, Card, FormError, Input, Spinner};
use dioxus::prelude::*;
use features_courses::api::{self, ApiContext};

pub mod login_internals {
    pub enum LocalAttempt {
        Ok(String),
        Err(String),
    }

    pub enum LoginOutcome {
        Token(String),
        Err(String),
    }

    pub fn decide_outcome(
        local: LocalAttempt,
        firebase: Option<Result<String, String>>,
    ) -> LoginOutcome {
        match local {
            LocalAttempt::Ok(token) => LoginOutcome::Token(token),
            LocalAttempt::Err(_) => match firebase {
                Some(Ok(token)) => LoginOutcome::Token(token),
                Some(Err(msg)) => LoginOutcome::Err(format!("Sign in failed: {msg}")),
                None => LoginOutcome::Err("Sign in failed".into()),
            },
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct LoginProps {
    pub on_success: EventHandler<String>,
}

#[component]
pub fn Login(props: LoginProps) -> Element {
    let mut email = use_signal(String::new);
    let mut password = use_signal(String::new);
    let error = use_signal(|| None::<String>);
    let submitting = use_signal(|| false);
    let form_success = props.on_success.clone();
    let button_success = props.on_success.clone();

    rsx! {
        div { class: "auth-screen auth-login-card",
            Card {
                div { class: "auth-heading-block",
                    p { class: "auth-eyebrow", "AulaLite" }
                    h1 { class: "auth-title", "Sign in to AulaLite" }
                }
                form {
                    class: "auth-form",
                    onsubmit: move |event| {
                        event.prevent_default();
                        submit_login(email, password, error, submitting, form_success.clone());
                    },
                    div { class: "field",
                        label { class: "field-label", "Email" }
                        Input {
                            value: email.read().clone(),
                            placeholder: "you@example.com".to_string(),
                            input_type: "email".to_string(),
                            disabled: *submitting.read(),
                            on_input: move |value| email.set(value),
                        }
                    }
                    div { class: "field",
                        label { class: "field-label", "Password" }
                        Input {
                            value: password.read().clone(),
                            placeholder: "Your password".to_string(),
                            input_type: "password".to_string(),
                            disabled: *submitting.read(),
                            on_input: move |value| password.set(value),
                        }
                    }
                    FormError { message: error.read().clone() }
                    div { class: "actions",
                        if *submitting.read() {
                            Spinner {}
                        } else {
                            Button {
                                label: "Sign in".to_string(),
                                variant: ButtonVariant::Primary,
                                on_click: move |_| {
                                    submit_login(email, password, error, submitting, button_success.clone());
                                },
                            }
                        }
                    }
                }
            }
        }
    }
}

fn submit_login(
    email: Signal<String>,
    password: Signal<String>,
    mut error: Signal<Option<String>>,
    mut submitting: Signal<bool>,
    on_success: EventHandler<String>,
) {
    let email_value = email.read().clone();
    let password_value = password.read().clone();

    submitting.set(true);
    error.set(None);

    #[cfg(target_arch = "wasm32")]
    {
        use login_internals::{decide_outcome, LocalAttempt, LoginOutcome};
        use platform_bridge::PlatformBridge;

        wasm_bindgen_futures::spawn_local(async move {
            let api_ctx = api::use_api();
            let local = match api::local_login(&api_ctx, &email_value, &password_value).await {
                Ok(resp) => LocalAttempt::Ok(resp.id_token),
                Err(e) => LocalAttempt::Err(e),
            };
            let firebase = match local {
                LocalAttempt::Ok(_) => None,
                LocalAttempt::Err(_) => {
                    let bridge = platform_bridge::web::WebBridge;
                    Some(
                        bridge
                            .sign_in_email_password(&email_value, &password_value)
                            .await
                            .map_err(|e| format!("{e}")),
                    )
                }
            };
            match decide_outcome(local, firebase) {
                LoginOutcome::Token(token) => on_success.call(token),
                LoginOutcome::Err(msg) => error.set(Some(msg)),
            }
            submitting.set(false);
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (email_value, password_value, on_success);
        error.set(Some("Sign-in only available on web in Phase 0".into()));
        submitting.set(false);
    }
}
```

Note: `api::use_api()` is added in Task 6. This file now depends on it.

Note: the wasm `submit_login` body calls `api::use_api()` inside `spawn_local`. Reactive hooks must run at component render time, not inside async tasks — so move the `use_api()` call up into the component and pass the `api_ctx` snapshot into `submit_login`. Adjust the function signature: `submit_login(email, password, error, submitting, on_success, api_ctx)` and read `api_ctx` from `use_api()` once in the component body. Apply this fix during step 3.

- [ ] **Step 4: Run unit tests**

Run:

```powershell
cargo test -p features-auth --test login decide_outcome
```

Expected: PASS (three tests).

- [ ] **Step 5: Compile-check the form**

Run:

```powershell
cargo check -p features-auth
cargo check -p features-auth --target wasm32-unknown-unknown
```

Expected: PASS. If the wasm target is not installed, skip the second check and rely on the wasm CI run.

- [ ] **Step 6: Commit**

```powershell
git add -- crates/features-auth/src/login.rs crates/features-auth/tests/login.rs
git commit -m "feat: features-auth Login tries local-login before Firebase"
```

---

## Task 8: Frontend — drop the `auth-local-panel` from shell-web login route

**Files:**
- Modify: `crates/shell-web/src/routes/login.rs`

- [ ] **Step 1: Replace the route body**

Replace `crates/shell-web/src/routes/login.rs` with:

```rust
// crates/shell-web/src/routes/login.rs
use dioxus::prelude::*;
use dioxus_router::use_navigator;
use design_system::AulaLogo;
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
}
```

If `design_system::AulaLogo` is not yet present (it lands in the SaaS readiness plan), remove the `AulaLogo` line and keep the surrounding copy. The brand element is non-essential for this task.

- [ ] **Step 2: Compile-check**

Run:

```powershell
cargo check -p shell-web
```

Expected: PASS. If `AulaLogo` is missing, remove it per step 1's note and re-run.

- [ ] **Step 3: Commit**

```powershell
git add -- crates/shell-web/src/routes/login.rs
git commit -m "feat: remove dev profile-button panel from login route"
```

---

## Task 9: Frontend — fix the `Signal<ApiContext>` snapshot leak

**Files:**
- Modify: `crates/shell-web/src/lib.rs`

This is the step that actually stops the bounce. Remove the bare snapshot provider; require all consumers to read via the Signal.

- [ ] **Step 1: Remove the snapshot provider line**

In `crates/shell-web/src/lib.rs`, remove this block (currently at lines ~105-108):

```rust
// Bare ApiContext for components that use_context::<ApiContext>() directly
// (e.g. file_picker::FilePicker). Captured at mount; the 401-retry
// interceptor handles token refresh internally.
use_context_provider(|| api_ctx_signal.read().clone());
```

Keep the `use_context_provider::<Signal<ApiContext>>(|| api_ctx_signal);` line above it. Keep the `UserContextSignal` provider line above it.

- [ ] **Step 2: Compile-check (will fail, intentionally)**

Run:

```powershell
cargo check -p shell-web
```

Expected: FAIL — consumers that do `use_context::<ApiContext>()` (snapshot) now panic at runtime or fail to find the context. Compile-check probably passes since `use_context` is generic; you'll see the breakage at runtime. To force a compile-time failure for accidental future use, add a `#[deprecated]` shim. Instead of that, fix the consumers in Task 10.

- [ ] **Step 3: Commit**

```powershell
git add -- crates/shell-web/src/lib.rs
git commit -m "fix: provide only Signal<ApiContext>, not snapshot"
```

---

## Task 10: Frontend — migrate consumers to `use_api()`

**Files:**
- Modify: each file returned by the grep below.

- [ ] **Step 1: Find every `use_context::<ApiContext>()` call site**

Run:

```powershell
rg -n "use_context::<ApiContext>" crates
```

Expected: a finite list, including the files named in the spec: `lesson_outline_view.rs`, `lesson_files_editor.rs`, `lesson_video_editor.rs`, `file_picker.rs`, `file_asset_image.rs`, `live_room_replay.rs`. Possibly more.

- [ ] **Step 2: Replace each call site**

For each file, replace:

```rust
let api_ctx = use_context::<ApiContext>();
```

with:

```rust
let api_ctx = api::use_api();
```

Add `use features_courses::api;` (or the equivalent import) at the top of each file if not already present. Match the existing import style of the file.

For consumers that hold the value across async boundaries, make sure `api_ctx` is read inside the synchronous render path (not inside `spawn`/`spawn_local`) — `use_api()` is a hook and must be called at component-render time.

- [ ] **Step 3: Compile-check**

Run:

```powershell
cargo check -p features-courses
cargo check -p shell-web
```

Expected: PASS.

- [ ] **Step 4: Verify the grep is empty**

Run:

```powershell
rg -n "use_context::<ApiContext>" crates
```

Expected: zero results. Every consumer reads from the Signal via `use_api()`.

- [ ] **Step 5: Commit**

```powershell
git add -- crates/features-courses/src
git commit -m "refactor: migrate ApiContext consumers to use_api()"
```

---

## Task 11: Manual browser smoke — verify the bounce stops

**Files:** none.

- [ ] **Step 1: Bring up the local stack**

Run:

```powershell
docker compose up -d
curl.exe http://localhost:8080/healthz
```

Expected: 200 OK.

- [ ] **Step 2: Run the dev server**

From `crates/shell-web`:

```powershell
dx serve --platform web --port 3000
```

Expected: server starts at `http://localhost:3000`.

- [ ] **Step 3: Audit-seed the backend**

In another terminal:

```powershell
curl.exe -X POST http://localhost:8080/v1/dev/audit-seed
```

Expected: 200 with `{ "tenant_slug": "local-audit", "course_slug": "audit-course", ... }`.

- [ ] **Step 4: Manually verify login**

In a browser at `http://localhost:3000/login`:

1. Type `local.teacher@example.test` and `local-teacher-pass`. Click Sign in.
2. Expected: redirected to `/`, dashboard visible.
3. Click into a course. Expected: course detail loads. URL is `/courses/audit-course`, not `/login`.
4. Click "My Courses". Expected: course list. URL is `/courses`.
5. Refresh the page. Expected: still signed in.
6. Sign out. Expected: redirected to `/login`.
7. Sign in as `local.student@example.test` / `local-student-pass`. Walk the student visible routes.

If any step bounces back to `/login`, capture the page state and stop — there is still a snapshot consumer to migrate.

- [ ] **Step 5: Record the smoke result**

If everything works, no commit (no files changed). Note the result in the implementation summary. If something breaks, investigate per Task 10 step 4 (the grep must be empty).

---

## Task 12: Live-room safety — `percent-encoding` dep + middleware

**Files:**
- Modify: `crates/backend/Cargo.toml`
- Modify: `crates/backend/src/auth/middleware.rs`
- Test: `crates/backend/tests/local_login_bypass.rs`

Apply the design at `docs/superpowers/specs/2026-05-15-live-room-safety-auth-pass-design.md` under "Backend changes > crates/backend/src/auth/middleware.rs".

- [ ] **Step 1: Write the failing access-token-on-WS-upgrade test**

In `crates/backend/tests/local_login_bypass.rs`, add:

```rust
#[tokio::test]
async fn access_token_query_rejected_on_non_upgrade() {
    let pool = fixtures::pool().await;
    let config = local_config("local", true);
    let jwks = JwksCache::new("http://127.0.0.1:1/jwks", Duration::from_secs(3600));
    let verifier = Arc::new(Verifier::new(
        jwks,
        "elementors-206db",
        "https://securetoken.google.com/elementors-206db",
    ));
    let auth_state = AuthState {
        pool,
        verifier,
        local_login: config,
    };
    let app = Router::new()
        .route("/v1/me", get(backend::handlers::me::me))
        .layer(middleware::from_fn_with_state(auth_state, require_auth));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/me?access_token=local-teacher-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn access_token_query_accepted_when_ws_upgrade() {
    let pool = fixtures::pool().await;
    let config = local_config("local", true);
    let jwks = JwksCache::new("http://127.0.0.1:1/jwks", Duration::from_secs(3600));
    let verifier = Arc::new(Verifier::new(
        jwks,
        "elementors-206db",
        "https://securetoken.google.com/elementors-206db",
    ));
    let auth_state = AuthState {
        pool,
        verifier,
        local_login: config,
    };
    let app = Router::new()
        .route("/v1/me", get(backend::handlers::me::me))
        .layer(middleware::from_fn_with_state(auth_state, require_auth));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/me?access_token=local-teacher-token")
                .header("upgrade", "websocket")
                .header("connection", "upgrade")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn access_token_url_decoded_on_ws_upgrade() {
    let pool = fixtures::pool().await;
    // Token containing characters that require percent-encoding in a URL.
    let token = "tkn+/abc=";
    let encoded = "tkn%2B%2Fabc%3D";
    let config = LocalLoginConfig::from_profiles(
        "local",
        true,
        vec![LocalLoginProfile {
            name: "teacher".to_string(),
            token: token.to_string(),
            password: "teacher-pass".to_string(),
            email: "local.teacher@example.test".to_string(),
            display_name: "Local Teacher".to_string(),
            firebase_uid: "local-login-local-teacher".to_string(),
        }],
    );
    let jwks = JwksCache::new("http://127.0.0.1:1/jwks", Duration::from_secs(3600));
    let verifier = Arc::new(Verifier::new(
        jwks,
        "elementors-206db",
        "https://securetoken.google.com/elementors-206db",
    ));
    let auth_state = AuthState {
        pool,
        verifier,
        local_login: config,
    };
    let app = Router::new()
        .route("/v1/me", get(backend::handlers::me::me))
        .layer(middleware::from_fn_with_state(auth_state, require_auth));

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/v1/me?access_token={encoded}"))
                .header("upgrade", "websocket")
                .header("connection", "upgrade")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```powershell
cargo test -p backend --test local_login_bypass access_token
```

Expected: FAIL — the middleware currently accepts `access_token` on any request (or rejects it on all).

- [ ] **Step 3: Add the dependency**

In `crates/backend/Cargo.toml`, add to `[dependencies]`:

```toml
percent-encoding = "2"
```

- [ ] **Step 4: Implement the middleware changes**

Apply the changes documented in the 2026-05-15 design under "Backend changes > crates/backend/src/auth/middleware.rs":

- Add `fn is_websocket_upgrade(headers: &HeaderMap) -> bool` checking `Upgrade: websocket` (case-insensitive) and `Connection: upgrade`.
- `query_access_token` decodes via `percent_encoding::percent_decode_str(...).decode_utf8()` and rejects lossy decode.
- Token resolution order: `Authorization: Bearer <jwt>` first, `?access_token=<jwt>` second only when `is_websocket_upgrade`.
- `local_login.claims_for_token` lookup moves after the JWT verifier attempt.

The exact code shape is in the design doc. Lift it verbatim.

- [ ] **Step 5: Verify tests pass**

Run:

```powershell
cargo test -p backend --test local_login_bypass
```

Expected: PASS (all eleven tests, including the three new access_token ones).

- [ ] **Step 6: Commit**

```powershell
git add -- crates/backend/Cargo.toml Cargo.lock crates/backend/src/auth/middleware.rs crates/backend/tests/local_login_bypass.rs
git commit -m "feat: gate access_token query param to websocket upgrades"
```

---

## Task 13: Live-room safety — trace-scrub for query params in logs

**Files:**
- Create: `crates/backend/src/trace_scrub.rs`
- Modify: `crates/backend/src/lib.rs`
- Modify: `crates/backend/src/main.rs` (or wherever `tower-http::trace` is wired)

Apply per the 2026-05-15 design under "Backend changes > crates/backend/src/main.rs (or trace-wiring location)".

- [ ] **Step 1: Write the failing unit test**

In `crates/backend/src/trace_scrub.rs`, add at creation:

```rust
//! Span scrubber that redacts auth tokens from logged URIs.

use axum::http::Request;
use http::Uri;
use tower_http::trace::MakeSpan;
use tracing::Span;

#[derive(Clone, Debug, Default)]
pub struct ScrubbingMakeSpan;

impl<B> MakeSpan<B> for ScrubbingMakeSpan {
    fn make_span(&mut self, request: &Request<B>) -> Span {
        let scrubbed = scrub_uri(request.uri());
        tracing::info_span!(
            "http_request",
            method = %request.method(),
            uri = %scrubbed,
        )
    }
}

pub fn scrub_uri(uri: &Uri) -> String {
    let path = uri.path();
    let Some(query) = uri.query() else {
        return path.to_string();
    };
    let parts: Vec<String> = query
        .split('&')
        .map(|pair| {
            let (key, _value) = pair.split_once('=').unwrap_or((pair, ""));
            let key_lc = key.to_ascii_lowercase();
            if matches!(key_lc.as_str(), "access_token" | "jwt" | "token") {
                format!("{key}=[REDACTED]")
            } else {
                pair.to_string()
            }
        })
        .collect();
    format!("{path}?{}", parts.join("&"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrubs_access_token_jwt_token_from_uri() {
        let uri = Uri::from_static("/v1/sessions/abc/join?access_token=secret&foo=1&jwt=AAA");
        assert_eq!(
            scrub_uri(&uri),
            "/v1/sessions/abc/join?access_token=[REDACTED]&foo=1&jwt=[REDACTED]"
        );
    }

    #[test]
    fn preserves_path_when_no_query() {
        let uri = Uri::from_static("/v1/me");
        assert_eq!(scrub_uri(&uri), "/v1/me");
    }

    #[test]
    fn case_insensitive_key_match() {
        let uri = Uri::from_static("/x?Access_Token=foo");
        assert_eq!(scrub_uri(&uri), "/x?Access_Token=[REDACTED]");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail (or compile-check)**

Run:

```powershell
cargo test -p backend trace_scrub::tests
```

Expected: FAIL — module does not exist yet. After creating the file, this will compile and pass.

- [ ] **Step 3: Wire the module**

Add to `crates/backend/src/lib.rs` (top, after existing `pub mod`):

```rust
pub mod trace_scrub;
```

- [ ] **Step 4: Use the scrubber in trace wiring**

In `crates/backend/src/main.rs` (or wherever `tower_http::trace::TraceLayer::new_for_http()` is constructed), wrap with `.make_span_with(trace_scrub::ScrubbingMakeSpan)`. Run:

```powershell
rg -n "TraceLayer::new_for_http" crates\backend\src
```

Apply the `.make_span_with(backend::trace_scrub::ScrubbingMakeSpan)` modifier at the located call site.

- [ ] **Step 5: Verify tests pass**

Run:

```powershell
cargo test -p backend trace_scrub
```

Expected: PASS.

- [ ] **Step 6: Commit**

```powershell
git add -- crates/backend/src/trace_scrub.rs crates/backend/src/lib.rs crates/backend/src/main.rs
git commit -m "feat: scrub access_token/jwt/token from logged URIs"
```

---

## Task 14: Live-room safety — core-types `CommandFailed`, `display_name`, close codes

**Files:**
- Modify: `crates/core-types/src/live_room.rs`

Apply per the 2026-05-15 design under "core-types/src/live_room.rs".

- [ ] **Step 1: Add the new variant and fields**

In `crates/core-types/src/live_room.rs`:

- Add to `ServerEvent`:
  ```rust
  CommandFailed { command: String, reason: String },
  ```
- Add `display_name: String` to `HandRaiseChanged`, `StudentPublishing`, `StudentDemoted` (and the corresponding `ServerEvent` arms if the data is inlined).
- Add the close-codes module:
  ```rust
  pub mod close_codes {
      pub const AUTH_EXPIRED: u16 = 4001;
      pub const AUTH_INVALID: u16 = 4003;
  }
  ```

- [ ] **Step 2: Verify `#[serde(deny_unknown_fields)]` is absent**

Run:

```powershell
rg -n "deny_unknown_fields" crates\core-types
```

Expected: no occurrences on `ServerEvent` or the touched structs. If any exist, remove them or note as a follow-up.

- [ ] **Step 3: Compile-check**

Run:

```powershell
cargo check -p core-types
cargo check -p backend
cargo check -p features-courses
```

Expected: FAIL on backend or features-courses because match arms over `ServerEvent` are non-exhaustive. Add wildcard `_ =>` arms or explicit no-op handlers wherever the compiler complains. Apply minimal changes to make compile pass; full handling lands in Task 16 (frontend toast) and Task 15 (backend emission).

- [ ] **Step 4: Commit**

```powershell
git add -- crates/core-types/src/live_room.rs crates/backend/src crates/features-courses/src
git commit -m "feat: add CommandFailed event, display_name fields, close codes"
```

---

## Task 15: Live-room safety — `live_sessions.rs` handler hardening

**Files:**
- Modify: `crates/backend/src/handlers/live_sessions.rs`
- Test: `crates/backend/tests/live_room.rs`

Apply per the 2026-05-15 design under "Backend changes > crates/backend/src/handlers/live_sessions.rs". This task is large; split steps for review.

- [ ] **Step 1: Write the failing tests**

In `crates/backend/tests/live_room.rs`, add the tests listed in the 2026-05-15 design under "Test plan > Backend integration":

- `whep_bearer_e2e_real_jwt`
- `whep_main_url_has_no_jwt_in_query`
- `mediamtx_read_accepts_lowercase_bearer`
- `mediamtx_read_accepts_bearer_with_whitespace`
- `command_failure_emits_event`
- `messages_limit_clamped_to_200`
- `messages_inner_rejects_non_finite_to`
- `hand_raise_event_includes_display_name`

Test bodies follow the conventions in the existing `live_room.rs` test file. Use the helper functions already present for spinning up the broker, joining a session, and asserting on event streams. For exact test shapes, see the design doc's test plan section.

- [ ] **Step 2: Run tests to verify failures**

Run:

```powershell
cargo test -p backend --test live_room -- --nocapture
```

Expected: the eight new tests FAIL.

- [ ] **Step 3: Apply the handler changes**

Apply each change documented in the 2026-05-15 design:

- `main_url` and `screen_url` no longer append `?jwt=<jwt>` for `transport_mode = "webrtc"`.
- MediaMTX bearer-strip with whitespace/case tolerance + query-body `jwt=` fallback.
- `messages_inner`: `let limit = q.limit.unwrap_or(50).clamp(1, 200);`.
- `f64`-finite + `MAX_SAFE_SECS = 4_102_444_800.0` guard for `to_secs`.
- Drop the dead `_ => format!("{public_webrtc_url}/{sp}/whep")` arm.
- Every silent `let _ = …; return;` in command handlers becomes `tracing::warn!` + emit `ServerEvent::CommandFailed`. Applies to `AcceptHand`, `Chat`, `DeleteMessage`, `Kick`, `DemoteHand`. The `clear_student_publish_nonce` failure must occur before the broker emits `Demoted` — if clear fails, emit `CommandFailed`, suppress `Demoted`.
- `HandRaiseChanged`, `StudentPublishing`, `StudentDemoted` events gain `display_name: String` via the user-side join in `db::live_room::*`.

Exact code shape is in the design doc. Apply verbatim, preserving existing uncommitted local edits in the file.

- [ ] **Step 4: Verify tests pass**

Run:

```powershell
cargo test -p backend --test live_room
```

Expected: PASS (all tests, including the eight new ones).

- [ ] **Step 5: Commit**

```powershell
git add -- crates/backend/src/handlers/live_sessions.rs crates/backend/tests/live_room.rs crates/backend/src/db
git commit -m "feat: harden live_sessions handler and emit CommandFailed"
```

---

## Task 16: Live-room safety — `dev_seed.rs` starts_at consistency

**Files:**
- Modify: `crates/backend/src/handlers/dev_seed.rs`
- Test: `crates/backend/tests/audit_seed.rs`

Apply per the 2026-05-15 design.

- [ ] **Step 1: Write the failing test**

In `crates/backend/tests/audit_seed.rs`, add:

```rust
#[tokio::test]
async fn starts_at_consistent_between_series_and_session() {
    // Drive /v1/dev/audit-seed twice, assert the seeded series and session
    // share the same starts_at within one transaction.
    // Full test body follows the existing pattern in the file.
}
```

For the exact body, see the design doc and the existing fixture conventions in `audit_seed.rs`.

- [ ] **Step 2: Run test to verify failure**

Run:

```powershell
cargo test -p backend --test audit_seed starts_at_consistent
```

Expected: FAIL.

- [ ] **Step 3: Compute `starts_at` once and thread it**

In `crates/backend/src/handlers/dev_seed.rs`, change `seed` to compute `starts_at` once and pass it explicitly into both `get_or_create_live_series` and `get_or_create_live_session`.

- [ ] **Step 4: Verify**

Run:

```powershell
cargo test -p backend --test audit_seed
```

Expected: PASS.

- [ ] **Step 5: Commit**

```powershell
git add -- crates/backend/src/handlers/dev_seed.rs crates/backend/tests/audit_seed.rs
git commit -m "fix: thread single starts_at through audit-seed"
```

---

## Task 17: Live-room safety — `live_room_session` aggregate (new)

**Files:**
- Create: `crates/features-courses/src/live_room_session.rs`
- Modify: `crates/features-courses/src/lib.rs`

Apply per the 2026-05-15 design under "Frontend changes > crates/features-courses/src/live_room_session.rs (new)".

- [ ] **Step 1: Write the failing unit tests**

Create `crates/features-courses/src/live_room_session.rs` and add at the bottom:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_is_idempotent() {
        let mut session = LiveRoomSession::new(test_config(), test_api());
        // Stage 1: close() before any connect → no panic.
        pollster::block_on(session.close());
        // Stage 2: close() again → still no panic, still no work.
        pollster::block_on(session.close());
        assert!(session.is_closed());
    }

    #[test]
    fn build_ws_url_urlencodes_token() {
        let url = build_ws_url("http://x.test", "session-1", "tkn+/abc=");
        assert!(
            url.contains("access_token=tkn%2B%2Fabc%3D"),
            "expected url-encoded token in {url}"
        );
    }

    // end_class_calls_close_before_post follows the design's
    // "frontend logic" test plan.

    fn test_config() -> SessionConfig { /* per design */ }
    fn test_api() -> features_courses::api::ApiContext { /* per design */ }
}
```

- [ ] **Step 2: Run tests to verify failure**

Run:

```powershell
cargo test -p features-courses live_room_session::tests
```

Expected: FAIL — module does not yet have these functions.

- [ ] **Step 3: Implement the aggregate**

Implement `LiveRoomSession` per the design doc's struct + impl block. Add `pub mod live_room_session;` to `crates/features-courses/src/lib.rs`.

- [ ] **Step 4: Verify**

Run:

```powershell
cargo test -p features-courses live_room_session
```

Expected: PASS.

- [ ] **Step 5: Commit**

```powershell
git add -- crates/features-courses/src/live_room_session.rs crates/features-courses/src/lib.rs
git commit -m "feat: add LiveRoomSession aggregate with Drop"
```

---

## Task 18: Live-room safety — socket close, Drop, onclose

**Files:**
- Modify: `crates/features-courses/src/live_room_socket.rs`

Apply per the 2026-05-15 design under "Frontend changes > crates/features-courses/src/live_room_socket.rs".

- [ ] **Step 1: Write the failing tests**

Add `#[cfg(test)]` tests to the file:

```rust
#[test]
fn onclose_4001_triggers_reconnect_with_fresh_token() {
    // Following the design doc's test plan: simulate a close event with
    // code 4001, assert reconnect_with_fresh_token was queued exactly once.
}

#[test]
fn onclose_4003_does_not_reconnect() {
    // close code 4003 surfaces to UI without reconnect.
}
```

- [ ] **Step 2: Run tests to verify failure**

Run:

```powershell
cargo test -p features-courses live_room_socket
```

Expected: FAIL.

- [ ] **Step 3: Implement socket changes**

Apply per design:

- `pub fn close(&mut self)` — synchronous `ws.close()`, clears closures.
- `set_onclose(|code|)` dispatches on close code: 4001 → reconnect with fresh token, 4003 → surface and stop, other → backoff (cap 5 attempts, 30s).
- URL builder: `format!("?access_token={}", urlencoding::encode(token))`.
- `LiveRoomSocket` implements `Drop`.
- `parse_event` errors `tracing::debug!`.

- [ ] **Step 4: Verify**

Run:

```powershell
cargo test -p features-courses live_room_socket
```

Expected: PASS.

- [ ] **Step 5: Commit**

```powershell
git add -- crates/features-courses/src/live_room_socket.rs
git commit -m "feat: socket close/Drop + 4001/4003 onclose handling"
```

---

## Task 19: Live-room safety — WHIP and WHEP close, Drop, bearer

**Files:**
- Modify: `crates/features-courses/src/live_room_whip.rs`
- Modify: `crates/features-courses/src/live_room_whep.rs`

Apply per the 2026-05-15 design under "Frontend changes > crates/features-courses/src/live_room_whip.rs" and "crates/features-courses/src/live_room_whep.rs".

- [ ] **Step 1: WHIP changes**

In `live_room_whip.rs`:

- `pub async fn close(&mut self) -> Result<()>` — `pc.close()`, `DELETE resource_url` if present, stop all `MediaStreamTrack`s.
- `impl Drop` — best-effort `spawn_local` close.
- Replace `Reflect::get(&offer, "sdp")` with typed `offer.dyn_into::<RtcSessionDescription>().sdp()`.

- [ ] **Step 2: WHEP changes**

In `live_room_whep.rs`:

- `WhepViewer::close()` mirrors `WhipPublisher::close()` (DELETE on `resource_url`, stop tracks).
- `impl Drop`.
- Drop the URL-substring `?jwt=` guard; always attach `Authorization: Bearer <jwt>`.

- [ ] **Step 3: Compile-check**

Run:

```powershell
cargo check -p features-courses
```

Expected: PASS.

- [ ] **Step 4: Commit**

```powershell
git add -- crates/features-courses/src/live_room_whip.rs crates/features-courses/src/live_room_whep.rs
git commit -m "feat: WHIP/WHEP close + Drop + bearer-only auth"
```

---

## Task 20: Live-room safety — session-driven view, broadcast, route wiring

**Files:**
- Modify: `crates/shell-web/src/routes/live_session.rs`
- Modify: `crates/features-courses/src/live_room_view.rs`
- Modify: `crates/features-courses/src/live_room_broadcast.rs`
- Test: `crates/features-courses/tests/live_room_smoke.rs`

Apply per the 2026-05-15 design under "Frontend changes > crates/features-courses/src/live_room_view.rs", "live_room_broadcast.rs", and "crates/shell-web/src/routes/live_session.rs".

- [ ] **Step 1: Write the failing SSR tests**

In `crates/features-courses/tests/live_room_smoke.rs`, add:

```rust
#[test]
fn renders_command_failed_toast() {
    // Render view with a CommandFailed event already in state.
    // Assert .system-state--error toast is present with the reason.
}

#[test]
fn hand_raise_shows_real_display_name() {
    // Render view after a HandRaiseChanged event with display_name="Student Sam".
    // Assert "Student Sam" is in the rendered HTML; assert no "user-..." short id.
}
```

- [ ] **Step 2: Run tests to verify failures**

Run:

```powershell
cargo test -p features-courses --test live_room_smoke
```

Expected: FAIL.

- [ ] **Step 3: Wrap view/broadcast with LiveSessionShell**

In `crates/shell-web/src/routes/live_session.rs`:

- Wrap `LiveRoomView` / `LiveRoomBroadcast` inside a `LiveSessionShell` component that provides `Signal<LiveRoomSession>` and registers `use_on_destroy` to await `session.close()`.
- Remove the redundant `use_context_provider(|| api.clone())`.

Exact code shape in the design doc.

- [ ] **Step 4: Convert live_room_view to session-driven**

- `render_webrtc` / `render_hls` become `#[component] WebRtcStage` / `HlsStage` reading `Signal<LiveRoomSession>` via context.
- Hand-raise handler reads `display_name` from event payload (drop `format!("user-{short}")`).
- Subscribe to `ServerEvent::StudentPublishing` → `session.attach_student(user_id, publish_path)`.
- Subscribe to `ServerEvent::CommandFailed` → render `.system-state--error` toast in the existing right rail.

- [ ] **Step 5: Convert live_room_broadcast to session-driven**

- `go_live_flow(&mut LiveRoomSession, transport)` — store publisher inside the session, not a local.
- `end_class_flow(&mut LiveRoomSession)` — calls `session.end_class().await` (which closes locally first, then POSTs `/end-class`). Errors surface to UI.

- [ ] **Step 6: Verify**

Run:

```powershell
cargo test -p features-courses --test live_room_smoke
cargo check -p shell-web
```

Expected: PASS.

- [ ] **Step 7: Commit**

```powershell
git add -- crates/shell-web/src/routes/live_session.rs crates/features-courses/src/live_room_view.rs crates/features-courses/src/live_room_broadcast.rs crates/features-courses/tests/live_room_smoke.rs
git commit -m "feat: live-room view + broadcast become session-driven"
```

---

## Task 21: Test-quality fixes

**Files:**
- Modify: `crates/shell-web/tests/dashboard_smoke.rs`
- Modify: `crates/features-courses/tests/assignments_ssr.rs`

Apply per the 2026-05-15 design under "Test plan > Test-quality fixes from the review".

- [ ] **Step 1: Tighten dashboard_smoke tautological assertion**

In `crates/shell-web/tests/dashboard_smoke.rs`, find any assertion of the form:

```rust
assert!(
    html.contains("/schedule") || !html.contains("My Schedule"),
    ...
);
```

Replace with the strict form (per the design doc):

```rust
assert!(html.contains("href=\"/schedule\""), "schedule link should use /schedule: {html}");
assert!(!html.contains("href=\"/me/schedule\""), "leaked API path: {html}");
```

- [ ] **Step 2: Drop OR-form selector in assignments_ssr**

In `crates/features-courses/tests/assignments_ssr.rs`, change:

```rust
assert!(html.contains("assignment-shell") || html.contains("assignment-list"), "got: {html}");
```

to the strict form:

```rust
assert!(html.contains("assignment-shell"), "got: {html}");
```

- [ ] **Step 3: Verify**

Run:

```powershell
cargo test -p shell-web --test dashboard_smoke
cargo test -p features-courses --test assignments_ssr
```

Expected: PASS.

- [ ] **Step 4: Commit**

```powershell
git add -- crates/shell-web/tests/dashboard_smoke.rs crates/features-courses/tests/assignments_ssr.rs
git commit -m "test: tighten dashboard_smoke and assignments_ssr assertions"
```

---

## Task 22: Playwright — delete the two existing specs

**Files:**
- Delete: `tools/playwright-ui-check.spec.js`
- Delete: `tools/playwright-compose-ui-check.spec.js`

- [ ] **Step 1: Delete the files**

Run:

```powershell
Remove-Item tools\playwright-ui-check.spec.js
Remove-Item tools\playwright-compose-ui-check.spec.js
```

Expected: both files removed.

- [ ] **Step 2: Commit the deletion**

```powershell
git add -- tools/
git commit -m "chore: remove obsolete playwright specs"
```

Note: these files are listed as untracked in `git status` per the session-start snapshot. If they are not tracked, `git add tools/` after deletion is a no-op. In that case skip the commit.

---

## Task 23: Playwright — create the real-stack spec

**Files:**
- Create: `tools/ui-real-stack.spec.js`

- [ ] **Step 1: Write the spec**

Create `tools/ui-real-stack.spec.js`:

```javascript
const fs = require("node:fs");
const path = require("node:path");
const { expect, test } = require("@playwright/test");

const baseURL = process.env.BASE_URL || "http://127.0.0.1:3000";
const apiBase = process.env.API_BASE || "http://127.0.0.1:8080";
const screenshotDir = path.join(process.cwd(), "target", "playwright-ui");

const teacherEmail = process.env.LOCAL_LOGIN_EMAIL || "local.teacher@example.test";
const teacherPassword = process.env.LOCAL_LOGIN_TEACHER_PASSWORD || "local-teacher-pass";
const studentEmail = process.env.LOCAL_LOGIN_STUDENT_EMAIL || "local.student@example.test";
const studentPassword = process.env.LOCAL_LOGIN_STUDENT_PASSWORD || "local-student-pass";

test.describe.configure({ mode: "serial" });

let seedCtx = null;

test.beforeAll(async ({ request }) => {
  fs.mkdirSync(screenshotDir, { recursive: true });
  const seedResponse = await request.post(`${apiBase}/v1/dev/audit-seed`);
  expect(seedResponse.ok(), await seedResponse.text()).toBeTruthy();
  const seed = await seedResponse.json();

  const tokenResp = await request.post(`${apiBase}/v1/auth/local-login`, {
    data: { email: teacherEmail, password: teacherPassword },
  });
  expect(tokenResp.ok(), await tokenResp.text()).toBeTruthy();
  const { id_token: teacherToken } = await tokenResp.json();
  const headers = { Authorization: `Bearer ${teacherToken}` };

  const courses = await apiJson(request, `${apiBase}/v1/courses`, headers);
  const course = courses.find((c) => c.slug === seed.course_slug);
  expect(course).toBeTruthy();

  const sessions = await apiJson(request, `${apiBase}/v1/courses/${course.id}/sessions`, headers);
  expect(sessions.length).toBeGreaterThan(0);
  const liveStartsAt = new Date(Date.now() - 60_000).toISOString();
  const patch = await request.patch(`${apiBase}/v1/sessions/${sessions[0].session_id}`, {
    headers,
    data: { starts_at: liveStartsAt, duration_minutes: 45, title: "Audit Live Class", status: "scheduled" },
  });
  expect(patch.ok(), await patch.text()).toBeTruthy();

  const assignments = await apiJson(
    request,
    `${apiBase}/v1/courses/${course.id}/assignments?include_drafts=true`,
    headers,
  );
  expect(assignments.length).toBeGreaterThan(0);

  seedCtx = { seed, course, session: sessions[0], assignment: assignments[0] };
});

test("teacher walks the workspace via email+password login", async ({ page }) => {
  const { seed, course, session, assignment } = seedCtx;
  const consoleErrors = collectConsoleErrors(page);

  await page.goto(`${baseURL}/login`, { waitUntil: "domcontentloaded" });
  await expect(page.locator(".auth-composite")).toBeVisible();
  await screenshot(page, "teacher-login");

  await page.locator("input[type=email]").fill(teacherEmail);
  await page.locator("input[type=password]").fill(teacherPassword);
  await page.getByRole("button", { name: "Sign in" }).click();

  await expect(page.locator(".dashboard-hero")).toBeVisible({ timeout: 15000 });
  await assertStyleHealth(page, "teacher-dashboard");
  await screenshot(page, "teacher-dashboard");

  const routes = [
    { name: "courses", path: "/courses", selector: ".course-list-page", text: "Audit Course" },
    { name: "course-detail", path: `/courses/${seed.course_slug}`, selector: ".course-detail-hero", text: "Audit Course" },
    { name: "course-people", path: `/courses/${seed.course_slug}/people`, selector: ".course-people", text: "Members" },
    { name: "course-schedule", path: `/courses/${seed.course_slug}/schedule`, selector: ".schedule-agenda", text: "Audit Live Class" },
    { name: "assignments", path: `/courses/${seed.course_slug}/assignments`, selector: ".assignment-shell", text: assignment.title },
    { name: "assignment-detail", path: `/courses/${seed.course_slug}/assignments/${assignment.id}`, selector: ".assignment-shell", text: assignment.title },
    { name: "redeem", path: "/redeem", selector: ".workflow-page", text: "Redeem an enrollment code" },
    { name: "schedule", path: "/schedule", selector: ".schedule-agenda", text: "Audit Live Class" },
    { name: "live-session", path: `/courses/${seed.course_slug}/sessions/${session.session_id}`, selector: ".live-room-shell", text: "Go Live" },
  ];

  for (const route of routes) {
    await navigateSpa(page, route.path);
    await expect(page.locator(route.selector).first()).toBeVisible({ timeout: 15000 });
    await expect(page.locator("body")).toContainText(route.text);
    await assertStyleHealth(page, `teacher-${route.name}`);
    await screenshot(page, `teacher-${route.name}`);
  }

  // Bounce probe — the bug this whole project fixes.
  await navigateSpa(page, `/courses/${seed.course_slug}`);
  await expect(page).toHaveURL(new RegExp(`/courses/${seed.course_slug}$`));
  await navigateSpa(page, "/");
  await expect(page).toHaveURL(new RegExp("/$"));
  await navigateSpa(page, `/courses/${seed.course_slug}`);
  await expect(page).not.toHaveURL(/\/login$/);

  expect(consoleErrors.filter((m) => !isIgnorableConsoleError(m))).toEqual([]);
});

test("student walks the visible subset", async ({ page }) => {
  const { seed, session, assignment } = seedCtx;
  const consoleErrors = collectConsoleErrors(page);

  await page.goto(`${baseURL}/login`, { waitUntil: "domcontentloaded" });
  await page.locator("input[type=email]").fill(studentEmail);
  await page.locator("input[type=password]").fill(studentPassword);
  await page.getByRole("button", { name: "Sign in" }).click();
  await expect(page.locator(".dashboard-hero")).toBeVisible({ timeout: 15000 });
  await screenshot(page, "student-dashboard");

  const routes = [
    { name: "courses", path: "/courses", selector: ".course-list-page", text: "Audit Course" },
    { name: "course-detail", path: `/courses/${seed.course_slug}`, selector: ".course-detail-hero", text: "Audit Course" },
    { name: "assignments", path: `/courses/${seed.course_slug}/assignments`, selector: ".assignment-shell", text: assignment.title },
    { name: "assignment-detail", path: `/courses/${seed.course_slug}/assignments/${assignment.id}`, selector: ".assignment-shell", text: assignment.title },
    { name: "redeem", path: "/redeem", selector: ".workflow-page", text: "Redeem an enrollment code" },
    { name: "schedule", path: "/schedule", selector: ".schedule-agenda", text: "Audit Live Class" },
    { name: "live-session", path: `/courses/${seed.course_slug}/sessions/${session.session_id}`, selector: ".live-room-shell", text: "Audit Course" },
  ];

  for (const route of routes) {
    await navigateSpa(page, route.path);
    await expect(page.locator(route.selector).first()).toBeVisible({ timeout: 15000 });
    await expect(page.locator("body")).toContainText(route.text);
    await assertStyleHealth(page, `student-${route.name}`);
    await screenshot(page, `student-${route.name}`);
  }

  // Teacher-only controls must be absent.
  await navigateSpa(page, `/courses/${seed.course_slug}/sessions/${session.session_id}`);
  await expect(page.getByRole("button", { name: "Go Live" })).toHaveCount(0);
  await navigateSpa(page, "/courses");
  await expect(page.getByRole("button", { name: "New Course" })).toHaveCount(0);

  expect(consoleErrors.filter((m) => !isIgnorableConsoleError(m))).toEqual([]);
});

async function apiJson(request, url, headers) {
  const r = await request.get(url, { headers });
  expect(r.ok(), `${url}: ${await r.text()}`).toBeTruthy();
  return r.json();
}

async function navigateSpa(page, target) {
  await page.evaluate((p) => {
    history.pushState({}, "", p);
    window.dispatchEvent(new PopStateEvent("popstate"));
  }, target);
  await page.waitForLoadState("networkidle").catch(() => {});
}

async function assertStyleHealth(page, name) {
  const result = await page.evaluate(() => {
    const doc = document.documentElement;
    const body = document.body;
    const viewportWidth = doc.clientWidth;
    const viewportHeight = window.innerHeight;
    const els = Array.from(
      document.querySelectorAll("h1,h2,h3,p,a,button,label,input,textarea,select,.ds-card,.dashboard-stat,.schedule-item,.assignment-list__row,.course-card-art"),
    );
    const badBounds = [];
    for (const el of els) {
      if (el.closest("[class^='dx-'], [class*=' dx-']")) continue;
      const rect = el.getBoundingClientRect();
      const style = window.getComputedStyle(el);
      if (style.visibility === "hidden" || style.display === "none" || rect.width < 1 || rect.height < 1 || rect.bottom < 0 || rect.top > viewportHeight) continue;
      if (rect.left < -2 || rect.right > viewportWidth + 2) {
        badBounds.push({ tag: el.tagName.toLowerCase(), className: String(el.className || ""), left: Math.round(rect.left), right: Math.round(rect.right), viewportWidth });
      }
    }
    return {
      overflowX: Math.max(doc.scrollWidth, body.scrollWidth) - viewportWidth,
      bodyTextLength: (body.innerText || "").trim().length,
      badBounds,
    };
  });
  expect(result.bodyTextLength, `${name} text`).toBeGreaterThan(20);
  expect(result.overflowX, `${name} overflow`).toBeLessThanOrEqual(2);
  expect(result.badBounds, `${name} bounds`).toEqual([]);
}

async function screenshot(page, name) {
  await page.screenshot({ path: path.join(screenshotDir, `${name}.png`), fullPage: true });
}

function collectConsoleErrors(page) {
  const errors = [];
  page.on("console", (m) => { if (m.type() === "error") errors.push(m.text()); });
  page.on("pageerror", (e) => errors.push(e.message));
  return errors;
}

function isIgnorableConsoleError(message) {
  return /favicon|Firebase auth initialization failed|ERR_ABORTED|ResizeObserver loop limit exceeded/i.test(message);
}
```

- [ ] **Step 2: Sanity-check the spec parses**

Run:

```powershell
npx playwright test tools/ui-real-stack.spec.js --list
```

Expected: lists the two tests (`teacher walks the workspace via email+password login`, `student walks the visible subset`).

- [ ] **Step 3: Commit**

```powershell
git add -- tools/ui-real-stack.spec.js
git commit -m "test: add real-stack playwright spec for teacher and student"
```

---

## Task 24: Playwright — orchestration script and README

**Files:**
- Create: `tools/run-ui-check.ps1`
- Create: `tools/README.md`

- [ ] **Step 1: Write the script**

Create `tools/run-ui-check.ps1`:

```powershell
# Local-only orchestration for tools/ui-real-stack.spec.js.
# Brings up the backend stack, starts the frontend dev server,
# runs Playwright, then leaves both running so failures can be inspected.
# Pass -Clean to tear down compose + dx serve after a successful run.

[CmdletBinding()]
param(
    [switch]$Clean
)

$ErrorActionPreference = "Stop"

function Wait-ForUrl {
    param([string]$Url, [int]$TimeoutSeconds = 60)
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    while ((Get-Date) -lt $deadline) {
        try {
            $resp = Invoke-WebRequest -UseBasicParsing -Uri $Url -TimeoutSec 5
            if ($resp.StatusCode -lt 500) { return $true }
        } catch { Start-Sleep -Seconds 2 }
    }
    throw "timed out waiting for $Url"
}

function Get-EnvFromFile {
    param([string]$Path)
    if (-not (Test-Path $Path)) { return @{} }
    $result = @{}
    Get-Content $Path | Where-Object { $_ -match '^[A-Z_]+=' } | ForEach-Object {
        $kv = $_ -split '=', 2
        $result[$kv[0]] = $kv[1]
    }
    return $result
}

$envFile = Get-EnvFromFile -Path ".env"
foreach ($k in @("LOCAL_LOGIN_EMAIL","LOCAL_LOGIN_TEACHER_PASSWORD","LOCAL_LOGIN_STUDENT_EMAIL","LOCAL_LOGIN_STUDENT_PASSWORD")) {
    if ($envFile.ContainsKey($k)) { Set-Item "env:$k" $envFile[$k] }
}

Write-Host "Starting docker compose..."
docker compose up -d

Write-Host "Waiting for backend healthz..."
Wait-ForUrl -Url "http://localhost:8080/healthz" -TimeoutSeconds 60

Write-Host "Starting dx serve in background..."
$dxLog = Join-Path $PSScriptRoot "..\target\dx-serve.log"
$dxProcess = Start-Process -FilePath "dx" `
    -ArgumentList "serve","--platform","web","--port","3000" `
    -WorkingDirectory (Resolve-Path "crates/shell-web") `
    -PassThru `
    -RedirectStandardOutput $dxLog `
    -RedirectStandardError $dxLog

try {
    Write-Host "Waiting for dx serve at http://localhost:3000..."
    Wait-ForUrl -Url "http://localhost:3000" -TimeoutSeconds 180

    Write-Host "Running playwright spec..."
    & npx playwright test tools/ui-real-stack.spec.js
    $exit = $LASTEXITCODE
    if ($exit -ne 0) {
        Write-Host "playwright failed (exit $exit) — leaving stack running for inspection"
        exit $exit
    }
    Write-Host "playwright passed"
}
finally {
    if ($Clean) {
        Write-Host "Cleanup: stopping dx serve and docker compose..."
        if ($dxProcess -and -not $dxProcess.HasExited) {
            Stop-Process -Id $dxProcess.Id -Force
        }
        docker compose down
    }
}
```

- [ ] **Step 2: Write the README**

Create `tools/README.md`:

```markdown
# Local UI Verification Tools

## ui-real-stack.spec.js

Single Playwright spec that drives the real backend (docker compose) and real
frontend (`dx serve`) end-to-end. Logs in as Local Teacher and Local Student
using credentials from `.env`, walks every applicable route, asserts the
rendered DOM and CSS, and screenshots each page to `target/playwright-ui/`.

### Prereqs

- Docker Desktop running
- Node 20+ with `npx playwright install chromium`
- Rust toolchain (the `dx` CLI from `cargo install dioxus-cli`)
- `.env` populated, including `LOCAL_LOGIN_*` and `LOCAL_LOGIN_*_PASSWORD`

### Run

From repo root:

```powershell
.\tools\run-ui-check.ps1
```

To tear down on success:

```powershell
.\tools\run-ui-check.ps1 -Clean
```

### Output

- Screenshots: `target/playwright-ui/<role>-<route>.png`
- dx serve log: `target/dx-serve.log`
- Playwright report: `playwright-report/` (default Playwright output)

### Coverage

Two tests, serial mode:

1. **teacher walks the workspace via email+password login** — dashboard,
   courses, course detail, people, schedule, assignments (list + detail),
   redeem, my schedule, live session.
2. **student walks the visible subset** — dashboard, courses, course detail,
   assignments, redeem, schedule, live session. Asserts teacher-only
   controls (Go Live, New Course) are absent.

Both tests assert a bounce-probe: after login, navigating between courses
and dashboard must not redirect to `/login`.

### Not covered

- Real WebRTC media playback (stage element + WS handshake only).
- Mobile gestures, cross-browser. Chromium only.
- CI. Local-only by design.
```

- [ ] **Step 3: Commit**

```powershell
git add -- tools/run-ui-check.ps1 tools/README.md
git commit -m "tools: add run-ui-check.ps1 and README"
```

---

## Task 25: Final verification

**Files:** none.

- [ ] **Step 1: Format and compile-check**

Run:

```powershell
cargo fmt --all --check
cargo check -p backend
cargo check -p core-types
cargo check -p api-client
cargo check -p features-auth
cargo check -p features-courses
cargo check -p shell-web
```

Expected: PASS.

- [ ] **Step 2: Run all Rust tests**

Run:

```powershell
cargo test -p backend
cargo test -p core-types
cargo test -p features-auth
cargo test -p features-courses
cargo test -p shell-web
```

Expected: PASS when Postgres and other compose services are running. If a service is unavailable, record the missing service in the final summary.

- [ ] **Step 3: Confirm no stale snapshot consumers**

Run:

```powershell
rg -n "use_context::<ApiContext>" crates
```

Expected: zero results.

- [ ] **Step 4: Run the real-stack Playwright check**

Run:

```powershell
.\tools\run-ui-check.ps1
```

Expected: both Playwright tests pass. Screenshots present under `target/playwright-ui/`.

- [ ] **Step 5: Manual sanity — bounce probe**

In the still-running browser, navigate manually as Local Teacher and Local Student between `/`, `/courses`, `/courses/<slug>`, `/courses/<slug>/sessions/<id>`. Confirm no redirect to `/login`.

- [ ] **Step 6: Final implementation summary**

Summarize:

```text
Changed files:
- (list every committed file)

New endpoints:
- POST /v1/auth/local-login

Removed endpoints:
- POST /v1/dev/login
- GET /v1/dev/login/config

Frontend signal contract:
- Signal<ApiContext> is the only provider
- use_api() hook is the only read path

Live-room changes:
- per docs/superpowers/specs/2026-05-15-live-room-safety-auth-pass-design.md

Test results:
- cargo test -p backend: <result>
- cargo test -p features-auth: <result>
- cargo test -p features-courses: <result>
- cargo test -p shell-web: <result>
- tools/run-ui-check.ps1: <result>

Known environmental blockers:
- ...
```

Expected: final answer is concise; includes the dev URL if the server is still running and a path to the screenshots.

---

## Self-Review Notes

This plan has been self-reviewed against the spec:

- **Spec coverage:** Each section of `2026-05-16-local-login-and-real-stack-ui-test-design.md` maps to a task above:
  - Spec "Backend changes" → Tasks 2, 3, 4, 5, 12, 13, 14, 15, 16
  - Spec "Frontend changes" → Tasks 6, 7, 8, 9, 10, 17, 18, 19, 20
  - Spec "Playwright spec design" → Tasks 22, 23, 24
  - Spec "Rollout order" steps 1-10 → Tasks 2-5 / 6-10 / 12-20 / 22-24
  - Spec test plan → Tasks 2, 4, 7, 12, 13, 15, 16, 17, 18, 20, 21, 23
- **Placeholder scan:** Where this plan delegates to the 2026-05-15 design doc ("apply per the 2026-05-15 design"), it identifies the specific design section by name so the engineer can locate the exact code shape without re-deriving anything. The 2026-05-15 design is required reading for tasks 12-20.
- **Type consistency:** `LocalLoginProfile` field added in Task 2 is used by name in Tasks 3 and 4. `profile_for_email_password` method name is consistent across Tasks 2 and 4. `use_api()` and `ApiContext` references are consistent across Tasks 6, 7, 8, 9, 10.
- **Sequencing:** Tasks 1-11 deliver a working local login flow independently (without the live-room safety pass). Tasks 12-21 then land the safety pass. Tasks 22-25 verify end-to-end. Each task is independently committable.

If executing inline, expect total wall-clock around 6-10 hours depending on familiarity with Dioxus + WebRTC paths. If executing via subagents, expect each task to run as one or two subagent dispatches.
