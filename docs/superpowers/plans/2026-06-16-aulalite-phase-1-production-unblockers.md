# AulaLite Phase 1 Production Unblockers Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restore the broken web assets, keep backend library tests DB-free, unblock MFA challenge under enforcement, and sanitize course syllabus/grading markdown before storage.

**Architecture:** Keep this phase narrow and production-focused. Restore deleted tracked static assets, move DB-backed JIT tests into integration-test scope, add a small explicit MFA challenge exemption in auth middleware, and reuse the existing backend markdown sanitizer for course patch fields.

**Tech Stack:** Rust 1.94 workspace, Axum backend, SQLx/Postgres integration tests, Dioxus shell-web static assets, existing `services::sanitize::clean_markdown`.

---

## File Structure

- Restore: `crates/shell-web/public/assets/blur-bridge.js`
- Restore: `crates/shell-web/public/assets/scorm-bridge.js`
- Restore: `crates/shell-web/public/assets/whiteboard-bridge.js`
- Restore: `crates/shell-web/public/service-worker.js`
- Restore: `crates/shell-web/public/vendor/mediapipe/wasm/vision_wasm_internal.js`
- Restore: `crates/shell-web/public/vendor/mediapipe/wasm/vision_wasm_nosimd_internal.js`
- Modify: `crates/backend/src/auth/jit_provision.rs`
  - Remove the DB-backed `#[cfg(test)]` test module so `cargo test -p backend --lib` does not connect to Postgres.
- Create: `crates/backend/tests/jit_provision.rs`
  - Own the DB-backed JIT provisioning integration tests that were previously in library scope.
- Modify: `crates/backend/src/auth/middleware.rs`
  - Add pure helper functions for MFA challenge-route detection and MFA gate decisions.
  - Use those helpers in `require_auth`.
  - Add DB-free unit tests for the helper behavior.
- Modify: `crates/backend/src/handlers/courses.rs`
  - Add a helper that sanitizes optional markdown patch fields while preserving double-option semantics.
  - Pass sanitized owned values into `db::courses::UpdateCourse`.
- Modify: `crates/backend/tests/courses_crud.rs`
  - Add a DB-backed regression test that proves unsafe syllabus/grading markdown is sanitized before it is returned by the syllabus endpoint.

## Task 1: Restore Referenced Web Assets

**Files:**
- Restore: `crates/shell-web/public/assets/blur-bridge.js`
- Restore: `crates/shell-web/public/assets/scorm-bridge.js`
- Restore: `crates/shell-web/public/assets/whiteboard-bridge.js`
- Restore: `crates/shell-web/public/service-worker.js`
- Restore: `crates/shell-web/public/vendor/mediapipe/wasm/vision_wasm_internal.js`
- Restore: `crates/shell-web/public/vendor/mediapipe/wasm/vision_wasm_nosimd_internal.js`
- Verify references in: `crates/shell-web/index.html`

- [ ] **Step 1: Verify the referenced assets are missing**

Run:

```powershell
$paths = @(
  "crates/shell-web/public/assets/blur-bridge.js",
  "crates/shell-web/public/assets/scorm-bridge.js",
  "crates/shell-web/public/assets/whiteboard-bridge.js",
  "crates/shell-web/public/service-worker.js",
  "crates/shell-web/public/vendor/mediapipe/wasm/vision_wasm_internal.js",
  "crates/shell-web/public/vendor/mediapipe/wasm/vision_wasm_nosimd_internal.js"
)
$paths | ForEach-Object { [pscustomobject]@{ Path = $_; Exists = Test-Path $_ } }
```

Expected before restore: every row has `Exists = False`.

- [ ] **Step 2: Restore the tracked files from `HEAD`**

Run:

```powershell
git restore --source=HEAD -- `
  crates/shell-web/public/assets/blur-bridge.js `
  crates/shell-web/public/assets/scorm-bridge.js `
  crates/shell-web/public/assets/whiteboard-bridge.js `
  crates/shell-web/public/service-worker.js `
  crates/shell-web/public/vendor/mediapipe/wasm/vision_wasm_internal.js `
  crates/shell-web/public/vendor/mediapipe/wasm/vision_wasm_nosimd_internal.js
```

