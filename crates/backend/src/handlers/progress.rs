//! Lesson progress endpoints (learning-suite Cycle 2).
//!
//! - `PUT    /v1/courses/{cid}/lessons/{lid}/completion` — student marks complete
//! - `DELETE /v1/courses/{cid}/lessons/{lid}/completion` — student undoes it
//! - `GET    /v1/courses/{cid}/progress`                — caller's own progress
//! - `GET    /v1/courses/{cid}/progress/students`       — staff: per-student
//! - `GET    /v1/me/progress`                          — all enrolled courses

use axum::extract::{Extension, Path, State};
use axum::{routing, Json, Router};
use sqlx::PgPool;
use uuid::Uuid;

use crate::context::RequestContext;
use crate::db;
use crate::error::ApiError;
use crate::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/v1/courses/{cid}/lessons/{lid}/completion",
            routing::put(mark_complete).delete(unmark_complete),
        )
        .route("/v1/courses/{cid}/progress", routing::get(course_progress))
        .route(
            "/v1/courses/{cid}/progress/students",
            routing::get(course_student_progress),
        )
        .route("/v1/me/progress", routing::get(my_progress))
}

#[derive(Clone)]
struct TestState {
    pool: PgPool,
}

#[doc(hidden)]
pub fn router_for_tests(pool: PgPool) -> Router {
    Router::new()
        .route(
            "/v1/courses/{cid}/lessons/{lid}/completion",
            routing::put(mark_complete_t).delete(unmark_complete_t),
        )
        .route(
            "/v1/courses/{cid}/progress",
            routing::get(course_progress_t),
        )
        .route(
            "/v1/courses/{cid}/progress/students",
            routing::get(course_student_progress_t),
        )
        .route("/v1/me/progress", routing::get(my_progress_t))
        .with_state(TestState { pool })
}

fn is_org_admin(ctx: &RequestContext) -> bool {
    ctx.can_manage_organization()
}

fn require_tenant(ctx: &RequestContext) -> Result<Uuid, ApiError> {
    ctx.tenant_id
        .ok_or_else(|| ApiError::BadRequest("no active tenant".into()))
}

fn require_learner(ctx: &RequestContext) -> Result<(), ApiError> {
    ctx.has_capability(core_types::Capability::Learn)
        .then_some(())
        .ok_or(ApiError::Forbidden)
}

