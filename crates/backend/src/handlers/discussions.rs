// crates/backend/src/handlers/discussions.rs
//! Course discussion forums / Q&A: any enrolled member opens threads and
//! replies; course staff pin/lock and may delete anything; authors may delete
//! their own thread/post.
//!
//! Routes (all under the authed router):
//!   * GET    /v1/courses/{cid}/discussions        — list, pinned-first; read-gated
//!   * POST   /v1/courses/{cid}/discussions        — open a thread; read-gated (enrolled)
//!   * GET    /v1/discussions/{id}                 — thread + nested posts; read-gated
//!   * POST   /v1/discussions/{id}/posts           — reply; read-gated, honors `locked`
//!   * DELETE /v1/discussions/{id}                 — author or course staff
//!   * PATCH  /v1/discussions/{id}                 — pin/lock; course staff
//!   * DELETE /v1/discussions/{id}/posts/{pid}      — author or course staff
//!
//! On a new reply we best-effort notify the thread author via the
//! `services::notifications::notify` facade — mirroring the announcements
//! fan-out. A notify failure never fails the POST.
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

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct CreateThread {
    pub title: String,
    pub body_md: String,
}

#[derive(Deserialize)]
pub struct CreatePost {
    pub body_md: String,
    #[serde(default)]
    pub parent_post_id: Option<Uuid>,
}

#[derive(Deserialize)]
pub struct PatchThread {
    #[serde(default)]
    pub pinned: Option<bool>,
    #[serde(default)]
    pub locked: Option<bool>,
}

#[derive(Serialize)]
pub struct ThreadDto {
    pub id: Uuid,
    pub course_id: Uuid,
    pub author_user_id: Uuid,
    pub author_display_name: Option<String>,
    pub author_email: Option<String>,
    pub title: String,
    pub body_md: String,
    pub pinned: bool,
    pub locked: bool,
    pub reply_count: i64,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<db::discussions::DiscussionRow> for ThreadDto {
    fn from(r: db::discussions::DiscussionRow) -> Self {
        Self {
            id: r.id,
            course_id: r.course_id,
            author_user_id: r.author_user_id,
            author_display_name: r.author_display_name,
            author_email: r.author_email,
            title: r.title,
            body_md: r.body_md,
            pinned: r.pinned,
            locked: r.locked,
            reply_count: r.reply_count,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

#[derive(Serialize)]
pub struct PostDto {
    pub id: Uuid,
    pub discussion_id: Uuid,
    pub parent_post_id: Option<Uuid>,
    pub author_user_id: Uuid,
    pub author_display_name: Option<String>,
    pub author_email: Option<String>,
    pub body_md: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl From<db::discussions::DiscussionPostRow> for PostDto {
    fn from(r: db::discussions::DiscussionPostRow) -> Self {
        Self {
            id: r.id,
            discussion_id: r.discussion_id,
            parent_post_id: r.parent_post_id,
            author_user_id: r.author_user_id,
            author_display_name: r.author_display_name,
            author_email: r.author_email,
            body_md: r.body_md,
            created_at: r.created_at,
        }
    }
}

/// A thread plus its flat-but-ordered posts. The frontend assembles the nested
/// reply tree from each post's `parent_post_id`.
#[derive(Serialize)]
pub struct ThreadDetailDto {
    pub thread: ThreadDto,
    pub posts: Vec<PostDto>,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/v1/courses/{cid}/discussions",
            routing::get(list_threads).post(create_thread),
        )
        .route(
            "/v1/discussions/{id}",
            routing::get(get_thread)
                .delete(delete_thread)
                .patch(patch_thread),
        )
        .route("/v1/discussions/{id}/posts", routing::post(create_post))
        .route(
            "/v1/discussions/{id}/posts/{pid}",
            routing::delete(delete_post),
        )
}

// ---------------------------------------------------------------------------
// Authorization helpers (mirror handlers::announcements)
// ---------------------------------------------------------------------------

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
/// org_admin / platform_admin). This is also the "enrolled member" gate used
/// for opening threads and replying.
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

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn list_threads(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(cid): Path<Uuid>,
) -> Result<Json<Vec<ThreadDto>>, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    require_course_read(&s, &ctx, cid).await?;
    let rows = db::discussions::list_threads_for_course(&s.pool, tenant, cid)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(rows.into_iter().map(ThreadDto::from).collect()))
}

async fn create_thread(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(cid): Path<Uuid>,
    Json(body): Json<CreateThread>,
) -> Result<Json<ThreadDto>, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    // Any enrolled member (read-gated) may open a thread.
    require_course_read(&s, &ctx, cid).await?;

    let title = body.title.trim();
    let body_md = body.body_md.trim();
    if title.is_empty() {
        return Err(ApiError::Validation("title_required".into()));
    }
    if title.chars().count() > MAX_TITLE_LEN {
        return Err(ApiError::Validation("title_too_long".into()));
    }
    if body_md.is_empty() {
        return Err(ApiError::Validation("body_required".into()));
    }
    if body_md.chars().count() > MAX_BODY_LEN {
        return Err(ApiError::Validation("body_too_long".into()));
    }

    // Defense-in-depth: sanitize the stored title/body before persistence.
    let title = crate::services::sanitize::clean_text(title, MAX_TITLE_LEN);
    let body_md = crate::services::sanitize::clean_markdown(body_md, MAX_BODY_LEN);
    let row = db::discussions::insert_thread(&s.pool, tenant, cid, ctx.user_id, &title, &body_md)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(ThreadDto::from(row)))
}

