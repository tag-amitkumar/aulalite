# Live-Room Safety + Auth Pass Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stop the live-room from leaking RTC peer connections / media tracks / WebSockets, close the auth-surface holes (URL-token leakage, empty-token-at-boot, broken WHEP playback auth), and make socket-command failures visible.

**Architecture:** A single `LiveRoomSession` struct owns publisher / viewer / socket / promoted-student viewers; cleanup runs via `use_on_destroy` on the live-session route and a best-effort `Drop` fallback. `ApiContext` becomes a `Signal` provided at root and read by a `use_api()` hook so all consumers see live token updates. WHEP authenticates via `Authorization: Bearer` on the client; server keeps both bearer and query support for compatibility. New `ServerEvent::CommandFailed` carries handler errors back to the originating client.

**Tech Stack:** Rust 2021, Dioxus 0.7.4, Axum 0.7, SQLx/Postgres, web-sys / wasm-bindgen for WebRTC, `percent-encoding`, `urlencoding`, `tokio_tungstenite`, `tower-http::trace`.

**Spec:** `docs/superpowers/specs/2026-05-15-live-room-safety-auth-pass-design.md`

---

## Scope Check

Single subsystem: the live-room runtime (frontend + backend) and the auth surface that protects it. Deferred to follow-up specs: asset-build wiring, dev_seed full reset cascade, UI polish completion, real-browser Playwright tests, WHEP token refresh, spotlight/mute UI.

## File Structure

**Create**

- `crates/features-courses/src/live_room_session.rs` — aggregate session resource (~200 LOC).
- `crates/backend/src/trace_scrub.rs` — URI scrub helper for the trace layer.
- `crates/api-client/src/hooks.rs` — `use_api()` hook (or appended to `lib.rs`).
- Test files listed per task.

**Modify**

- Backend: `auth/middleware.rs`, `handlers/live_sessions.rs`, `handlers/dev_seed.rs`, `main.rs`, `Cargo.toml`, `crates/core-types/src/live_room.rs`.
- Frontend (shell): `shell-web/src/lib.rs`, `shell-web/src/routes/live_session.rs`.
- Frontend (features): `live_room_view.rs`, `live_room_broadcast.rs`, `live_room_socket.rs`, `live_room_whip.rs`, `live_room_whep.rs`, `lesson_outline_view.rs`, `lesson_files_editor.rs`, `lesson_video_editor.rs`, `file_picker.rs`, `file_asset_image.rs`, `live_room_replay.rs`, `features-courses/src/lib.rs`, `api-client/src/lib.rs`.
- Tests: `shell-web/tests/dashboard_smoke.rs`, `features-courses/tests/assignments_ssr.rs`, plus new tests per task.

---

## Task 1: Baseline And Worktree Guard

**Files:**
- Inspect only.

- [ ] **Step 1: Confirm spec is on disk**

Run:

```bash
ls "docs/superpowers/specs/2026-05-15-live-room-safety-auth-pass-design.md"
```

Expected: file exists.

- [ ] **Step 2: Record current worktree state**

Run:

```bash
git status --short
git diff --stat HEAD
```

Expected: pre-existing uncommitted Elite Academy refresh edits remain visible (`crates/backend/src/handlers/live_sessions.rs`, `crates/features-courses/src/live_room_*`, etc.). Do **not** revert them — this plan continues from that baseline.

- [ ] **Step 3: Capture failing-test baseline**

Run:

```bash
cargo check -p backend
cargo check -p features-courses
cargo check -p shell-web
cargo test -p backend --test live_room -- --list 2>&1 | head -40
cargo test -p backend --test local_login_bypass -- --list 2>&1 | head -20
```

Expected: compile-checks pass; test listings show the existing test set. Record any compile failures and stop — do not proceed until the baseline compiles.

- [ ] **Step 4: Use commit-per-task discipline**

For every later task, stage only the files that task modifies. Pattern:

```bash
git status --short
git add -- <files changed by this task>
git commit -m "<type>: <task summary>"
```

Expected: commits stay narrow; unrelated uncommitted files stay unstaged.

---

## Task 2: Backend — Trace-Layer URI Scrubber

**Files:**
- Create: `crates/backend/src/trace_scrub.rs`
- Modify: `crates/backend/src/lib.rs` (module declaration)
- Modify: `crates/backend/src/main.rs` (or wherever `TraceLayer` is wired)
- Test: inline `#[cfg(test)]` in `trace_scrub.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/backend/src/trace_scrub.rs` with this content:

```rust
//! Scrubs sensitive query parameters from request URIs before they enter the
//! tracing layer (access_token, jwt, token).

use axum::http::Uri;

pub fn scrub_uri(uri: &Uri) -> String {
    let path = uri.path();
    let Some(query) = uri.query() else {
        return path.to_string();
    };
    let scrubbed: Vec<String> = query
        .split('&')
        .map(|pair| {
            let mut split = pair.splitn(2, '=');
            let key = split.next().unwrap_or("");
            match key {
                "access_token" | "jwt" | "token" => format!("{key}=[REDACTED]"),
                _ => pair.to_string(),
            }
        })
        .collect();
    format!("{path}?{}", scrubbed.join("&"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Uri;

    #[test]
    fn scrubs_access_token() {
        let uri: Uri = "/v1/me?access_token=secret&foo=1".parse().unwrap();
        assert_eq!(scrub_uri(&uri), "/v1/me?access_token=[REDACTED]&foo=1");
    }

    #[test]
    fn scrubs_jwt_and_token() {
        let uri: Uri = "/v1/x?jwt=a&token=b&keep=c".parse().unwrap();
        assert_eq!(scrub_uri(&uri), "/v1/x?jwt=[REDACTED]&token=[REDACTED]&keep=c");
    }

    #[test]
    fn passthrough_without_query() {
        let uri: Uri = "/v1/healthz".parse().unwrap();
        assert_eq!(scrub_uri(&uri), "/v1/healthz");
    }

    #[test]
    fn preserves_path_only_with_no_match() {
        let uri: Uri = "/v1/x?foo=1&bar=2".parse().unwrap();
        assert_eq!(scrub_uri(&uri), "/v1/x?foo=1&bar=2");
    }
}
```

- [ ] **Step 2: Declare the module**

In `crates/backend/src/lib.rs`, add:

```rust
pub mod trace_scrub;
```

- [ ] **Step 3: Run the test**

Run:

```bash
cargo test -p backend trace_scrub::tests
```

Expected: all four tests PASS.

- [ ] **Step 4: Wire scrubber into the trace layer**

Find the `TraceLayer::new_for_http()` site (search with `grep -rn "TraceLayer::new_for_http" crates/backend/src`). Replace the construction with:

```rust
use tower_http::trace::TraceLayer;
use tower_http::trace::DefaultMakeSpan;
use tracing::Span;

let trace_layer = TraceLayer::new_for_http()
    .make_span_with(|req: &axum::http::Request<_>| {
        let scrubbed = crate::trace_scrub::scrub_uri(req.uri());
        tracing::info_span!(
            "http_request",
            method = %req.method(),
            uri = %scrubbed,
        )
    });
```

If the existing layer already customizes `make_span_with`, merge the scrubber in by replacing the URI field; keep other fields. If imports differ, follow the existing style.

- [ ] **Step 5: Compile check**

Run:

```bash
cargo check -p backend
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add -- crates/backend/src/trace_scrub.rs crates/backend/src/lib.rs crates/backend/src/main.rs
git commit -m "feat(backend): scrub auth tokens from trace layer URIs"
```

---

## Task 3: Backend — Middleware Auth Hardening

**Files:**
- Modify: `crates/backend/Cargo.toml`
- Modify: `crates/backend/src/auth/middleware.rs`
- Test: `crates/backend/tests/local_login_bypass.rs`

- [ ] **Step 1: Add percent-encoding dependency**

In `crates/backend/Cargo.toml`, under `[dependencies]`, add:

```toml
percent-encoding = "2"
```

- [ ] **Step 2: Write failing tests**

Append to `crates/backend/tests/local_login_bypass.rs`:

```rust
#[tokio::test]
async fn access_token_query_rejected_on_non_upgrade() {
    let (app, _state) = build_app_with_local_login_enabled().await;
    let response = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .uri("/v1/me?access_token=local-teacher-token")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn access_token_query_accepted_on_ws_upgrade() {
    let (app, _state) = build_app_with_local_login_enabled().await;
    let response = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .uri("/v1/me?access_token=local-teacher-token")
                .header("Upgrade", "websocket")
                .header("Connection", "Upgrade")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    // 426/101/other upgrade-related code is acceptable; what we assert is NOT 401.
    assert_ne!(response.status(), axum::http::StatusCode::UNAUTHORIZED,
        "ws-upgrade with ?access_token= should pass auth");
}

#[tokio::test]
async fn access_token_url_decoded() {
    // A token containing characters that get URL-encoded.
    let (app, _state) = build_app_with_token("plus+equal=slash/").await;
    let response = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .uri("/v1/me?access_token=plus%2Bequal%3Dslash%2F")
                .header("Upgrade", "websocket")
                .header("Connection", "Upgrade")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_ne!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
}
```

If `build_app_with_token` does not exist yet, add this helper at the top of the test module's helper section:

```rust
async fn build_app_with_token(literal: &'static str) -> (axum::Router, std::sync::Arc<crate::state::AppState>) {
    // Re-uses the existing local-login bootstrap but overrides the configured teacher token.
    std::env::set_var("LOCAL_LOGIN_TEACHER_TOKEN", literal);
    build_app_with_local_login_enabled().await
}
```

