# Phase 1d-a Audit Closure Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the remaining web audit blockers with real app behavior: CourseDetail tabs with real data, assignment reference attachments, local audit seed data, browser checklist evidence, and then Docker/8080 stack health.

**Architecture:** Add the smallest missing backend read endpoints for CourseDetail, expose them through `features-courses::api`, then wire the existing `features-courses` components from `shell-web` route code. Local audit seed support builds on the local-login bypass and remains disabled in production-like environments.

**Tech Stack:** Rust, Axum, SQLx/Postgres, Dioxus 0.7, Dioxus Router, existing `features-courses` component crate, Docker Compose for final stack health.

---

## Current State And Preflight

The worktree is expected to contain uncommitted Firebase/local-login-bypass work from the prior audit step. This phase builds on that work. Do not revert it.

Before starting implementation, checkpoint that existing work so Phase 1d-a tasks can be reviewed separately.

## File Structure

**Backend read models and handlers**

- Create: `crates/backend/src/db/course_detail.rs`
  - Read-only SQL helpers for course outline, members, and sessions.
- Modify: `crates/backend/src/db/mod.rs`
  - Export `course_detail`.
- Modify: `crates/backend/src/handlers/courses.rs`
  - Add DTOs and handlers for:
    - `GET /v1/courses/:id/modules-with-lessons`
    - `GET /v1/courses/:id/members`
    - `GET /v1/courses/:id/sessions`
- Test: `crates/backend/tests/course_detail_tabs.rs`
  - Integration tests for the new read endpoints.

**Local audit seed and login profiles**

- Modify: `crates/backend/src/auth/local_login.rs`
  - Replace single-token model with local profiles while preserving existing `LOCAL_LOGIN_TOKEN` compatibility.
- Modify: `crates/backend/src/handlers/dev_login.rs`
  - Return profile config and accept profile login requests.
- Create: `crates/backend/src/handlers/dev_seed.rs`
  - Dev-only `POST /v1/dev/audit-seed` route.
- Modify: `crates/backend/src/handlers/mod.rs`
  - Export `dev_seed`.
- Modify: `crates/backend/src/lib.rs`
  - Mount seed route in the public dev route group.
- Test: `crates/backend/tests/local_login_bypass.rs`
  - Update for profile behavior.
- Test: `crates/backend/tests/audit_seed.rs`
  - Guard and seed behavior tests.

**Frontend API and route wiring**

- Modify: `crates/features-courses/src/api.rs`
  - Add DTOs and methods for course outline, members, sessions, course patch, invite/code mutations, module/lesson mutations, assignment patch.
- Modify: `crates/features-courses/src/assignment_editor.rs`
  - Add reference attachment picker for existing draft assignments.
- Modify: `crates/shell-web/src/routes/login.rs`
  - Render local teacher/student bypass buttons from dev-login profile config.
- Modify: `crates/shell-web/src/routes/course_detail.rs`
  - Replace placeholder tab body with real outline, people, edit, and schedule bodies.
- Test: `crates/shell-web/tests/shell_routes_smoke.rs`
  - Add CourseDetail tab smoke checks.

**Docs and audit checklist**

- Modify: `docs/superpowers/plans/2026-05-10-aulalite-phase-1-5-shell-wiring-exit-checklist.md`
  - Record exact verification outcomes.

---

### Task 0: Checkpoint Existing Local Bypass Work

**Files:**
- Stage only existing local-bypass/Firebase audit files.
- Do not stage `.claude/`.

- [ ] **Step 1: Inspect current worktree**

Run:

```powershell
git status --short --branch
```

Expected: dirty files include local-login/Firebase/Docker changes and the Phase 1.5 checklist. `.claude/` may be untracked.

- [ ] **Step 2: Review staged state is empty**

Run:

```powershell
git diff --cached --name-only
```

Expected: no output.

- [ ] **Step 3: Stage the existing audit-bypass files only**

Run:

```powershell
git add `
  .env.example `
  Cargo.lock `
  crates/backend/src/auth/middleware.rs `
  crates/backend/src/auth/mod.rs `
  crates/backend/src/auth/local_login.rs `
  crates/backend/src/handlers/mod.rs `
  crates/backend/src/handlers/dev_login.rs `
  crates/backend/src/lib.rs `
  crates/backend/src/main.rs `
  crates/backend/tests/local_login_bypass.rs `
  crates/features-courses/src/api.rs `
  crates/shell-web/index.html `
  crates/shell-web/src/routes/login.rs `
  docker-compose.yml `
  docs/superpowers/plans/2026-05-10-aulalite-phase-1-5-shell-wiring-exit-checklist.md
```

- [ ] **Step 4: Verify only intended files are staged**

Run:

```powershell
git diff --cached --name-only
```

Expected output:

```text
.env.example
Cargo.lock
crates/backend/src/auth/local_login.rs
crates/backend/src/auth/middleware.rs
crates/backend/src/auth/mod.rs
crates/backend/src/handlers/dev_login.rs
crates/backend/src/handlers/mod.rs
crates/backend/src/lib.rs
crates/backend/src/main.rs
crates/backend/tests/local_login_bypass.rs
crates/features-courses/src/api.rs
crates/shell-web/index.html
crates/shell-web/src/routes/login.rs
docker-compose.yml
docs/superpowers/plans/2026-05-10-aulalite-phase-1-5-shell-wiring-exit-checklist.md
```

- [ ] **Step 5: Commit the checkpoint**

Run:

```powershell
git commit -m "feat(auth): add local audit login bypass"
```

Expected: commit succeeds. The worktree may still show `.claude/` untracked.

---

### Task 1: Backend Tests For CourseDetail Read Endpoints

**Files:**
- Create: `crates/backend/tests/course_detail_tabs.rs`

- [ ] **Step 1: Write failing endpoint tests**

Create `crates/backend/tests/course_detail_tabs.rs` with:

```rust
mod fixtures;

use fixtures::*;
use serde_json::json;
use uuid::Uuid;

async fn seed_course(pool: &sqlx::PgPool) -> (Uuid, Uuid, Uuid, Uuid) {
    let tenant = create_tenant(pool).await;
    let (teacher, _, _) = create_user(pool).await;
    let (student, _, _) = create_user(pool).await;
    attach_membership(pool, tenant, teacher, "teacher").await;
    attach_membership(pool, tenant, student, "student").await;

    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();

    let course: Uuid = sqlx::query_scalar(
        "INSERT INTO courses (tenant_id, slug, title, owner_user_id, status)
         VALUES ($1, $2, 'Audit Course', $3, 'published') RETURNING id",
    )
    .bind(tenant)
    .bind(format!("audit-{}", Uuid::new_v4()))
    .bind(teacher)
    .fetch_one(&mut *tx)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO course_memberships (course_id, user_id, tenant_id, role, status)
         VALUES ($1, $2, $3, 'teacher', 'active'), ($1, $4, $3, 'student', 'active')",
    )
    .bind(course)
    .bind(teacher)
    .bind(tenant)
    .bind(student)
    .execute(&mut *tx)
    .await
    .unwrap();

    let module: Uuid = sqlx::query_scalar(
        "INSERT INTO modules (tenant_id, course_id, title, sort_order)
         VALUES ($1, $2, 'Week 1', 10) RETURNING id",
    )
    .bind(tenant)
    .bind(course)
    .fetch_one(&mut *tx)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO lessons (tenant_id, course_id, module_id, type, title, body_md, sort_order)
         VALUES ($1, $2, $3, 'rich_text', 'Welcome', 'Read this first', 10)",
    )
    .bind(tenant)
    .bind(course)
    .bind(module)
    .execute(&mut *tx)
    .await
    .unwrap();

    let series: Uuid = sqlx::query_scalar(
        "INSERT INTO live_session_series
            (tenant_id, course_id, title, starts_at, duration_minutes, frequency,
             end_kind, occurrence_count, primary_teacher_id, recording_enabled)
         VALUES
            ($1, $2, 'Weekly Class', now() + interval '1 day', 60, 'none',
             'count', 1, $3, true)
         RETURNING id",
    )
    .bind(tenant)
    .bind(course)
    .bind(teacher)
    .fetch_one(&mut *tx)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO live_sessions
            (tenant_id, course_id, series_id, occurrence_index, title, starts_at,
             duration_minutes, primary_teacher_id, recording_enabled)
         VALUES ($1, $2, $3, 0, 'Weekly Class', now() + interval '1 day', 60, $4, true)",
    )
    .bind(tenant)
    .bind(course)
    .bind(series)
    .bind(teacher)
    .execute(&mut *tx)
    .await
    .unwrap();

    tx.commit().await.unwrap();
    (tenant, teacher, student, course)
}

fn app_for(
    pool: sqlx::PgPool,
    tenant: Uuid,
    user: Uuid,
    role: core_types::TenantRole,
) -> axum::Router {
    build_test_app(
        backend::handlers::courses::router_for_tests(pool.clone()),
        StubAuth {
            pool,
            user_id: user,
            firebase_uid: format!("fb-{user}"),
            email: format!("{user}@example.test"),
            tenant_id: Some(tenant),
            tenant_role: Some(role),
        },
    )
}

#[tokio::test]
async fn course_outline_returns_modules_with_lessons_for_member() {
    let pool = pool().await;
    let (tenant, _teacher, student, course) = seed_course(&pool).await;
    let app = app_for(pool, tenant, student, core_types::TenantRole::Student);

    let (status, body) = fire(
        &app,
        "GET",
        &format!("/v1/courses/{course}/modules-with-lessons"),
        None,
    )
    .await;

    assert_eq!(status, 200, "{body}");
    assert_eq!(body[0]["title"], "Week 1");
    assert_eq!(body[0]["lessons"][0]["title"], "Welcome");
    assert_eq!(body[0]["lessons"][0]["type"], "rich_text");
}