Expected: command exits successfully with no output.

- [ ] **Step 3: Verify every referenced file now exists**

Run:

```powershell
$paths = @(
  "crates/shell-web/public/assets/blur-bridge.js",
  "crates/shell-web/public/assets/scorm-bridge.js",
  "crates/shell-web/public/assets/whiteboard-bridge.js",
  "crates/shell-web/public/service-worker.js",
  "crates/shell-web/public/vendor/mediapipe/wasm/vision_wasm_internal.js",
  "crates/shell-web/public/vendor/mediapipe/wasm/vision_wasm_nosimd_internal.js"
)
$missing = $paths | Where-Object { -not (Test-Path $_) }
if ($missing) { throw "Missing restored assets: $($missing -join ', ')" }
"all referenced assets exist"
```

Expected: `all referenced assets exist`.

- [ ] **Step 4: Verify `index.html` references match restored paths**

Run:

```powershell
Select-String -Path crates/shell-web/index.html -Pattern "blur-bridge|whiteboard-bridge|scorm-bridge|service-worker|vision_wasm" | ForEach-Object { $_.Line.Trim() }
```

Expected output includes:

```text
<script src="/assets/blur-bridge.js"></script>
<script src="/assets/whiteboard-bridge.js"></script>
<script src="/assets/scorm-bridge.js"></script>
.register("/service-worker.js")
```

- [ ] **Step 5: Commit restored assets**

Run:

```powershell
git add `
  crates/shell-web/public/assets/blur-bridge.js `
  crates/shell-web/public/assets/scorm-bridge.js `
  crates/shell-web/public/assets/whiteboard-bridge.js `
  crates/shell-web/public/service-worker.js `
  crates/shell-web/public/vendor/mediapipe/wasm/vision_wasm_internal.js `
  crates/shell-web/public/vendor/mediapipe/wasm/vision_wasm_nosimd_internal.js
git commit -m "fix(web): restore public runtime assets"
```

Expected: commit succeeds and contains only the restored asset files.

## Task 2: Move JIT Provisioning DB Tests Out Of Library Scope

**Files:**
- Modify: `crates/backend/src/auth/jit_provision.rs`
- Create: `crates/backend/tests/jit_provision.rs`

- [ ] **Step 1: Write the integration test file**

Create `crates/backend/tests/jit_provision.rs` with:

```rust
mod fixtures;

use backend::auth::jit_provision::ensure_user;
use backend::auth::verify::FirebaseClaims;
use chrono::Utc;

fn claims(uid: &str, email: &str, name: Option<&str>) -> FirebaseClaims {
    FirebaseClaims {
        sub: uid.into(),
        email: Some(email.into()),
        email_verified: Some(true),
        name: name.map(String::from),
        picture: None,
        aud: "aulalite-dev".into(),
        iss: "https://securetoken.google.com/aulalite-dev".into(),
        exp: Utc::now().timestamp() + 600,
        iat: Utc::now().timestamp(),
        auth_time: None,
    }
}

#[tokio::test]
async fn first_call_creates_user() {
    let pool = fixtures::pool().await;
    let uid = format!("fbuid_{}", uuid::Uuid::new_v4());
    let email = format!("u_{}@example.test", uuid::Uuid::new_v4());

    let provisioned = ensure_user(&pool, &claims(&uid, &email, Some("U")))
        .await
        .unwrap();

    assert_eq!(provisioned.firebase_uid, uid);
    assert_eq!(provisioned.email.to_lowercase(), email.to_lowercase());
}

#[tokio::test]
async fn second_call_is_idempotent_and_updates_last_seen() {
    let pool = fixtures::pool().await;
    let uid = format!("fbuid_{}", uuid::Uuid::new_v4());
    let email = format!("u_{}@example.test", uuid::Uuid::new_v4());
    let user_claims = claims(&uid, &email, Some("U"));

    let first = ensure_user(&pool, &user_claims).await.unwrap();
    let second = ensure_user(&pool, &user_claims).await.unwrap();

    assert_eq!(first.user_id, second.user_id);
}
```

