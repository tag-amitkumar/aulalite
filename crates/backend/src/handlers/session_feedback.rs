// crates/backend/src/handlers/session_feedback.rs
//! Post-session feedback / ratings.
//!
//! Routes (under the authed router, session-scoped):
//!   * POST /v1/sessions/{id}/feedback — enrolled participant; rating 1..5 + comment
//!   * GET  /v1/sessions/{id}/feedback — staff-only summary (count, average, recent)
//!
//! Both routes first resolve the session's `course_id` (tenant-scoped under RLS
//! via `db::session_feedback::fetch_session_course`) and then gate against the
//! course: the write requires read access (an enrolled participant), the read
//! requires staff. This mirrors the gating style in `handlers::announcements`.
use axum::extract::{Extension, Path, State};
use axum::{routing, Json, Router};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::context::RequestContext;
use crate::db;
use crate::error::ApiError;
use crate::AppState;

const MAX_COMMENT_LEN: usize = 1000;
const RECENT_COMMENT_LIMIT: i64 = 20;

#[derive(Deserialize)]
pub struct SubmitFeedback {
    pub rating: i32,
    #[serde(default)]
    pub comment: Option<String>,
}

#[derive(Serialize)]
pub struct FeedbackDto {
    pub id: Uuid,
    pub session_id: Uuid,
    pub user_id: Uuid,
    pub rating: i32,
    pub comment: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl From<db::session_feedback::FeedbackRow> for FeedbackDto {
    fn from(r: db::session_feedback::FeedbackRow) -> Self {
        Self {
            id: r.id,
            session_id: r.session_id,
            user_id: r.user_id,
            rating: r.rating,
            comment: r.comment,
            created_at: r.created_at,
        }
    }
}

/// Response for POST: the upserted row plus a convenience flag the form uses to
/// flip into "thanks, here's your rating" mode.
#[derive(Serialize)]
pub struct SubmitFeedbackResponse {
    #[serde(flatten)]
    pub feedback: FeedbackDto,
}

#[derive(Serialize)]
pub struct FeedbackCommentDto {
    pub rating: i32,
    pub comment: String,
    pub author_display_name: Option<String>,
    pub author_email: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl From<db::session_feedback::FeedbackCommentRow> for FeedbackCommentDto {
    fn from(r: db::session_feedback::FeedbackCommentRow) -> Self {
        Self {
            rating: r.rating,
            comment: r.comment,
            author_display_name: r.author_display_name,
            author_email: r.author_email,
            created_at: r.created_at,
        }
    }
}

#[derive(Serialize)]
pub struct FeedbackSummaryDto {
    pub count: i64,
    pub average: Option<f64>,
    pub recent_comments: Vec<FeedbackCommentDto>,
}

pub fn routes() -> Router<AppState> {
    Router::new().route(
        "/v1/sessions/{id}/feedback",
        routing::get(get_feedback).post(submit_feedback),
    )
}

fn is_org_admin(ctx: &RequestContext) -> bool {
    ctx.can_manage_organization()
}

/// Resolve the session's course, tenant-scoped under RLS. NotFound when the
/// session does not exist in the caller's tenant.
async fn resolve_course(s: &AppState, tenant: Uuid, session_id: Uuid) -> Result<Uuid, ApiError> {
    let course = db::session_feedback::fetch_session_course(&s.pool, tenant, session_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::NotFound)?;
    Ok(course.course_id)
}

/// Read gate: anyone who can read the course (owner / any active member /
/// org_admin / platform_admin).
async fn require_course_read(
    s: &AppState,
    ctx: &RequestContext,
    course_id: Uuid,
) -> Result<(), ApiError> {
    if ctx.can_manage_organization() {
        return Ok(());
    }
    if !db::courses::caller_can_read_course(
        &s.pool,
        course_id,
        ctx.user_id,
        ctx.tenant_id,
        is_org_admin(ctx),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?
    {
        return Err(ApiError::Forbidden);
    }
    Ok(())
}

/// Staff gate: course owner / active teacher-ta member / org_admin / platform_admin.
async fn require_course_staff(
    s: &AppState,
    ctx: &RequestContext,
    course_id: Uuid,
) -> Result<(), ApiError> {
    if ctx.can_manage_organization() {
        return Ok(());
    }
    if !db::courses::caller_can_staff_course(
        &s.pool,
        course_id,
        ctx.user_id,
        ctx.tenant_id,
        is_org_admin(ctx),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?
    {
        return Err(ApiError::Forbidden);
    }
    Ok(())
}

/// POST /v1/sessions/{id}/feedback — an enrolled participant rates the session.
/// Upserts so a student can revise their rating. Returns the persisted row.
async fn submit_feedback(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(session_id): Path<Uuid>,
    Json(body): Json<SubmitFeedback>,
) -> Result<Json<SubmitFeedbackResponse>, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    let course_id = resolve_course(&s, tenant, session_id).await?;
    // Writing requires the caller to be able to read the course (enrolled
    // participant or staff). Staff rating their own session is harmless.
    require_course_read(&s, &ctx, course_id).await?;

    if !(1..=5).contains(&body.rating) {
        return Err(ApiError::Validation("rating_out_of_range".into()));
    }
    // Normalize the comment: trim, treat empty as None, validate by char count
    // (not byte length) so multibyte comments aren't rejected early.
    let comment = body
        .comment
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty());
    if let Some(c) = comment {
        if c.chars().count() > MAX_COMMENT_LEN {
            return Err(ApiError::Validation("comment_too_long".into()));
        }
    }

    let row = db::session_feedback::upsert(
        &s.pool,
        tenant,
        session_id,
        ctx.user_id,
        body.rating,
        comment,
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(SubmitFeedbackResponse {
        feedback: FeedbackDto::from(row),
    }))
}

/// GET /v1/sessions/{id}/feedback — staff-only aggregate summary.
async fn get_feedback(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(session_id): Path<Uuid>,
) -> Result<Json<FeedbackSummaryDto>, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    let course_id = resolve_course(&s, tenant, session_id).await?;
    require_course_staff(&s, &ctx, course_id).await?;

    let summary = db::session_feedback::summary(&s.pool, tenant, session_id, RECENT_COMMENT_LIMIT)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(FeedbackSummaryDto {
        count: summary.count,
        average: summary.average,
        recent_comments: summary
            .recent_comments
            .into_iter()
            .map(FeedbackCommentDto::from)
            .collect(),
    }))
}
