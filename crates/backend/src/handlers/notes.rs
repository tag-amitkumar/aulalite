// crates/backend/src/handlers/notes.rs
//! Student personal notes + lesson bookmarks. Everything here is scoped to the
//! CALLER (`user_id = ctx.user_id`) and readable only by the owner.
//!
//! Routes (authed):
//!   * GET    /v1/lessons/{id}/notes     — the caller's own note (may be empty)
//!   * PUT    /v1/lessons/{id}/notes     — upsert the caller's note; empty body
//!                                        deletes the row
//!   * GET    /v1/me/bookmarks          — the caller's bookmarks, newest-first
//!   * POST   /v1/lessons/{id}/bookmark  — bookmark the lesson (idempotent)
//!   * DELETE /v1/lessons/{id}/bookmark  — remove the bookmark (idempotent)
//!
//! Authorization: the lesson must exist in the active tenant AND the caller must
//! be able to READ its course (`db::courses::caller_can_read_course`). That
//! gate prevents attaching private notes/bookmarks to lessons the caller cannot
//! see. Ownership scoping (user_id filter) lives in `db::notes`.
use axum::extract::{Extension, Path, State};
use axum::{routing, Json, Router};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::context::RequestContext;
use crate::db;
use crate::error::ApiError;
use crate::AppState;

const MAX_NOTE_LEN: usize = 20_000;

#[derive(Deserialize)]
pub struct PutNote {
    pub body: String,
}

#[derive(Serialize)]
pub struct NoteDto {
    pub lesson_id: Uuid,
    pub body: String,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<db::notes::NoteRow> for NoteDto {
    fn from(r: db::notes::NoteRow) -> Self {
        Self {
            lesson_id: r.lesson_id,
            body: r.body,
            updated_at: r.updated_at,
        }
    }
}

/// GET /v1/lessons/{id}/notes returns this even when there is no stored row, so
/// the client can render an empty editor without branching on 404.
#[derive(Serialize)]
pub struct NoteResponse {
    pub lesson_id: Uuid,
    pub body: String,
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Serialize)]
pub struct BookmarkDto {
    pub lesson_id: Uuid,
    pub course_id: Uuid,
    pub course_slug: String,
    pub lesson_title: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl From<db::notes::BookmarkRow> for BookmarkDto {
    fn from(r: db::notes::BookmarkRow) -> Self {
        Self {
            lesson_id: r.lesson_id,
            course_id: r.course_id,
            course_slug: r.course_slug,
            lesson_title: r.lesson_title,
            created_at: r.created_at,
        }
    }
}

#[derive(Serialize)]
pub struct BookmarkStateDto {
    pub lesson_id: Uuid,
    pub bookmarked: bool,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/v1/lessons/{id}/notes",
            routing::get(get_note).put(put_note),
        )
        .route("/v1/me/bookmarks", routing::get(list_bookmarks))
        .route(
            "/v1/lessons/{id}/bookmark",
            routing::post(add_bookmark).delete(remove_bookmark),
        )
}

fn is_org_admin(ctx: &RequestContext) -> bool {
    ctx.can_manage_organization()
}

/// Resolve the lesson's course and require the caller can READ it. Returns the
/// active tenant id on success. 404 when the lesson is missing in this tenant;
/// 403 when the caller can't read the course.
async fn require_lesson_read(
    state: &AppState,
    ctx: &RequestContext,
    lesson_id: Uuid,
) -> Result<Uuid, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    let course_id = db::notes::lesson_course_id(&state.pool, tenant, lesson_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::NotFound)?;
    if ctx.can_manage_organization() {
        return Ok(tenant);
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
    Ok(tenant)
}

async fn get_note(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(lesson_id): Path<Uuid>,
) -> Result<Json<NoteResponse>, ApiError> {
    let tenant = require_lesson_read(&s, &ctx, lesson_id).await?;
    let row = db::notes::fetch_note(&s.pool, tenant, ctx.user_id, lesson_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(match row {
        Some(r) => NoteResponse {
            lesson_id: r.lesson_id,
            body: r.body,
            updated_at: Some(r.updated_at),
        },
        None => NoteResponse {
            lesson_id,
            body: String::new(),
            updated_at: None,
        },
    }))
}

async fn put_note(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(lesson_id): Path<Uuid>,
    Json(body): Json<PutNote>,
) -> Result<Json<NoteResponse>, ApiError> {
    let tenant = require_lesson_read(&s, &ctx, lesson_id).await?;
    // Validate by char count so multibyte notes aren't rejected early.
    let trimmed_end = body.body.trim_end();
    if trimmed_end.chars().count() > MAX_NOTE_LEN {
        return Err(ApiError::Validation("note_too_long".into()));
    }
    // An empty note clears the row rather than persisting a blank.
    if trimmed_end.trim().is_empty() {
        db::notes::delete_note(&s.pool, tenant, ctx.user_id, lesson_id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        return Ok(Json(NoteResponse {
            lesson_id,
            body: String::new(),
            updated_at: None,
        }));
    }
    let row = db::notes::upsert_note(&s.pool, tenant, ctx.user_id, lesson_id, trimmed_end)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(NoteResponse {
        lesson_id: row.lesson_id,
        body: row.body,
        updated_at: Some(row.updated_at),
    }))
}

async fn list_bookmarks(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<Vec<BookmarkDto>>, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    let rows = db::notes::list_bookmarks(&s.pool, tenant, ctx.user_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(rows.into_iter().map(BookmarkDto::from).collect()))
}

async fn add_bookmark(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(lesson_id): Path<Uuid>,
) -> Result<Json<BookmarkStateDto>, ApiError> {
    let tenant = require_lesson_read(&s, &ctx, lesson_id).await?;
    db::notes::add_bookmark(&s.pool, tenant, ctx.user_id, lesson_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(BookmarkStateDto {
        lesson_id,
        bookmarked: true,
    }))
}

async fn remove_bookmark(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(lesson_id): Path<Uuid>,
) -> Result<Json<BookmarkStateDto>, ApiError> {
    // Removal doesn't require a fresh read gate beyond tenant scope: the row is
    // already owned by the caller and the DB delete is user-scoped. But we keep
    // the read gate for symmetry and to 404 a non-existent lesson cleanly.
    let tenant = require_lesson_read(&s, &ctx, lesson_id).await?;
    db::notes::remove_bookmark(&s.pool, tenant, ctx.user_id, lesson_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(BookmarkStateDto {
        lesson_id,
        bookmarked: false,
    }))
}