- [ ] **Step 2: Run the new integration test compile check**

Run:

```powershell
cargo test -p backend --test jit_provision --no-run
```

Expected before removing the old library tests: compile succeeds, because the new test code should be valid even while duplicate library tests still exist.

- [ ] **Step 3: Remove the DB-backed unit-test module from `jit_provision.rs`**

Delete this entire block from `crates/backend/src/auth/jit_provision.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::ensure_user;
    use crate::auth::verify::FirebaseClaims;
    use chrono::Utc;
    use sqlx::postgres::PgPoolOptions;

    async fn pool() -> sqlx::PgPool {
        let url = std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://aulalite:changeme@localhost:55432/aulalite".into());
        PgPoolOptions::new()
            .max_connections(2)
            .connect(&url)
            .await
            .unwrap()
    }

    fn claims(uid: &str, email: &str, name: Option<&str>) -> FirebaseClaims {
        FirebaseClaims {
            sub: uid.into(),
            email: Some(email.into()),
            email_verified: Some(true),
            name: name.map(String::from),
            picture: None,
            aud: "aulalite-dev".into(),
            iss: "https://securetoken.google.com/aulalite-dev".into(),
            exp: Utc::now().timestamp() + 600,
            iat: Utc::now().timestamp(),
            auth_time: None,
        }
    }

    #[tokio::test]
    async fn first_call_creates_user() {
        let pool = pool().await;
        let uid = format!("fbuid_{}", uuid::Uuid::new_v4());
        let email = format!("u_{}@example.test", uuid::Uuid::new_v4());

        let provisioned = ensure_user(&pool, &claims(&uid, &email, Some("U")))
            .await
            .unwrap();

        assert_eq!(provisioned.firebase_uid, uid);
        assert_eq!(provisioned.email.to_lowercase(), email.to_lowercase());
    }

    #[tokio::test]
    async fn second_call_is_idempotent_and_updates_last_seen() {
        let pool = pool().await;
        let uid = format!("fbuid_{}", uuid::Uuid::new_v4());
        let email = format!("u_{}@example.test", uuid::Uuid::new_v4());
        let claims = claims(&uid, &email, Some("U"));

        let first = ensure_user(&pool, &claims).await.unwrap();
        let second = ensure_user(&pool, &claims).await.unwrap();

        assert_eq!(first.user_id, second.user_id);
    }
}
```

- [ ] **Step 4: Verify backend library tests no longer run JIT DB tests**

Run:

```powershell
cargo test -p backend --lib auth::jit_provision
```

Expected: exits successfully without trying to connect to Postgres. The output should show no `PoolTimedOut` and no attempt to run `first_call_creates_user` or `second_call_is_idempotent_and_updates_last_seen` as library tests.

- [ ] **Step 5: Verify the integration test target still compiles**

Run:

```powershell
cargo test -p backend --test jit_provision --no-run
```

Expected: test binary builds successfully.

- [ ] **Step 6: Commit the test-scope change**

Run:

```powershell
git add crates/backend/src/auth/jit_provision.rs crates/backend/tests/jit_provision.rs
git commit -m "test(backend): move jit provisioning tests to integration scope"
```

Expected: commit succeeds with one modified source file and one new integration test file.

## Task 3: Exempt The MFA Challenge Route From The MFA Gate

**Files:**
- Modify: `crates/backend/src/auth/middleware.rs`

- [ ] **Step 1: Add failing DB-free middleware unit tests**

In `crates/backend/src/auth/middleware.rs`, replace the existing `#[cfg(test)] mod tests` with:

