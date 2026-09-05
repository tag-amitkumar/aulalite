# Instant-Start Session Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a one-tap "Start session now" action on the course page so teachers can launch an ad-hoc live session without going through the series scheduler, with a 15s-polled "Live now" banner for students on the course page.

**Architecture:** Two new backend endpoints (`POST /v1/courses/:cid/sessions/start-now`, `GET /v1/courses/:cid/active-session`) backed by a partial-unique-index race guard. A new client poll hook drives both the teacher button's conflict-recovery state and a student-only banner. The button reuses the existing live-room broadcast route — start-now creates a row already in `status='live'` so the broadcast view lands on a ready session.

**Tech Stack:** Rust (axum + sqlx + tokio on the backend, dioxus on the wasm32 client), Postgres, MediaMTX, dioxus-router, Playwright for e2e.

**Spec:** `docs/superpowers/specs/2026-05-17-instant-start-session-design.md` (commit 205206b).

---

## File map

### Backend
- Create: `migrations/20260517000019_live_sessions_one_live_per_course.sql` — partial unique index migration.
- Modify: `crates/backend/src/db/live_sessions.rs` — add `find_live_for_course` helper.
- Modify: `crates/backend/src/handlers/live_sessions.rs` — add `CreateStartNow` body type, `StartNowResponse`/`ActiveSessionResponse` DTOs, two new handlers (`start_now`, `active_session`), wire into `routes()` + `router_for_tests`.
- Modify: `crates/backend/tests/live_sessions_test.rs` (or a sibling file `live_sessions_start_now_test.rs`) — integration tests.

### Frontend (features-courses crate)
- Modify: `crates/features-courses/src/api.rs` — add `StartNowBody`, `StartNowResponse`, `ActiveSessionResponse`, `start_session_now`, `get_active_session`.
- Create: `crates/features-courses/src/active_session.rs` — `use_active_session_poll` hook + `ActiveSessionInfo` struct.
- Create: `crates/features-courses/src/live_now_banner.rs` — `LiveNowBanner` component.
- Create: `crates/features-courses/src/start_now_button.rs` — `StartNowButton` + `StartNowState` enum.
- Create: `crates/features-courses/src/start_now_modal.rs` — `StartNowModal`.
- Modify: `crates/features-courses/src/course_detail.rs` — add `start_now_slot` and `banner_slot` optional props to `CourseDetailProps`.
- Modify: `crates/features-courses/src/lib.rs` — `pub mod` the four new modules.

### Frontend (shell-web crate)
- Modify: `crates/shell-web/src/routes/course_detail.rs` — wire `StartNowButton` into the actions slot and `LiveNowBanner` into the banner slot of `CourseDetailView`.

### E2E
- Modify: `tools/ui-real-stack.spec.js` — add two new tests inside the existing describe block (teacher start-now happy path, student live-now banner).

---

## Architecture decisions baked into the plan

These resolve open questions from the spec by reading the actual code:

1. **Storage table.** The occurrence table is `live_sessions` (not `session_occurrences`). Spec was updated to match.
2. **Conflict-row response shape.** `ApiError::Conflict` currently serializes as `{"error":"conflict: …"}`. To return structured `{active_session_id, title, starts_at}` with HTTP 409, the `start_now` handler will build a custom `axum::response::Response` instead of returning `ApiError`. The handler's return type becomes `Result<Response, ApiError>` — happy path constructs a `200 Json(StartNowResponse)`, conflict constructs a `409 Json(ConflictBody)`.
3. **Born-live mechanism.** Reuse `db::live_sessions::go_live` to transition the row in the same transaction as the insert. That function is permissive (`WHERE status IN ('scheduled', 'live')`), refreshes `actual_started_at` only if NULL, and re-mints the publish nonce. The start-now handler does **not** return the nonce or publish URL — the broadcast view's existing `POST /go-live` flow runs on landing, which re-mints a fresh nonce. The first nonce minted in start-now is wasted but harmless (it never reaches the client). The mediamtx `publish_started` notification is best-effort and idempotent in our integration (errors are logged-only).
4. **Frontend route to navigate to after start-now.** `Route::LiveSession { slug, session_id }` (path `/courses/:slug/sessions/:session_id`). The existing `live_session.rs` route handler decides between broadcast and student view based on `can_admin`.
5. **Conflict-state banner data.** The 409 response from `start-now` returns the same shape used by `/active-session`'s `active` object (plus the active session id). The teacher button can flip to "Join active session" using either source.
6. **Poll cadence and ownership.** The hook lives in features-courses (`use_active_session_poll`), is mounted by the shell route once per course-detail render, runs every 15s on success, backs off to 30s on transient error, stops on 401.

---

## Task list

### Task 1: Partial unique index migration

**Files:**
- Create: `migrations/20260517000019_live_sessions_one_live_per_course.sql`

- [ ] **Step 1: Write the migration SQL**

Create `migrations/20260517000019_live_sessions_one_live_per_course.sql` with this exact content:

```sql
-- migrations/20260517000019_live_sessions_one_live_per_course.sql
-- Enforce "at most one live session per course" at the database level.
-- Prevents race conditions in start-now flows where two simultaneous
-- requests both pass an application-level "is there a live row?" check.
CREATE UNIQUE INDEX live_sessions_one_live_per_course
    ON live_sessions (course_id)
    WHERE status = 'live';
```

- [ ] **Step 2: Verify the migration applies cleanly**

Run from the repo root:

```bash
cargo sqlx migrate run --source migrations
```

Expected output: `Applied 20260517000019/migrate live_sessions_one_live_per_course (X.Xms)`. No errors.

- [ ] **Step 3: Verify the constraint works**

Manual check (one-liner):

```bash
psql "$DATABASE_URL" -c "SELECT indexdef FROM pg_indexes WHERE indexname = 'live_sessions_one_live_per_course';"
```

Expected output should contain `CREATE UNIQUE INDEX … ON public.live_sessions USING btree (course_id) WHERE (status = 'live'::text)`.

- [ ] **Step 4: Commit**

```bash
git add migrations/20260517000019_live_sessions_one_live_per_course.sql
git commit -m "feat(db): one-live-session-per-course partial unique index"
```

---

### Task 2: DB helper for active-session lookup

**Files:**
- Modify: `crates/backend/src/db/live_sessions.rs` (append after existing `load_for_join` around line 335)

- [ ] **Step 1: Write a failing host test for the helper**

Add this to the bottom of `crates/backend/src/db/live_sessions.rs` (inside the existing `#[cfg(test)] mod tests` block if one exists; otherwise add the module). Then prepare to add an integration test next instead — `find_live_for_course` will be exercised through the handler tests in Task 7, so we don't need a separate unit test for it. **Skip this step**; go straight to implementing the helper in Step 2.

- [ ] **Step 2: Add the helper function**

Append to `crates/backend/src/db/live_sessions.rs` immediately after the `load_for_join` function:

```rust
/// Returns the currently-live session for a course, if any. Used by the
/// start-now conflict check and the active-session read endpoint.
///
/// Returned fields are the minimum needed for the 409 / banner UI:
/// `id`, `title`, `starts_at` (the scheduled start), `actual_started_at`
/// (when it went live — may equal starts_at for ad-hoc sessions), and
/// `transport_mode`.
#[derive(Debug, sqlx::FromRow, Clone)]
pub struct ActiveSessionRow {
    pub id: Uuid,
    pub title: String,
    pub starts_at: chrono::DateTime<chrono::Utc>,
    pub actual_started_at: Option<chrono::DateTime<chrono::Utc>>,
    pub transport_mode: String,
}

pub async fn find_live_for_course(
    pool: &PgPool,
    course_id: Uuid,
) -> sqlx::Result<Option<ActiveSessionRow>> {
    sqlx::query_as::<_, ActiveSessionRow>(
        "SELECT id, title, starts_at, actual_started_at, transport_mode
           FROM live_sessions
          WHERE course_id = $1
            AND status = 'live'
          LIMIT 1",
    )
    .bind(course_id)
    .fetch_optional(pool)
    .await
}
```

- [ ] **Step 3: Verify it compiles**

```bash
cargo check -p backend
```

Expected: no errors. (If sqlx::Uuid import is missing in scope, it already is — `Uuid` is used throughout this file.)

- [ ] **Step 4: Commit**

```bash
git add crates/backend/src/db/live_sessions.rs
git commit -m "feat(backend): find_live_for_course helper"
```

---

### Task 3: Request / response DTOs for the new endpoints

**Files:**
- Modify: `crates/backend/src/handlers/live_sessions.rs` (add types near the existing DTOs around line 15-58)

- [ ] **Step 1: Add the four new DTOs**

In `crates/backend/src/handlers/live_sessions.rs`, just below the existing `OccurrenceDto` declaration (before `SeriesCreatedDto`), add:

```rust
#[derive(Deserialize, Default, Debug)]
pub struct CreateStartNow {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub duration_minutes: Option<i32>,
    #[serde(default)]
    pub recording_enabled: Option<bool>,
}

#[derive(Serialize, Debug)]
pub struct StartNowResponse {
    pub session_id: Uuid,
    pub series_id: Uuid,
    pub title: String,
    pub starts_at: chrono::DateTime<chrono::Utc>,
    pub duration_minutes: i32,
    pub recording_enabled: bool,
    pub transport_mode: String,
    pub status: String,
}

#[derive(Serialize, Debug)]
pub struct ActiveSessionInfo {
    pub session_id: Uuid,
    pub title: String,
    pub starts_at: chrono::DateTime<chrono::Utc>,
    pub transport_mode: String,
}

#[derive(Serialize, Debug)]
pub struct ActiveSessionResponse {
    pub active: Option<ActiveSessionInfo>,
}

#[derive(Serialize, Debug)]
pub struct StartNowConflictBody {
    pub error: String,
    pub active_session_id: Uuid,
    pub title: String,
    pub starts_at: chrono::DateTime<chrono::Utc>,
}
```

- [ ] **Step 2: Verify it compiles**

```bash
cargo check -p backend
```

Expected: no errors.

- [ ] **Step 3: Commit**

```bash
git add crates/backend/src/handlers/live_sessions.rs
git commit -m "feat(backend): start-now + active-session DTOs"
```

---

### Task 4: `start_now` handler — happy-path test first