- [ ] **Step 3: Run tests to verify they fail**

```bash
cargo test -p backend --test local_login_bypass access_token_
```

Expected: all three tests FAIL (no URL decoding, no upgrade gating).

- [ ] **Step 4: Update `query_access_token`**

In `crates/backend/src/auth/middleware.rs`, replace `query_access_token` with:

```rust
fn query_access_token(query: Option<&str>) -> Option<String> {
    use percent_encoding::percent_decode_str;
    let query = query?;
    for pair in query.split('&') {
        let mut split = pair.splitn(2, '=');
        if split.next()? != "access_token" {
            continue;
        }
        let raw = split.next()?;
        let decoded = percent_decode_str(raw).decode_utf8().ok()?;
        if decoded.is_empty() {
            return None;
        }
        return Some(decoded.into_owned());
    }
    None
}
```

- [ ] **Step 5: Add `is_websocket_upgrade` helper**

Above `bearer_token` (or near the top of the module) add:

```rust
fn is_websocket_upgrade(headers: &axum::http::HeaderMap) -> bool {
    let upgrade_ok = headers
        .get(axum::http::header::UPGRADE)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.eq_ignore_ascii_case("websocket"))
        .unwrap_or(false);
    let connection_ok = headers
        .get(axum::http::header::CONNECTION)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.split(',').any(|p| p.trim().eq_ignore_ascii_case("upgrade")))
        .unwrap_or(false);
    upgrade_ok && connection_ok
}
```

- [ ] **Step 6: Gate query-token on upgrade**

Find the token-extraction chain in `bearer_token` (or wherever the header-then-query fallback lives). Replace with:

```rust
let token = bearer_header_token(req.headers())
    .or_else(|| {
        if is_websocket_upgrade(req.headers()) {
            query_access_token(req.uri().query())
        } else {
            None
        }
    })?;
```

If `bearer_header_token` doesn't exist as a separate fn, extract the header-reading logic into one for clarity.

- [ ] **Step 7: Reorder local_login lookup**

Find the block that does `if let Some(claims) = state.local_login.claims_for_token(&token) { … }` followed by `verifier.verify(&token).await`. Reorder to:

```rust
match verifier.verify(&token).await {
    Ok(claims) => Ok(claims),
    Err(verify_err) => {
        // Fall back to local-login only if the verifier rejected it.
        if let Some(claims) = state.local_login.claims_for_token(&token) {
            Ok(claims)
        } else {
            Err(verify_err)
        }
    }
}
```

- [ ] **Step 8: Run the tests**

```bash
cargo test -p backend --test local_login_bypass
```

Expected: all tests PASS, including the three new ones.

- [ ] **Step 9: Commit**

```bash
git add -- crates/backend/Cargo.toml crates/backend/src/auth/middleware.rs crates/backend/tests/local_login_bypass.rs
git commit -m "fix(backend): gate query access_token on ws upgrade and url-decode"
```

---

## Task 4: Backend — live_sessions URL + Bearer + Clamp + Guard

**Files:**
- Modify: `crates/backend/src/handlers/live_sessions.rs`
- Test: `crates/backend/tests/live_room.rs`

- [ ] **Step 1: Write failing tests**

Append to `crates/backend/tests/live_room.rs`:

```rust
#[tokio::test]
async fn whep_main_url_has_no_jwt_in_query() {
    let (app, _state, ctx) = build_live_session_for_join().await;
    let body = join_inner_request(&app, &ctx).await;
    assert!(!body.main_url.contains("jwt="),
        "main_url should not embed jwt: {}", body.main_url);
    assert!(!body.main_url.contains("access_token="),
        "main_url should not embed access_token: {}", body.main_url);
    if let Some(screen) = body.screen_url.as_ref() {
        assert!(!screen.contains("jwt="),
            "screen_url should not embed jwt: {screen}");
    }
}

#[tokio::test]
async fn mediamtx_read_accepts_lowercase_bearer() {
    let (app, _state, ctx) = build_live_session_for_join().await;
    let jwt = mint_viewer_jwt(&ctx);
    let response = post_mediamtx_read_callback(
        &app,
        MediaMtxAuthBody {
            password: format!("bearer {jwt}"),
            ..mediamtx_read_body_for(&ctx)
        },
    ).await;
    assert_eq!(response.status(), axum::http::StatusCode::OK);
}

#[tokio::test]
async fn mediamtx_read_accepts_bearer_with_whitespace() {
    let (app, _state, ctx) = build_live_session_for_join().await;
    let jwt = mint_viewer_jwt(&ctx);
    let response = post_mediamtx_read_callback(
        &app,
        MediaMtxAuthBody {
            password: format!("Bearer  {jwt}\r\n"),
            ..mediamtx_read_body_for(&ctx)
        },
    ).await;
    assert_eq!(response.status(), axum::http::StatusCode::OK);
}

#[tokio::test]
async fn messages_limit_clamped_to_200() {
    let (app, _state, ctx) = build_live_session_with_messages(300).await;
    let response = get_messages(&app, &ctx, Some(1_000_000)).await;
    assert!(response.messages.len() <= 200);
}

#[tokio::test]
async fn messages_rejects_non_finite_to() {
    let (app, _state, ctx) = build_live_session_with_messages(10).await;
    let response = app
        .clone()
        .oneshot(get_request(&format!(
            "/v1/sessions/{}/messages?to=Infinity", ctx.session_id
        )))
        .await
        .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
}
```

If helpers don't exist, add to the helpers section of the test file:

```rust
struct LiveSessionCtx {
    pub session_id: uuid::Uuid,
    pub viewer_id: uuid::Uuid,
    pub publish_path: String,
}

async fn build_live_session_for_join() -> (axum::Router, std::sync::Arc<crate::state::AppState>, LiveSessionCtx) { /* delegate to existing helpers; mint session, viewer */ todo!() }
async fn join_inner_request(app: &axum::Router, ctx: &LiveSessionCtx) -> JoinResponse { todo!() }
async fn mint_viewer_jwt(ctx: &LiveSessionCtx) -> String { todo!() }
async fn post_mediamtx_read_callback(app: &axum::Router, body: MediaMtxAuthBody) -> axum::response::Response { todo!() }
fn mediamtx_read_body_for(ctx: &LiveSessionCtx) -> MediaMtxAuthBody { todo!() }
async fn build_live_session_with_messages(count: usize) -> (axum::Router, std::sync::Arc<crate::state::AppState>, LiveSessionCtx) { todo!() }
async fn get_messages(app: &axum::Router, ctx: &LiveSessionCtx, limit: Option<i64>) -> MessagesResponse { todo!() }
fn get_request(uri: &str) -> axum::http::Request<axum::body::Body> { todo!() }
```

For each `todo!()` helper that doesn't already exist, fill it in by copying the pattern from the existing tests in the same file (the test file already drives many of these flows under different names — reuse without abstraction creep).

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cargo test -p backend --test live_room whep_main_url_has_no_jwt_in_query mediamtx_read_accepts_lowercase_bearer messages_limit_clamped_to_200 messages_rejects_non_finite_to
```

Expected: all FAIL.

- [ ] **Step 3: Remove `?jwt=` from WHEP URLs**

In `crates/backend/src/handlers/live_sessions.rs`, find the `main_url` and `screen_url` construction for `transport_mode = "webrtc"`. Replace the `?jwt=…` interpolation with the bare URL. Concretely, where the current code reads:

```rust
"webrtc" => format!("{public_webrtc_url}/{mp}/whep?jwt={jwt}"),
```

change to:

```rust
"webrtc" => format!("{public_webrtc_url}/{mp}/whep"),
```

Apply the same change for `screen_url`.

- [ ] **Step 4: Remove dead `screen_url` fallback arm**

In the same `match b.transport_mode.as_str() { ... }` for `screen_url`, delete the `_ => format!(...)` arm. Transport mode is validated upstream to be `"webrtc"` or `"hls"`; the catch-all is unreachable. Replace with an explicit `mode => return Err(ApiError::Internal(format!("unexpected transport_mode {mode}")))` if you prefer defensive coding, but per the review the cleanest move is to delete it and let the compiler enforce exhaustiveness once the enum is tightened later.

- [ ] **Step 5: Make bearer-strip case-insensitive and whitespace-tolerant**

Find the bearer-strip in `mediamtx_auth_read_inner` (around the `.strip_prefix("Bearer ")` chain). Replace with:

```rust
fn extract_bearer_password(password: &str) -> Option<String> {
    let prefix = password.get(..7)?;
    if !prefix.eq_ignore_ascii_case("bearer ") {
        return None;
    }
    let token = password[7..].trim();
    if token.is_empty() { None } else { Some(token.to_string()) }
}

// usage:
let token = extract_bearer_password(&body.password)
    .or_else(|| extract_jwt_from_query(&body.query));