```rust
#[cfg(test)]
mod tests {
    use super::{is_mfa_challenge_request, parse_role, should_require_mfa_step_up};
    use axum::body::Body;
    use axum::http::{Method, Request};

    fn request(method: Method, uri: &str) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .body(Body::empty())
            .unwrap()
    }

    #[test]
    fn parse_role_maps_database_values() {
        assert_eq!(
            parse_role("org_admin").unwrap(),
            core_types::TenantRole::OrgAdmin
        );
        assert_eq!(
            parse_role("teacher").unwrap(),
            core_types::TenantRole::Teacher
        );
        assert_eq!(parse_role("ta").unwrap(), core_types::TenantRole::Ta);
        assert_eq!(
            parse_role("student").unwrap(),
            core_types::TenantRole::Student
        );
        assert_eq!(
            parse_role("parent").unwrap(),
            core_types::TenantRole::Parent
        );
    }

    #[test]
    fn mfa_challenge_route_is_identified_only_for_post_challenge() {
        assert!(is_mfa_challenge_request(&request(
            Method::POST,
            "/v1/auth/mfa/challenge"
        )));
        assert!(!is_mfa_challenge_request(&request(
            Method::GET,
            "/v1/auth/mfa/challenge"
        )));
        assert!(!is_mfa_challenge_request(&request(
            Method::POST,
            "/v1/me/mfa/disable"
        )));
    }

    #[test]
    fn mfa_gate_skips_challenge_and_sso_session_tokens() {
        assert!(should_require_mfa_step_up(true, true, false, false));
        assert!(!should_require_mfa_step_up(true, true, false, true));
        assert!(!should_require_mfa_step_up(true, true, true, false));
        assert!(!should_require_mfa_step_up(false, true, false, false));
        assert!(!should_require_mfa_step_up(true, false, false, false));
    }
}
```

- [ ] **Step 2: Run the new tests to verify they fail before implementation**

Run:

```powershell
cargo test -p backend --lib auth::middleware::tests::mfa_
```

Expected before implementation: compile fails because `is_mfa_challenge_request` and `should_require_mfa_step_up` do not exist.

- [ ] **Step 3: Import `Method`**

Change the imports at the top of `crates/backend/src/auth/middleware.rs` from:

```rust
use axum::http::HeaderMap;
```

to:

```rust
use axum::http::{HeaderMap, Method};
```

- [ ] **Step 4: Add the pure MFA gate helpers**

Add these helper functions above `resolve_token` in `crates/backend/src/auth/middleware.rs`:

```rust
fn mfa_enforcement_enabled() -> bool {
    std::env::var("AULALITE_MFA_ENFORCE")
        .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
}

fn is_mfa_challenge_request(req: &Request) -> bool {
    req.method() == Method::POST && req.uri().path() == "/v1/auth/mfa/challenge"
}

fn should_require_mfa_step_up(
    enforcement_enabled: bool,
    user_mfa_enabled: bool,
    sso_session_token: bool,
    challenge_request: bool,
) -> bool {
    enforcement_enabled && user_mfa_enabled && !sso_session_token && !challenge_request
}
```

- [ ] **Step 5: Use the helpers in `require_auth`**

Replace the current MFA gate in `crates/backend/src/auth/middleware.rs`:

```rust
    if std::env::var("AULALITE_MFA_ENFORCE")
        .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
        && crate::db::mfa::is_enabled(&state.pool, user.user_id).await
        && claims.iss != crate::services::oidc::SSO_SESSION_ISS
    {
        return Err(ApiError::Unauthorized("mfa_required".into()));
    }
```

with:

```rust
    let mfa_enforced = mfa_enforcement_enabled();
    let challenge_request = is_mfa_challenge_request(&req);
    if mfa_enforced {
        let user_mfa_enabled = crate::db::mfa::is_enabled(&state.pool, user.user_id).await;
        let sso_session_token = claims.iss == crate::services::oidc::SSO_SESSION_ISS;
        if should_require_mfa_step_up(
            mfa_enforced,
            user_mfa_enabled,
            sso_session_token,
            challenge_request,
        ) {
            return Err(ApiError::Unauthorized("mfa_required".into()));
        }
    }
```

- [ ] **Step 6: Run the MFA middleware unit tests**

Run:

```powershell
cargo test -p backend --lib auth::middleware::tests::mfa_
```

Expected: both MFA helper tests pass.

- [ ] **Step 7: Run all middleware unit tests**

Run:

```powershell
cargo test -p backend --lib auth::middleware::tests
```

Expected: all middleware unit tests pass.

- [ ] **Step 8: Commit the MFA middleware fix**

Run:

```powershell
git add crates/backend/src/auth/middleware.rs
git commit -m "fix(auth): allow mfa challenge during enforcement"
```

Expected: commit succeeds with only `middleware.rs` changed.

## Task 4: Sanitize Course Syllabus And Grading Markdown On Patch

**Files:**
- Modify: `crates/backend/src/handlers/courses.rs`
- Modify: `crates/backend/tests/courses_crud.rs`

- [ ] **Step 1: Add the failing course markdown regression test**

Append this test to `crates/backend/tests/courses_crud.rs` after `owner_can_patch_title_and_status`:

```rust
#[tokio::test]
async fn owner_patch_sanitizes_syllabus_and_grading_markdown() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (user, fbuid, email) = create_user(&pool).await;
    attach_membership(&pool, tenant, user, "teacher").await;

    let app = build_test_app(
        backend::handlers::courses::router_for_tests(pool.clone()),
        StubAuth {
            pool: pool.clone(),
            user_id: user,
            firebase_uid: fbuid,
            email,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Teacher),
        },
    );

    let (_, body) = fire(
        &app,
        "POST",
        "/v1/courses",
        Some(serde_json::json!({ "title": "Safety" })),
    )
    .await;
    let id = body["id"].as_str().unwrap().to_string();

    let (status, body) = fire(
        &app,
        "PATCH",
        &format!("/v1/courses/{id}"),
        Some(serde_json::json!({
            "syllabus_md": "# Course plan\n<script>alert(1)</script>\n[bad](javascript:alert(1))",
            "grading_policy_md": "Pass <img src=x onerror=alert(1)> **work** <b>only</b>"
        })),
    )
    .await;
    assert_eq!(status, 200, "{body}");

    let (status, body) = fire(&app, "GET", &format!("/v1/courses/{id}/syllabus"), None).await;
    assert_eq!(status, 200, "{body}");

    let syllabus = body["syllabus_md"].as_str().unwrap();
    let grading = body["grading_policy_md"].as_str().unwrap();

    assert!(syllabus.contains("# Course plan"), "{syllabus}");
    assert!(!syllabus.to_ascii_lowercase().contains("<script"), "{syllabus}");
    assert!(
        !syllabus.to_ascii_lowercase().contains("javascript:"),
        "{syllabus}"
    );
    assert!(grading.contains("**work**"), "{grading}");
    assert!(grading.contains("only"), "{grading}");
    assert!(!grading.to_ascii_lowercase().contains("<img"), "{grading}");
    assert!(!grading.to_ascii_lowercase().contains("onerror"), "{grading}");
    assert!(!grading.to_ascii_lowercase().contains("<b>"), "{grading}");
}
```

- [ ] **Step 2: Run the new test to verify it fails before implementation**

Run with a migrated test database:

```powershell
cargo test -p backend --test courses_crud owner_patch_sanitizes_syllabus_and_grading_markdown -- --nocapture
```

Expected before implementation: test fails because stored syllabus/grading markdown still contains raw HTML or unsafe URL scheme text.

- [ ] **Step 3: Add the optional markdown sanitizer helper**

In `crates/backend/src/handlers/courses.rs`, add this helper below the `PatchCourse` struct:

```rust
fn sanitize_optional_markdown_patch(value: Option<&Option<String>>) -> Option<Option<String>> {
    value.map(|maybe_raw| {
        maybe_raw.as_ref().and_then(|raw| {
            let cleaned = crate::services::sanitize::clean_markdown(
                raw,
                crate::services::validate::MAX_BODY_LEN,
            );
            (!cleaned.is_empty()).then_some(cleaned)
        })
    })
}
```

- [ ] **Step 4: Sanitize patch fields before calling `update_course`**

In `patch_inner`, add these owned sanitized values before opening the update transaction:

```rust
    let syllabus_md = sanitize_optional_markdown_patch(body.syllabus_md.as_ref());
    let grading_policy_md = sanitize_optional_markdown_patch(body.grading_policy_md.as_ref());
```

