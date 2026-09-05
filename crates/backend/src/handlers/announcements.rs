// crates/backend/src/handlers/announcements.rs
//! Course announcements: staff post; everyone who can read the course reads.
//!
//! Routes (all under the authed router, course-scoped):
//!   * GET    /v1/courses/{cid}/announcements        — list newest-first, read-gated
//!   * POST   /v1/courses/{cid}/announcements        — staff-only; title + body_md
//!   * DELETE /v1/courses/{cid}/announcements/{id}     — author or course staff
//!
//! On POST we fan a "course_announcement" notification out to every enrolled
//! student via the `services::notifications::notify` facade — mirroring the
//! grade-release path in `handlers::submissions::notify_grade_released`. The
//! notify title/body are plain text the facade HTML-escapes on the email path;
//! the body we pass is a short escaped-safe summary, not the raw markdown.
use axum::extract::{Extension, Path, State};
use axum::{routing, Json, Router};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::context::RequestContext;
use crate::db;
use crate::error::ApiError;
use crate::AppState;

const MAX_TITLE_LEN: usize = 200;
const MAX_BODY_LEN: usize = 10_000;

#[derive(Deserialize)]
pub struct CreateAnnouncement {
    pub title: String,
    pub body_md: String,
}

#[derive(Serialize)]
pub struct AnnouncementDto {
    pub id: Uuid,
    pub course_id: Uuid,
    pub author_user_id: Uuid,
    pub author_display_name: Option<String>,
    pub author_email: Option<String>,
    pub title: String,
    pub body_md: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<db::announcements::AnnouncementRow> for AnnouncementDto {
    fn from(r: db::announcements::AnnouncementRow) -> Self {
        Self {
            id: r.id,
            course_id: r.course_id,
            author_user_id: r.author_user_id,
            author_display_name: r.author_display_name,
            author_email: r.author_email,
            title: r.title,
            body_md: r.body_md,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/v1/courses/{cid}/announcements",
            routing::get(list).post(create),
        )
        .route(
            "/v1/courses/{cid}/announcements/{id}",
            routing::delete(delete_one),
        )
}

fn is_org_admin(ctx: &RequestContext) -> bool {
    ctx.can_manage_organization()
}

/// Course-scoped staff gate (course owner / active teacher-ta member / org_admin
/// / platform_admin). `platform_admin` always passes.
async fn require_course_staff(
    state: &AppState,
    ctx: &RequestContext,
    course_id: Uuid,
) -> Result<(), ApiError> {
    if ctx.can_manage_organization() {
        return Ok(());
    }
    if !db::courses::caller_can_staff_course(
        &state.pool,
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

/// Read gate: anyone who can read the course (owner / any active member /
/// org_admin / platform_admin).
async fn require_course_read(
    state: &AppState,
    ctx: &RequestContext,
    course_id: Uuid,
) -> Result<(), ApiError> {
    if ctx.can_manage_organization() {
        return Ok(());
    }
    if !db::courses::caller_can_read_course(
        &state.pool,
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

async fn list(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(cid): Path<Uuid>,
) -> Result<Json<Vec<AnnouncementDto>>, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    require_course_read(&s, &ctx, cid).await?;
    let rows = db::announcements::list_for_course(&s.pool, tenant, cid)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(rows.into_iter().map(AnnouncementDto::from).collect()))
}

async fn create(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(cid): Path<Uuid>,
    Json(body): Json<CreateAnnouncement>,
) -> Result<Json<AnnouncementDto>, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    require_course_staff(&s, &ctx, cid).await?;

    let title = body.title.trim();
    let body_md = body.body_md.trim();
    if title.is_empty() {
        return Err(ApiError::Validation("title_required".into()));
    }
    // Validate by char count (not byte length) so multibyte titles aren't
    // rejected early. Limits: title <= 200 chars, body_md <= 10000 chars.
    if title.chars().count() > MAX_TITLE_LEN {
        return Err(ApiError::Validation("title_too_long".into()));
    }
    if body_md.is_empty() {
        return Err(ApiError::Validation("body_required".into()));
    }
    if body_md.chars().count() > MAX_BODY_LEN {
        return Err(ApiError::Validation("body_too_long".into()));
    }

    // Defense-in-depth: sanitize the stored title/body (strip control chars,
    // raw HTML, and dangerous URL schemes) before persistence.
    let title = crate::services::sanitize::clean_text(title, MAX_TITLE_LEN);
    let body_md = crate::services::sanitize::clean_markdown(body_md, MAX_BODY_LEN);
    let row = db::announcements::insert(&s.pool, tenant, cid, ctx.user_id, &title, &body_md)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let dto = AnnouncementDto::from(row);
    // Best-effort fan-out to enrolled students. The announcement is already
    // committed; a notify failure must never fail the POST.
    notify_enrolled_students(&s, tenant, cid, &dto).await;
    Ok(Json(dto))
}

async fn delete_one(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path((cid, id)): Path<(Uuid, Uuid)>,
) -> Result<axum::http::StatusCode, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    // The row must exist (and be in this course/tenant) to authorize.
    let row = db::announcements::fetch_in_course(&s.pool, tenant, cid, id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::NotFound)?;
    // Author may always delete their own; otherwise require course staff.
    if row.author_user_id != ctx.user_id {
        require_course_staff(&s, &ctx, cid).await?;
    }
    let deleted = db::announcements::delete_in_course(&s.pool, tenant, cid, id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !deleted {
        return Err(ApiError::NotFound);
    }
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// Best-effort "new announcement" notification to every enrolled student.
/// Builds a deep link to the course announcements tab and fans out via the
/// notify facade (in-app + email + push, honoring each user's preferences).
/// NEVER returns/propagates an error.
async fn notify_enrolled_students(
    state: &AppState,
    tenant_id: Uuid,
    course_id: Uuid,
    dto: &AnnouncementDto,
) {
    let student_ids =
        match db::announcements::list_enrolled_student_ids(&state.pool, tenant_id, course_id).await
        {
            Ok(ids) => ids,
            Err(e) => {
                tracing::warn!(?e, %course_id, "notify_enrolled_students: student lookup failed");
                return;
            }
        };
    if student_ids.is_empty() {
        return;
    }
    let title = format!("New announcement: {}", dto.title);
    // A short plain-text preview of the body for the notification body. The
    // notify facade HTML-escapes this on the email path; we pass a collapsed,
    // truncated plain-text summary (not the raw markdown) so the in-app/email
    // body stays a single tidy line.
    let preview = body_preview(&dto.body_md);
    let link = format!(
        "{}/courses/{}/announcements",
        state.app_origin.trim_end_matches('/'),
        course_id
    );
    for student_id in student_ids {
        crate::services::notifications::notify(
            &state.pool,
            state.email_notifier.as_ref(),
            state.push_sender.as_ref(),
            tenant_id,
            student_id,
            "course_announcement",
            &title,
            Some(&preview),
            Some(&link),
        )
        .await;
    }
}

/// Collapse markdown whitespace to a single line and truncate to ~140 chars for
/// a notification preview. Pure so it's unit-testable.
fn body_preview(body_md: &str) -> String {
    let collapsed: String = body_md.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() > 140 {
        let head: String = collapsed.chars().take(137).collect();
        format!("{head}…")
    } else {
        collapsed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_preview_collapses_and_truncates() {
        assert_eq!(body_preview("hello   world\n\nfoo"), "hello world foo");
        let long = "x ".repeat(200);
        let p = body_preview(&long);
        assert!(p.chars().count() <= 140, "preview too long: {}", p.len());
        assert!(p.ends_with('…'), "expected ellipsis: {p}");
    }

    #[test]
    fn body_preview_empty_stays_empty() {
        assert_eq!(body_preview("   \n  "), "");
    }
}