async fn require_course_read(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
) -> Result<(), ApiError> {
    if db::courses::caller_can_read_course(
        pool,
        course_id,
        ctx.user_id,
        ctx.tenant_id,
        is_org_admin(ctx),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?
    {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
    }
}

async fn require_course_staff(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
) -> Result<(), ApiError> {
    if ctx.can_manage_organization() {
        return Ok(());
    }
    if db::courses::caller_can_staff_course(
        pool,
        course_id,
        ctx.user_id,
        ctx.tenant_id,
        is_org_admin(ctx),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?
    {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
    }
}

#[derive(serde::Serialize)]
pub struct CompletionResponse {
    pub lesson_id: Uuid,
    pub completed: bool,
}

async fn mark_complete(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path((course_id, lesson_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<CompletionResponse>, ApiError> {
    set_completion(&state.pool, &ctx, course_id, lesson_id, true).await
}

async fn unmark_complete(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path((course_id, lesson_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<CompletionResponse>, ApiError> {
    set_completion(&state.pool, &ctx, course_id, lesson_id, false).await
}

async fn mark_complete_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path((course_id, lesson_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<CompletionResponse>, ApiError> {
    set_completion(&s.pool, &ctx, course_id, lesson_id, true).await
}

async fn unmark_complete_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path((course_id, lesson_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<CompletionResponse>, ApiError> {
    set_completion(&s.pool, &ctx, course_id, lesson_id, false).await
}

async fn set_completion(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
    lesson_id: Uuid,
    completed: bool,
) -> Result<Json<CompletionResponse>, ApiError> {
    require_learner(ctx)?;
    let tenant_id = require_tenant(ctx)?;
    require_course_read(pool, ctx, course_id).await?;

    let mut tx = db::begin_with_context(pool, ctx.user_id, ctx.tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !db::courses::has_active_course_membership_roles(
        &mut tx,
        tenant_id,
        course_id,
        ctx.user_id,
        "student",
        "student",
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?
    {
        return Err(ApiError::Forbidden);
    }
    if !db::progress::lesson_in_course(&mut tx, course_id, lesson_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
    {
        return Err(ApiError::NotFound);
    }
    if completed {
        let newly =
            db::progress::mark_complete(&mut tx, tenant_id, course_id, lesson_id, ctx.user_id)
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?;
        // Best-effort XP award (idempotent across unmark/remark via dedup).
        if newly {
            if let Err(e) = db::gamification::award(
                &mut tx,
                tenant_id,
                ctx.user_id,
                Some(course_id),
                "lesson_completed",
                db::gamification::XP_LESSON_COMPLETED,
                &format!("lesson:{lesson_id}:{}", ctx.user_id),
            )
            .await
            {
                tracing::warn!(?e, %lesson_id, "lesson XP award failed");
            }
            // Best-effort certificate eligibility (idempotent): this lesson
            // may have been the last completion requirement.
            if let Err(e) =
                db::certificates::sync_eligibility(&mut tx, tenant_id, course_id, ctx.user_id).await
            {
                tracing::warn!(?e, %course_id, "certificate eligibility sync failed");
            }
        }
    } else {
        db::progress::unmark_complete(&mut tx, lesson_id, ctx.user_id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
    }
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(CompletionResponse {
        lesson_id,
        completed,
    }))
}

#[derive(serde::Serialize)]
pub struct CourseProgressResponse {
    pub course_id: Uuid,
    pub completed_lesson_ids: Vec<Uuid>,
    pub completed: i64,
    pub total: i64,
    pub resume_lesson_id: Option<Uuid>,
    pub resume_lesson_title: Option<String>,
}

async fn course_progress(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(course_id): Path<Uuid>,
) -> Result<Json<CourseProgressResponse>, ApiError> {
    course_progress_inner(&state.pool, &ctx, course_id).await
}

async fn course_progress_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(course_id): Path<Uuid>,
) -> Result<Json<CourseProgressResponse>, ApiError> {
    course_progress_inner(&s.pool, &ctx, course_id).await
}

async fn course_progress_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
) -> Result<Json<CourseProgressResponse>, ApiError> {
    require_course_read(pool, ctx, course_id).await?;

    let mut tx = db::begin_with_context(pool, ctx.user_id, ctx.tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let completed_lesson_ids = db::progress::completed_lesson_ids(&mut tx, course_id, ctx.user_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    // Course items = lessons + published module-attached quizzes. A quiz
    // counts as complete once it has at least one submitted attempt.
    let lesson_total = db::progress::lesson_count(&mut tx, course_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let quiz_total = db::quizzes::module_quiz_count(&mut tx, course_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let quiz_completed = db::quizzes::completed_module_quiz_count(&mut tx, course_id, ctx.user_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let resume = db::progress::first_incomplete_lesson(&mut tx, course_id, ctx.user_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(CourseProgressResponse {
        course_id,
        completed: completed_lesson_ids.len() as i64 + quiz_completed,
        completed_lesson_ids,
        total: lesson_total + quiz_total,
        resume_lesson_id: resume.as_ref().map(|(id, _)| *id),
        resume_lesson_title: resume.map(|(_, title)| title),
    }))
}

#[derive(serde::Serialize)]
pub struct StudentProgressRow {
    pub user_id: Uuid,
    pub display_name: Option<String>,
    pub email: String,
    pub completed: i64,
    pub total: i64,
}

async fn course_student_progress(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(course_id): Path<Uuid>,
) -> Result<Json<Vec<StudentProgressRow>>, ApiError> {
    course_student_progress_inner(&state.pool, &ctx, course_id).await
}

async fn course_student_progress_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(course_id): Path<Uuid>,
) -> Result<Json<Vec<StudentProgressRow>>, ApiError> {
    course_student_progress_inner(&s.pool, &ctx, course_id).await
}

async fn course_student_progress_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
) -> Result<Json<Vec<StudentProgressRow>>, ApiError> {
    require_course_staff(pool, ctx, course_id).await?;

    let mut tx = db::begin_with_context(pool, ctx.user_id, ctx.tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let total = db::progress::lesson_count(&mut tx, course_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let rows = db::progress::course_student_progress(&mut tx, course_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(
        rows.into_iter()
            .map(
                |(user_id, display_name, email, completed)| StudentProgressRow {
                    user_id,
                    display_name,
                    email,
                    completed,
                    total,
                },
            )
            .collect(),
    ))
}

#[derive(serde::Serialize)]
pub struct MyCourseProgress {
    pub course_id: Uuid,
    pub slug: String,
    pub title: String,
    pub completed: i64,
    pub total: i64,
    pub last_activity_at: Option<chrono::DateTime<chrono::Utc>>,
    pub resume_lesson_id: Option<Uuid>,
    pub resume_lesson_title: Option<String>,
}

async fn my_progress(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<Vec<MyCourseProgress>>, ApiError> {
    my_progress_inner(&state.pool, &ctx).await
}

async fn my_progress_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<Vec<MyCourseProgress>>, ApiError> {
    my_progress_inner(&s.pool, &ctx).await
}

async fn my_progress_inner(
    pool: &PgPool,
    ctx: &RequestContext,
) -> Result<Json<Vec<MyCourseProgress>>, ApiError> {
    let tenant_id = require_tenant(ctx)?;
    let mut tx = db::begin_with_context(pool, ctx.user_id, ctx.tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let rows = db::progress::my_course_progress(&mut tx, ctx.user_id, tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let mut out = Vec::with_capacity(rows.len());
    for (course_id, slug, title, lesson_total, lesson_completed, last_activity_at) in rows {
        // Fold published module-attached quizzes into the item counts (a
        // quiz is complete once any attempt is submitted).
        let quiz_total = db::quizzes::module_quiz_count(&mut tx, course_id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        let quiz_completed =
            db::quizzes::completed_module_quiz_count(&mut tx, course_id, ctx.user_id)
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?;
        let total = lesson_total + quiz_total;
        let completed = lesson_completed + quiz_completed;
        let resume = if completed < total {
            db::progress::first_incomplete_lesson(&mut tx, course_id, ctx.user_id)
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?
        } else {
            None
        };
        out.push(MyCourseProgress {
            course_id,
            slug,
            title,
            completed,
            total,
            last_activity_at,
            resume_lesson_id: resume.as_ref().map(|(id, _)| *id),
            resume_lesson_title: resume.map(|(_, t)| t),
        });
    }
    Ok(Json(out))
}