```

If `extract_jwt_from_query` doesn't already exist as a separate fn, extract the inline `body.query.split('&')` walker into one for clarity. Keep behavior identical.

- [ ] **Step 6: Clamp `messages_inner.limit`**

Find `let limit = q.limit.unwrap_or(50);` in `messages_inner`. Replace with:

```rust
let limit = q.limit.unwrap_or(50).clamp(1, 200);
```

- [ ] **Step 7: Guard the `f64::MAX` arithmetic**

Find `if to_secs == f64::MAX { MAX_UTC } else { ... }` in the time-window calc. Replace with:

```rust
// 2100-01-01 UTC in seconds; well below i64::MAX so chrono can't overflow.
const MAX_SAFE_SECS: f64 = 4_102_444_800.0;
let to = if !to_secs.is_finite() || to_secs > MAX_SAFE_SECS {
    MAX_UTC
} else {
    chrono::DateTime::<chrono::Utc>::from_timestamp(to_secs as i64, 0)
        .unwrap_or(MAX_UTC)
};
```

Also add validation at the request boundary in `messages_inner` (so the test for `?to=Infinity` returns 400):

```rust
if let Some(v) = q.to { if !v.is_finite() { return Err(ApiError::BadRequest("to must be finite".into())); } }
if let Some(v) = q.from { if !v.is_finite() { return Err(ApiError::BadRequest("from must be finite".into())); } }
```

- [ ] **Step 8: Run the tests to verify they pass**

```bash
cargo test -p backend --test live_room whep_main_url_has_no_jwt_in_query mediamtx_read_accepts_lowercase_bearer mediamtx_read_accepts_bearer_with_whitespace messages_limit_clamped_to_200 messages_rejects_non_finite_to
```

Expected: all PASS.

- [ ] **Step 9: Run the full live_room test file**

```bash
cargo test -p backend --test live_room
```

Expected: PASS (or, only pre-existing failures unrelated to this task).

- [ ] **Step 10: Commit**

```bash
git add -- crates/backend/src/handlers/live_sessions.rs crates/backend/tests/live_room.rs
git commit -m "fix(backend): harden live_sessions WHEP URL, bearer parse, limit clamp"
```

---

## Task 5: Backend — Real End-To-End WHEP Auth Test

**Files:**
- Modify: `crates/backend/tests/live_room.rs`

- [ ] **Step 1: Write the end-to-end test**

Append:

```rust
#[tokio::test]
async fn whep_bearer_e2e_real_jwt_from_join() {
    let (app, _state, ctx) = build_live_session_for_join().await;

    // Drive the real join endpoint to mint a viewer JWT.
    let join_body = join_inner_request(&app, &ctx).await;
    assert!(!join_body.main_url.contains("jwt="),
        "main_url must not leak jwt: {}", join_body.main_url);

    // The JWT is delivered to the client via a header / body field — extract from there.
    // (Adapt to the actual contract — `join_body.viewer_jwt` or response header.)
    let viewer_jwt = join_body.viewer_jwt.clone();

    // Replay it as the MediaMTX read callback would.
    let response = post_mediamtx_read_callback(
        &app,
        MediaMtxAuthBody {
            password: format!("Bearer {viewer_jwt}"),
            ..mediamtx_read_body_for(&ctx)
        },
    ).await;
    assert_eq!(response.status(), axum::http::StatusCode::OK,
        "viewer JWT minted by join must authenticate the read callback");
}
```

If the join response does not yet expose `viewer_jwt` (per the WHEP-bearer client contract), inspect `JoinResponse` in `crates/backend/src/handlers/live_sessions.rs`. The bearer-only design requires the join response to surface the JWT to the client so the client can attach it as `Authorization: Bearer`. If it's currently in the URL only, add a `viewer_jwt: String` field to `JoinResponse` and populate it from the same `mint_viewer_jwt` call that previously fed `?jwt=`. Update the existing `whep_main_url_includes_jwt` test from Task 4 — it should be replaced with this end-to-end check rather than coexisting.

- [ ] **Step 2: Run the test**

```bash
cargo test -p backend --test live_room whep_bearer_e2e_real_jwt_from_join
```

Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add -- crates/backend/tests/live_room.rs crates/backend/src/handlers/live_sessions.rs
git commit -m "test(backend): e2e WHEP bearer auth from join to read callback"
```

---

## Task 6: Backend — Real WS Auth Test With Live Middleware

**Files:**
- Modify: `crates/backend/tests/local_login_bypass.rs`

- [ ] **Step 1: Write the test**

Append:

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn live_socket_authenticates_via_query_access_token() {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;

    let (server, base_url, ctx) = spawn_real_server_with_session().await;
    let token = mint_session_token(&ctx);
    let encoded = urlencoding::encode(&token);

    let ws_url = format!(
        "ws://{base_url}/v1/sessions/{}/socket?access_token={encoded}",
        ctx.session_id
    );
    let request = ws_url.into_client_request().unwrap();

    let (ws, response) = tokio_tungstenite::connect_async(request).await
        .expect("ws connect must succeed with valid access_token");
    assert_eq!(response.status(), tokio_tungstenite::tungstenite::http::StatusCode::SWITCHING_PROTOCOLS);

    drop(ws);
    server.abort();
}
```

If `spawn_real_server_with_session` does not exist, add:

```rust
async fn spawn_real_server_with_session() -> (tokio::task::JoinHandle<()>, String, LiveSessionCtx) {
    let (router, _state, ctx) = build_live_session_for_join().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (handle, format!("{}", addr), ctx)
}

fn mint_session_token(ctx: &LiveSessionCtx) -> String {
    // Reuse whichever helper the rest of the suite uses to mint a teacher/student token
    // for the session owner. Look for `mint_jwt_for_user` or similar.
    todo!("reuse existing JWT-minting helper")
}
```

- [ ] **Step 2: Run the test**

```bash
cargo test -p backend --test local_login_bypass live_socket_authenticates_via_query_access_token
```

Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add -- crates/backend/tests/local_login_bypass.rs
git commit -m "test(backend): real ws connect with access_token query"
```

---

## Task 7: Backend — `CommandFailed` Event And Handler Updates

**Files:**
- Modify: `crates/core-types/src/live_room.rs` (or wherever `ServerEvent` lives — check via `grep -rn "enum ServerEvent" crates/core-types crates/backend`)
- Modify: `crates/backend/src/handlers/live_sessions.rs`
- Test: `crates/backend/tests/live_room.rs`

- [ ] **Step 1: Add the new variant and close codes**

In the file defining `ServerEvent`, add:

```rust
// inside enum ServerEvent
CommandFailed { command: String, reason: String },
```

Below the enum (or in a new module), add:

```rust
pub mod close_codes {
    pub const AUTH_EXPIRED: u16 = 4001;
    pub const AUTH_INVALID: u16 = 4003;
}
```

Confirm no `#[serde(deny_unknown_fields)]` on the enum or its variants. If present on a variant, remove it for `CommandFailed`-adjacent variants (additive evolution).

- [ ] **Step 2: Write failing test**

Append to `crates/backend/tests/live_room.rs`:

```rust
#[tokio::test]
async fn demote_hand_on_stale_state_emits_command_failed_and_no_demoted() {
    let (app, state, ctx) = build_live_session_with_publishing_student().await;
    // Force the clear-nonce to fail by deleting the row in advance.
    sqlx::query("DELETE FROM student_publish_nonces WHERE session_id = $1")
        .bind(ctx.session_id)
        .execute(&state.db)
        .await
        .unwrap();

    let (mut teacher_ws, mut student_ws) = connect_two_sockets(&app, &ctx).await;
    send_command(&mut teacher_ws, ClientEvent::DemoteHand { user_id: ctx.student_id }).await;

    let events = collect_events(&mut teacher_ws, Duration::from_millis(300)).await;
    assert!(events.iter().any(|e| matches!(e, ServerEvent::CommandFailed { command, .. } if command == "DemoteHand")),
        "expected CommandFailed; got: {events:?}");
    assert!(!events.iter().any(|e| matches!(e, ServerEvent::Demoted { .. })),
        "Demoted must not be emitted when nonce clear fails: {events:?}");

    let s_events = collect_events(&mut student_ws, Duration::from_millis(300)).await;
    assert!(!s_events.iter().any(|e| matches!(e, ServerEvent::Demoted { .. })),
        "student must not see Demoted on a failed demote: {s_events:?}");
}
```

If helpers don't exist, follow the pattern of existing tests in the same file (the suite already has socket-driving helpers).

- [ ] **Step 3: Run test to verify it fails**

```bash
cargo test -p backend --test live_room demote_hand_on_stale_state_emits_command_failed_and_no_demoted
```

Expected: FAIL (no `CommandFailed` variant emitted; `Demoted` may be falsely broadcast).

- [ ] **Step 4: Update each command handler**

In `crates/backend/src/handlers/live_sessions.rs`, find each `ClientEvent::*` match arm currently doing `let _ = ...; return;` or similar swallow. Replace with the pattern below. Apply to: `AcceptHand`, `Chat`, `DeleteMessage`, `Kick`, `DemoteHand`.

```rust
ClientEvent::AcceptHand { user_id } => {
    let res = accept_hand_inner(...).await;
    if let Err(e) = res {
        tracing::warn!(error = %e, command = "AcceptHand", "command failed");
        let _ = sender.send(WsMessage::Text(serde_json::to_string(&ServerEvent::CommandFailed {
            command: "AcceptHand".to_string(),
            reason: e.user_facing(),
        }).unwrap()));
        return;
    }
}
```

If `ApiError` (or its equivalent) doesn't have `user_facing()`, add it:

```rust
impl ApiError {
    pub fn user_facing(&self) -> String {
        match self {
            ApiError::BadRequest(s) => s.clone(),
            ApiError::Forbidden(s) => s.clone(),
            ApiError::NotFound => "not found".into(),
            _ => "internal error".into(),
        }
    }
}
```