#[tokio::test]
async fn course_members_returns_active_people_for_teacher() {
    let pool = pool().await;
    let (tenant, teacher, _student, course) = seed_course(&pool).await;
    let app = app_for(pool, tenant, teacher, core_types::TenantRole::Teacher);

    let (status, body) = fire(
        &app,
        "GET",
        &format!("/v1/courses/{course}/members"),
        None,
    )
    .await;

    assert_eq!(status, 200, "{body}");
    let members = body.as_array().unwrap();
    assert!(members.iter().any(|m| m["role"] == "teacher"));
    assert!(members.iter().any(|m| m["role"] == "student"));
}

#[tokio::test]
async fn course_sessions_returns_schedule_for_member() {
    let pool = pool().await;
    let (tenant, _teacher, student, course) = seed_course(&pool).await;
    let app = app_for(pool, tenant, student, core_types::TenantRole::Student);

    let (status, body) = fire(
        &app,
        "GET",
        &format!("/v1/courses/{course}/sessions"),
        None,
    )
    .await;

    assert_eq!(status, 200, "{body}");
    assert_eq!(body[0]["title"], "Weekly Class");
    assert_eq!(body[0]["status"], "scheduled");
    assert_eq!(body[0]["duration_minutes"], 60);
}

#[tokio::test]
async fn non_member_cannot_read_course_outline() {
    let pool = pool().await;
    let (tenant, _teacher, _student, course) = seed_course(&pool).await;
    let (outsider, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, outsider, "student").await;
    let app = app_for(pool, tenant, outsider, core_types::TenantRole::Student);

    let (status, _) = fire(
        &app,
        "GET",
        &format!("/v1/courses/{course}/modules-with-lessons"),
        None,
    )
    .await;

    assert_eq!(status, 404);
}