async fn get_thread(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<ThreadDetailDto>, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    let thread = db::discussions::fetch_thread(&s.pool, tenant, id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::NotFound)?;
    // Authorize against the thread's course.
    require_course_read(&s, &ctx, thread.course_id).await?;
    let posts = db::discussions::list_posts_for_thread(&s.pool, tenant, id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(ThreadDetailDto {
        thread: ThreadDto::from(thread),
        posts: posts.into_iter().map(PostDto::from).collect(),
    }))
}

async fn create_post(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    Json(body): Json<CreatePost>,
) -> Result<Json<PostDto>, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    let thread = db::discussions::fetch_thread(&s.pool, tenant, id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::NotFound)?;
    // Any enrolled member of the thread's course may reply.
    require_course_read(&s, &ctx, thread.course_id).await?;
    // A locked thread accepts no new replies — except from course staff (so
    // teachers can still post a closing note). Students get a clear 403.
    if thread.locked
        && require_course_staff(&s, &ctx, thread.course_id)
            .await
            .is_err()
    {
        return Err(ApiError::Validation("thread_locked".into()));
    }

    let body_md = body.body_md.trim();
    if body_md.is_empty() {
        return Err(ApiError::Validation("body_required".into()));
    }
    if body_md.chars().count() > MAX_BODY_LEN {
        return Err(ApiError::Validation("body_too_long".into()));
    }

    // A non-null parent must belong to this same thread, else nesting could
    // point across threads. Cheap guard via fetch_post.
    if let Some(parent_id) = body.parent_post_id {
        let parent = db::discussions::fetch_post(&s.pool, tenant, parent_id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?
            .ok_or(ApiError::NotFound)?;
        if parent.discussion_id != id {
            return Err(ApiError::Validation("parent_not_in_thread".into()));
        }
    }

    // Defense-in-depth: sanitize the stored reply body before persistence.
    let body_md = crate::services::sanitize::clean_markdown(body_md, MAX_BODY_LEN);
    let row = db::discussions::insert_post(
        &s.pool,
        tenant,
        id,
        body.parent_post_id,
        ctx.user_id,
        &body_md,
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;

    let dto = PostDto::from(row);
    // Best-effort notify the thread author of the new reply (skip self-replies).
    notify_thread_author(&s, tenant, &thread, &dto, ctx.user_id).await;
    Ok(Json(dto))
}

async fn patch_thread(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    Json(body): Json<PatchThread>,
) -> Result<Json<ThreadDto>, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    let thread = db::discussions::fetch_thread(&s.pool, tenant, id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::NotFound)?;
    // Pin/lock are staff-only.
    require_course_staff(&s, &ctx, thread.course_id).await?;
    if body.pinned.is_none() && body.locked.is_none() {
        return Err(ApiError::Validation("no_changes".into()));
    }
    let updated = db::discussions::set_thread_flags(&s.pool, tenant, id, body.pinned, body.locked)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(ThreadDto::from(updated)))
}

async fn delete_thread(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<axum::http::StatusCode, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    let thread = db::discussions::fetch_thread(&s.pool, tenant, id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::NotFound)?;
    // Author may always delete their own thread; otherwise require course staff.
    if thread.author_user_id != ctx.user_id {
        require_course_staff(&s, &ctx, thread.course_id).await?;
    }
    let deleted = db::discussions::delete_thread(&s.pool, tenant, id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !deleted {
        return Err(ApiError::NotFound);
    }
    Ok(axum::http::StatusCode::NO_CONTENT)
}

async fn delete_post(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path((id, pid)): Path<(Uuid, Uuid)>,
) -> Result<axum::http::StatusCode, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    let post = db::discussions::fetch_post(&s.pool, tenant, pid)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::NotFound)?;
    if post.discussion_id != id {
        return Err(ApiError::NotFound);
    }
    // Need the thread to resolve the course for the staff gate.
    let thread = db::discussions::fetch_thread(&s.pool, tenant, id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::NotFound)?;
    // Author may always delete their own post; otherwise require course staff.
    if post.author_user_id != ctx.user_id {
        require_course_staff(&s, &ctx, thread.course_id).await?;
    }
    let deleted = db::discussions::delete_post(&s.pool, tenant, pid)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !deleted {
        return Err(ApiError::NotFound);
    }
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// Best-effort "new reply" notification to the thread author. Skips when the
/// replier is the author. NEVER returns/propagates an error.
async fn notify_thread_author(
    state: &AppState,
    tenant_id: Uuid,
    thread: &db::discussions::DiscussionRow,
    reply: &PostDto,
    replier_user_id: Uuid,
) {
    if thread.author_user_id == replier_user_id {
        return;
    }
    let title = format!("New reply: {}", thread.title);
    let preview = body_preview(&reply.body_md);
    let link = format!(
        "{}/discussions/{}",
        state.app_origin.trim_end_matches('/'),
        thread.id
    );
    crate::services::notifications::notify(
        &state.pool,
        state.email_notifier.as_ref(),
        state.push_sender.as_ref(),
        tenant_id,
        thread.author_user_id,
        "discussion_reply",
        &title,
        Some(&preview),
        Some(&link),
    )
    .await;
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