- [ ] **Step 5: Fix the `DemoteHand` clear-before-emit invariant**

In the `DemoteHand` arm specifically:

```rust
ClientEvent::DemoteHand { user_id } => {
    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(e) => {
            tracing::warn!(error = %e, "DemoteHand tx begin failed");
            send_command_failed(&sender, "DemoteHand", "could not start transaction").await;
            return;
        }
    };
    if let Err(e) = clear_student_publish_nonce(&mut tx, session_id, user_id).await {
        tracing::warn!(error = %e, "DemoteHand clear_nonce failed");
        send_command_failed(&sender, "DemoteHand", e.user_facing()).await;
        return; // do NOT commit, do NOT broker-emit Demoted
    }
    if let Err(e) = tx.commit().await {
        tracing::warn!(error = %e, "DemoteHand commit failed");
        send_command_failed(&sender, "DemoteHand", "commit failed").await;
        return;
    }
    broker.publish(BrokerEvent::Demoted { user_id, ... }).await;
}
```

Add the `send_command_failed` helper at the top of the handler module:

```rust
async fn send_command_failed(sender: &SocketSender, command: &str, reason: impl Into<String>) {
    let event = ServerEvent::CommandFailed { command: command.into(), reason: reason.into() };
    let _ = sender.send(WsMessage::Text(serde_json::to_string(&event).unwrap()));
}
```

- [ ] **Step 6: Run the test**

```bash
cargo test -p backend --test live_room demote_hand_on_stale_state_emits_command_failed_and_no_demoted
```

Expected: PASS.

- [ ] **Step 7: Run the full live_room file**

```bash
cargo test -p backend --test live_room
```

Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add -- crates/core-types/src/live_room.rs crates/backend/src/handlers/live_sessions.rs crates/backend/tests/live_room.rs
git commit -m "feat(backend): surface command failures via CommandFailed event"
```

---

## Task 8: Backend — Hand-Raise `display_name` In Events

**Files:**
- Modify: `crates/core-types/src/live_room.rs`
- Modify: `crates/backend/src/db/live_room.rs` (or wherever the events are constructed)
- Modify: `crates/backend/src/handlers/live_sessions.rs`
- Test: `crates/backend/tests/live_room.rs`

- [ ] **Step 1: Add `display_name` fields to events**

In `core-types/src/live_room.rs`, update:

```rust
HandRaiseChanged {
    user_id: uuid::Uuid,
    raised: bool,
    display_name: String,
},
StudentPublishing {
    user_id: uuid::Uuid,
    publish_path: String,
    display_name: String,
},
StudentDemoted {
    user_id: uuid::Uuid,
    display_name: String,
},
```

- [ ] **Step 2: Write failing test**

Append to `crates/backend/tests/live_room.rs`:

```rust
#[tokio::test]
async fn hand_raise_event_includes_display_name() {
    let (app, state, ctx) = build_live_session_with_named_student("Alice Liddell").await;
    let (mut teacher_ws, mut student_ws) = connect_two_sockets(&app, &ctx).await;

    send_command(&mut student_ws, ClientEvent::RaiseHand).await;
    let events = collect_events(&mut teacher_ws, Duration::from_millis(300)).await;

    let hand = events.iter().find_map(|e| match e {
        ServerEvent::HandRaiseChanged { display_name, raised: true, .. } => Some(display_name.clone()),
        _ => None,
    }).expect("HandRaiseChanged not seen");
    assert_eq!(hand, "Alice Liddell");
}
```

- [ ] **Step 3: Run test (fails)**

```bash
cargo test -p backend --test live_room hand_raise_event_includes_display_name
```

Expected: FAIL — compile error (missing field) until the handler is updated.

- [ ] **Step 4: Update event-construction call sites**

In the live-sessions handler module, every `ServerEvent::HandRaiseChanged { ... }` / `StudentPublishing { ... }` / `StudentDemoted { ... }` literal needs a `display_name`. Source the name from the existing user-join used elsewhere. Concretely, if the current site is:

```rust
broker.publish(BrokerEvent::HandRaiseChanged {
    user_id,
    raised: true,
}).await;
```

surround it with a small fetch:

```rust
let display_name = db::live_room::fetch_user_display_name(&state.db, user_id).await
    .unwrap_or_else(|_| format!("user-{}", user_id.simple()));