**Files:**
- Modify: `crates/backend/tests/live_sessions_test.rs` (or create `crates/backend/tests/start_now_test.rs` if that file doesn't exist — check first with `ls crates/backend/tests/`)

- [ ] **Step 1: Locate the existing live-sessions integration test file**

```bash
ls crates/backend/tests/
```

Expected: a file named `live_sessions_test.rs` or similar. If multiple test files exist for live-sessions, prefer the one that already has `router_for_tests` usage. If none exists, create `crates/backend/tests/start_now_test.rs`.

- [ ] **Step 2: Write the failing happy-path test**

Append this to the chosen test file (replace the file path in the import line if you created a new file — `use backend::…` should already exist in existing test files):

```rust
#[sqlx::test(migrations = "migrations")]
async fn start_now_creates_one_off_occurrence(pool: PgPool) {
    let (router, course_id, teacher_token) = seed_course_with_teacher(&pool).await;

    let body = serde_json::json!({});
    let resp = post_json(
        &router,
        &format!("/v1/courses/{course_id}/sessions/start-now"),
        &teacher_token,
        body,
    )
    .await;

    assert_eq!(resp.status(), 200, "body: {}", resp.body_text());
    let dto: serde_json::Value = resp.json().await;
    assert_eq!(dto["status"], "live");
    assert_eq!(dto["transport_mode"], "webrtc");
    assert_eq!(dto["duration_minutes"], 60);

    let session_id = dto["session_id"].as_str().expect("session_id");
    let row: (String, String) = sqlx::query_as(
        "SELECT status, transport_mode FROM live_sessions WHERE id = $1",
    )
    .bind(uuid::Uuid::parse_str(session_id).unwrap())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(row.0, "live");
    assert_eq!(row.1, "webrtc");
}
```

If `seed_course_with_teacher`, `post_json`, and `resp.body_text()` / `resp.json()` helpers don't exist in the test file, **first** copy the test-helper pattern from the most recently-written live-sessions test in the same file (look for `sqlx::test` + an existing seeder). The helpers should:
- `seed_course_with_teacher(&pool)` → return `(Router, Uuid, String)` (router, course_id, bearer token for an owning teacher).
- `post_json(router, path, token, body)` → run the request through the router and return a small response wrapper.

If no helper exists, follow the pattern in `crates/backend/tests/courses_test.rs` (whichever file uses `router_for_tests(pool)` and Axum's `tower::ServiceExt::oneshot`).

- [ ] **Step 3: Run the test and confirm it fails because the route doesn't exist yet**

```bash
cargo test -p backend --test live_sessions_test start_now_creates_one_off_occurrence
```

(Replace `live_sessions_test` with the actual test file's stem if different.) Expected: FAIL with `404 not found` or a routing error. If it fails for a different reason (e.g., helper missing), fix the helper first.

- [ ] **Step 4: Commit the failing test**

```bash
git add crates/backend/tests/
git commit -m "test(backend): failing start-now happy-path test"
```

---

### Task 5: `start_now` handler — implementation

**Files:**
- Modify: `crates/backend/src/handlers/live_sessions.rs`

- [ ] **Step 1: Add module-local imports the new handler needs**

At the top of `crates/backend/src/handlers/live_sessions.rs`, ensure these imports are present (add any that are missing):

```rust
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
```

- [ ] **Step 2: Add a private helper that does the actual work**

Add a new function in the same file, just below `create_series_inner`:

```rust
async fn start_now_inner(
    state: &AppStateOrTest,
    ctx: &RequestContext,
    course_id: Uuid,
    body: CreateStartNow,
) -> Result<Response, ApiError> {
    let pool = state.pool();
    let mediamtx = state.mediamtx();
    let public_webrtc_url = state.public_webrtc_url();

    // 1. Auth: must be able to admin this course.
    let allowed = db::courses::caller_can_admin_course(
        pool,
        course_id,
        ctx.user_id,
        is_org_admin(ctx),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !allowed {
        return Err(ApiError::Forbidden);
    }

    // 2. Fast-path conflict check (clean error before any inserts).
    if let Some(active) = db::live_sessions::find_live_for_course(pool, course_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
    {
        return Ok(conflict_response(&active));
    }

    // 3. Resolve defaults.
    let now = chrono::Utc::now();
    let title = body.title.unwrap_or_else(|| {
        format!("Quick session — {}", now.format("%b %-d, %Y %-I:%M %p UTC"))
    });
    let duration_minutes = body.duration_minutes.unwrap_or(60);
    if !(5..=480).contains(&duration_minutes) {
        return Err(ApiError::BadRequest(format!(
            "duration_minutes must be between 5 and 480, got {duration_minutes}"
        )));
    }

    // 4. Reuse the existing series-create handler with an ad-hoc preset.
    //    This commits its own transaction; on return we have a `scheduled`
    //    occurrence and need to transition it to `live`.
    let series = CreateSeries {
        title: title.clone(),
        starts_at: now,
        duration_minutes,
        frequency: "none".to_string(),
        byweekday: None,
        end_kind: "count".to_string(),
        occurrence_count: Some(1),
        end_until: None,
        primary_teacher_id: None,
        recording_enabled: body.recording_enabled,
        transport_mode: "webrtc".to_string(),
    };
    let created = create_series_inner(pool, ctx, course_id, series).await?;
    let occurrence = created.0.occurrences.into_iter().next().ok_or_else(|| {
        ApiError::Internal("create_series_inner returned no occurrences".into())
    })?;
    let series_dto = created.0.series;

    // 5. Transition the new row to `live`. Use the same db::go_live the
    //    /go-live endpoint uses so the row gets paths + a nonce. The
    //    broadcast view will re-POST /go-live on mount, which is safe
    //    (db::go_live's WHERE clause accepts `status IN ('scheduled','live')`)
    //    and re-mints the nonce.
    let tenant_id = ctx
        .tenant_id
        .ok_or_else(|| ApiError::BadRequest("no tenant".into()))?;
    let main_path = crate::services::mediamtx::path_for_session(
        tenant_id,
        course_id,
        occurrence.id,
    );
    let screen_path = crate::services::mediamtx::screen_path_for_session(
        tenant_id,
        course_id,
        occurrence.id,
    );
    let nonce_plain = mint_publish_nonce();
    let nonce_hash = db::live_sessions::hash_nonce(&nonce_plain);
    let nonce_expires_at =
        now + chrono::Duration::from_std(PUBLISH_NONCE_TTL).unwrap();

    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let live_row = match db::live_sessions::go_live(
        &mut tx,
        occurrence.id,
        &main_path,
        Some(&screen_path),
        &nonce_hash,
        nonce_expires_at,
    )
    .await
    {
        Ok(Some(row)) => row,
        Ok(None) => {
            return Err(ApiError::SessionStateInvalid(
                "session not in scheduled/live state immediately after creation"
                    .into(),
            ));
        }
        Err(e) => {
            // sqlx error — check for unique-violation (race lost to a
            // concurrent start-now).
            let msg = e.to_string();
            if msg.contains("live_sessions_one_live_per_course") {
                // Re-read the active session and return 409.
                drop(tx);
                if let Some(active) =
                    db::live_sessions::find_live_for_course(pool, course_id)
                        .await
                        .map_err(|e| ApiError::Internal(e.to_string()))?
                {
                    return Ok(conflict_response(&active));
                }
                return Err(ApiError::Internal(msg));
            }
            return Err(ApiError::Internal(msg));
        }
    };
    db::audit::emit_audit_event(
        &mut tx,
        tenant_id,
        ctx.user_id,
        "live_session.start_now",
        "live_session",
        live_row.id,
        None,
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    // 6. Best-effort mediamtx notify.
    if let Err(e) = mediamtx.publish_started(&main_path).await {
        tracing::warn!(?e, %main_path, "publish_started best-effort failed");
    }

    let _ = public_webrtc_url; // unused here; the broadcast route will hand back the publish URL on /go-live.

    let response = StartNowResponse {
        session_id: live_row.id,
        series_id: series_dto.id,
        title,
        starts_at: now,
        duration_minutes,
        recording_enabled: live_row.recording_enabled,
        transport_mode: live_row.transport_mode,
        status: live_row.status,
    };
    Ok((StatusCode::OK, axum::Json(response)).into_response())
}

fn conflict_response(active: &db::live_sessions::ActiveSessionRow) -> Response {
    let body = StartNowConflictBody {
        error: "a session is already live in this course".into(),
        active_session_id: active.id,
        title: active.title.clone(),
        starts_at: active.actual_started_at.unwrap_or(active.starts_at),
    };
    (StatusCode::CONFLICT, axum::Json(body)).into_response()
}
```

This handler references `AppStateOrTest`, `mint_publish_nonce`, `PUBLISH_NONCE_TTL`, and `crate::services::mediamtx::path_for_session` etc. The first one (`AppStateOrTest`) is new — see Step 3. The others already exist in this file (search for `mint_publish_nonce` to confirm).

- [ ] **Step 3: Add a thin abstraction so the same handler works for both production `AppState` and `LiveRoomTestState`**

Add this trait near the top of `crates/backend/src/handlers/live_sessions.rs`:

```rust
/// Internal trait so `start_now_inner` and `active_session_inner` can be
/// called from both the production and test routers without duplicating
/// the handler body. The two states have the same fields we need; this
/// trait erases the `State<>` extractor difference.
trait AppStateOrTest {
    fn pool(&self) -> &PgPool;
    fn mediamtx(&self) -> &dyn MediaMtxClient;
    fn public_webrtc_url(&self) -> &str;
}

impl AppStateOrTest for AppState {
    fn pool(&self) -> &PgPool {
        &self.pool
    }
    fn mediamtx(&self) -> &dyn MediaMtxClient {
        self.mediamtx.as_ref()
    }
    fn public_webrtc_url(&self) -> &str {
        &self.mediamtx_public_webrtc_url
    }
}

impl AppStateOrTest for LiveRoomTestState {
    fn pool(&self) -> &PgPool {
        &self.pool
    }
    fn mediamtx(&self) -> &dyn MediaMtxClient {
        self.mediamtx.as_ref()
    }
    fn public_webrtc_url(&self) -> &str {
        &self.public_webrtc_url
    }
}
```

(Add `use crate::services::mediamtx::MediaMtxClient;` at the top of the file if not already imported.)

- [ ] **Step 4: Add the two route handlers — one for `AppState`, one for `LiveRoomTestState`**

After the existing `go_live_t` handler, add:

```rust
async fn start_now(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(cid): Path<Uuid>,
    Json(body): Json<CreateStartNow>,
) -> Result<Response, ApiError> {
    start_now_inner(&s, &ctx, cid, body).await
}

async fn start_now_t(
    State(s): State<LiveRoomTestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(cid): Path<Uuid>,
    Json(body): Json<CreateStartNow>,
) -> Result<Response, ApiError> {
    start_now_inner(&s, &ctx, cid, body).await
}
```

- [ ] **Step 5: Wire the route**

Find `live_room_routes()` in `crates/backend/src/handlers/live_sessions.rs` (around line 1000). It already contains routes like `/v1/sessions/:id/go-live`. We need start-now in a course-scoped Router. There are two production routers in this file: `routes()` (the series scheduler routes, scoped to `AppState`) and `live_room_routes()` (live-room routes, scoped to `AppState` too).

Add the start-now POST and active-session GET routes inside `live_room_routes()` since they use the LiveRoom-related state (mediamtx, public_webrtc_url). Locate the section near line 1002 that adds `/v1/sessions/:id/go-live`, and add adjacent:

```rust
        .route(
            "/v1/courses/:cid/sessions/start-now",
            routing::post(start_now),
        )
        .route(
            "/v1/courses/:cid/active-session",
            routing::get(active_session),
        )
```

Likewise in the test-router section around line 1036 (where `go_live_t` is registered), add:

```rust
        .route(
            "/v1/courses/:cid/sessions/start-now",
            routing::post(start_now_t),
        )
        .route(
            "/v1/courses/:cid/active-session",
            routing::get(active_session_t),
        )
```

(The `active_session_t` handler is added in Task 6 — you can leave the test route commented out until then, but the production route + production handler must compile now.)

- [ ] **Step 6: Run cargo check and fix compile errors**

```bash
cargo check -p backend
```

Expected: compiles cleanly. If errors mention `AppState` fields like `mediamtx_public_webrtc_url` not existing, search `crates/backend/src/lib.rs` for the actual field name and adjust the trait impl.

- [ ] **Step 7: Run the Task 4 test — it should pass now**

```bash
cargo test -p backend --test live_sessions_test start_now_creates_one_off_occurrence
```

Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add crates/backend/src/handlers/live_sessions.rs
git commit -m "feat(backend): POST /v1/courses/:cid/sessions/start-now"
```

---

### Task 6: `active_session` GET handler

**Files:**
- Modify: `crates/backend/src/handlers/live_sessions.rs`

- [ ] **Step 1: Add the inner handler**

Append after `start_now_inner`:

```rust
async fn active_session_inner(
    state: &impl AppStateOrTest,
    ctx: &RequestContext,
    course_id: Uuid,
) -> Result<Json<ActiveSessionResponse>, ApiError> {
    let pool = state.pool();
    // Any course member (teacher, co-teacher, or enrolled student) may read.
    let allowed = db::courses::caller_can_read_course(
        pool,
        course_id,
        ctx.user_id,
        is_org_admin(ctx),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !allowed {
        return Err(ApiError::NotFound);
    }

    let active = db::live_sessions::find_live_for_course(pool, course_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .map(|row| ActiveSessionInfo {
            session_id: row.id,
            title: row.title,
            starts_at: row.actual_started_at.unwrap_or(row.starts_at),
            transport_mode: row.transport_mode,
        });

    Ok(Json(ActiveSessionResponse { active }))
}
```

- [ ] **Step 2: Add the production and test route handlers**

After the inner handler:

```rust
async fn active_session(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(cid): Path<Uuid>,
) -> Result<Json<ActiveSessionResponse>, ApiError> {
    active_session_inner(&s, &ctx, cid).await
}

async fn active_session_t(
    State(s): State<LiveRoomTestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(cid): Path<Uuid>,
) -> Result<Json<ActiveSessionResponse>, ApiError> {
    active_session_inner(&s, &ctx, cid).await
}
```

- [ ] **Step 3: Uncomment / add the test-router route from Task 5 Step 5**

Make sure the test-router section has:

```rust
        .route(
            "/v1/courses/:cid/active-session",
            routing::get(active_session_t),
        )
```

- [ ] **Step 4: Verify it compiles and the prior test still passes**

```bash
cargo check -p backend
cargo test -p backend --test live_sessions_test start_now_creates_one_off_occurrence
```

Expected: both succeed.

- [ ] **Step 5: Commit**

```bash
git add crates/backend/src/handlers/live_sessions.rs
git commit -m "feat(backend): GET /v1/courses/:cid/active-session"
```

---

### Task 7: Backend integration tests for the remaining cases

**Files:**
- Modify: `crates/backend/tests/live_sessions_test.rs` (or the start-now test file from Task 4)

- [ ] **Step 1: Write the 409-on-existing-live test**

Add to the test file:

```rust
#[sqlx::test(migrations = "migrations")]
async fn start_now_returns_409_when_live_session_exists(pool: PgPool) {
    let (router, course_id, teacher_token) = seed_course_with_teacher(&pool).await;

    // First call creates a live session.
    let first = post_json(
        &router,
        &format!("/v1/courses/{course_id}/sessions/start-now"),
        &teacher_token,
        serde_json::json!({}),
    )
    .await;
    assert_eq!(first.status(), 200);
    let first_id = first.json::<serde_json::Value>().await["session_id"]
        .as_str()
        .unwrap()
        .to_string();

    // Second call: 409 with the first session's id.
    let second = post_json(
        &router,
        &format!("/v1/courses/{course_id}/sessions/start-now"),
        &teacher_token,
        serde_json::json!({}),
    )
    .await;
    assert_eq!(second.status(), 409);
    let body: serde_json::Value = second.json().await;
    assert_eq!(body["active_session_id"], first_id);
    assert!(body["title"].as_str().unwrap().starts_with("Quick session"));
}
```

- [ ] **Step 2: Write the atomic-conflict test**

```rust
#[sqlx::test(migrations = "migrations")]
async fn start_now_conflict_check_is_atomic(pool: PgPool) {
    let (router, course_id, teacher_token) = seed_course_with_teacher(&pool).await;
    let r1 = router.clone();
    let r2 = router.clone();
    let t1 = teacher_token.clone();
    let t2 = teacher_token.clone();
    let cid = course_id;

    // Fire both calls concurrently.
    let (a, b) = tokio::join!(
        async move {
            post_json(
                &r1,
                &format!("/v1/courses/{cid}/sessions/start-now"),
                &t1,
                serde_json::json!({}),
            )
            .await
        },
        async move {
            post_json(
                &r2,
                &format!("/v1/courses/{cid}/sessions/start-now"),
                &t2,
                serde_json::json!({}),
            )
            .await
        }
    );

    let statuses = vec![a.status(), b.status()];
    assert!(
        statuses.contains(&200) && statuses.contains(&409),
        "expected one 200 and one 409, got {statuses:?}"
    );

    // Exactly one live row in the DB.
    let live_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM live_sessions WHERE course_id = $1 AND status = 'live'",
    )
    .bind(course_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(live_count, 1);
}
```

- [ ] **Step 3: Write the forbidden-for-non-admin test**

```rust
#[sqlx::test(migrations = "migrations")]
async fn start_now_requires_can_admin(pool: PgPool) {
    let (router, course_id, _teacher_token, student_token) =
        seed_course_with_teacher_and_student(&pool).await;

    let resp = post_json(
        &router,
        &format!("/v1/courses/{course_id}/sessions/start-now"),
        &student_token,
        serde_json::json!({}),
    )
    .await;
    assert_eq!(resp.status(), 403);
}
```

(If `seed_course_with_teacher_and_student` doesn't exist, write it next to `seed_course_with_teacher` — it should additionally enroll a student in the course and return that student's bearer token.)

- [ ] **Step 4: Write the active-session read tests**

```rust
#[sqlx::test(migrations = "migrations")]
async fn active_session_returns_none_when_no_live(pool: PgPool) {
    let (router, course_id, teacher_token) = seed_course_with_teacher(&pool).await;
    let resp = get_json(
        &router,
        &format!("/v1/courses/{course_id}/active-session"),
        &teacher_token,
    )
    .await;
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await;
    assert!(body["active"].is_null());
}

#[sqlx::test(migrations = "migrations")]
async fn active_session_returns_the_live_one(pool: PgPool) {
    let (router, course_id, teacher_token) = seed_course_with_teacher(&pool).await;
    let start = post_json(
        &router,
        &format!("/v1/courses/{course_id}/sessions/start-now"),
        &teacher_token,
        serde_json::json!({}),
    )
    .await;
    let sid = start.json::<serde_json::Value>().await["session_id"]
        .as_str()
        .unwrap()
        .to_string();

    let resp = get_json(
        &router,
        &format!("/v1/courses/{course_id}/active-session"),
        &teacher_token,
    )
    .await;
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await;
    assert_eq!(body["active"]["session_id"], sid);
}

#[sqlx::test(migrations = "migrations")]
async fn active_session_allows_enrolled_student(pool: PgPool) {
    let (router, course_id, teacher_token, student_token) =
        seed_course_with_teacher_and_student(&pool).await;
    let _start = post_json(
        &router,
        &format!("/v1/courses/{course_id}/sessions/start-now"),
        &teacher_token,
        serde_json::json!({}),
    )
    .await;
    let resp = get_json(
        &router,
        &format!("/v1/courses/{course_id}/active-session"),
        &student_token,
    )
    .await;
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await;
    assert!(!body["active"].is_null());
}

#[sqlx::test(migrations = "migrations")]
async fn active_session_forbids_non_member(pool: PgPool) {
    let (router, course_id, _teacher_token, _student_token, outsider_token) =
        seed_course_with_outsider(&pool).await;
    let resp = get_json(
        &router,
        &format!("/v1/courses/{course_id}/active-session"),
        &outsider_token,
    )
    .await;
    assert_eq!(resp.status(), 404);
}
```

(If `seed_course_with_outsider` doesn't exist, write it: same as the basic seeder plus a second user in the same tenant who is not a course member. `caller_can_read_course` returns false for that user → handler returns `ApiError::NotFound` → 404.)

- [ ] **Step 4b: Write the unique-violation mapping test**

This drives the fallback path inside `start_now_inner` where two concurrent transactions both pass the fast-path check and the second one trips the partial unique index. We simulate it by holding a transaction open with a freshly-inserted live row, then running start-now from a different connection.

```rust
#[sqlx::test(migrations = "migrations")]
async fn start_now_maps_unique_violation_to_409(pool: PgPool) {
    let (router, course_id, teacher_token) = seed_course_with_teacher(&pool).await;

    // Insert a live row directly via a held-open transaction so the
    // fast-path SELECT in start-now sees no live row, but the unique
    // index will fire on the start-now's INSERT.
    let mut tx = pool.begin().await.unwrap();
    let direct_id: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO live_sessions (tenant_id, course_id, series_id, occurrence_index, title,
                                    status, starts_at, duration_minutes, primary_teacher_id,
                                    recording_enabled, transport_mode)
         SELECT tenant_id, $1,
                (SELECT id FROM live_session_series WHERE course_id = $1 LIMIT 1),
                999, 'racey live', 'live', now(), 60, owner_user_id, false, 'webrtc'
           FROM courses WHERE id = $1
         RETURNING id",
    )
    .bind(course_id)
    .fetch_one(&mut *tx)
    .await
    .unwrap();

    // From a separate connection, run start-now. It should see no live
    // row in the fast-path (the held-open tx hasn't committed) and then
    // hit the unique index when inserting its own live row.
    let resp = post_json(
        &router,
        &format!("/v1/courses/{course_id}/sessions/start-now"),
        &teacher_token,
        serde_json::json!({}),
    )
    .await;

    // Commit the held tx so its row is now visible.
    tx.commit().await.unwrap();
    let _ = direct_id;

    assert_eq!(resp.status(), 409, "expected 409, got body: {}", resp.body_text());
}
```

If the seeder doesn't already insert a `live_session_series` row for the course, this query's subselect will fail. In that case, modify the test to first POST a series via the existing scheduler API, then borrow its id.

- [ ] **Step 5: Write the recording-default test**

```rust
#[sqlx::test(migrations = "migrations")]
async fn start_now_defaults_recording_to_tenant_setting(pool: PgPool) {
    let (router, course_id, teacher_token) = seed_course_with_teacher(&pool).await;
    // Tenant default in the seeder should be `true`. Confirm:
    let tenant_default: bool = sqlx::query_scalar(
        "SELECT recording_default FROM tenants t
            JOIN courses c ON c.tenant_id = t.id
           WHERE c.id = $1",
    )
    .bind(course_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    let resp = post_json(
        &router,
        &format!("/v1/courses/{course_id}/sessions/start-now"),
        &teacher_token,
        serde_json::json!({}),
    )
    .await;
    let body: serde_json::Value = resp.json().await;
    assert_eq!(body["recording_enabled"], tenant_default);
}
```

- [ ] **Step 6: Run all the new tests**

```bash
cargo test -p backend --test live_sessions_test
```

Expected: all new tests pass; no regressions in existing tests.

- [ ] **Step 7: Commit**

```bash
git add crates/backend/tests/
git commit -m "test(backend): start-now + active-session integration coverage"
```

---

### Task 8: Frontend API functions

**Files:**
- Modify: `crates/features-courses/src/api.rs`

- [ ] **Step 1: Add DTOs and functions**

Add to `crates/features-courses/src/api.rs` (place them near the existing live-session-related functions; if none, add at end of the file before the test module if any):

```rust
#[derive(Clone, Debug, serde::Serialize, Default, PartialEq)]
pub struct StartNowBody {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_minutes: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recording_enabled: Option<bool>,
}

#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct StartNowResponseDto {
    pub session_id: String,
    pub series_id: String,
    pub title: String,
    pub starts_at: String,
    pub duration_minutes: i32,
    pub recording_enabled: bool,
    pub transport_mode: String,
    pub status: String,
}

#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct StartNowConflictDto {
    pub error: String,
    pub active_session_id: String,
    pub title: String,
    pub starts_at: String,
}

#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct ActiveSessionInfoDto {
    pub session_id: String,
    pub title: String,
    pub starts_at: String,
    pub transport_mode: String,
}

#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct ActiveSessionResponseDto {
    pub active: Option<ActiveSessionInfoDto>,
}

/// Result type for start-now: distinguishes 409-conflict (carries the
/// active session info) from other API errors.
pub enum StartNowOutcome {
    Created(StartNowResponseDto),
    Conflict(StartNowConflictDto),
    Failed(ApiError),
}

pub async fn start_session_now(
    ctx: &ApiContext,
    course_id: &str,
    body: &StartNowBody,
) -> StartNowOutcome {
    let path = format!("/v1/courses/{course_id}/sessions/start-now");
    match fetch_json::<StartNowResponseDto>(ctx, "POST", &path, Some(body)).await {
        Ok(dto) => StartNowOutcome::Created(dto),
        Err(ApiError::Status(409, body_str)) => {
            match serde_json::from_str::<StartNowConflictDto>(&body_str) {
                Ok(conflict) => StartNowOutcome::Conflict(conflict),
                Err(_) => StartNowOutcome::Failed(ApiError::Status(409, body_str)),
            }
        }
        Err(e) => StartNowOutcome::Failed(e),
    }
}

pub async fn get_active_session(
    ctx: &ApiContext,
    course_id: &str,
) -> Result<ActiveSessionResponseDto, ApiError> {
    let path = format!("/v1/courses/{course_id}/active-session");
    fetch_json(ctx, "GET", &path, None::<&()>).await
}
```

- [ ] **Step 2: Verify wasm32 build still works**

```bash
cargo check -p features-courses --target wasm32-unknown-unknown
```

Expected: no errors. (If wasm target isn't installed, run `rustup target add wasm32-unknown-unknown` first.)

- [ ] **Step 3: Verify host build still works**

```bash
cargo check -p features-courses
```

Expected: no errors.

- [ ] **Step 4: Commit**

```bash
git add crates/features-courses/src/api.rs
git commit -m "feat(features-courses): start-session-now + active-session api"
```

---

### Task 9: `use_active_session_poll` hook

**Files:**
- Create: `crates/features-courses/src/active_session.rs`
- Modify: `crates/features-courses/src/lib.rs`

- [ ] **Step 1: Create the new module**

Create `crates/features-courses/src/active_session.rs`:

```rust
// crates/features-courses/src/active_session.rs
//! Hook that polls `/v1/courses/:cid/active-session` while mounted so the
//! teacher's start-now button and the student's "Live now" banner can react
//! within ~15s of a teacher hitting Start.
//!
//! Backoff: success → 15s; transient failure (network / 5xx) → 30s; 401
//! short-circuits and stops polling (the ApiContext refresh path resumes
//! on next user interaction).

use crate::api::{self, ActiveSessionInfoDto, ApiContext, ApiError};
use dioxus::prelude::*;

#[derive(Clone, Debug, PartialEq)]
pub enum PollState {
    Loading,
    Active(ActiveSessionInfoDto),
    Idle,
    Stopped,
}

/// Polls `/active-session` for `course_id` every 15s on success, 30s after
/// a transient error. Returns a Signal that components can read.
///
/// The poll loop terminates when the component using this hook unmounts
/// (dioxus drops the scoped future).
pub fn use_active_session_poll(course_id: String) -> Signal<PollState> {
    let api = use_context::<Signal<ApiContext>>();
    let mut state = use_signal(|| PollState::Loading);

    use_future(move || {
        let course_id = course_id.clone();
        let api = api;
        async move {
            loop {
                let ctx = api.read().clone();
                let delay_ms = match api::get_active_session(&ctx, &course_id).await {
                    Ok(resp) => {
                        let new_state = match resp.active {
                            Some(info) => PollState::Active(info),
                            None => PollState::Idle,
                        };
                        state.set(new_state);
                        15_000_u32
                    }
                    Err(ApiError::Status(401, _)) => {
                        state.set(PollState::Stopped);
                        return;
                    }
                    Err(_) => 30_000_u32,
                };
                #[cfg(target_arch = "wasm32")]
                gloo_timers::future::TimeoutFuture::new(delay_ms).await;
                #[cfg(not(target_arch = "wasm32"))]
                {
                    // Host build: yield once so tests that mount the hook
                    // don't spin. The poll is wasm-only in practice.
                    let _ = delay_ms;
                    futures_util::future::ready(()).await;
                    return;
                }
            }
        }
    });

    state
}
```

- [ ] **Step 2: Register the module**

In `crates/features-courses/src/lib.rs`, add:

```rust
pub mod active_session;
```

(Place it alphabetically with the existing `pub mod` declarations.)

- [ ] **Step 3: Verify both targets build**

```bash
cargo check -p features-courses
cargo check -p features-courses --target wasm32-unknown-unknown
```

Expected: both succeed. If `futures_util` isn't in `Cargo.toml`, either add it as a dependency or rewrite the host-side branch to use `std::future::ready(()).await`.

- [ ] **Step 4: Commit**

```bash
git add crates/features-courses/src/active_session.rs crates/features-courses/src/lib.rs
git commit -m "feat(features-courses): use_active_session_poll hook"
```

---

### Task 10: `LiveNowBanner` component

**Files:**
- Create: `crates/features-courses/src/live_now_banner.rs`
- Modify: `crates/features-courses/src/lib.rs`

- [ ] **Step 1: Create the component**

Create `crates/features-courses/src/live_now_banner.rs`:

```rust
// crates/features-courses/src/live_now_banner.rs
//! Banner that students see on a course page when a live session is in
//! progress. Driven by `use_active_session_poll`. Hidden from admins —
//! their `StartNowButton` already reflects the conflict state.

use design_system::{Button, ButtonVariant};
use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct LiveNowBannerProps {
    pub title: String,
    pub on_join: EventHandler<()>,
}

#[component]
pub fn LiveNowBanner(props: LiveNowBannerProps) -> Element {
    rsx! {
        div { class: "live-now-banner motion-page",
            span { class: "live-now-banner-dot", aria_hidden: "true" }
            span { class: "live-now-banner-label", "Live now" }
            span { class: "live-now-banner-title", "{props.title}" }
            Button {
                label: "Join".to_string(),
                variant: ButtonVariant::Primary,
                button_type: "button".to_string(),
                on_click: move |_| props.on_join.call(()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_title_and_join_button() {
        fn app() -> Element {
            rsx! {
                LiveNowBanner {
                    title: "Quick session — May 17, 2:32 PM UTC".to_string(),
                    on_join: |_| {},
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        let _ = vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("Live now"), "missing 'Live now' label: {html}");
        assert!(
            html.contains("Quick session"),
            "missing title: {html}"
        );
        assert!(html.contains(">Join</button>"), "missing Join button: {html}");
    }
}
```

- [ ] **Step 2: Register the module**

In `crates/features-courses/src/lib.rs`, add:

```rust
pub mod live_now_banner;
```

- [ ] **Step 3: Run the unit test**

```bash
cargo test -p features-courses live_now_banner
```

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/features-courses/src/live_now_banner.rs crates/features-courses/src/lib.rs
git commit -m "feat(features-courses): LiveNowBanner component"
```

---

### Task 11: `StartNowModal` component

**Files:**
- Create: `crates/features-courses/src/start_now_modal.rs`
- Modify: `crates/features-courses/src/lib.rs`

- [ ] **Step 1: Create the modal**

Create `crates/features-courses/src/start_now_modal.rs`:

```rust
// crates/features-courses/src/start_now_modal.rs
//! Optional "Customize…" modal for the start-now flow. Lets the teacher
//! override title, duration, and recording before posting.

use design_system::{Button, ButtonVariant, Field, Input, Select, SelectOption, Toggle};
use dioxus::prelude::*;

#[derive(Clone, Debug, PartialEq)]
pub struct StartNowDraft {
    pub title: String,
    pub duration_minutes: i32,
    pub recording_enabled: bool,
}

#[derive(Props, Clone, PartialEq)]
pub struct StartNowModalProps {
    pub initial: StartNowDraft,
    pub submitting: bool,
    pub on_submit: EventHandler<StartNowDraft>,
    pub on_cancel: EventHandler<()>,
}

#[component]
pub fn StartNowModal(props: StartNowModalProps) -> Element {
    let mut draft = use_signal(|| props.initial.clone());

    let duration_options = vec![
        SelectOption { value: "30".to_string(), label: "30 min".to_string() },
        SelectOption { value: "45".to_string(), label: "45 min".to_string() },
        SelectOption { value: "60".to_string(), label: "60 min".to_string() },
        SelectOption { value: "90".to_string(), label: "90 min".to_string() },
    ];

    rsx! {
        div { class: "start-now-modal-backdrop",
            div { class: "start-now-modal",
                h2 { "Start session now" }
                Field { label: "Title".to_string(),
                    Input {
                        value: draft.read().title.clone(),
                        input_type: "text".to_string(),
                        disabled: props.submitting,
                        on_input: move |v| {
                            let mut d = draft.read().clone();
                            d.title = v;
                            draft.set(d);
                        },
                    }
                }
                Field { label: "Duration".to_string(),
                    Select {
                        value: draft.read().duration_minutes.to_string(),
                        options: duration_options.clone(),
                        disabled: props.submitting,
                        on_change: move |v: String| {
                            let mut d = draft.read().clone();
                            if let Ok(n) = v.parse::<i32>() {
                                d.duration_minutes = n;
                                draft.set(d);
                            }
                        },
                    }
                }
                Field { label: "Recording".to_string(),
                    Toggle {
                        checked: draft.read().recording_enabled,
                        disabled: props.submitting,
                        on_change: move |checked: bool| {
                            let mut d = draft.read().clone();
                            d.recording_enabled = checked;
                            draft.set(d);
                        },
                    }
                }
                div { class: "start-now-modal-actions",
                    Button {
                        label: "Cancel".to_string(),
                        variant: ButtonVariant::Ghost,
                        button_type: "button".to_string(),
                        disabled: props.submitting,
                        on_click: move |_| props.on_cancel.call(()),
                    }
                    Button {
                        label: if props.submitting { "Starting…".to_string() } else { "Start".to_string() },
                        variant: ButtonVariant::Primary,
                        button_type: "button".to_string(),
                        disabled: props.submitting,
                        on_click: move |_| props.on_submit.call(draft.read().clone()),
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft() -> StartNowDraft {
        StartNowDraft {
            title: "Quick session — May 17, 2:32 PM UTC".to_string(),
            duration_minutes: 60,
            recording_enabled: true,
        }
    }

    #[test]
    fn renders_initial_title_and_duration() {
        fn app() -> Element {
            rsx! {
                StartNowModal {
                    initial: super::tests::draft(),
                    submitting: false,
                    on_submit: |_| {},
                    on_cancel: |_| {},
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        let _ = vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("Quick session"),
            "title not prefilled: {html}"
        );
        assert!(
            html.contains("Start session now"),
            "modal header missing: {html}"
        );
    }

    #[test]
    fn shows_starting_label_while_submitting() {
        fn app() -> Element {
            rsx! {
                StartNowModal {
                    initial: super::tests::draft(),
                    submitting: true,
                    on_submit: |_| {},
                    on_cancel: |_| {},
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        let _ = vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("Starting…"), "submit-pending label missing: {html}");
    }
}
```

If `Toggle` is not the design-system's exact name, search `crates/design-system/src/lib.rs` for the toggle/switch component and replace. The `Select` and `Field` components are already used in `series_scheduler.rs` so their imports are confirmed.

- [ ] **Step 2: Register the module**

In `crates/features-courses/src/lib.rs`:

```rust
pub mod start_now_modal;
```

- [ ] **Step 3: Run the tests**

```bash
cargo test -p features-courses start_now_modal
```

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/features-courses/src/start_now_modal.rs crates/features-courses/src/lib.rs
git commit -m "feat(features-courses): StartNowModal component"
```

---

### Task 12: `StartNowButton` component (split button with conflict state)

**Files:**
- Create: `crates/features-courses/src/start_now_button.rs`
- Modify: `crates/features-courses/src/lib.rs`

- [ ] **Step 1: Create the component**

Create `crates/features-courses/src/start_now_button.rs`:

```rust
// crates/features-courses/src/start_now_button.rs
//! Split button rendered in the CourseDetail header for admins. Primary
//! click starts an ad-hoc session with defaults; caret opens
//! `StartNowModal` for customization. When a session is already live in
//! the course (driven by `use_active_session_poll`), the button relabels
//! to "Join active session" and the caret hides.

use design_system::{Button, ButtonVariant};
use dioxus::prelude::*;

#[derive(Clone, Debug, PartialEq)]
pub enum StartNowState {
    /// No live session — show "Start session now" + caret.
    Idle,
    /// A live session exists — show "Join active session" only.
    Conflict { active_session_id: String },
    /// Posting `start-now`. Button disabled with spinner-style label.
    Submitting,
}

#[derive(Props, Clone, PartialEq)]
pub struct StartNowButtonProps {
    pub state: StartNowState,
    pub on_quick_start: EventHandler<()>,
    pub on_customize: EventHandler<()>,
    pub on_join_active: EventHandler<String>,
}

#[component]
pub fn StartNowButton(props: StartNowButtonProps) -> Element {
    match &props.state {
        StartNowState::Idle => rsx! {
            span { class: "start-now-split",
                Button {
                    label: "Start session now".to_string(),
                    variant: ButtonVariant::Primary,
                    button_type: "button".to_string(),
                    on_click: move |_| props.on_quick_start.call(()),
                }
                Button {
                    label: "▾".to_string(),
                    variant: ButtonVariant::Primary,
                    button_type: "button".to_string(),
                    on_click: move |_| props.on_customize.call(()),
                }
            }
        },
        StartNowState::Submitting => rsx! {
            Button {
                label: "Starting…".to_string(),
                variant: ButtonVariant::Primary,
                button_type: "button".to_string(),
                disabled: true,
                on_click: move |_| {},
            }
        },
        StartNowState::Conflict { active_session_id } => {
            let id = active_session_id.clone();
            rsx! {
                Button {
                    label: "Join active session".to_string(),
                    variant: ButtonVariant::Primary,
                    button_type: "button".to_string(),
                    on_click: move |_| props.on_join_active.call(id.clone()),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_state_renders_primary_plus_caret() {
        fn app() -> Element {
            rsx! {
                StartNowButton {
                    state: StartNowState::Idle,
                    on_quick_start: |_| {},
                    on_customize: |_| {},
                    on_join_active: |_| {},
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        let _ = vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("Start session now"), "missing primary: {html}");
        assert!(html.contains("▾"), "missing caret: {html}");
    }

    #[test]
    fn conflict_state_relabels_and_hides_caret() {
        fn app() -> Element {
            rsx! {
                StartNowButton {
                    state: StartNowState::Conflict { active_session_id: "session-1".to_string() },
                    on_quick_start: |_| {},
                    on_customize: |_| {},
                    on_join_active: |_| {},
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        let _ = vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("Join active session"),
            "missing relabeled button: {html}"
        );
        assert!(!html.contains("▾"), "caret should be hidden: {html}");
        assert!(
            !html.contains("Start session now"),
            "primary label should be hidden: {html}"
        );
    }

    #[test]
    fn submitting_state_disables_button() {
        fn app() -> Element {
            rsx! {
                StartNowButton {
                    state: StartNowState::Submitting,
                    on_quick_start: |_| {},
                    on_customize: |_| {},
                    on_join_active: |_| {},
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        let _ = vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("Starting…"), "missing submitting label: {html}");
        assert!(html.contains("disabled"), "button should be disabled: {html}");
    }
}
```

- [ ] **Step 2: Register the module**

In `crates/features-courses/src/lib.rs`:

```rust
pub mod start_now_button;
```

- [ ] **Step 3: Run the tests**

```bash
cargo test -p features-courses start_now_button
```

Expected: all three tests PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/features-courses/src/start_now_button.rs crates/features-courses/src/lib.rs
git commit -m "feat(features-courses): StartNowButton component with conflict state"
```

---

### Task 13: Extend `CourseDetail` to accept extra header actions and a banner slot

**Files:**
- Modify: `crates/features-courses/src/course_detail.rs`

- [ ] **Step 1: Update the existing test for the "Edit" affordance so it still passes once we add new props**

Note: the existing tests `course_detail_header_shows_edit_affordance_for_admin` and `course_detail_header_hides_edit_affordance_for_non_admin` live in `crates/shell-web/src/routes/course_detail.rs:794-843`. They construct `CourseDetail` props directly. Adding new **optional** props with defaults will leave them passing — we won't need to touch the test file. Verify the existing test still passes after Step 3.

- [ ] **Step 2: Modify the props and the render body**

Replace the existing `CourseDetailProps` block in `crates/features-courses/src/course_detail.rs` (lines 8-17) with:

```rust
#[derive(Props, Clone, PartialEq)]
pub struct CourseDetailProps {
    pub course_title: String,
    pub course_status: String,
    pub course_cover_asset_id: Option<String>,
    pub can_admin: bool,
    pub active_tab: String,
    pub on_tab_change: EventHandler<String>,
    /// Additional actions to render in the PageHeader actions slot
    /// (e.g., StartNowButton for admins). Rendered between the status
    /// badge and the existing Edit button.
    #[props(default)]
    pub extra_actions: Option<Element>,
    /// Banner rendered below the PageHeader and above the tabs.
    /// Used for the student-facing LiveNowBanner.
    #[props(default)]
    pub banner: Option<Element>,
    pub children: Element,
}
```

Then in the `actions` block (currently lines 59-74), inject `extra_actions`:

```rust
    let actions: Option<Element> = if props.can_admin {
        let on_change_edit = props.on_tab_change.clone();
        let extras = props.extra_actions.clone();
        Some(rsx! {
            Badge { label: status_label, tone: status_tone }
            { extras }
            Button {
                label: "Edit".to_string(),
                variant: ButtonVariant::Ghost,
                button_type: "button".to_string(),
                on_click: move |_| on_change_edit.call("edit".to_string()),
            }
        })
    } else {
        let extras = props.extra_actions.clone();
        Some(rsx! {
            Badge { label: status_label, tone: status_tone }
            { extras }
        })
    };
```

And in the main `rsx!` block, insert the banner just below `PageHeader`:

```rust
    rsx! {
        div { class: "course-detail motion-page",
            if let Some(asset_id) = &props.course_cover_asset_id {
                div { class: "course-detail-banner",
                    FileAssetImage {
                        asset_id: asset_id.clone(),
                        alt: props.course_title.clone(),
                        class: Some("course-banner-img".to_string()),
                    }
                }
            }
            PageHeader {
                kicker: "Course".to_string(),
                title: props.course_title.clone(),
                variant: PageHeaderVariant::Hero,
                actions: actions,
            }
            { props.banner.clone() }
            Tabs { tabs: tabs, active: props.active_tab.clone(),
                on_change: move |k| on_change.call(k) }
            div { class: "course-detail-body", {props.children} }
        }
    }
```

- [ ] **Step 3: Verify the existing CourseDetail tests still pass**

```bash
cargo test -p shell-web course_detail_header
```

Expected: `course_detail_header_shows_edit_affordance_for_admin` and `course_detail_header_hides_edit_affordance_for_non_admin` both PASS without modification (the new props default to `None`).

- [ ] **Step 4: Verify both crates compile**

```bash
cargo check -p features-courses
cargo check -p shell-web
```

Expected: both succeed. (shell-web constructs `CourseDetail` directly without the new props, which is fine because they default.)

- [ ] **Step 5: Commit**

```bash
git add crates/features-courses/src/course_detail.rs
git commit -m "feat(features-courses): CourseDetail extra_actions + banner slots"
```

---

### Task 14: Wire `StartNowButton`, `StartNowModal`, and `LiveNowBanner` into the shell route

**Files:**
- Modify: `crates/shell-web/src/routes/course_detail.rs`

- [ ] **Step 1: Add imports**

At the top of `crates/shell-web/src/routes/course_detail.rs`, alongside the existing `features_courses::…` imports, add:

```rust
use features_courses::active_session::{use_active_session_poll, PollState};
use features_courses::api::{StartNowBody, StartNowOutcome};
use features_courses::live_now_banner::LiveNowBanner;
use features_courses::start_now_button::{StartNowButton, StartNowState};
use features_courses::start_now_modal::{StartNowDraft, StartNowModal};
```

- [ ] **Step 2: Add a helper that derives `StartNowState` from poll state + submission flag**

Just below the `can_admin_course` function, add:

```rust
fn derive_start_now_state(
    poll: &PollState,
    submitting: bool,
) -> StartNowState {
    if submitting {
        return StartNowState::Submitting;
    }
    match poll {
        PollState::Active(info) => StartNowState::Conflict {
            active_session_id: info.session_id.clone(),
        },
        _ => StartNowState::Idle,
    }
}
```

- [ ] **Step 3: Build the start-now + banner machinery inside `render_with_tab`**

After the line `let can_admin = can_admin_course(&user, c);` in the `Some(Ok(c))` branch (currently around line 143), add this block:

```rust
            let course_id_for_now = course_id.clone();
            let slug_for_now = slug.clone();
            let poll_signal = use_active_session_poll(course_id.clone());
            let mut submitting = use_signal(|| false);
            let mut show_modal = use_signal(|| false);
            let mut toast = use_toast_sender();
            let api_clone = api.clone();

            let on_quick_start = {
                let api = api_clone.clone();
                let course_id = course_id_for_now.clone();
                let slug = slug_for_now.clone();
                let nav = nav;
                move |_: ()| {
                    let api = api.clone();
                    let course_id = course_id.clone();
                    let slug = slug.clone();
                    spawn(async move {
                        submitting.set(true);
                        let outcome = api::start_session_now(
                            &api,
                            &course_id,
                            &StartNowBody::default(),
                        )
                        .await;
                        match outcome {
                            StartNowOutcome::Created(dto) => {
                                nav.push(Route::LiveSession {
                                    slug: slug.clone(),
                                    session_id: dto.session_id,
                                });
                            }
                            StartNowOutcome::Conflict(conflict) => {
                                toast.push(
                                    ToastLevel::Warning,
                                    "Already live",
                                    "A session is already live in this course.",
                                );
                                nav.push(Route::LiveSession {
                                    slug: slug.clone(),
                                    session_id: conflict.active_session_id,
                                });
                            }
                            StartNowOutcome::Failed(e) => {
                                let (title_text, body_text) = match &e {
                                    api::ApiError::Status(403, _) => (
                                        "You no longer have permission",
                                        "Refresh the page and sign in again.".to_string(),
                                    ),
                                    _ => ("Couldn't start the session", format!("{e}")),
                                };
                                toast.push(ToastLevel::Danger, title_text, body_text);
                            }
                        }
                        submitting.set(false);
                    });
                }
            };

            let on_customize = move |_: ()| show_modal.set(true);

            let on_modal_cancel = move |_: ()| show_modal.set(false);

            let on_modal_submit = {
                let api = api_clone.clone();
                let course_id = course_id_for_now.clone();
                let slug = slug_for_now.clone();
                let nav = nav;
                move |d: StartNowDraft| {
                    let api = api.clone();
                    let course_id = course_id.clone();
                    let slug = slug.clone();
                    spawn(async move {
                        submitting.set(true);
                        let body = StartNowBody {
                            title: Some(d.title),
                            duration_minutes: Some(d.duration_minutes),
                            recording_enabled: Some(d.recording_enabled),
                        };
                        let outcome =
                            api::start_session_now(&api, &course_id, &body).await;
                        match outcome {
                            StartNowOutcome::Created(dto) => {
                                show_modal.set(false);
                                nav.push(Route::LiveSession {
                                    slug,
                                    session_id: dto.session_id,
                                });
                            }
                            StartNowOutcome::Conflict(conflict) => {
                                show_modal.set(false);
                                toast.push(
                                    ToastLevel::Warning,
                                    "Already live",
                                    "A session is already live in this course.",
                                );
                                nav.push(Route::LiveSession {
                                    slug,
                                    session_id: conflict.active_session_id,
                                });
                            }
                            StartNowOutcome::Failed(e) => {
                                let (title_text, body_text) = match &e {
                                    api::ApiError::Status(403, _) => (
                                        "You no longer have permission",
                                        "Refresh the page and sign in again.".to_string(),
                                    ),
                                    _ => ("Couldn't start the session", format!("{e}")),
                                };
                                toast.push(ToastLevel::Danger, title_text, body_text);
                            }
                        }
                        submitting.set(false);
                    });
                }
            };

            let on_join_active = {
                let slug = slug_for_now.clone();
                let nav = nav;
                move |session_id: String| {
                    nav.push(Route::LiveSession {
                        slug: slug.clone(),
                        session_id,
                    });
                }
            };

            let poll_snapshot = poll_signal.read().clone();
            let start_state = derive_start_now_state(&poll_snapshot, *submitting.read());

            let extra_actions: Option<Element> = if can_admin {
                let modal_open = *show_modal.read();
                Some(rsx! {
                    StartNowButton {
                        state: start_state,
                        on_quick_start: on_quick_start,
                        on_customize: on_customize,
                        on_join_active: on_join_active.clone(),
                    }
                    if modal_open {
                        StartNowModal {
                            initial: StartNowDraft {
                                title: default_quick_session_title(),
                                duration_minutes: 60,
                                recording_enabled: true,
                            },
                            submitting: *submitting.read(),
                            on_submit: on_modal_submit,
                            on_cancel: on_modal_cancel,
                        }
                    }
                })
            } else {
                None
            };

            let banner: Option<Element> = if !can_admin {
                match &poll_snapshot {
                    PollState::Active(info) => {
                        let on_join = on_join_active.clone();
                        let active_id = info.session_id.clone();
                        let title = info.title.clone();
                        Some(rsx! {
                            LiveNowBanner {
                                title,
                                on_join: move |_| on_join(active_id.clone()),
                            }
                        })
                    }
                    _ => None,
                }
            } else {
                None
            };
```

Add at module scope (top of the file, near `DEFAULT_LESSON_TYPE`):

```rust
fn default_quick_session_title() -> String {
    let now = chrono::Utc::now();
    format!(
        "Quick session — {}",
        now.format("%b %-d, %Y %-I:%M %p UTC")
    )
}
```

And in the `CourseDetailView` invocation (around line 174), pass the new slots:

```rust
            CourseDetailView {
                course_title: title,
                course_status: status,
                course_cover_asset_id: cover,
                can_admin: can_admin,
                active_tab: active_tab.to_string(),
                on_tab_change: move |tab: String| {
                    // existing match stays unchanged
                },
                extra_actions: extra_actions,
                banner: banner,
                { tab_body }
            }
```

- [ ] **Step 4: Confirm `chrono` is a dependency of shell-web**

```bash
grep -n "chrono" crates/shell-web/Cargo.toml
```

If chrono is not listed, add `chrono = { workspace = true, features = ["std", "clock"] }` to `[dependencies]`. The features-courses crate already uses chrono so the workspace entry exists.

- [ ] **Step 5: Verify it compiles**

```bash
cargo check -p shell-web
```

Expected: no errors. Common fix-ups:
- If `nav` move-into-closure complains, capture `let nav = nav;` once per closure rather than reusing the outer binding.
- If `submitting.set` is unreachable inside a non-`mut`-captured signal, change the outer binding to `let mut submitting = use_signal(...)`.
- If the modal can't see the toast, hoist `let mut toast = use_toast_sender();` above its first use.

- [ ] **Step 6: Verify the existing shell-web tests still pass**

```bash
cargo test -p shell-web course_detail
```

Expected: existing tests still pass.

- [ ] **Step 7: Commit**

```bash
git add crates/shell-web/src/routes/course_detail.rs
git commit -m "feat(shell-web): wire StartNowButton/Modal + LiveNowBanner into CourseDetail"
```

---

### Task 15: Verify `/go-live` re-POST from broadcast view is safe

**Files:**
- (Read-only verification — no edits unless a real bug is found.)

- [ ] **Step 1: Re-read the live-room broadcast entry point**

```bash
sed -n '1,60p' crates/features-courses/src/live_room_broadcast.rs
```

Find the place where the broadcast view POSTs `/go-live` on mount.

- [ ] **Step 2: Confirm `db::live_sessions::go_live` accepts already-live rows**

Reopen `crates/backend/src/db/live_sessions.rs` and verify the `go_live` function's WHERE clause is `WHERE id = $1 AND status IN ('scheduled', 'live')`. This was confirmed during planning; re-confirm it's unchanged.

- [ ] **Step 3: Confirm the GO_LIVE window allows immediate ad-hoc go-live**

In `crates/backend/src/handlers/live_sessions.rs` find `GO_LIVE_WINDOW_BEFORE` and `GO_LIVE_WINDOW_AFTER`. Confirm that `now ∈ [starts_at - GO_LIVE_WINDOW_BEFORE, starts_at + GO_LIVE_WINDOW_AFTER]` holds when `starts_at == now`. If the window is e.g. `±5 min`, both ends include `now`, so this is satisfied.

If the window is one-sided (e.g., only `[starts_at, starts_at + window]`), this still passes for `starts_at == now`. No change needed.

- [ ] **Step 4: Run the existing live-session test suite to confirm no regressions**

```bash
cargo test -p backend --test live_sessions_test
```

Expected: all pre-existing tests still pass alongside the new ones.

- [ ] **Step 5: Document the verification result**

If everything checked out, no commit. If you found a gap (e.g., the GO_LIVE_WINDOW was one-sided in the wrong direction, or `go_live` doesn't accept `'live'` status), pause and surface the issue — it will need a small follow-up fix that should be added as a new task in this plan.

---

### Task 16: Playwright e2e — teacher start-now happy path

**Files:**
- Modify: `tools/ui-real-stack.spec.js`

- [ ] **Step 1: Add the test inside the existing serial describe**

Append the following test to `tools/ui-real-stack.spec.js` (after the existing `teacher walks the workspace…` test):

```js
test("teacher starts an instant session from the course page", async ({ page, request }) => {
  const { course, seed } = seedCtx;
  const consoleErrors = collectConsoleErrors(page);

  // Sign in as teacher via the existing local-login UI.
  await page.goto(`${baseURL}/login`, { waitUntil: "domcontentloaded" });
  await page.fill('input[name="email"]', teacherEmail);
  await page.fill('input[name="password"]', teacherPassword);
  await page.click('button[type="submit"]');
  await page.waitForURL(/\/$/);

  // End any in-progress session from earlier tests so the conflict
  // check doesn't trip. Best-effort.
  const tokenResp = await request.post(`${apiBase}/v1/auth/local-login`, {
    data: { email: teacherEmail, password: teacherPassword },
  });
  const { id_token: teacherToken } = await tokenResp.json();
  const active = await apiJson(
    request,
    `${apiBase}/v1/courses/${course.id}/active-session`,
    { Authorization: `Bearer ${teacherToken}` },
  );
  if (active && active.active) {
    await request.post(`${apiBase}/v1/sessions/${active.active.session_id}/end-class`, {
      headers: { Authorization: `Bearer ${teacherToken}` },
    });
  }

  await page.goto(`${baseURL}/courses/${seed.course_slug}`);
  await expect(page.getByRole("button", { name: "Start session now" })).toBeVisible();
  await screenshot(page, "instant-session-button");

  // Click the primary; expect navigation to /courses/{slug}/sessions/{id}.
  await page.getByRole("button", { name: "Start session now" }).click();
  await page.waitForURL(new RegExp(`/courses/${seed.course_slug}/sessions/[0-9a-f-]+`));
  await screenshot(page, "instant-session-broadcast");

  // Verify a `live` row exists for this course.
  const verify = await apiJson(
    request,
    `${apiBase}/v1/courses/${course.id}/active-session`,
    { Authorization: `Bearer ${teacherToken}` },
  );
  expect(verify.active).not.toBeNull();
  expect(verify.active.title).toContain("Quick session");

  expect(consoleErrors, consoleErrors.join("\n")).toEqual([]);
});
```

(If `apiJson`, `collectConsoleErrors`, and `screenshot` are not defined in this file, they exist — search the existing file for their definitions.)

- [ ] **Step 2: Run the spec**

Pre-req: dev API + dev web server running. The repo's existing dev script handles this.

```bash
npx playwright test tools/ui-real-stack.spec.js -g "starts an instant session"
```

Expected: PASS. If it fails because the button label differs, fix the button label in `start_now_button.rs` to match (the canonical wording is the spec's: "Start session now").

- [ ] **Step 3: Commit**

```bash
git add tools/ui-real-stack.spec.js
git commit -m "test(e2e): teacher start-now happy path"
```

---

### Task 17: Playwright e2e — student sees the Live now banner

**Files:**
- Modify: `tools/ui-real-stack.spec.js`

- [ ] **Step 1: Add the student-banner test**

Append after the Task 16 test:

```js
test("student sees Live now banner within ~20s of teacher starting", async ({ browser, request }) => {
  const { course, seed } = seedCtx;
  const tokenResp = await request.post(`${apiBase}/v1/auth/local-login`, {
    data: { email: teacherEmail, password: teacherPassword },
  });
  const { id_token: teacherToken } = await tokenResp.json();

  // Reset to a known state: end any in-progress session.
  const cur = await apiJson(
    request,
    `${apiBase}/v1/courses/${course.id}/active-session`,
    { Authorization: `Bearer ${teacherToken}` },
  );
  if (cur && cur.active) {
    await request.post(`${apiBase}/v1/sessions/${cur.active.session_id}/end-class`, {
      headers: { Authorization: `Bearer ${teacherToken}` },
    });
  }

  // Student context: open the course page first, before the teacher starts.
  const studentCtx = await browser.newContext();
  const studentPage = await studentCtx.newPage();
  await studentPage.goto(`${baseURL}/login`, { waitUntil: "domcontentloaded" });
  await studentPage.fill('input[name="email"]', studentEmail);
  await studentPage.fill('input[name="password"]', studentPassword);
  await studentPage.click('button[type="submit"]');
  await studentPage.waitForURL(/\/$/);
  await studentPage.goto(`${baseURL}/courses/${seed.course_slug}`);

  // Confirm banner is NOT visible at first.
  await expect(studentPage.locator(".live-now-banner")).toHaveCount(0);

  // Teacher triggers start-now via API (faster than driving the UI).
  const startResp = await request.post(
    `${apiBase}/v1/courses/${course.id}/sessions/start-now`,
    { headers: { Authorization: `Bearer ${teacherToken}` }, data: {} },
  );
  expect(startResp.ok(), await startResp.text()).toBeTruthy();

  // Banner should appear within ~20s (15s poll + slack).
  await expect(studentPage.locator(".live-now-banner")).toBeVisible({ timeout: 25_000 });
  await expect(studentPage.getByText("Live now")).toBeVisible();
  await screenshot(studentPage, "live-now-banner");

  await studentCtx.close();
});
```

- [ ] **Step 2: Run the spec**

```bash
npx playwright test tools/ui-real-stack.spec.js -g "Live now banner"
```

Expected: PASS within ~30 seconds.

- [ ] **Step 3: Commit**

```bash
git add tools/ui-real-stack.spec.js
git commit -m "test(e2e): student Live now banner appears via poll"
```

---

## Final verification

- [ ] **Run the full test suite for affected crates**

```bash
cargo test -p backend
cargo test -p features-courses
cargo test -p shell-web
```

Expected: all pass.

- [ ] **Run the full Playwright spec**

```bash
npx playwright test tools/ui-real-stack.spec.js
```

Expected: all pass (the existing tests should still pass alongside the two new ones).

- [ ] **Lint**

```bash
cargo clippy --workspace -- -D warnings
```

Expected: no warnings introduced.

- [ ] **Final commit if any cleanup happened**

```bash
git status
# If clean, you're done.
```