#[tokio::test]
async fn student_cannot_read_people_tab_admin_data() {
    let pool = pool().await;
    let (tenant, _teacher, student, course) = seed_course(&pool).await;
    let app = app_for(pool, tenant, student, core_types::TenantRole::Student);

    let (status, _) = fire(
        &app,
        "GET",
        &format!("/v1/courses/{course}/members"),
        None,
    )
    .await;

    assert_eq!(status, 403);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```powershell
& cmd.exe /d /s /c '"C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat" >nul && set DATABASE_URL=postgres://aulalite:changeme@localhost:55432/aulalite && cargo test -p backend --test course_detail_tabs -- --nocapture'
```

Expected: tests compile, then fail with `404` for the new routes or compile failure because the routes are not implemented.

- [ ] **Step 3: Commit failing tests**

Run:

```powershell
git add crates/backend/tests/course_detail_tabs.rs
git commit -m "test(backend): cover course detail read endpoints"
```

---

### Task 2: Backend CourseDetail Read Endpoints

**Files:**
- Create: `crates/backend/src/db/course_detail.rs`
- Modify: `crates/backend/src/db/mod.rs`
- Modify: `crates/backend/src/handlers/courses.rs`

- [ ] **Step 1: Create read-only SQL helper module**

Create `crates/backend/src/db/course_detail.rs`:

```rust
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct LessonSummaryRow {
    pub id: Uuid,
    pub course_id: Uuid,
    pub module_id: Uuid,
    pub r#type: String,
    pub title: String,
    pub body_md: Option<String>,
    pub video_asset_id: Option<Uuid>,
    pub live_session_id: Option<Uuid>,
    pub sort_order: i32,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ModuleSummaryRow {
    pub id: Uuid,
    pub course_id: Uuid,
    pub title: String,
    pub sort_order: i32,
}

#[derive(Debug, Clone)]
pub struct ModuleWithLessonsRow {
    pub module: ModuleSummaryRow,
    pub lessons: Vec<LessonSummaryRow>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct CourseMemberRow {
    pub user_id: Uuid,
    pub display_name: Option<String>,
    pub email: String,
    pub role: String,
    pub status: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct CourseSessionRow {
    pub session_id: Uuid,
    pub course_id: Uuid,
    pub course_title: String,
    pub course_slug: String,
    pub title: String,
    pub starts_at: chrono::DateTime<chrono::Utc>,
    pub duration_minutes: i32,
    pub status: String,
    pub diverged: bool,
}

pub async fn outline(pool: &PgPool, course_id: Uuid) -> sqlx::Result<Vec<ModuleWithLessonsRow>> {
    let modules: Vec<ModuleSummaryRow> = sqlx::query_as(
        "SELECT id, course_id, title, sort_order
           FROM modules
          WHERE course_id = $1
          ORDER BY sort_order, title",
    )
    .bind(course_id)
    .fetch_all(pool)
    .await?;

    let lessons: Vec<LessonSummaryRow> = sqlx::query_as(
        "SELECT id, course_id, module_id, type::text AS type, title, body_md,
                video_asset_id, live_session_id, sort_order
           FROM lessons
          WHERE course_id = $1
          ORDER BY module_id, sort_order, title",
    )
    .bind(course_id)
    .fetch_all(pool)
    .await?;

    Ok(modules
        .into_iter()
        .map(|module| {
            let module_lessons = lessons
                .iter()
                .filter(|lesson| lesson.module_id == module.id)
                .cloned()
                .collect();
            ModuleWithLessonsRow {
                module,
                lessons: module_lessons,
            }
        })
        .collect())
}

pub async fn members(pool: &PgPool, course_id: Uuid) -> sqlx::Result<Vec<CourseMemberRow>> {
    sqlx::query_as(
        "SELECT u.id AS user_id,
                u.display_name,
                u.email::text AS email,
                cm.role,
                cm.status
           FROM course_memberships cm
           JOIN users u ON u.id = cm.user_id
          WHERE cm.course_id = $1
          ORDER BY cm.role, u.email::text",
    )
    .bind(course_id)
    .fetch_all(pool)
    .await
}

pub async fn sessions(pool: &PgPool, course_id: Uuid) -> sqlx::Result<Vec<CourseSessionRow>> {
    sqlx::query_as(
        "SELECT ls.id AS session_id,
                ls.course_id,
                c.title AS course_title,
                c.slug AS course_slug,
                ls.title,
                ls.starts_at,
                ls.duration_minutes,
                ls.status,
                ls.diverged
           FROM live_sessions ls
           JOIN courses c ON c.id = ls.course_id
          WHERE ls.course_id = $1
          ORDER BY ls.starts_at, ls.occurrence_index",
    )
    .bind(course_id)
    .fetch_all(pool)
    .await
}
```

- [ ] **Step 2: Export the helper module**

Modify `crates/backend/src/db/mod.rs` and add:

```rust
pub mod course_detail;
```

Place it after `pub mod courses;`.

- [ ] **Step 3: Add DTOs and routes to courses handler**

Modify `crates/backend/src/handlers/courses.rs`.

Add DTOs near `CourseDto`:

```rust
#[derive(Serialize)]
pub struct LessonSummaryDto {
    pub id: Uuid,
    pub course_id: Uuid,
    pub module_id: Uuid,
    pub r#type: String,
    pub title: String,
    pub body_md: Option<String>,
    pub video_asset_id: Option<Uuid>,
    pub live_session_id: Option<Uuid>,
    pub sort_order: i32,
}

#[derive(Serialize)]
pub struct ModuleWithLessonsDto {
    pub id: Uuid,
    pub course_id: Uuid,
    pub title: String,
    pub sort_order: i32,
    pub lessons: Vec<LessonSummaryDto>,
}

#[derive(Serialize)]
pub struct CourseMemberDto {
    pub user_id: Uuid,
    pub display_name: Option<String>,
    pub email: String,
    pub role: String,
    pub status: String,
}

#[derive(Serialize)]
pub struct CourseSessionDto {
    pub session_id: Uuid,
    pub course_id: Uuid,
    pub course_title: String,
    pub course_slug: String,
    pub title: String,
    pub starts_at: chrono::DateTime<chrono::Utc>,
    pub duration_minutes: i32,
    pub status: String,
    pub diverged: bool,
}
```

Update `routes()`:

```rust
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/courses", routing::post(create).get(list))
        .route(
            "/v1/courses/:id/modules-with-lessons",
            routing::get(get_outline),
        )
        .route("/v1/courses/:id/members", routing::get(get_members))
        .route("/v1/courses/:id/sessions", routing::get(get_sessions))
        .route(
            "/v1/courses/:id",
            routing::get(get_one).patch(patch).delete(delete_one),
        )
}
```

Update `router_for_tests()` the same way, using `_t` handlers:

```rust
#[doc(hidden)]
pub fn router_for_tests(pool: PgPool) -> Router {
    Router::new()
        .route("/v1/courses", routing::post(create_t).get(list_t))
        .route(
            "/v1/courses/:id/modules-with-lessons",
            routing::get(get_outline_t),
        )
        .route("/v1/courses/:id/members", routing::get(get_members_t))
        .route("/v1/courses/:id/sessions", routing::get(get_sessions_t))
        .route(
            "/v1/courses/:id",
            routing::get(get_one_t).patch(patch_t).delete(delete_one_t),
        )
        .with_state(TestState { pool })
}
```

Add production handlers after `get_one`:

```rust
async fn get_outline(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<ModuleWithLessonsDto>>, ApiError> {
    get_outline_inner(&state.pool, &ctx, id).await
}

async fn get_members(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<CourseMemberDto>>, ApiError> {
    get_members_inner(&state.pool, &ctx, id).await
}

async fn get_sessions(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<CourseSessionDto>>, ApiError> {
    get_sessions_inner(&state.pool, &ctx, id).await
}
```

Add test handlers after `get_one_t`:

```rust
async fn get_outline_t(
    State(state): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<ModuleWithLessonsDto>>, ApiError> {
    get_outline_inner(&state.pool, &ctx, id).await
}

async fn get_members_t(
    State(state): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<CourseMemberDto>>, ApiError> {
    get_members_inner(&state.pool, &ctx, id).await
}

async fn get_sessions_t(
    State(state): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<CourseSessionDto>>, ApiError> {
    get_sessions_inner(&state.pool, &ctx, id).await
}
```

Add inner functions before `patch_inner`:

```rust
async fn require_can_read_course(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
) -> Result<(), ApiError> {
    let allowed = db::courses::caller_can_read_course(pool, course_id, ctx.user_id, is_org_admin(ctx))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if allowed {
        Ok(())
    } else {
        Err(ApiError::CourseNotFound)
    }
}

async fn get_outline_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
) -> Result<Json<Vec<ModuleWithLessonsDto>>, ApiError> {
    require_can_read_course(pool, ctx, course_id).await?;
    let rows = db::course_detail::outline(pool, course_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(
        rows.into_iter()
            .map(|row| ModuleWithLessonsDto {
                id: row.module.id,
                course_id: row.module.course_id,
                title: row.module.title,
                sort_order: row.module.sort_order,
                lessons: row
                    .lessons
                    .into_iter()
                    .map(|lesson| LessonSummaryDto {
                        id: lesson.id,
                        course_id: lesson.course_id,
                        module_id: lesson.module_id,
                        r#type: lesson.r#type,
                        title: lesson.title,
                        body_md: lesson.body_md,
                        video_asset_id: lesson.video_asset_id,
                        live_session_id: lesson.live_session_id,
                        sort_order: lesson.sort_order,
                    })
                    .collect(),
            })
            .collect(),
    ))
}

async fn get_members_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
) -> Result<Json<Vec<CourseMemberDto>>, ApiError> {
    let allowed = db::courses::caller_can_admin_course(pool, course_id, ctx.user_id, is_org_admin(ctx))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !allowed {
        return Err(ApiError::Forbidden);
    }
    let rows = db::course_detail::members(pool, course_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(
        rows.into_iter()
            .map(|row| CourseMemberDto {
                user_id: row.user_id,
                display_name: row.display_name,
                email: row.email,
                role: row.role,
                status: row.status,
            })
            .collect(),
    ))
}

async fn get_sessions_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
) -> Result<Json<Vec<CourseSessionDto>>, ApiError> {
    require_can_read_course(pool, ctx, course_id).await?;
    let rows = db::course_detail::sessions(pool, course_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(
        rows.into_iter()
            .map(|row| CourseSessionDto {
                session_id: row.session_id,
                course_id: row.course_id,
                course_title: row.course_title,
                course_slug: row.course_slug,
                title: row.title,
                starts_at: row.starts_at,
                duration_minutes: row.duration_minutes,
                status: row.status,
                diverged: row.diverged,
            })
            .collect(),
    ))
}
```

- [ ] **Step 4: Run focused backend tests**

Run:

```powershell
& cmd.exe /d /s /c '"C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat" >nul && set DATABASE_URL=postgres://aulalite:changeme@localhost:55432/aulalite && cargo test -p backend --test course_detail_tabs -- --nocapture'
```

Expected: 5 tests pass.

- [ ] **Step 5: Commit backend read endpoints**

Run:

```powershell
git add crates/backend/src/db/course_detail.rs crates/backend/src/db/mod.rs crates/backend/src/handlers/courses.rs crates/backend/tests/course_detail_tabs.rs
git commit -m "feat(backend): add course detail read endpoints"
```

---

### Task 3: Frontend API Surface For CourseDetail Tabs

**Files:**
- Modify: `crates/features-courses/src/api.rs`

- [ ] **Step 1: Add DTOs**

Append after `CourseDto` in `crates/features-courses/src/api.rs`:

```rust
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct LessonSummaryDto {
    pub id: String,
    pub course_id: String,
    pub module_id: String,
    #[serde(rename = "type")]
    pub r#type: String,
    pub title: String,
    pub body_md: Option<String>,
    pub video_asset_id: Option<String>,
    pub live_session_id: Option<String>,
    pub sort_order: i32,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct ModuleWithLessonsDto {
    pub id: String,
    pub course_id: String,
    pub title: String,
    pub sort_order: i32,
    pub lessons: Vec<LessonSummaryDto>,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct CourseMemberDto {
    pub user_id: String,
    pub display_name: Option<String>,
    pub email: String,
    pub role: String,
    pub status: String,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct CourseSessionDto {
    pub session_id: String,
    pub course_id: String,
    pub course_title: String,
    pub course_slug: String,
    pub title: String,
    pub starts_at: String,
    pub duration_minutes: i32,
    pub status: String,
    pub diverged: bool,
}
```

- [ ] **Step 2: Add request bodies**

Append after the DTOs:

```rust
#[derive(serde::Serialize)]
pub struct PatchCourseBody<'a> {
    pub title: Option<&'a str>,
    pub description: Option<&'a str>,
    pub status: Option<&'a str>,
    pub cover_asset_id: Option<Option<&'a str>>,
}

#[derive(serde::Serialize)]
pub struct CreateModuleBody<'a> {
    pub title: &'a str,
}

#[derive(serde::Serialize)]
pub struct ReorderModulesBody {
    pub module_ids: Vec<String>,
}

#[derive(serde::Serialize)]
pub struct CreateLessonBody<'a> {
    #[serde(rename = "type")]
    pub r#type: &'a str,
    pub title: &'a str,
    pub body_md: Option<&'a str>,
    pub live_session_id: Option<&'a str>,
}

#[derive(serde::Serialize)]
pub struct ReorderLessonsBody {
    pub lesson_ids: Vec<String>,
}

#[derive(serde::Serialize)]
pub struct PatchAssignmentBody {
    pub attachment_asset_ids: Option<Vec<String>>,
}

#[derive(serde::Serialize)]
pub struct CreateInvitationBody<'a> {
    pub email: &'a str,
    pub role: &'a str,
}

#[derive(serde::Serialize)]
pub struct CreateCodeBody {
    pub max_uses: Option<i32>,
    pub expires_at: Option<String>,
}
```

- [ ] **Step 3: Add API functions**

Append after `create_course`:

```rust
pub async fn patch_course(
    ctx: &ApiContext,
    course_id: &str,
    body: &PatchCourseBody<'_>,
) -> Result<CourseDto, ApiError> {
    fetch_json(ctx, "PATCH", &format!("/v1/courses/{course_id}"), Some(body)).await
}

pub async fn get_course_outline(
    ctx: &ApiContext,
    course_id: &str,
) -> Result<Vec<ModuleWithLessonsDto>, ApiError> {
    fetch_json(
        ctx,
        "GET",
        &format!("/v1/courses/{course_id}/modules-with-lessons"),
        None::<&()>,
    )
    .await
}

pub async fn list_course_members(
    ctx: &ApiContext,
    course_id: &str,
) -> Result<Vec<CourseMemberDto>, ApiError> {
    fetch_json(ctx, "GET", &format!("/v1/courses/{course_id}/members"), None::<&()>).await
}

pub async fn list_course_sessions(
    ctx: &ApiContext,
    course_id: &str,
) -> Result<Vec<CourseSessionDto>, ApiError> {
    fetch_json(ctx, "GET", &format!("/v1/courses/{course_id}/sessions"), None::<&()>).await
}

pub async fn create_module(
    ctx: &ApiContext,
    course_id: &str,
    body: &CreateModuleBody<'_>,
) -> Result<ModuleWithLessonsDto, ApiError> {
    fetch_json(ctx, "POST", &format!("/v1/courses/{course_id}/modules"), Some(body)).await
}

pub async fn reorder_modules(
    ctx: &ApiContext,
    course_id: &str,
    body: &ReorderModulesBody,
) -> Result<serde_json::Value, ApiError> {
    fetch_json(
        ctx,
        "POST",
        &format!("/v1/courses/{course_id}/modules/reorder"),
        Some(body),
    )
    .await
}

pub async fn create_lesson(
    ctx: &ApiContext,
    course_id: &str,
    module_id: &str,
    body: &CreateLessonBody<'_>,
) -> Result<LessonSummaryDto, ApiError> {
    fetch_json(
        ctx,
        "POST",
        &format!("/v1/courses/{course_id}/modules/{module_id}/lessons"),
        Some(body),
    )
    .await
}

pub async fn reorder_lessons(
    ctx: &ApiContext,
    course_id: &str,
    module_id: &str,
    body: &ReorderLessonsBody,
) -> Result<serde_json::Value, ApiError> {
    fetch_json(
        ctx,
        "POST",
        &format!("/v1/courses/{course_id}/modules/{module_id}/lessons/reorder"),
        Some(body),
    )
    .await
}

pub async fn patch_assignment(
    ctx: &ApiContext,
    assignment_id: &str,
    body: &PatchAssignmentBody,
) -> Result<AssignmentDto, ApiError> {
    fetch_json(ctx, "PATCH", &format!("/v1/assignments/{assignment_id}"), Some(body)).await
}
```

Append after invitation/redeem functions:

```rust
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct CourseInvitationDto {
    pub id: String,
    pub email: String,
    pub role: String,
    pub status: String,
    pub expires_at: String,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct CodeSummaryDto {
    pub id: String,
    pub last4: String,
    pub max_uses: Option<i32>,
    pub uses: i32,
    pub expires_at: Option<String>,
}

pub async fn list_course_invitations(
    ctx: &ApiContext,
    course_id: &str,
) -> Result<Vec<CourseInvitationDto>, ApiError> {
    fetch_json(ctx, "GET", &format!("/v1/courses/{course_id}/invitations"), None::<&()>).await
}

pub async fn create_course_invitation(
    ctx: &ApiContext,
    course_id: &str,
    body: &CreateInvitationBody<'_>,
) -> Result<serde_json::Value, ApiError> {
    fetch_json(ctx, "POST", &format!("/v1/courses/{course_id}/invitations"), Some(body)).await
}

pub async fn revoke_course_invitation(
    ctx: &ApiContext,
    course_id: &str,
    invitation_id: &str,
) -> Result<serde_json::Value, ApiError> {
    fetch_json(
        ctx,
        "DELETE",
        &format!("/v1/courses/{course_id}/invitations/{invitation_id}"),
        None::<&()>,
    )
    .await
}

pub async fn list_enrollment_codes(
    ctx: &ApiContext,
    course_id: &str,
) -> Result<Vec<CodeSummaryDto>, ApiError> {
    fetch_json(ctx, "GET", &format!("/v1/courses/{course_id}/codes"), None::<&()>).await
}

pub async fn create_enrollment_code(
    ctx: &ApiContext,
    course_id: &str,
    body: &CreateCodeBody,
) -> Result<serde_json::Value, ApiError> {
    fetch_json(ctx, "POST", &format!("/v1/courses/{course_id}/codes"), Some(body)).await
}

pub async fn revoke_enrollment_code(
    ctx: &ApiContext,
    course_id: &str,
    code_id: &str,
) -> Result<serde_json::Value, ApiError> {
    fetch_json(
        ctx,
        "DELETE",
        &format!("/v1/courses/{course_id}/codes/{code_id}"),
        None::<&()>,
    )
    .await
}
```

- [ ] **Step 4: Run frontend API compile check**

Run:

```powershell
cargo check -p features-courses --target wasm32-unknown-unknown
```

Expected: compile succeeds.

- [ ] **Step 5: Commit API surface**

Run:

```powershell
git add crates/features-courses/src/api.rs
git commit -m "feat(api): expose course detail tab endpoints"
```

---

### Task 4: Wire CourseDetail Tabs In shell-web

**Files:**
- Modify: `crates/shell-web/src/routes/course_detail.rs`
- Modify: `crates/shell-web/tests/shell_routes_smoke.rs`

- [ ] **Step 1: Add SSR smoke checks for tabs**

Append to `crates/shell-web/tests/shell_routes_smoke.rs`:

```rust
#[test]
fn course_detail_outline_route_renders() {
    let mut dom = dom_for_path("/courses/math", true);
    let _ = dom.rebuild_in_place();
    let html = render(&dom);
    assert!(html.len() > 50, "html too short: {html}");
}

#[test]
fn course_people_route_renders() {
    let mut dom = dom_for_path("/courses/math/people", true);
    let _ = dom.rebuild_in_place();
    let html = render(&dom);
    assert!(html.len() > 50, "html too short: {html}");
}

#[test]
fn course_schedule_route_renders() {
    let mut dom = dom_for_path("/courses/math/schedule", true);
    let _ = dom.rebuild_in_place();
    let html = render(&dom);
    assert!(html.len() > 50, "html too short: {html}");
}
```

- [ ] **Step 2: Run the route smokes**

Run:

```powershell
cargo test -p shell-web --test shell_routes_smoke
```

Expected: tests pass or existing route errors expose missing context. Fix only missing context in the test harness before continuing.

- [ ] **Step 3: Replace placeholder tab body with real tab dispatch**

In `crates/shell-web/src/routes/course_detail.rs`, add imports:

```rust
use features_courses::course_builder::{CourseBuilder, LessonNode, ModuleNode};
use features_courses::course_people::{ActiveCode, CoursePeople, Member, PendingInvite};
use features_courses::schedule_view::{ScheduleEntry, ScheduleView};
use features_courses::series_scheduler::{SeriesDraft, SeriesScheduler};
```

Add helper functions above `render_with_tab`:

```rust
fn map_modules(rows: Vec<api::ModuleWithLessonsDto>) -> Vec<ModuleNode> {
    rows.into_iter()
        .map(|module| ModuleNode {
            id: module.id,
            title: module.title,
            lessons: module
                .lessons
                .into_iter()
                .map(|lesson| LessonNode {
                    id: lesson.id,
                    title: lesson.title,
                    r#type: lesson.r#type,
                })
                .collect(),
        })
        .collect()
}

fn map_members(rows: Vec<api::CourseMemberDto>) -> Vec<Member> {
    rows.into_iter()
        .map(|member| Member {
            user_id: member.user_id,
            display_name: member.display_name.unwrap_or_else(|| member.email.clone()),
            email: member.email,
            role: member.role,
            status: member.status,
        })
        .collect()
}

fn map_sessions(rows: Vec<api::CourseSessionDto>) -> Vec<ScheduleEntry> {
    rows.into_iter()
        .map(|session| ScheduleEntry {
            session_id: session.session_id,
            course_title: session.course_title,
            course_slug: session.course_slug,
            title: session.title,
            starts_at_display: session.starts_at,
            duration_minutes: session.duration_minutes,
            status: session.status,
            diverged: session.diverged,
            can_edit: true,
        })
        .collect()
}
```

Add tab rendering helpers:

```rust
fn outline_tab(api_ctx: ApiContext, course_id: String, can_admin: bool) -> Element {
    let mut error = use_signal(|| None::<String>);
    let mut resource = use_resource({
        let api_ctx = api_ctx.clone();
        let course_id = course_id.clone();
        move || {
            let api_ctx = api_ctx.clone();
            let course_id = course_id.clone();
            async move { api::get_course_outline(&api_ctx, &course_id).await }
        }
    });

    match resource.read_unchecked().as_ref() {
        Some(Ok(rows)) => {
            let modules = map_modules(rows.clone());
            let api_for_module = api_ctx.clone();
            let course_for_module = course_id.clone();
            rsx! {
                div { class: "course-outline-tab",
                    if let Some(err) = error.read().as_ref() {
                        p { class: "error", "{err}" }
                    }
                    CourseBuilder {
                        modules,
                        on_add_module: move |_| {
                            let api = api_for_module.clone();
                            let course_id = course_for_module.clone();
                            spawn(async move {
                                let body = api::CreateModuleBody { title: "New module" };
                                if let Err(e) = api::create_module(&api, &course_id, &body).await {
                                    error.set(Some(format!("{e}")));
                                } else {
                                    resource.restart();
                                }
                            });
                        },
                        on_add_lesson: move |module_id: String| {
                            let api = api_ctx.clone();
                            let course_id = course_id.clone();
                            spawn(async move {
                                let body = api::CreateLessonBody {
                                    r#type: "rich_text",
                                    title: "New lesson",
                                    body_md: Some("Draft lesson content"),
                                    live_session_id: None,
                                };
                                let _ = api::create_lesson(&api, &course_id, &module_id, &body).await;
                            });
                        },
                        on_lesson_clicked: move |_lesson_id: String| {},
                        on_modules_reordered: move |module_ids: Vec<String>| {
                            let api = api_ctx.clone();
                            let course_id = course_id.clone();
                            spawn(async move {
                                let body = api::ReorderModulesBody { module_ids };
                                let _ = api::reorder_modules(&api, &course_id, &body).await;
                            });
                        },
                        on_lessons_reordered: move |(module_id, lesson_ids): (String, Vec<String>)| {
                            let api = api_ctx.clone();
                            let course_id = course_id.clone();
                            spawn(async move {
                                let body = api::ReorderLessonsBody { lesson_ids };
                                let _ = api::reorder_lessons(&api, &course_id, &module_id, &body).await;
                            });
                        },
                    }
                    if !can_admin {
                        p { class: "muted", "Read-only course outline." }
                    }
                }
            }
        }
        Some(Err(e)) => rsx! { p { class: "error", "Could not load outline: {e}" } },
        None => rsx! { p { "Loading outline" } },
    }
}

fn people_tab(api_ctx: ApiContext, course_id: String) -> Element {
    let members = use_resource({
        let api_ctx = api_ctx.clone();
        let course_id = course_id.clone();
        move || {
            let api_ctx = api_ctx.clone();
            let course_id = course_id.clone();
            async move { api::list_course_members(&api_ctx, &course_id).await }
        }
    });
    let invites = use_resource({
        let api_ctx = api_ctx.clone();
        let course_id = course_id.clone();
        move || {
            let api_ctx = api_ctx.clone();
            let course_id = course_id.clone();
            async move { api::list_course_invitations(&api_ctx, &course_id).await }
        }
    });
    let codes = use_resource({
        let api_ctx = api_ctx.clone();
        let course_id = course_id.clone();
        move || {
            let api_ctx = api_ctx.clone();
            let course_id = course_id.clone();
            async move { api::list_enrollment_codes(&api_ctx, &course_id).await }
        }
    });

    match (
        members.read_unchecked().as_ref(),
        invites.read_unchecked().as_ref(),
        codes.read_unchecked().as_ref(),
    ) {
        (Some(Ok(member_rows)), Some(Ok(invite_rows)), Some(Ok(code_rows))) => {
            let pending_invites = invite_rows
                .clone()
                .into_iter()
                .map(|invite| PendingInvite {
                    id: invite.id,
                    email: invite.email,
                    role: invite.role,
                    expires_at: invite.expires_at,
                })
                .collect();
            let active_codes = code_rows
                .clone()
                .into_iter()
                .map(|code| ActiveCode {
                    id: code.id,
                    last4: code.last4,
                    uses: code.uses,
                    max_uses: code.max_uses,
                })
                .collect();
            rsx! {
                CoursePeople {
                    members: map_members(member_rows.clone()),
                    pending_invites,
                    active_codes,
                    on_invite_clicked: move |_| {},
                    on_code_clicked: move |_| {},
                    on_revoke_invite: move |id: String| {
                        let api = api_ctx.clone();
                        let course_id = course_id.clone();
                        spawn(async move {
                            let _ = api::revoke_course_invitation(&api, &course_id, &id).await;
                        });
                    },
                    on_revoke_code: move |id: String| {
                        let api = api_ctx.clone();
                        let course_id = course_id.clone();
                        spawn(async move {
                            let _ = api::revoke_enrollment_code(&api, &course_id, &id).await;
                        });
                    },
                }
            }
        }
        (Some(Err(e)), _, _) | (_, Some(Err(e)), _) | (_, _, Some(Err(e))) => {
            rsx! { p { class: "error", "Could not load people data: {e}" } }
        }
        _ => rsx! { p { "Loading people" } },
    }
}

fn schedule_tab(api_ctx: ApiContext, course_id: String) -> Element {
    let sessions = use_resource({
        let api_ctx = api_ctx.clone();
        let course_id = course_id.clone();
        move || {
            let api_ctx = api_ctx.clone();
            let course_id = course_id.clone();
            async move { api::list_course_sessions(&api_ctx, &course_id).await }
        }
    });
    match sessions.read_unchecked().as_ref() {
        Some(Ok(rows)) => rsx! {
            div { class: "course-schedule-tab",
                ScheduleView {
                    entries: map_sessions(rows.clone()),
                    on_cancel: move |_session_id: String| {},
                    on_reschedule: move |_session_id: String| {},
                }
                SeriesScheduler {
                    initial: SeriesDraft {
                        title: "New live session".to_string(),
                        starts_at_iso: String::new(),
                        duration_minutes: 60,
                        frequency: "none".to_string(),
                        byweekday: vec![],
                        end_kind: "count".to_string(),
                        occurrence_count: Some(1),
                        end_until_iso: None,
                        recording_enabled: Some(true),
                    },
                    preview: vec![],
                    on_change: move |_draft: SeriesDraft| {},
                    on_submit: move |_draft: SeriesDraft| {},
                    submitting: false,
                    error: None,
                }
            }
        },
        Some(Err(e)) => rsx! { p { class: "error", "Could not load schedule: {e}" } },
        None => rsx! { p { "Loading schedule" } },
    }
}
```

Inside the existing `Some(Ok(c))` branch, replace:

```rust
div { "Tab content for {active_tab} — wired components land in subsequent route files." }
```

with:

```rust
{
    let tab_body = match active_tab {
        "outline" => outline_tab(api.clone(), c.id.clone(), user.is_teacher()),
        "people" => people_tab(api.clone(), c.id.clone()),
        "edit" => rsx! { p { "Edit form is wired in Task 5." } },
        "schedule" => schedule_tab(api.clone(), c.id.clone()),
        _ => rsx! { p { "Unsupported tab." } },
    };
    tab_body
}
```

- [ ] **Step 4: Run shell route smokes**

Run:

```powershell
cargo test -p shell-web --test shell_routes_smoke
```

Expected: all shell route smokes pass. Warnings are acceptable.

- [ ] **Step 5: Commit CourseDetail tab shell wiring**

Run:

```powershell
git add crates/shell-web/src/routes/course_detail.rs crates/shell-web/tests/shell_routes_smoke.rs
git commit -m "feat(shell-web): wire course detail tabs to real data"
```

---

### Task 5: Course Edit Tab And Assignment Reference Attachments

**Files:**
- Modify: `crates/shell-web/src/routes/course_detail.rs`
- Modify: `crates/features-courses/src/assignment_editor.rs`

- [ ] **Step 1: Add CourseDetail edit tab body**

In `crates/shell-web/src/routes/course_detail.rs`, add:

```rust
fn edit_tab(api_ctx: ApiContext, course: api::CourseDto) -> Element {
    let mut title = use_signal(|| course.title.clone());
    let mut description = use_signal(|| course.description.clone().unwrap_or_default());
    let mut status = use_signal(|| course.status.clone());
    let mut saving = use_signal(|| false);
    let mut error = use_signal(|| None::<String>);
    let course_id = course.id.clone();

    rsx! {
        form {
            class: "course-edit-tab",
            onsubmit: move |evt| evt.prevent_default(),
            label { "Title" }
            input {
                value: "{title}",
                oninput: move |evt| title.set(evt.value()),
            }
            label { "Description" }
            textarea {
                rows: 5,
                value: "{description}",
                oninput: move |evt| description.set(evt.value()),
            }
            label { "Status" }
            select {
                value: "{status}",
                onchange: move |evt| status.set(evt.value()),
                option { value: "draft", "Draft" }
                option { value: "published", "Published" }
                option { value: "archived", "Archived" }
            }
            if let Some(err) = error.read().as_ref() {
                p { class: "error", "{err}" }
            }
            button {
                disabled: *saving.read(),
                onclick: move |_| {
                    let api = api_ctx.clone();
                    let course_id = course_id.clone();
                    spawn(async move {
                        saving.set(true);
                        let title_v = title.read().clone();
                        let description_v = description.read().clone();
                        let status_v = status.read().clone();
                        let body = api::PatchCourseBody {
                            title: Some(&title_v),
                            description: Some(&description_v),
                            status: Some(&status_v),
                            cover_asset_id: None,
                        };
                        match api::patch_course(&api, &course_id, &body).await {
                            Ok(_) => error.set(None),
                            Err(e) => error.set(Some(format!("{e}"))),
                        }
                        saving.set(false);
                    });
                },
                if *saving.read() { "Saving" } else { "Save course" }
            }
        }
    }
}
```

Replace the Task 4 edit placeholder:

```rust
"edit" => rsx! { p { "Edit form is wired in Task 5." } },
```

with:

```rust
"edit" => edit_tab(api.clone(), c.clone()),
```

- [ ] **Step 2: Add attachment state to AssignmentEditor**

In `crates/features-courses/src/assignment_editor.rs`, after `release_mode` signal, add:

```rust
let mut attachment_asset_ids = use_signal(|| {
    initial
        .as_ref()
        .map(|a| a.attachment_asset_ids.clone())
        .unwrap_or_default()
});
```

- [ ] **Step 3: Add existing-assignment file picker**

Before the error block in the `rsx!` form, add:

```rust
fieldset {
    legend { "Reference attachments" }
    if let Some(existing) = props.initial.as_ref() {
        crate::file_picker::FilePicker {
            purpose: "attachment".to_string(),
            linked_entity_type: "assignment_attachment".to_string(),
            linked_entity_id: existing.id.clone(),
            allowed_types: crate::file_picker::validation::ATTACHMENT_TYPES
                .iter()
                .map(|s| s.to_string())
                .collect(),
            max_size_bytes: crate::file_picker::validation::ATTACHMENT_MAX,
            button_label: "Attach reference file".to_string(),
            on_uploaded: move |asset_id: String| {
                let mut ids = attachment_asset_ids.read().clone();
                if !ids.contains(&asset_id) {
                    ids.push(asset_id.clone());
                }
                attachment_asset_ids.set(ids.clone());
                let api = props.api.clone();
                let assignment_id = existing.id.clone();
                spawn(async move {
                    let body = api::PatchAssignmentBody {
                        attachment_asset_ids: Some(ids),
                    };
                    let _ = api::patch_assignment(&api, &assignment_id, &body).await;
                });
            },
        }
        if !attachment_asset_ids.read().is_empty() {
            ul { class: "assignment-editor__attachments",
                for id in attachment_asset_ids.read().iter() {
                    li { "{id}" }
                }
            }
        }
    } else {
        p { class: "muted", "Save the assignment draft before attaching reference files." }
    }
}
```

- [ ] **Step 4: Run web build**

Run:

```powershell
dx build --platform web --package shell-web
```

Expected: build completes successfully. Existing warnings are acceptable.

- [ ] **Step 5: Commit edit and attachment wiring**

Run:

```powershell
git add crates/shell-web/src/routes/course_detail.rs crates/features-courses/src/assignment_editor.rs
git commit -m "feat(web): complete course edit and assignment attachments"
```

---

### Task 6: Local Login Profiles For Teacher And Student

**Files:**
- Modify: `crates/backend/src/auth/local_login.rs`
- Modify: `crates/backend/src/handlers/dev_login.rs`
- Modify: `crates/features-courses/src/api.rs`
- Modify: `crates/shell-web/src/routes/login.rs`
- Modify: `.env.example`
- Modify: `docker-compose.yml`
- Test: `crates/backend/tests/local_login_bypass.rs`

- [ ] **Step 1: Update backend tests first**

Replace the helper in `crates/backend/tests/local_login_bypass.rs` with:

```rust
fn local_config(app_env: &str, enabled: bool) -> LocalLoginConfig {
    LocalLoginConfig::from_profiles(
        app_env.to_string(),
        enabled,
        vec![
            backend::auth::local_login::LocalLoginProfile {
                name: "teacher".to_string(),
                token: "local-teacher-token".to_string(),
                email: "local.teacher@example.test".to_string(),
                display_name: "Local Teacher".to_string(),
                firebase_uid: "local-login-local-teacher".to_string(),
            },
            backend::auth::local_login::LocalLoginProfile {
                name: "student".to_string(),
                token: "local-student-token".to_string(),
                email: "local.student@example.test".to_string(),
                display_name: "Local Student".to_string(),
                firebase_uid: "local-login-local-student".to_string(),
            },
        ],
    )
}
```

Update token assertions:

```rust
assert_eq!(json["id_token"], "local-teacher-token");
```

Update middleware auth header:

```rust
.header("authorization", "Bearer local-teacher-token")
```

Append a new test:

```rust
#[tokio::test]
async fn local_dev_login_can_return_student_profile_token() {
    let pool = fixtures::pool().await;
    let app = dev_login_routes(DevLoginState {
        pool,
        config: local_config("local", true),
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/dev/login")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"profile":"student"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = json_response(response).await;
    assert_eq!(json["id_token"], "local-student-token");
}
```

- [ ] **Step 2: Run tests and verify failure**

Run:

```powershell
& cmd.exe /d /s /c '"C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat" >nul && set DATABASE_URL=postgres://aulalite:changeme@localhost:55432/aulalite && cargo test -p backend --test local_login_bypass -- --nocapture'
```

Expected: compile failure for missing `LocalLoginProfile` or request body handling.

- [ ] **Step 3: Implement profile config**

Replace `crates/backend/src/auth/local_login.rs` with:

```rust
use chrono::Utc;

use crate::auth::verify::FirebaseClaims;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalLoginProfile {
    pub name: String,
    pub token: String,
    pub email: String,
    pub display_name: String,
    pub firebase_uid: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalLoginConfig {
    pub app_env: String,
    pub enabled: bool,
    pub profiles: Vec<LocalLoginProfile>,
}

impl LocalLoginConfig {
    pub fn from_env() -> Self {
        let teacher_email = std::env::var("LOCAL_LOGIN_EMAIL")
            .unwrap_or_else(|_| "local.teacher@example.test".into());
        let teacher = LocalLoginProfile {
            name: "teacher".into(),
            token: std::env::var("LOCAL_LOGIN_TEACHER_TOKEN")
                .or_else(|_| std::env::var("LOCAL_LOGIN_TOKEN"))
                .unwrap_or_default(),
            email: teacher_email.clone(),
            display_name: std::env::var("LOCAL_LOGIN_DISPLAY_NAME")
                .unwrap_or_else(|_| "Local Teacher".into()),
            firebase_uid: std::env::var("LOCAL_LOGIN_FIREBASE_UID")
                .unwrap_or_else(|_| firebase_uid_for_email(&teacher_email)),
        };
        let student_email = std::env::var("LOCAL_LOGIN_STUDENT_EMAIL")
            .unwrap_or_else(|_| "local.student@example.test".into());
        let student = LocalLoginProfile {
            name: "student".into(),
            token: std::env::var("LOCAL_LOGIN_STUDENT_TOKEN").unwrap_or_default(),
            email: student_email.clone(),
            display_name: std::env::var("LOCAL_LOGIN_STUDENT_DISPLAY_NAME")
                .unwrap_or_else(|_| "Local Student".into()),
            firebase_uid: std::env::var("LOCAL_LOGIN_STUDENT_FIREBASE_UID")
                .unwrap_or_else(|_| firebase_uid_for_email(&student_email)),
        };
        Self::from_profiles(
            std::env::var("APP_ENV").unwrap_or_else(|_| "production".into()),
            env_bool("LOCAL_LOGIN_BYPASS_ENABLED"),
            vec![teacher, student],
        )
    }

    pub fn from_profiles(
        app_env: String,
        enabled: bool,
        profiles: Vec<LocalLoginProfile>,
    ) -> Self {
        Self {
            app_env,
            enabled,
            profiles,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
            && !self.is_production()
            && self.profiles.iter().any(LocalLoginProfile::is_complete)
    }

    pub fn public_profiles(&self) -> Vec<LocalLoginProfile> {
        if !self.is_enabled() {
            return Vec::new();
        }
        self.profiles
            .iter()
            .filter(|profile| profile.is_complete())
            .cloned()
            .collect()
    }

    pub fn default_profile(&self) -> Option<&LocalLoginProfile> {
        self.profile("teacher").or_else(|| {
            self.profiles
                .iter()
                .find(|profile| profile.is_complete())
        })
    }

    pub fn profile(&self, name: &str) -> Option<&LocalLoginProfile> {
        self.profiles
            .iter()
            .find(|profile| profile.name == name && profile.is_complete())
    }

    pub fn profile_for_token(&self, token: &str) -> Option<&LocalLoginProfile> {
        if !self.is_enabled() {
            return None;
        }
        self.profiles
            .iter()
            .find(|profile| profile.is_complete() && profile.token == token)
    }

    pub fn is_token(&self, token: &str) -> bool {
        self.profile_for_token(token).is_some()
    }

    pub fn claims_for_token(&self, token: &str) -> Option<FirebaseClaims> {
        self.profile_for_token(token).map(LocalLoginProfile::claims)
    }

    fn is_production(&self) -> bool {
        matches!(
            self.app_env.trim().to_ascii_lowercase().as_str(),
            "production" | "prod"
        )
    }
}

impl LocalLoginProfile {
    pub fn is_complete(&self) -> bool {
        !self.token.trim().is_empty()
            && !self.email.trim().is_empty()
            && !self.firebase_uid.trim().is_empty()
            && !self.name.trim().is_empty()
    }

    pub fn claims(&self) -> FirebaseClaims {
        let now = Utc::now().timestamp();
        FirebaseClaims {
            sub: self.firebase_uid.clone(),
            email: Some(self.email.clone()),
            email_verified: Some(true),
            name: Some(self.display_name.clone()),
            picture: None,
            aud: "local-login-bypass".into(),
            iss: "local-login-bypass".into(),
            exp: now + 24 * 60 * 60,
            iat: now,
            auth_time: Some(now),
        }
    }
}

fn env_bool(name: &str) -> bool {
    std::env::var(name)
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

fn firebase_uid_for_email(email: &str) -> String {
    let safe_email: String = email
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect();
    format!("local-login-{safe_email}")
}
```

In `crates/backend/src/auth/middleware.rs`, replace:

```rust
let claims = if state.local_login.is_token(&token) {
    state.local_login.claims()
} else {
```

with:

```rust
let claims = if let Some(claims) = state.local_login.claims_for_token(&token) {
    claims
} else {
```

- [ ] **Step 4: Update dev login handler**

Modify `crates/backend/src/handlers/dev_login.rs`.

Add:

```rust
use serde::Deserialize;
```

Replace response structs with:

```rust
#[derive(Serialize)]
struct DevLoginProfileResponse {
    name: String,
    email: String,
    display_name: String,
}

#[derive(Serialize)]
struct DevLoginConfigResponse {
    enabled: bool,
    profiles: Vec<DevLoginProfileResponse>,
}

#[derive(Deserialize)]
struct DevLoginRequest {
    profile: Option<String>,
}
```

Replace `config`:

```rust
async fn config(State(state): State<DevLoginState>) -> Json<DevLoginConfigResponse> {
    let profiles = state
        .config
        .public_profiles()
        .into_iter()
        .map(|profile| DevLoginProfileResponse {
            name: profile.name,
            email: profile.email,
            display_name: profile.display_name,
        })
        .collect::<Vec<_>>();
    Json(DevLoginConfigResponse {
        enabled: state.config.is_enabled(),
        profiles,
    })
}
```

Replace `login` signature and body:

```rust
async fn login(
    State(state): State<DevLoginState>,
    body: Option<Json<DevLoginRequest>>,
) -> Result<Json<DevLoginResponse>, ApiError> {
    if !state.config.is_enabled() {
        return Err(ApiError::Forbidden);
    }

    let requested = body
        .as_ref()
        .and_then(|Json(body)| body.profile.as_deref())
        .unwrap_or("teacher");
    let profile = state
        .config
        .profile(requested)
        .or_else(|| state.config.default_profile())
        .ok_or(ApiError::Forbidden)?;

    let claims = profile.claims();
    ensure_user(&state.pool, &claims)
        .await
        .map_err(|err| ApiError::Internal(format!("local user provisioning failed: {err}")))?;

    Ok(Json(DevLoginResponse {
        id_token: profile.token.clone(),
    }))
}
```

- [ ] **Step 5: Update frontend API DTOs**

In `crates/features-courses/src/api.rs`, replace `DevLoginConfigDto` with:

```rust
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct DevLoginProfileDto {
    pub name: String,
    pub email: String,
    pub display_name: String,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct DevLoginConfigDto {
    pub enabled: bool,
    pub profiles: Vec<DevLoginProfileDto>,
}

#[derive(serde::Serialize)]
pub struct DevLoginRequest<'a> {
    pub profile: &'a str,
}
```

Replace `dev_login` with:

```rust
pub async fn dev_login(ctx: &ApiContext, profile: &str) -> Result<DevLoginResponseDto, ApiError> {
    fetch_json(ctx, "POST", "/v1/dev/login", Some(&DevLoginRequest { profile })).await
}
```

- [ ] **Step 6: Update login route local buttons**

In `crates/shell-web/src/routes/login.rs`, replace the local testing card body with:

```rust
if let Some(Ok(config)) = dev_login_config.read().as_ref() {
    if config.enabled {
        div { class: "auth-screen",
            div { class: "card",
                h2 { "Local testing" }
                p { "Bypass Firebase sign-in for checklist verification." }
                for profile in config.profiles.clone() {
                    {
                        let label = format!("Continue as {}", profile.display_name);
                        let profile_name = profile.name.clone();
                        let mut dev_on_success = dev_on_success.clone();
                        rsx! {
                            button {
                                class: "btn-primary",
                                onclick: move |_| {
                                    let profile_name = profile_name.clone();
                                    let mut dev_on_success = dev_on_success.clone();
                                    spawn(async move {
                                        let api_ctx = ApiContext {
                                            base_url: String::new(),
                                            id_token: String::new(),
                                        };
                                        if let Ok(response) = api::dev_login(&api_ctx, &profile_name).await {
                                            dev_on_success(response.id_token);
                                        }
                                    });
                                },
                                "{label}"
                            }
                        }
                    }
                }
            }
        }
    }
}
```

Delete the old `dev_login` closure if it is no longer used.

- [ ] **Step 7: Update env templates**

In `.env.example`, add:

```dotenv
LOCAL_LOGIN_TEACHER_TOKEN=
LOCAL_LOGIN_STUDENT_TOKEN=
LOCAL_LOGIN_STUDENT_EMAIL=local.student@example.test
LOCAL_LOGIN_STUDENT_DISPLAY_NAME=Local Student
LOCAL_LOGIN_STUDENT_FIREBASE_UID=local-login-local-student
```

In `docker-compose.yml`, backend environment, add:

```yaml
      LOCAL_LOGIN_TEACHER_TOKEN: ${LOCAL_LOGIN_TEACHER_TOKEN:-}
      LOCAL_LOGIN_STUDENT_TOKEN: ${LOCAL_LOGIN_STUDENT_TOKEN:-}
      LOCAL_LOGIN_STUDENT_EMAIL: ${LOCAL_LOGIN_STUDENT_EMAIL:-local.student@example.test}
      LOCAL_LOGIN_STUDENT_DISPLAY_NAME: ${LOCAL_LOGIN_STUDENT_DISPLAY_NAME:-Local Student}
      LOCAL_LOGIN_STUDENT_FIREBASE_UID: ${LOCAL_LOGIN_STUDENT_FIREBASE_UID:-local-login-local-student}
```

- [ ] **Step 8: Run local login tests**

Run:

```powershell
& cmd.exe /d /s /c '"C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat" >nul && set DATABASE_URL=postgres://aulalite:changeme@localhost:55432/aulalite && cargo test -p backend --test local_login_bypass -- --nocapture'
```

Expected: 4 tests pass.

- [ ] **Step 9: Commit profile login**

Run:

```powershell
git add .env.example docker-compose.yml crates/backend/src/auth/local_login.rs crates/backend/src/auth/middleware.rs crates/backend/src/handlers/dev_login.rs crates/backend/tests/local_login_bypass.rs crates/features-courses/src/api.rs crates/shell-web/src/routes/login.rs
git commit -m "feat(auth): support local audit login profiles"
```

---

### Task 7: Dev Audit Seed Endpoint

**Files:**
- Create: `crates/backend/src/handlers/dev_seed.rs`
- Modify: `crates/backend/src/handlers/mod.rs`
- Modify: `crates/backend/src/lib.rs`
- Create: `crates/backend/tests/audit_seed.rs`
- Modify: `.env.example`
- Modify: `docker-compose.yml`

- [ ] **Step 1: Write failing seed tests**

Create `crates/backend/tests/audit_seed.rs`:

```rust
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

mod fixtures;

use backend::auth::local_login::{LocalLoginConfig, LocalLoginProfile};
use backend::handlers::dev_seed::{routes as seed_routes, DevSeedState};

fn config(app_env: &str, enabled: bool) -> LocalLoginConfig {
    LocalLoginConfig::from_profiles(
        app_env.to_string(),
        enabled,
        vec![
            LocalLoginProfile {
                name: "teacher".to_string(),
                token: "teacher-token".to_string(),
                email: "local.teacher@example.test".to_string(),
                display_name: "Local Teacher".to_string(),
                firebase_uid: "local-login-local-teacher".to_string(),
            },
            LocalLoginProfile {
                name: "student".to_string(),
                token: "student-token".to_string(),
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

#[tokio::test]
async fn audit_seed_disabled_in_production() {
    let pool = fixtures::pool().await;
    let app = seed_routes(DevSeedState {
        pool,
        config: config("production", true),
        enabled: true,
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/dev/audit-seed")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn audit_seed_creates_course_for_teacher_and_student() {
    let pool = fixtures::pool().await;
    let app = seed_routes(DevSeedState {
        pool: pool.clone(),
        config: config("local", true),
        enabled: true,
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/dev/audit-seed")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = json_response(response).await;
    assert_eq!(body["tenant_slug"], "local-audit");
    assert_eq!(body["course_slug"], "audit-course");
    assert!(body["enrollment_code"].as_str().unwrap().len() >= 8);

    let teacher_courses: i64 = sqlx::query_scalar(
        "SELECT count(*)
           FROM users u
           JOIN course_memberships cm ON cm.user_id = u.id
           JOIN courses c ON c.id = cm.course_id
          WHERE u.firebase_uid = 'local-login-local-teacher'
            AND c.slug = 'audit-course'
            AND cm.role = 'teacher'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(teacher_courses, 1);

    let student_courses: i64 = sqlx::query_scalar(
        "SELECT count(*)
           FROM users u
           JOIN course_memberships cm ON cm.user_id = u.id
           JOIN courses c ON c.id = cm.course_id
          WHERE u.firebase_uid = 'local-login-local-student'
            AND c.slug = 'audit-course'
            AND cm.role = 'student'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(student_courses, 1);
}
```

- [ ] **Step 2: Run tests and verify failure**

Run:

```powershell
& cmd.exe /d /s /c '"C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat" >nul && set DATABASE_URL=postgres://aulalite:changeme@localhost:55432/aulalite && cargo test -p backend --test audit_seed -- --nocapture'
```

Expected: compile failure for missing `dev_seed`.

- [ ] **Step 3: Implement seed handler**

Create `crates/backend/src/handlers/dev_seed.rs`:

```rust
use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use serde::Serialize;
use sqlx::PgPool;

use crate::auth::jit_provision::ensure_user;
use crate::auth::local_login::LocalLoginConfig;
use crate::error::ApiError;

#[derive(Clone)]
pub struct DevSeedState {
    pub pool: PgPool,
    pub config: LocalLoginConfig,
    pub enabled: bool,
}

#[derive(Serialize)]
pub struct AuditSeedResponse {
    pub tenant_slug: String,
    pub course_slug: String,
    pub teacher_email: String,
    pub student_email: String,
    pub enrollment_code: String,
}

pub fn routes(state: DevSeedState) -> Router {
    Router::new()
        .route("/v1/dev/audit-seed", post(seed))
        .with_state(state)
}

async fn seed(State(state): State<DevSeedState>) -> Result<Json<AuditSeedResponse>, ApiError> {
    if !state.enabled || !state.config.is_enabled() {
        return Err(ApiError::Forbidden);
    }

    let teacher = state.config.profile("teacher").ok_or(ApiError::Forbidden)?.clone();
    let student = state.config.profile("student").ok_or(ApiError::Forbidden)?.clone();

    let teacher_user = ensure_user(&state.pool, &teacher.claims())
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let student_user = ensure_user(&state.pool, &student.claims())
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let tenant_id: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO tenants (slug, name, status)
         VALUES ('local-audit', 'Local Audit Tenant', 'active')
         ON CONFLICT (slug) DO UPDATE SET name = EXCLUDED.name
         RETURNING id",
    )
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;

    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    sqlx::query(
        "INSERT INTO tenant_memberships (tenant_id, user_id, role, status)
         VALUES ($1, $2, 'teacher', 'active'), ($1, $3, 'student', 'active')
         ON CONFLICT (tenant_id, user_id) DO UPDATE
            SET role = EXCLUDED.role, status = 'active'",
    )
    .bind(tenant_id)
    .bind(teacher_user.id)
    .bind(student_user.id)
    .execute(&mut *tx)
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;

    let course_id: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO courses (tenant_id, slug, title, description, status, owner_user_id)
         VALUES ($1, 'audit-course', 'Audit Course', 'Seeded course for local audit.', 'published', $2)
         ON CONFLICT (tenant_id, slug) DO UPDATE
            SET title = EXCLUDED.title,
                description = EXCLUDED.description,
                status = 'published'
         RETURNING id",
    )
    .bind(tenant_id)
    .bind(teacher_user.id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;

    sqlx::query(
        "INSERT INTO course_memberships (course_id, user_id, tenant_id, role, status)
         VALUES ($1, $2, $4, 'teacher', 'active'), ($1, $3, $4, 'student', 'active')
         ON CONFLICT (course_id, user_id) DO UPDATE
            SET role = EXCLUDED.role, status = 'active'",
    )
    .bind(course_id)
    .bind(teacher_user.id)
    .bind(student_user.id)
    .bind(tenant_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;

    let module_id: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO modules (tenant_id, course_id, title, sort_order)
         VALUES ($1, $2, 'Week 1', 10)
         ON CONFLICT DO NOTHING
         RETURNING id",
    )
    .bind(tenant_id)
    .bind(course_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?
    .unwrap_or_else(uuid::Uuid::nil);

    let module_id = if module_id.is_nil() {
        sqlx::query_scalar("SELECT id FROM modules WHERE course_id = $1 AND title = 'Week 1' LIMIT 1")
            .bind(course_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?
    } else {
        module_id
    };

    sqlx::query(
        "INSERT INTO lessons (tenant_id, course_id, module_id, type, title, body_md, sort_order)
         SELECT $1, $2, $3, 'rich_text', 'Welcome', 'Seeded lesson content.', 10
         WHERE NOT EXISTS (
             SELECT 1 FROM lessons WHERE course_id = $2 AND module_id = $3 AND title = 'Welcome'
         )",
    )
    .bind(tenant_id)
    .bind(course_id)
    .bind(module_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;

    let series_id: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO live_session_series
            (tenant_id, course_id, title, starts_at, duration_minutes, frequency,
             end_kind, occurrence_count, primary_teacher_id, recording_enabled)
         VALUES
            ($1, $2, 'Audit Live Class', now() + interval '1 day', 60, 'none',
             'count', 1, $3, true)
         RETURNING id",
    )
    .bind(tenant_id)
    .bind(course_id)
    .bind(teacher_user.id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;

    sqlx::query(
        "INSERT INTO live_sessions
            (tenant_id, course_id, series_id, occurrence_index, title, starts_at,
             duration_minutes, primary_teacher_id, recording_enabled)
         VALUES ($1, $2, $3, 0, 'Audit Live Class', now() + interval '1 day', 60, $4, true)",
    )
    .bind(tenant_id)
    .bind(course_id)
    .bind(series_id)
    .bind(teacher_user.id)
    .execute(&mut *tx)
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;

    sqlx::query(
        "INSERT INTO assignments
            (tenant_id, course_id, title, instructions_md, grading_mode, max_points,
             accepts_text, accepts_files, status, published_at, created_by)
         SELECT $1, $2, 'Audit Assignment', 'Submit a PDF or short response.',
                'numeric', 100, true, true, 'published', now(), $3
         WHERE NOT EXISTS (
             SELECT 1 FROM assignments WHERE course_id = $2 AND title = 'Audit Assignment'
         )",
    )
    .bind(tenant_id)
    .bind(course_id)
    .bind(teacher_user.id)
    .execute(&mut *tx)
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;

    let enrollment_code = "AUDIT123";
    sqlx::query(
        "INSERT INTO enrollment_codes (tenant_id, course_id, code, max_uses, created_by)
         VALUES ($1, $2, $3, 50, $4)
         ON CONFLICT (code) DO UPDATE SET expires_at = NULL",
    )
    .bind(tenant_id)
    .bind(course_id)
    .bind(enrollment_code)
    .bind(teacher_user.id)
    .execute(&mut *tx)
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;

    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(AuditSeedResponse {
        tenant_slug: "local-audit".into(),
        course_slug: "audit-course".into(),
        teacher_email: teacher.email,
        student_email: student.email,
        enrollment_code: enrollment_code.into(),
    }))
}
```

- [ ] **Step 4: Mount seed route**

In `crates/backend/src/handlers/mod.rs`, add:

```rust
pub mod dev_seed;
```

In `crates/backend/src/lib.rs`, extend `public` router:

```rust
.merge(handlers::dev_seed::routes(handlers::dev_seed::DevSeedState {
    pool: state.pool.clone(),
    config: state.local_login.clone(),
    enabled: std::env::var("LOCAL_AUDIT_SEED_ENABLED")
        .map(|value| matches!(value.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false),
}))
```

- [ ] **Step 5: Add env vars**

In `.env.example`, add:

```dotenv
LOCAL_AUDIT_SEED_ENABLED=false
```

In `docker-compose.yml`, backend environment, add:

```yaml
      LOCAL_AUDIT_SEED_ENABLED: ${LOCAL_AUDIT_SEED_ENABLED:-false}
```

- [ ] **Step 6: Run seed tests**

Run:

```powershell
& cmd.exe /d /s /c '"C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat" >nul && set DATABASE_URL=postgres://aulalite:changeme@localhost:55432/aulalite && cargo test -p backend --test audit_seed -- --nocapture'
```

Expected: 2 tests pass.

- [ ] **Step 7: Commit seed endpoint**

Run:

```powershell
git add .env.example docker-compose.yml crates/backend/src/handlers/dev_seed.rs crates/backend/src/handlers/mod.rs crates/backend/src/lib.rs crates/backend/tests/audit_seed.rs
git commit -m "feat(dev): add guarded local audit seed"
```

---

### Task 8: Verification Sweep And Checklist Evidence

**Files:**
- Modify: `docs/superpowers/plans/2026-05-10-aulalite-phase-1-5-shell-wiring-exit-checklist.md`

- [ ] **Step 1: Run backend checks**

Run:

```powershell
& cmd.exe /d /s /c '"C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat" >nul && set DATABASE_URL=postgres://aulalite:changeme@localhost:55432/aulalite && cargo test -p backend --test course_detail_tabs --test audit_seed --test local_login_bypass -- --nocapture'
```

Expected: all focused backend tests pass.

- [ ] **Step 2: Run backend compile**

Run:

```powershell
& cmd.exe /d /s /c '"C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat" >nul && cargo check -p backend --lib -j 1'
```

Expected: exit 0.

- [ ] **Step 3: Run web checks**

Run:

```powershell
cargo test -p shell-web --test shell_routes_smoke
dx build --platform web --package shell-web
```

Expected: both exit 0. Existing warnings are acceptable.

- [ ] **Step 4: Run Docker config check**

Run:

```powershell
docker compose config --quiet
```

Expected: exit 0.

- [ ] **Step 5: Update checklist automated evidence**

In `docs/superpowers/plans/2026-05-10-aulalite-phase-1-5-shell-wiring-exit-checklist.md`, add an entry under automated verification:

```markdown
- [x] Phase 1d-a focused verification:
      `cargo test -p backend --test course_detail_tabs --test audit_seed --test local_login_bypass -- --nocapture`,
      `cargo check -p backend --lib -j 1`,
      `cargo test -p shell-web --test shell_routes_smoke`, and
      `dx build --platform web --package shell-web` passed on 2026-05-10.
```

- [ ] **Step 6: Commit verification doc update**

Run:

```powershell
git add docs/superpowers/plans/2026-05-10-aulalite-phase-1-5-shell-wiring-exit-checklist.md
git commit -m "docs(checklist): record Phase 1d-a automated verification"
```

---

### Task 9: Browser Audit Run With Local Seed

**Files:**
- Modify: `docs/superpowers/plans/2026-05-10-aulalite-phase-1-5-shell-wiring-exit-checklist.md`

- [ ] **Step 1: Ensure local env enables audit seed and both profiles**

Set these values in `.env` locally. Do not commit `.env`.

```dotenv
APP_ENV=local
LOCAL_LOGIN_BYPASS_ENABLED=true
LOCAL_AUDIT_SEED_ENABLED=true
LOCAL_LOGIN_TEACHER_TOKEN=local-dev-teacher-token
LOCAL_LOGIN_STUDENT_TOKEN=local-dev-student-token
LOCAL_LOGIN_EMAIL=local.teacher@example.test
LOCAL_LOGIN_STUDENT_EMAIL=local.student@example.test
```

- [ ] **Step 2: Start backend for audit**

Use Docker if the stack is available:

```powershell
docker compose up -d
```

If Docker still hangs, use the alternate-port temporary database smoke command from the existing checklist notes and record that Docker is still blocked.

- [ ] **Step 3: Seed audit data**

Run:

```powershell
Invoke-RestMethod -Method Post http://localhost:8080/v1/dev/audit-seed
```

Expected response shape:

```text
tenant_slug : local-audit
course_slug : audit-course
teacher_email : local.teacher@example.test
student_email : local.student@example.test
enrollment_code : AUDIT123
```

- [ ] **Step 4: Run teacher browser flow**

Manual steps:

```text
1. Open the web app in Chrome.
2. Click "Continue as Local Teacher".
3. Confirm URL becomes /.
4. Confirm dashboard shows Audit Course.
5. Open Audit Course.
6. Confirm Outline tab shows Week 1 and Welcome lesson.
7. Confirm People tab shows Local Teacher and Local Student.
8. Confirm Schedule tab shows Audit Live Class.
9. Confirm Edit tab can save a title or description change.
10. Open Assignments, then Audit Assignment.
11. Confirm teacher-side assignment editor can attach a reference file after draft/edit path is available.
```

- [ ] **Step 5: Run student browser flow**

Manual steps:

```text
1. Sign out.
2. Click "Continue as Local Student".
3. Confirm dashboard shows Audit Course.
4. Open Audit Course.
5. Confirm Outline tab is readable.
6. Open Assignments.
7. Open Audit Assignment.
8. Attach a small PDF or text file.
9. Submit the assignment.
10. Confirm status changes to submitted.
```

- [ ] **Step 6: Update checklist manual evidence**

For each item proven in the browser, change `[ ]` to `[x]` and append concise evidence:

```markdown
- [x] Sign in as teacher in Chrome. Verified via local teacher profile on 2026-05-10.
```

Leave unchecked any item not actually verified.

- [ ] **Step 7: Commit checklist manual evidence**

Run:

```powershell
git add docs/superpowers/plans/2026-05-10-aulalite-phase-1-5-shell-wiring-exit-checklist.md
git commit -m "docs(checklist): record browser audit evidence"
```

---

### Task 10: Docker/8080 Stack Health

**Files:**
- Modify: `docs/superpowers/plans/2026-05-10-aulalite-phase-1-5-shell-wiring-exit-checklist.md`

- [ ] **Step 1: Check current port owner**

Run:

```powershell
Get-NetTCPConnection -LocalPort 8080 -State Listen -ErrorAction SilentlyContinue |
  Select-Object LocalAddress,LocalPort,OwningProcess |
  ForEach-Object {
    $p = Get-Process -Id $_.OwningProcess -ErrorAction SilentlyContinue
    [PSCustomObject]@{
      LocalAddress=$_.LocalAddress
      LocalPort=$_.LocalPort
      PID=$_.OwningProcess
      ProcessName=$p.ProcessName
      Path=$p.Path
    }
  }
```

Expected: no listener, or backend/Docker owns port 8080.

- [ ] **Step 2: If Apache owns port 8080, stop only that Apache process after confirming it is local dev Apache**

Run:

```powershell
Get-NetTCPConnection -LocalPort 8080 -State Listen -ErrorAction SilentlyContinue |
  ForEach-Object { Get-Process -Id $_.OwningProcess -ErrorAction SilentlyContinue }
```

If the process is `httpd` and this machine does not need it for another local app, run:

```powershell
Get-NetTCPConnection -LocalPort 8080 -State Listen -ErrorAction SilentlyContinue |
  ForEach-Object { Get-Process -Id $_.OwningProcess -ErrorAction SilentlyContinue } |
  Where-Object { $_.ProcessName -eq 'httpd' } |
  Stop-Process -Force
```

Expected: port 8080 becomes free.

- [ ] **Step 3: Remove hung Docker CLI processes**

Run:

```powershell
Get-Process docker,docker-compose -ErrorAction SilentlyContinue |
  Where-Object { $_.StartTime -gt (Get-Date).AddHours(-1) } |
  Stop-Process -Force
```

Expected: only CLI processes from this hour are stopped. Docker Desktop remains running.

- [ ] **Step 4: Verify Docker daemon responds**

Run:

```powershell
docker version --format '{{json .Server.Version}}'
```

Expected: prints a Docker Engine version and exits 0.

- [ ] **Step 5: Start stack**

Run:

```powershell
docker compose up -d
```

Expected: postgres, redis, minio, mediamtx, and backend containers start.

- [ ] **Step 6: Verify health**

Run:

```powershell
curl.exe -sS --max-time 10 http://localhost:8080/healthz
```

Expected:

```text
ok
```

- [ ] **Step 7: Update checklist stack health**

If Step 5 and Step 6 pass, mark:

```markdown
- [x] `docker compose up -d`
- [x] `curl http://localhost:8080/healthz` returns `ok`
```

If blocked, leave both unchecked and add exact command/error evidence.

- [ ] **Step 8: Commit stack-health evidence**

Run:

```powershell
git add docs/superpowers/plans/2026-05-10-aulalite-phase-1-5-shell-wiring-exit-checklist.md
git commit -m "docs(checklist): record Docker stack health evidence"
```

---

### Task 11: Final Verification And Report

**Files:** none unless checklist evidence changes.

- [ ] **Step 1: Run whitespace check**

Run:

```powershell
git diff --check
```

Expected: exit 0.

- [ ] **Step 2: Run final focused test suite**

Run:

```powershell
& cmd.exe /d /s /c '"C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat" >nul && set DATABASE_URL=postgres://aulalite:changeme@localhost:55432/aulalite && cargo test -p backend --test course_detail_tabs --test audit_seed --test local_login_bypass -- --nocapture'
cargo test -p shell-web --test shell_routes_smoke
dx build --platform web --package shell-web
```

Expected: all commands exit 0.

- [ ] **Step 3: Inspect final git status**

Run:

```powershell
git status --short --branch
```

Expected: branch is ahead by the new commits. Only intentionally untracked local files remain.

- [ ] **Step 4: Report**

Report:

```text
Implemented Phase 1d-a audit closure.

Evidence:
- backend focused tests: pass
- shell route smokes: pass
- dx web build: pass
- browser audit: pass/fail items listed in checklist
- Docker stack health: pass or exact blocker

Remaining out-of-scope features:
- notifications
- parent role/dashboard
- TA moderation/scoping changes
- desktop/mobile platform bridge auth
```