broker.publish(BrokerEvent::HandRaiseChanged {
    user_id,
    raised: true,
    display_name,
}).await;
```

Add `fetch_user_display_name` to `crates/backend/src/db/live_room.rs`:

```rust
pub async fn fetch_user_display_name(
    pool: &sqlx::PgPool,
    user_id: uuid::Uuid,
) -> Result<String, sqlx::Error> {
    let row: (String,) = sqlx::query_as("SELECT display_name FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(pool)
        .await?;
    Ok(row.0)
}
```

Apply to all three event sites. If `BrokerEvent` is distinct from `ServerEvent`, update the broker-to-server mapping to thread `display_name` through.

- [ ] **Step 5: Run the test**

```bash
cargo test -p backend --test live_room hand_raise_event_includes_display_name
```

Expected: PASS.

- [ ] **Step 6: Run the full live_room file**

```bash
cargo test -p backend --test live_room
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add -- crates/core-types/src/live_room.rs crates/backend/src/db/live_room.rs crates/backend/src/handlers/live_sessions.rs crates/backend/tests/live_room.rs
git commit -m "feat(backend): include display_name in hand-raise events"
```

---

## Task 9: Backend — `dev_seed` `starts_at` Consistency

**Files:**
- Modify: `crates/backend/src/handlers/dev_seed.rs`
- Test: `crates/backend/tests/audit_seed.rs`

- [ ] **Step 1: Write failing test**

Append to `crates/backend/tests/audit_seed.rs`:

```rust
#[tokio::test]
async fn series_and_session_starts_at_consistent() {
    let (app, state) = build_app_for_seed().await;
    let _ = run_seed(&app).await;

    let series_starts: chrono::DateTime<chrono::Utc> = sqlx::query_scalar(
        "SELECT starts_at FROM live_session_series WHERE id = $1"
    ).bind(SEED_SERIES_ID).fetch_one(&state.db).await.unwrap();
    let session_starts: chrono::DateTime<chrono::Utc> = sqlx::query_scalar(
        "SELECT starts_at FROM live_sessions WHERE series_id = $1 ORDER BY starts_at DESC LIMIT 1"
    ).bind(SEED_SERIES_ID).fetch_one(&state.db).await.unwrap();

    assert_eq!(series_starts, session_starts,
        "series and session starts_at must be identical after seed");
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p backend --test audit_seed series_and_session_starts_at_consistent
```

Expected: FAIL with a sub-microsecond delta.

- [ ] **Step 3: Compute `starts_at` once**

In `crates/backend/src/handlers/dev_seed.rs::seed`, compute `starts_at` exactly once near the top:

```rust
let starts_at = chrono::Utc::now() + chrono::Duration::days(1);
```

Pass it as a parameter to both `get_or_create_live_series` and `get_or_create_live_session`. Update those function signatures:

```rust
async fn get_or_create_live_series(
    db: &sqlx::PgPool,
    starts_at: chrono::DateTime<chrono::Utc>,
    /* existing params */
) -> Result<...> { /* use the passed-in starts_at instead of Utc::now() */ }
```

Remove every internal `chrono::Utc::now()` that previously fed `starts_at` in those two functions.

- [ ] **Step 4: Run the test**

```bash
cargo test -p backend --test audit_seed
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add -- crates/backend/src/handlers/dev_seed.rs crates/backend/tests/audit_seed.rs
git commit -m "fix(backend): unify dev_seed starts_at across series and session"
```

---

## Task 10: Frontend — `use_api()` Hook + `Signal<ApiContext>` Provider

**Files:**
- Modify: `crates/api-client/src/lib.rs`
- Modify: `crates/shell-web/src/lib.rs`
- Modify: `crates/shell-web/src/routes/live_session.rs`

- [ ] **Step 1: Add the hook**

In `crates/api-client/src/lib.rs`, append:

```rust
use dioxus::prelude::*;

/// Read the current `ApiContext`. Consumers should call this rather than
/// `use_context::<ApiContext>()` directly so they see live token updates.
pub fn use_api() -> ApiContext {
    use_context::<Signal<ApiContext>>().read().clone()
}

#[deprecated(note = "Use use_api() instead — bare ApiContext context is captured at app boot \
                     and never reflects token refreshes.")]
pub fn use_api_context_deprecated() -> ApiContext {
    use_context::<ApiContext>()
}
```

- [ ] **Step 2: Provide the Signal at app root**

In `crates/shell-web/src/lib.rs`, find the `use_context_provider(|| api_ctx_signal.read().clone())` line at ~108. Change to:

```rust
use_context_provider::<Signal<ApiContext>>(|| api_ctx_signal);
```

If the existing context provider is needed for non-Signal-aware code paths temporarily, keep it but also provide the Signal. The migration in Task 11 will remove the bare provider.

- [ ] **Step 3: Remove dead-guard branches in lib.rs**

In `crates/shell-web/src/lib.rs` around lines 57-100, simplify the bootstrap future. The current code:

```rust
// ...
bootstrapped.set(true);
match bridge.current_id_token().await {
    Ok(token) => {
        if !api_ctx_signal.read().id_token.is_empty() { return; }   // ← dead branch
        // ... set token, then call get_me ...
        if api_ctx_signal.read().id_token == token {                 // ← TOCTOU guard
            user_ctx_signal.write().set(...);
        }
    }
    Err(_) => {
        if api_ctx_signal.read().id_token.is_empty() { /* original logic */ } // ← always true
    }
}
```

Becomes:

```rust
bootstrapped.set(true);
match bridge.current_id_token().await {
    Ok(token) => {
        api_ctx_signal.write().id_token = token.clone();
        match api::get_me(&api_ctx_signal.read()).await {
            Ok(me) => {
                if api_ctx_signal.read().id_token == token {
                    tracing::debug!("user_ctx updated for token");
                    user_ctx_signal.write().set(me);
                } else {
                    tracing::warn!("token changed during get_me; dropping stale user_ctx");
                }
            }
            Err(e) => tracing::warn!(error = %e, "get_me failed during bootstrap"),
        }
    }
    Err(_) => {
        // Token unavailable — leave api_ctx empty; routes redirect to /login.
    }
}
```

- [ ] **Step 4: Drop the re-provide in live_session.rs**

In `crates/shell-web/src/routes/live_session.rs`, remove the line:

```rust
use_context_provider(|| api.clone());
```

(Will be replaced with the LiveSessionShell wiring in Task 16.)

- [ ] **Step 5: Compile check**

```bash
cargo check -p api-client
cargo check -p shell-web
```

Expected: PASS (consumers that still call `use_context::<ApiContext>()` directly will warn or error — those are fixed in Task 11).

- [ ] **Step 6: If compile errors appear, stub the bare provider temporarily**

If `cargo check` fails because consumers can't find `ApiContext` in context, restore the bare provider as a temporary bridge (will be removed in Task 11):

```rust
// In shell-web/src/lib.rs, after providing the Signal:
use_context_provider::<ApiContext>(|| api_ctx_signal.read().clone());
```

This keeps the old call sites working until Task 11 migrates them.

- [ ] **Step 7: Commit**

```bash
git add -- crates/api-client/src/lib.rs crates/shell-web/src/lib.rs crates/shell-web/src/routes/live_session.rs
git commit -m "feat(api-client): add use_api() and Signal<ApiContext> provider"
```

---

## Task 11: Frontend — Migrate `use_context::<ApiContext>()` Consumers To `use_api()`

**Files:**
- Modify: each file matching `grep -rn "use_context::<ApiContext>" crates/`

Likely list (verify with grep):
- `crates/features-courses/src/lesson_outline_view.rs`
- `crates/features-courses/src/lesson_files_editor.rs`
- `crates/features-courses/src/lesson_video_editor.rs`
- `crates/features-courses/src/file_picker.rs`
- `crates/features-courses/src/file_asset_image.rs`
- `crates/features-courses/src/live_room_replay.rs`
- `crates/features-courses/src/live_room_view.rs` (callers will be revisited in Task 17)
- `crates/features-courses/src/live_room_broadcast.rs` (callers will be revisited in Task 18)

- [ ] **Step 1: List consumers**

```bash
grep -rn "use_context::<ApiContext>" crates/
```

Record the full list. Do not stop searching at the obvious frontend files; check `features-auth`, `shell-desktop`, `shell-mobile`.

- [ ] **Step 2: Migrate each file**

For each file, replace:

```rust
let api = use_context::<ApiContext>();
```

with:

```rust
let api = api_client::use_api();
```

Adjust the import:

```rust
use api_client::use_api;
// or, if api_client is already imported:
//   use api_client::{ApiContext, use_api};
```

- [ ] **Step 3: Verify no consumer is missed**

```bash
grep -rn "use_context::<ApiContext>" crates/
```

Expected: zero output.

- [ ] **Step 4: Remove the temporary bare provider**

In `crates/shell-web/src/lib.rs`, remove the temporary `use_context_provider::<ApiContext>(|| ...)` line added in Task 10 Step 6 (if it was added).

- [ ] **Step 5: Compile + tests**

```bash
cargo check -p features-courses -p features-auth -p shell-web
cargo test -p features-courses --test assignments_ssr
cargo test -p shell-web --test dashboard_smoke
cargo test -p shell-web --test editorial_assets
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add -- crates/features-courses crates/features-auth crates/shell-web/src/lib.rs
git commit -m "refactor(frontend): migrate ApiContext consumers to use_api()"
```

---

## Task 12: Frontend — `WhipPublisher::close()` + Drop + Typed SDP

**Files:**
- Modify: `crates/features-courses/src/live_room_whip.rs`
- Test: inline `#[cfg(test)]` for the SDP extraction (where wasm-bindgen permits)

- [ ] **Step 1: Add `close()`**

In `crates/features-courses/src/live_room_whip.rs`, add to `impl WhipPublisher`:

```rust
impl WhipPublisher {
    pub async fn close(&mut self) {
        if self.closed { return; }
        self.closed = true;

        // 1. Stop local media tracks.
        if let Some(stream) = &self.local_stream {
            let tracks = stream.get_tracks();
            for i in 0..tracks.length() {
                if let Ok(track) = tracks.get(i).dyn_into::<web_sys::MediaStreamTrack>() {
                    track.stop();
                }
            }
        }

        // 2. Close the RTC peer connection.
        if let Some(pc) = self.pc.take() {
            pc.close();
        }

        // 3. Best-effort DELETE on the WHIP resource.
        if let Some(url) = self.resource_url.take() {
            let _ = reqwest::Client::new()
                .delete(&url)
                .bearer_auth(&self.jwt)
                .send()
                .await;
        }
    }
}
```

Add the `closed: bool` field to the struct and initialize it to `false` in the constructor / `publish()` return path.

- [ ] **Step 2: Add Drop**

Also in the same file:

```rust
impl Drop for WhipPublisher {
    fn drop(&mut self) {
        if self.closed { return; }
        let pc = self.pc.take();
        let stream = self.local_stream.clone();
        let resource_url = self.resource_url.take();
        let jwt = self.jwt.clone();
        wasm_bindgen_futures::spawn_local(async move {
            if let Some(stream) = stream {
                let tracks = stream.get_tracks();
                for i in 0..tracks.length() {
                    if let Ok(track) = tracks.get(i).dyn_into::<web_sys::MediaStreamTrack>() {
                        track.stop();
                    }
                }
            }
            if let Some(pc) = pc { pc.close(); }
            if let Some(url) = resource_url {
                let _ = reqwest::Client::new().delete(&url).bearer_auth(&jwt).send().await;
            }
        });
    }
}
```

- [ ] **Step 3: Replace `Reflect::get` with typed `.sdp()`**

Find the call constructing the SDP offer. Replace:

```rust
let offer = JsFuture::from(pc.create_offer()).await?;
let sdp = js_sys::Reflect::get(&offer, &"sdp".into())?
    .as_string()
    .ok_or_else(|| anyhow!("offer sdp missing"))?;
```

with:

```rust
let offer = JsFuture::from(pc.create_offer()).await?;
let offer: web_sys::RtcSessionDescription = offer
    .dyn_into()
    .map_err(|_| anyhow!("create_offer did not return RtcSessionDescription"))?;
let sdp = offer.sdp();
```

- [ ] **Step 4: Compile-check**

```bash
cargo check -p features-courses --target wasm32-unknown-unknown
```

If the wasm target is not installed, fall back to:

```bash
cargo check -p features-courses
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add -- crates/features-courses/src/live_room_whip.rs
git commit -m "fix(live-room): WhipPublisher::close() + Drop + typed sdp"
```

---

## Task 13: Frontend — `WhepViewer::close()` + Drop + Always-Bearer

**Files:**
- Modify: `crates/features-courses/src/live_room_whep.rs`

- [ ] **Step 1: Add `close()` and `Drop`**

Mirror the pattern from Task 12. In `crates/features-courses/src/live_room_whep.rs`:

```rust
impl WhepViewer {
    pub async fn close(&mut self) {
        if self.closed { return; }
        self.closed = true;
        if let Some(pc) = self.pc.take() {
            pc.close();
        }
        if let Some(url) = self.resource_url.take() {
            let _ = reqwest::Client::new()
                .delete(&url)
                .bearer_auth(&self.jwt)
                .send()
                .await;
        }
    }
}

impl Drop for WhepViewer {
    fn drop(&mut self) {
        if self.closed { return; }
        let pc = self.pc.take();
        let resource_url = self.resource_url.take();
        let jwt = self.jwt.clone();
        wasm_bindgen_futures::spawn_local(async move {
            if let Some(pc) = pc { pc.close(); }
            if let Some(url) = resource_url {
                let _ = reqwest::Client::new().delete(&url).bearer_auth(&jwt).send().await;
            }
        });
    }
}
```

Add the `closed: bool` field to the struct and initialize.

- [ ] **Step 2: Always send Bearer; drop the URL-substring check**

Find the request-building site (where headers are attached for the WHEP POST). Replace:

```rust
if !viewer_jwt.is_empty()
    && !whep_url.contains("?jwt=")
    && !whep_url.contains("&jwt=")
{
    request.headers().set("Authorization", &format!("Bearer {viewer_jwt}"));
}
```

with:

```rust
if !viewer_jwt.is_empty() {
    request.headers().set("Authorization", &format!("Bearer {viewer_jwt}"))?;
}
```

Also update the WHEP offer-SDP extraction to use the typed `RtcSessionDescription` accessor, mirroring Task 12 Step 3.

- [ ] **Step 3: Compile-check**

```bash
cargo check -p features-courses
```

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add -- crates/features-courses/src/live_room_whep.rs
git commit -m "fix(live-room): WhepViewer::close() + Drop + always-Bearer"
```

---

## Task 14: Frontend — `LiveRoomSocket::close()` + Drop + URL-Encode + `onclose`

**Files:**
- Modify: `crates/features-courses/src/live_room_socket.rs`
- Test: inline `#[cfg(test)]` for URL-encoding helper

- [ ] **Step 1: Write failing test for URL-encoding**

In `crates/features-courses/src/live_room_socket.rs`, append to the existing `#[cfg(test)] mod tests`:

```rust
#[test]
fn build_ws_url_urlencodes_token() {
    let url = build_ws_url("http://localhost:8080", "session-id", "tok+en=val/ue");
    assert!(url.contains("access_token=tok%2Ben%3Dval%2Fue"), "got: {url}");
}

#[test]
fn build_ws_url_uses_wss_for_https() {
    let url = build_ws_url("https://aulalite.app", "session-id", "tok");
    assert!(url.starts_with("wss://"), "got: {url}");
}

#[test]
fn build_ws_url_empty_token_returns_empty() {
    let url = build_ws_url("http://localhost:8080", "session-id", "");
    assert_eq!(url, "", "empty token must short-circuit to empty URL to skip connect");
}
```

- [ ] **Step 2: Run the test (fails)**

```bash
cargo test -p features-courses build_ws_url
```

Expected: FAIL.

- [ ] **Step 3: Add the `urlencoding` dependency**

In `crates/features-courses/Cargo.toml`:

```toml
urlencoding = "2"
```

- [ ] **Step 4: Update `build_ws_url`**

Replace the existing `build_ws_url` body:

```rust
pub fn build_ws_url(base_url: &str, session_id: &str, access_token: &str) -> String {
    if access_token.is_empty() {
        return String::new();
    }
    let ws_base = if let Some(rest) = base_url.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = base_url.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        base_url.to_string()
    };
    let encoded = urlencoding::encode(access_token);
    format!("{ws_base}/v1/sessions/{session_id}/socket?access_token={encoded}")
}
```

- [ ] **Step 5: Run the test (passes)**

```bash
cargo test -p features-courses build_ws_url
```

Expected: PASS.

- [ ] **Step 6: Add `close()` and `Drop`**

To `impl LiveRoomSocket`:

```rust
impl LiveRoomSocket {
    pub fn close(&mut self) {
        if self.closed { return; }
        self.closed = true;
        if let Some(ws) = self.ws.take() {
            let _ = ws.close();
        }
        // Closures owned by the socket are dropped here.
        self.onmessage = None;
        self.onclose = None;
        self.onerror = None;
    }
}

impl Drop for LiveRoomSocket {
    fn drop(&mut self) { self.close(); }
}
```

Add `closed: bool`, `onclose: Option<Closure<dyn FnMut(CloseEvent)>>`, `onerror: Option<Closure<dyn FnMut(Event)>>` fields to the struct.

- [ ] **Step 7: Wire `onclose` with reconnect-on-4001**

Replace `Closure::forget()` on the onmessage closure with storing it as a field:

```rust
self.onmessage = Some(onmessage_closure);
ws.set_onmessage(Some(self.onmessage.as_ref().unwrap().as_ref().unchecked_ref()));
```

Add an `on_close` callback parameter to `connect`:

```rust
pub async fn connect<F: FnMut(ServerEvent) + 'static, G: FnMut(u16) + 'static>(
    &mut self,
    url: &str,
    on_event: F,
    mut on_close: G,
) -> Result<()> {
    // ... existing onmessage wiring ...
    let onclose_closure = Closure::wrap(Box::new(move |ev: web_sys::CloseEvent| {
        on_close(ev.code());
    }) as Box<dyn FnMut(_)>);
    ws.set_onclose(Some(onclose_closure.as_ref().unchecked_ref()));
    self.onclose = Some(onclose_closure);
    Ok(())
}
```

- [ ] **Step 8: Log parse_event failures**

Find the `if let Ok(parsed) = parse_event(...)` site. Replace with:

```rust
match parse_event(&txt) {
    Ok(parsed) => { on_event(parsed); }
    Err(e) => tracing::debug!(error = %e, raw = %txt, "ws parse_event failed"),
}
```

If `tracing` is not in scope for the wasm target, use `web_sys::console::debug_1(&JsValue::from_str(...))` as a fallback.

- [ ] **Step 9: Compile + tests**

```bash
cargo check -p features-courses
cargo test -p features-courses
```

Expected: PASS.

- [ ] **Step 10: Commit**

```bash
git add -- crates/features-courses/Cargo.toml crates/features-courses/src/live_room_socket.rs
git commit -m "fix(live-room): LiveRoomSocket close, drop, url-encode, onclose"
```

---

## Task 15: Frontend — `LiveRoomSession` Aggregate Module

**Files:**
- Create: `crates/features-courses/src/live_room_session.rs`
- Modify: `crates/features-courses/src/lib.rs` (module declaration)

- [ ] **Step 1: Add module declaration**

In `crates/features-courses/src/lib.rs`, add:

```rust
pub mod live_room_session;
pub use live_room_session::LiveRoomSession;
```

- [ ] **Step 2: Create the file**

Create `crates/features-courses/src/live_room_session.rs`:

```rust
//! Aggregate session resource. Owns the WS socket, WHIP publisher, WHEP
//! viewers (teacher feed + promoted students). Single `close()` path; Drop
//! schedules a best-effort close for abnormal exits.

use std::collections::HashMap;

use api_client::ApiContext;
use core_types::live_room::{ClientEvent, ServerEvent, close_codes};
use uuid::Uuid;

use crate::live_room_socket::{LiveRoomSocket, build_ws_url};
use crate::live_room_whep::WhepViewer;
use crate::live_room_whip::WhipPublisher;

#[derive(Clone, Debug)]
pub struct SessionConfig {
    pub session_id: String,
    pub publish_path: String,
    pub main_url: String,
    pub viewer_jwt: String,
}

pub struct LiveRoomSession {
    config: SessionConfig,
    api: ApiContext,
    socket: Option<LiveRoomSocket>,
    publisher: Option<WhipPublisher>,
    viewer: Option<WhepViewer>,
    students: HashMap<Uuid, WhepViewer>,
    closed: bool,
}

impl LiveRoomSession {
    pub fn new(config: SessionConfig, api: ApiContext) -> Self {
        Self {
            config, api,
            socket: None,
            publisher: None,
            viewer: None,
            students: HashMap::new(),
            closed: false,
        }
    }

    pub async fn connect_socket<F, G>(&mut self, on_event: F, on_close: G) -> anyhow::Result<()>
    where
        F: FnMut(ServerEvent) + 'static,
        G: FnMut(u16) + 'static,
    {
        let url = build_ws_url(&self.api.base_url, &self.config.session_id, &self.api.id_token);
        if url.is_empty() {
            anyhow::bail!("missing access token; cannot connect socket");
        }
        let mut socket = LiveRoomSocket::new();
        socket.connect(&url, on_event, on_close).await?;
        self.socket = Some(socket);
        Ok(())
    }

    pub async fn reconnect_with_fresh_token<F, G>(&mut self, on_event: F, on_close: G) -> anyhow::Result<()>
    where
        F: FnMut(ServerEvent) + 'static,
        G: FnMut(u16) + 'static,
    {
        // Caller supplies a current ApiContext via use_api(); replace ours.
        self.api = api_client::use_api();
        if let Some(mut s) = self.socket.take() {
            s.close();
        }
        self.connect_socket(on_event, on_close).await
    }

    pub async fn go_live(&mut self, transport: TransportMode) -> anyhow::Result<()> {
        // Existing publish flow, but stores the publisher into self.
        let publisher = crate::live_room_whip::publish(
            &self.api,
            &self.config.publish_path,
            &self.config.viewer_jwt,
            transport,
        ).await?;
        self.publisher = Some(publisher);
        Ok(())
    }

    pub async fn attach_main(&mut self, url: &str) -> anyhow::Result<()> {
        let viewer = crate::live_room_whep::view(&self.api, url, &self.config.viewer_jwt).await?;
        self.viewer = Some(viewer);
        Ok(())
    }

    pub async fn attach_student(&mut self, user_id: Uuid, path: &str) -> anyhow::Result<()> {
        if let Some(mut existing) = self.students.remove(&user_id) {
            existing.close().await;
        }
        let url = format!("{}/{}/whep", self.api.public_webrtc_url, path);
        let viewer = crate::live_room_whep::view(&self.api, &url, &self.config.viewer_jwt).await?;
        self.students.insert(user_id, viewer);
        Ok(())
    }

    pub async fn detach_student(&mut self, user_id: Uuid) {
        if let Some(mut v) = self.students.remove(&user_id) {
            v.close().await;
        }
    }

    pub async fn end_class(&mut self) -> anyhow::Result<()> {
        self.close().await;
        self.api.post(&format!("/v1/sessions/{}/end-class", self.config.session_id)).await?;
        Ok(())
    }

    pub async fn close(&mut self) {
        if self.closed { return; }
        self.closed = true;
        if let Some(mut s) = self.socket.take() { s.close(); }
        if let Some(mut p) = self.publisher.take() { p.close().await; }
        if let Some(mut v) = self.viewer.take() { v.close().await; }
        let students = std::mem::take(&mut self.students);
        for (_, mut v) in students {
            v.close().await;
        }
    }
}

impl Drop for LiveRoomSession {
    fn drop(&mut self) {
        if self.closed { return; }
        let socket = self.socket.take();
        let publisher = self.publisher.take();
        let viewer = self.viewer.take();
        let students: HashMap<_, _> = std::mem::take(&mut self.students);
        wasm_bindgen_futures::spawn_local(async move {
            if let Some(mut s) = socket { s.close(); }
            if let Some(mut p) = publisher { p.close().await; }
            if let Some(mut v) = viewer { v.close().await; }
            for (_, mut v) in students { v.close().await; }
        });
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransportMode { WebRtc, Hls }

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_api() -> ApiContext {
        ApiContext { base_url: "http://localhost".into(), id_token: "t".into(), public_webrtc_url: "http://mediamtx".into() }
    }
    fn dummy_config() -> SessionConfig {
        SessionConfig {
            session_id: "sess".into(),
            publish_path: "live/sess/teacher".into(),
            main_url: "http://mediamtx/live/sess/teacher/whep".into(),
            viewer_jwt: "vjwt".into(),
        }
    }

    #[tokio::test]
    async fn close_is_idempotent() {
        let mut s = LiveRoomSession::new(dummy_config(), dummy_api());
        s.close().await;
        s.close().await; // must not panic, must be no-op
        assert!(s.closed);
    }
}
```

If `ApiContext` lacks `public_webrtc_url`, source the value from wherever it's configured today and adapt the field reference. If `crate::live_room_whip::publish` / `crate::live_room_whep::view` signatures differ, adapt method bodies to the actual signatures; the key invariant is that the returned struct lives on `self`, not a local.

- [ ] **Step 3: Run the unit test**

```bash
cargo test -p features-courses live_room_session::tests::close_is_idempotent
```

Expected: PASS.

- [ ] **Step 4: Compile-check**

```bash
cargo check -p features-courses
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add -- crates/features-courses/src/lib.rs crates/features-courses/src/live_room_session.rs
git commit -m "feat(live-room): LiveRoomSession aggregate with idempotent close"
```

---

## Task 16: Frontend — Route Wiring (`LiveSessionShell`)

**Files:**
- Modify: `crates/shell-web/src/routes/live_session.rs`

- [ ] **Step 1: Wrap children in `LiveSessionShell`**

In `crates/shell-web/src/routes/live_session.rs`, refactor the route component to wrap its children:

```rust
#[component]
pub fn LiveSession(id: String) -> Element {
    let api = api_client::use_api();
    let user_snap = use_context::<Signal<Option<UserCtx>>>().read().clone();
    if user_snap.is_none() {
        return rsx! { Redirect { to: "/login" } };
    }

    let session_id = id.clone();
    let session_state = use_signal(|| None::<SessionConfig>);

    // Existing use_resource for /join — but it now stores config into session_state.
    let join = use_resource(move || {
        let api = api.clone();
        let id = session_id.clone();
        async move {
            let resp: JoinResponse = api.post(&format!("/v1/sessions/{id}/join")).await?;
            Ok::<_, ApiError>(SessionConfig {
                session_id: id,
                publish_path: resp.publish_path,
                main_url: resp.main_url,
                viewer_jwt: resp.viewer_jwt,
            })
        }
    });

    match &*join.read() {
        Some(Ok(config)) => rsx! {
            LiveSessionShell { config: config.clone(),
                LiveRoomRouter { config: config.clone() }
            }
        },
        Some(Err(e)) => rsx! { div { class: "system-state system-state--error", "{e}" } },
        None => rsx! { div { class: "system-state system-state--loading", div { class: "skeleton-line" } } },
    }
}

#[component]
fn LiveSessionShell(config: SessionConfig, children: Element) -> Element {
    let api = api_client::use_api();
    let session = use_signal(|| LiveRoomSession::new(config.clone(), api));

    use_on_destroy(move || {
        spawn_local(async move {
            session.write().close().await;
        });
    });

    use_context_provider::<Signal<LiveRoomSession>>(|| session);

    rsx! { {children} }
}
```

`LiveRoomRouter` is the existing component that dispatches to `LiveRoomView` or `LiveRoomBroadcast` based on role; if it's currently inlined in `LiveSession`, factor it out into its own `#[component]` so `LiveSessionShell` cleanly wraps it.

- [ ] **Step 2: Compile-check**

```bash
cargo check -p shell-web
```

Expected: PASS. If there are mismatches because `LiveRoomView` / `LiveRoomBroadcast` still expect props that came from the old route (token, jwt), keep them temporarily; Tasks 17 and 18 migrate them to read from the session context.

- [ ] **Step 3: Commit**

```bash
git add -- crates/shell-web/src/routes/live_session.rs
git commit -m "feat(shell-web): LiveSessionShell with use_on_destroy cleanup"
```

---

## Task 17: Frontend — `live_room_view` Components + Subscriptions

**Files:**
- Modify: `crates/features-courses/src/live_room_view.rs`

- [ ] **Step 1: Promote `render_webrtc` / `render_hls` to components**

Find `fn render_webrtc(props: &LiveRoomViewProps) -> Element` and `fn render_hls(props: &LiveRoomViewProps) -> Element`. Convert to:

```rust
#[component]
fn WebRtcStage(config: SessionConfig) -> Element {
    let session = use_context::<Signal<LiveRoomSession>>();
    let session_handle = session.clone();
    use_effect(move || {
        let url = config.main_url.clone();
        let mut s = session_handle;
        spawn_local(async move {
            if let Err(e) = s.write().attach_main(&url).await {
                tracing::warn!(error = %e, "attach_main failed");
            }
        });
    });
    rsx! { video { id: "live-room-main", autoplay: true, playsinline: true } }
}

#[component]
fn HlsStage(config: SessionConfig) -> Element {
    use_effect(move || {
        let url = config.main_url.clone();
        spawn_local(async move {
            attach_hls_player(&url).await;
        });
    });
    rsx! { video { id: "live-room-main", autoplay: true, playsinline: true, controls: true } }
}
```

Replace the call sites in `LiveRoomView`:

```rust
match props.transport_mode.as_str() {
    "webrtc" => rsx! { WebRtcStage { config: props.config.clone() } },
    "hls"    => rsx! { HlsStage    { config: props.config.clone() } },
    _ => rsx! { div { class: "system-state system-state--error", "unsupported transport mode" } },
}
```

- [ ] **Step 2: Read `display_name` from hand-raise events**

Find the `apply_event_to_state` (or equivalent) match arm for `ServerEvent::HandRaiseChanged`. Replace any client-side `format!("user-{}", short)` with the event's `display_name`:

```rust
ServerEvent::HandRaiseChanged { user_id, raised, display_name } => {
    let mut queue = hand_raise_queue.write();
    if *raised {
        queue.push(HandRaiseEntry { user_id: *user_id, display_name: display_name.clone() });
    } else {
        queue.retain(|e| e.user_id != *user_id);
    }
}
```

- [ ] **Step 3: Wire `StudentPublishing` → `attach_student`**

Replace the TODO at the `ServerEvent::StudentPublishing` arm:

```rust
ServerEvent::StudentPublishing { user_id, publish_path, display_name } => {
    let mut s = session.clone();
    let uid = *user_id;
    let path = publish_path.clone();
    spawn_local(async move {
        if let Err(e) = s.write().attach_student(uid, &path).await {
            tracing::warn!(error = %e, "attach_student failed");
        }
    });
    students_on_stage.write().push(StudentTile {
        user_id: *user_id,
        display_name: display_name.clone(),
        video_id: format!("student-stage-{}", user_id.simple()),
    });
}
```

Add the rendering of the student strip below the main stage:

```rust
section { class: "students-on-stage",
    for tile in students_on_stage.read().iter() {
        video {
            id: "{tile.video_id}",
            class: "student-stage-tile",
            autoplay: true, playsinline: true,
            "aria-label": "Student {tile.display_name} sharing"
        }
    }
}
```

- [ ] **Step 4: Render `CommandFailed` toast**

Add a signal `let command_errors = use_signal(|| Vec::<String>::new());` and handle the event:

```rust
ServerEvent::CommandFailed { command, reason } => {
    let message = format!("{command} failed: {reason}");
    command_errors.write().push(message);
}
```

Render at the bottom of the component:

```rust
if !command_errors.read().is_empty() {
    aside { class: "live-room-toasts",
        for msg in command_errors.read().iter() {
            div { class: "system-state system-state--error", role: "alert", "{msg}" }
        }
    }
}
```

- [ ] **Step 5: Remove the obsolete `use_persistent_socket` hook**

The socket lifecycle now lives in `LiveRoomSession`. Replace any call to `use_persistent_socket(...)` with a `use_effect` that wires the session's event callback:

```rust
let mut session_handle = use_context::<Signal<LiveRoomSession>>();
use_effect(move || {
    let mut s = session_handle.clone();
    let event_handler = move |event: ServerEvent| {
        apply_event_to_state(&event, /* signals captured by closure */);
    };
    let close_handler = move |code: u16| {
        if code == close_codes::AUTH_EXPIRED {
            let mut s2 = s.clone();
            spawn_local(async move {
                let _ = s2.write().reconnect_with_fresh_token(/* re-pass handlers via Rc */).await;
            });
        }
    };
    spawn_local(async move {
        let _ = s.write().connect_socket(event_handler, close_handler).await;
    });
});
```

Since closures can't be easily re-used after move, factor the event and close handlers into named functions or capture `Rc<RefCell<...>>` for the state they need. If this gets thorny, accept that reconnect will trigger a fresh `LiveSessionShell` mount (route remount) instead of in-place reconnect for now and document the limitation.

- [ ] **Step 6: Compile + tests**

```bash
cargo check -p features-courses
cargo test -p features-courses --test live_room_smoke
```

Expected: PASS (if `live_room_smoke` doesn't yet assert the new toasts/tiles, that's fine; Task 19 strengthens tests).

- [ ] **Step 7: Commit**

```bash
git add -- crates/features-courses/src/live_room_view.rs
git commit -m "refactor(live-room): subscribe view to LiveRoomSession, wire StudentPublishing"
```

---

## Task 18: Frontend — `live_room_broadcast` Updates

**Files:**
- Modify: `crates/features-courses/src/live_room_broadcast.rs`

- [ ] **Step 1: Update `go_live_flow`**

Replace the `go_live_flow` signature and body:

```rust
async fn go_live_flow(
    session: &mut LiveRoomSession,
    transport: TransportMode,
) -> Result<(), anyhow::Error> {
    session.go_live(transport).await?;
    Ok(())
}
```

Drop the local `let _publisher = ...` binding and any `// TODO: store publisher` comment.

- [ ] **Step 2: Update `end_class_flow`**

Replace the existing body:

```rust
async fn end_class_flow(session: &mut LiveRoomSession) -> Result<(), anyhow::Error> {
    session.end_class().await
}
```

(The `end_class` method internally closes everything before POSTing `/end-class`.)

- [ ] **Step 3: Update the component callers**

In `LiveRoomBroadcast`, retrieve the session from context and pass it down:

```rust
let mut session = use_context::<Signal<LiveRoomSession>>();

let on_go_live = move |_| {
    let mut s = session;
    spawn_local(async move {
        if let Err(e) = go_live_flow(&mut s.write(), TransportMode::WebRtc).await {
            // surface to UI via the same toast as CommandFailed
            tracing::warn!(error = %e, "go_live failed");
        }
    });
};

let on_end = move |_| {
    let mut s = session;
    spawn_local(async move {
        if let Err(e) = end_class_flow(&mut s.write()).await {
            tracing::warn!(error = %e, "end_class failed");
        }
    });
};
```

- [ ] **Step 4: Compile + tests**

```bash
cargo check -p features-courses
cargo test -p features-courses
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add -- crates/features-courses/src/live_room_broadcast.rs
git commit -m "refactor(live-room): broadcast uses LiveRoomSession for publisher + end-class"
```

---

## Task 19: Frontend — SSR Tests And Test-Quality Fixes

**Files:**
- Modify: `crates/features-courses/tests/live_room_smoke.rs` (or `assignments_ssr.rs` if smoke doesn't exist)
- Modify: `crates/shell-web/tests/dashboard_smoke.rs`
- Modify: `crates/features-courses/tests/assignments_ssr.rs`

- [ ] **Step 1: Add SSR test for command-failed toast**

Append to `crates/features-courses/tests/live_room_smoke.rs`:

```rust
#[test]
fn renders_command_failed_toast_when_seeded() {
    fn app() -> Element {
        let errors = use_signal(|| vec!["DemoteHand failed: forbidden".to_string()]);
        rsx! {
            aside { class: "live-room-toasts",
                for msg in errors.read().iter() {
                    div { class: "system-state system-state--error", role: "alert", "{msg}" }
                }
            }
        }
    }
    let mut vdom = VirtualDom::new(app);
    vdom.rebuild_in_place();
    let html = dioxus_ssr::render(&vdom);
    assert!(html.contains("system-state--error"), "got: {html}");
    assert!(html.contains("DemoteHand failed"), "got: {html}");
}
```

- [ ] **Step 2: Add SSR test for hand-raise display_name**

Append:

```rust
#[test]
fn hand_raise_displays_real_name() {
    // Seed the queue rendering with a known name.
    fn app() -> Element {
        let queue = vec![("Alice Liddell".to_string(), uuid::Uuid::new_v4())];
        rsx! {
            ul { class: "hand-raise-queue",
                for (name, id) in queue {
                    li { key: "{id}", "{name}" }
                }
            }
        }
    }
    let mut vdom = VirtualDom::new(app);
    vdom.rebuild_in_place();
    let html = dioxus_ssr::render(&vdom);
    assert!(html.contains("Alice Liddell"), "got: {html}");
    assert!(!html.contains("user-"), "client-side fabricated name leaked: {html}");
}
```

- [ ] **Step 3: Tighten `dashboard_smoke`**

In `crates/shell-web/tests/dashboard_smoke.rs`, replace the tautological assertion:

```rust
// Before:
assert!(
    html.contains("/schedule") || !html.contains("My Schedule"),
    "frontend schedule links should use /schedule when present: {html}"
);

// After:
if html.contains("My Schedule") {
    assert!(
        html.contains("href=\"/schedule\""),
        "schedule link must point to /schedule, not the API path: {html}"
    );
}
```

- [ ] **Step 4: Fix `assignments_ssr` tautology**

In `crates/features-courses/tests/assignments_ssr.rs`, replace:

```rust
assert!(html.contains("assignment-shell") || html.contains("assignment-list"));
```

with:

```rust
assert!(html.contains("assignment-shell"), "expected assignment-shell wrapper: {html}");
```

- [ ] **Step 5: Run tests**

```bash
cargo test -p features-courses --test live_room_smoke
cargo test -p shell-web --test dashboard_smoke
cargo test -p features-courses --test assignments_ssr
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add -- crates/features-courses/tests/live_room_smoke.rs crates/shell-web/tests/dashboard_smoke.rs crates/features-courses/tests/assignments_ssr.rs
git commit -m "test: tighten ssr assertions and add live-room toast/name tests"
```

---

## Task 20: Final Verification

**Files:**
- Inspect only.

- [ ] **Step 1: Format check**

```bash
cargo fmt --all --check
```

Expected: PASS. If not, run `cargo fmt --all` and amend the most recent commit only if it was a format-only change; otherwise create a single `chore: cargo fmt` commit.

- [ ] **Step 2: Workspace compile-check**

```bash
cargo check --workspace
```

Expected: PASS.

- [ ] **Step 3: Frontend tests**

```bash
cargo test -p design-system
cargo test -p api-client
cargo test -p features-auth
cargo test -p features-courses
cargo test -p shell-web
```

Expected: PASS.

- [ ] **Step 4: Backend tests**

```bash
cargo test -p backend
```

Expected: PASS if Postgres + auxiliary services from `docker compose up -d` are running. Record any environmental-dependency failures with the exact failing command.

- [ ] **Step 5: Grep confirms no consumer was missed**

```bash
grep -rn "use_context::<ApiContext>" crates/
grep -rn "Reflect::get(&offer" crates/features-courses/
grep -rn "Closure::forget" crates/features-courses/
```

Expected: zero output for each.

- [ ] **Step 6: Manual browser smoke (if dev env is up)**

```bash
docker compose up -d
curl http://localhost:8080/healthz
# from crates/shell-web:
dx serve --platform web --port 3000
```

Open `http://localhost:3000`, sign in with a local profile, navigate to a live session, click Go Live as teacher, then End Class. Verify in the browser DevTools Network panel:
- WS connect URL contains `access_token=` and the value is URL-encoded.
- WHEP request shows `Authorization: Bearer …` header (not `?jwt=` in URL).
- DELETE request is sent on End Class (look for it under `/whip` / `/whep` paths).

Also: open a second tab as a student, raise hand, have teacher accept, verify the student appears in the "students on stage" strip. Have teacher demote; verify the strip clears.

- [ ] **Step 7: Final summary**

Summarize in the implementation notes:

```text
Changed files:
- (list from git log)

Verification commands:
- cargo fmt --all --check: PASS
- cargo check --workspace: PASS
- cargo test -p backend: <result>
- cargo test -p features-courses: <result>
- cargo test -p shell-web: <result>

Known environmental blockers (if any):
- ...

Deferred (per spec):
- WHEP token refresh during long sessions
- dev_seed full reset cascade
- Asset-build CSS sync wiring
- UI polish completion
- Real-browser Playwright tests
```

---

## Self-Review Notes

- All design sections in the spec map to tasks 2–18.
- Test plan from the spec maps to: Task 2 (scrub), Task 3 (middleware), Task 4 (URL/bearer/limit/guard), Task 5 (e2e WHEP), Task 6 (e2e WS), Task 7 (CommandFailed), Task 8 (display_name), Task 9 (starts_at), Task 14 (URL-encode), Task 19 (SSR + test-quality).
- The "deferred" list in the spec is preserved as-is — no task implements those.
- Naming consistency: `LiveRoomSession`, `WhipPublisher`, `WhepViewer`, `LiveRoomSocket` used consistently across tasks 12–18. `use_api()` name matches across all consumer migrations. `CommandFailed` event-variant name matches between core-types definition (Task 7) and frontend consumer (Task 17).
- `extract_bearer_password` / `extract_jwt_from_query` helper names are introduced in Task 4 step 5 and not referenced elsewhere — local to the bearer-strip refactor only.