Place the snippet immediately before:

```rust
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
```

- [ ] **Step 5: Pass the sanitized values into `UpdateCourse`**

In the `db::courses::UpdateCourse` initializer in `patch_inner`, replace:

```rust
            // Map Option<Option<String>> -> Option<Option<&str>> per element.
            syllabus_md: body.syllabus_md.as_ref().map(|o| o.as_deref()),
            grading_policy_md: body.grading_policy_md.as_ref().map(|o| o.as_deref()),
```

with:

```rust
            // Map sanitized Option<Option<String>> -> Option<Option<&str>> per element.
            syllabus_md: syllabus_md.as_ref().map(|o| o.as_deref()),
            grading_policy_md: grading_policy_md.as_ref().map(|o| o.as_deref()),
```

- [ ] **Step 6: Run the course markdown regression test**

Run:

```powershell
cargo test -p backend --test courses_crud owner_patch_sanitizes_syllabus_and_grading_markdown -- --nocapture
```

Expected: test passes.

- [ ] **Step 7: Run backend sanitizer unit tests**

Run:

```powershell
cargo test -p backend --lib services::sanitize::tests
```

Expected: all sanitizer tests pass.

- [ ] **Step 8: Commit the course markdown fix**

Run:

```powershell
git add crates/backend/src/handlers/courses.rs crates/backend/tests/courses_crud.rs
git commit -m "fix(courses): sanitize syllabus markdown"
```

Expected: commit succeeds with one handler change and one integration-test change.

## Task 5: Phase 1 Verification

**Files:**
- Verify: full Phase 1 change set

- [ ] **Step 1: Verify backend library tests are DB-free**

Run:

```powershell
cargo test -p backend --lib
```

Expected: exits successfully without `PoolTimedOut` and without requiring Postgres.

- [ ] **Step 2: Verify workspace library tests are DB-free**

Run:

```powershell
cargo test --workspace --lib
```

Expected: exits successfully without `PoolTimedOut` and without requiring Postgres.

- [ ] **Step 3: Verify backend all-target compilation**

Run:

```powershell
cargo check -p backend --all-targets
```

Expected: exits successfully.

- [ ] **Step 4: Verify shell-web still compiles for wasm**

Run:

```powershell
cargo check -p shell-web --target wasm32-unknown-unknown
```

Expected: exits successfully.

- [ ] **Step 5: Verify focused frontend library tests**

Run:

```powershell
cargo test -p features-courses --lib
cargo test -p design-system --lib
cargo test -p shell-web --lib
```

Expected: all three commands exit successfully.

- [ ] **Step 6: Verify DB-backed Phase 1 integration tests when Postgres is available**

Run with a migrated test database:

```powershell
cargo test -p backend --test jit_provision
cargo test -p backend --test courses_crud owner_patch_sanitizes_syllabus_and_grading_markdown
```

Expected: both commands pass.

- [ ] **Step 7: Verify public asset status**

Run:

```powershell
git status --short -- crates/shell-web/public
```

Expected: no deleted files under `crates/shell-web/public`.

- [ ] **Step 8: Review final diff**

Run:

```powershell
git status --short
git log --oneline -5
```

Expected:

- Recent commits include:
  - `fix(web): restore public runtime assets`
  - `test(backend): move jit provisioning tests to integration scope`
  - `fix(auth): allow mfa challenge during enforcement`
  - `fix(courses): sanitize syllabus markdown`
- Remaining working-tree changes, if any, are unrelated to Phase 1 and are not staged.

## Follow-On Plan Boundaries

Create separate implementation plans after Phase 1 for:

- Phase 2 MFA UX and admin reset
- Phase 3 mobile/native auth parity and secure token storage
- Phase 4 push notifications
- Phase 5 integration admin screens
- Phase 6 SCORM ZIP import
- Phase 7 question banks and randomized assessments
- Phase 8 live-class production hardening
- Phase 9 operational dashboards
- Phase 10 route/layout/CSS/CI finish
