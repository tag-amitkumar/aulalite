// crates/backend/src/handlers/recordings.rs
//! Recording chapters: named timestamp bookmarks into a recording's timeline.
//!
//! Course staff author and delete chapters; anyone who can read the course
//! reads them back on the replay view to seek the <video>. These live here
//! (NOT in `handlers::live_sessions`) but reuse the existing
//! `/v1/sessions/{id}/recording` URL family so the replay UI keys everything by
//! `session_id` — the recording's id is resolved server-side via
//! `db::recordings::fetch_by_session`.
//!
//! Routes (authed router):
//!   * GET    /v1/sessions/{id}/recording/chapters       — read-gated, timeline order
//!   * POST   /v1/sessions/{id}/recording/chapters       — staff-only; label + position
//!   * DELETE /v1/sessions/{id}/recording/chapters/{cid}   — staff-only
//!
//! TENANT-SCOPED under RLS: the chapter reads/writes run inside a tx opened with
//! `db::begin_with_context` so the `tenant_isolation` policy on
//! `recording_chapters` applies under the non-bypass `aulalite_app` role. Runtime
//! sqlx only (no compile-time macros).
use axum::extract::{Extension, Path, State};
use axum::{routing, Json, Router};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::context::RequestContext;
use crate::db;
use crate::error::ApiError;
use crate::AppState;

const MAX_LABEL_LEN: usize = 120;
/// Upper bound on a chapter position. Recordings are capped well below this in
/// practice; this just rejects absurd/overflowing client input. 24h in seconds.
const MAX_POSITION_SECONDS: i64 = 24 * 60 * 60;

#[derive(Deserialize)]
pub struct CreateChapter {
    pub label: String,
    pub position_seconds: i64,
}

#[derive(Serialize)]
pub struct ChapterDto {
    pub id: Uuid,
    pub recording_id: Uuid,
    pub label: String,
    pub position_seconds: i32,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl From<db::recordings::RecordingChapterRow> for ChapterDto {
    fn from(r: db::recordings::RecordingChapterRow) -> Self {
        Self {
            id: r.id,
            recording_id: r.recording_id,
            label: r.label,
            position_seconds: r.position_seconds,
            created_at: r.created_at,
        }
    }
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/v1/sessions/{id}/recording/chapters",
            routing::get(list).post(create),
        )
        .route(
            "/v1/sessions/{id}/recording/chapters/{cid}",
            routing::delete(delete_one),
        )
}

fn is_org_admin(ctx: &RequestContext) -> bool {
    ctx.can_manage_organization()
}

/// Resolve the session for `session_id`, validate tenancy, and return its
/// `course_id`. Mirrors the front-matter of `live_sessions::recording_inner`:
/// the session must exist, be in the caller's active tenant, and (404 otherwise)
/// be visible to them.
async fn resolve_course_id(
    s: &AppState,
    ctx: &RequestContext,
    session_id: Uuid,
) -> Result<(Uuid, Uuid), ApiError> {
    let tenant_id = ctx
        .tenant_id
        .ok_or_else(|| ApiError::BadRequest("no tenant".into()))?;
    let session = db::live_sessions::load_for_join_with_context(
        &s.pool,
        ctx.user_id,
        ctx.tenant_id,
        session_id,
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?
    .ok_or(ApiError::NotFound)?;
    if session.tenant_id != tenant_id {
        return Err(ApiError::NotFound);
    }
    Ok((session.course_id, tenant_id))
}

/// Resolve the recording row id for a session, or 404 if there is no recording.
/// Runs inside an RLS tx so the lookup is tenant-scoped.
async fn resolve_recording_id(
    s: &AppState,
    ctx: &RequestContext,
    session_id: Uuid,
) -> Result<Uuid, ApiError> {
    let mut tx = db::begin_with_context(&s.pool, ctx.user_id, ctx.tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let row = db::recordings::fetch_by_session(&mut *tx, session_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::NotFound)?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(row.id)
}

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

async fn list(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(session_id): Path<Uuid>,
) -> Result<Json<Vec<ChapterDto>>, ApiError> {
    let (course_id, _tenant) = resolve_course_id(&s, &ctx, session_id).await?;
    require_course_read(&s, &ctx, course_id).await?;
    let recording_id = resolve_recording_id(&s, &ctx, session_id).await?;

    let mut tx = db::begin_with_context(&s.pool, ctx.user_id, ctx.tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let rows = db::recordings::list_chapters(&mut *tx, recording_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(rows.into_iter().map(ChapterDto::from).collect()))
}

async fn create(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(session_id): Path<Uuid>,
    Json(body): Json<CreateChapter>,
) -> Result<Json<ChapterDto>, ApiError> {
    let (course_id, tenant) = resolve_course_id(&s, &ctx, session_id).await?;
    require_course_staff(&s, &ctx, course_id).await?;

    let label = body.label.trim();
    if label.is_empty() {
        return Err(ApiError::Validation("label_required".into()));
    }
    // Validate by char count (not byte length) so multibyte labels aren't
    // rejected early.
    if label.chars().count() > MAX_LABEL_LEN {
        return Err(ApiError::Validation("label_too_long".into()));
    }
    if body.position_seconds < 0 {
        return Err(ApiError::Validation("position_negative".into()));
    }
    if body.position_seconds > MAX_POSITION_SECONDS {
        return Err(ApiError::Validation("position_out_of_range".into()));
    }
    let position_seconds = body.position_seconds as i32;

    let recording_id = resolve_recording_id(&s, &ctx, session_id).await?;

    let mut tx = db::begin_with_context(&s.pool, ctx.user_id, ctx.tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let row =
        db::recordings::insert_chapter(&mut tx, tenant, recording_id, label, position_seconds)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(ChapterDto::from(row)))
}

async fn delete_one(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path((session_id, chapter_id)): Path<(Uuid, Uuid)>,
) -> Result<axum::http::StatusCode, ApiError> {
    let (course_id, _tenant) = resolve_course_id(&s, &ctx, session_id).await?;
    require_course_staff(&s, &ctx, course_id).await?;
    let recording_id = resolve_recording_id(&s, &ctx, session_id).await?;

    let mut tx = db::begin_with_context(&s.pool, ctx.user_id, ctx.tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let deleted = db::recordings::delete_chapter(&mut tx, recording_id, chapter_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !deleted {
        return Err(ApiError::NotFound);
    }
    Ok(axum::http::StatusCode::NO_CONTENT)
}
