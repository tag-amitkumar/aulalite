// crates/backend/src/handlers/assignments.rs
use axum::extract::{Extension, Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::{routing, Json, Router};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::context::RequestContext;
use crate::db;
use crate::error::ApiError;
use crate::AppState;

#[derive(Deserialize)]
pub struct CreateAssignment {
    pub title: String,
    #[serde(default)]
    pub instructions_md: String,
    pub grading_mode: String,
    pub max_points: Option<i32>,
    pub lesson_id: Option<Uuid>,
    #[serde(default = "default_true")]
    pub allow_late: bool,
    #[serde(default)]
    pub lock_on_submit: bool,
    #[serde(default = "default_true")]
    pub accepts_text: bool,
    #[serde(default = "default_true")]
    pub accepts_files: bool,
    #[serde(default = "default_release_mode")]
    pub release_mode: String,
    #[serde(default)]
    pub late_penalty_percent: i32,
    #[serde(default)]
    pub max_resubmissions: i32,
    pub due_at: Option<DateTime<Utc>>,
}

fn default_true() -> bool {
    true
}
fn default_release_mode() -> String {
    "instant".into()
}

#[derive(Deserialize, Default)]
pub struct PatchAssignment {
    pub title: Option<String>,
    pub instructions_md: Option<String>,
    pub grading_mode: Option<String>,
    #[serde(default, with = "::serde_with::rust::double_option")]
    pub max_points: Option<Option<i32>>,
    pub allow_late: Option<bool>,
    pub lock_on_submit: Option<bool>,
    pub accepts_text: Option<bool>,
    pub accepts_files: Option<bool>,
    pub release_mode: Option<String>,
    pub late_penalty_percent: Option<i32>,
    pub max_resubmissions: Option<i32>,
    pub attachment_asset_ids: Option<Vec<Uuid>>,
    #[serde(default, with = "::serde_with::rust::double_option")]
    pub due_at: Option<Option<DateTime<Utc>>>,
    #[serde(default, with = "::serde_with::rust::double_option")]
    pub lesson_id: Option<Option<Uuid>>,
}

#[derive(Serialize)]
pub struct AssignmentDto {
    pub id: Uuid,
    pub course_id: Uuid,
    pub lesson_id: Option<Uuid>,
    pub title: String,
    pub instructions_md: String,
    pub grading_mode: String,
    pub max_points: Option<i32>,
    pub allow_late: bool,
    pub lock_on_submit: bool,
    pub accepts_text: bool,
    pub accepts_files: bool,
    pub release_mode: String,
    pub late_penalty_percent: i32,
    pub max_resubmissions: i32,
    pub attachment_asset_ids: Vec<Uuid>,
    pub due_at: Option<DateTime<Utc>>,
    pub status: String,
    pub published_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<db::assignments::AssignmentRow> for AssignmentDto {
    fn from(r: db::assignments::AssignmentRow) -> Self {
        Self {
            id: r.id,
            course_id: r.course_id,
            lesson_id: r.lesson_id,
            title: r.title,
            instructions_md: r.instructions_md,
            grading_mode: r.grading_mode,
            max_points: r.max_points,
            allow_late: r.allow_late,
            lock_on_submit: r.lock_on_submit,
            accepts_text: r.accepts_text,
            accepts_files: r.accepts_files,
            release_mode: r.release_mode,
            late_penalty_percent: r.late_penalty_percent,
            max_resubmissions: r.max_resubmissions,
            attachment_asset_ids: r.attachment_asset_ids,
            due_at: r.due_at,
            status: r.status,
            published_at: r.published_at,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

#[derive(Deserialize, Default)]
pub struct ListQuery {
    #[serde(default)]
    pub include_drafts: bool,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/v1/courses/{cid}/assignments",
            routing::post(create).get(list_for_course),
        )
        .route(
            "/v1/lessons/{lid}/assignments",
            routing::get(list_for_lesson),
        )
        .route(
            "/v1/assignments/{id}",
            routing::get(get_one).patch(patch).delete(delete_one),
        )
        .route("/v1/assignments/{id}/publish", routing::post(publish))
        .route("/v1/assignments/{id}/unpublish", routing::post(unpublish))
}

#[doc(hidden)]
pub fn router_for_tests(pool: PgPool) -> Router {
    Router::new()
        .route(
            "/v1/courses/{cid}/assignments",
            routing::post(create_t).get(list_for_course_t),
        )
        .route(
            "/v1/lessons/{lid}/assignments",
            routing::get(list_for_lesson_t),
        )
        .route(
            "/v1/assignments/{id}",
            routing::get(get_one_t).patch(patch_t).delete(delete_one_t),
        )
        .route("/v1/assignments/{id}/publish", routing::post(publish_t))
        .route("/v1/assignments/{id}/unpublish", routing::post(unpublish_t))
        .with_state(TestState { pool })
}

#[derive(Clone)]
struct TestState {
    pool: PgPool,
}

fn require_teacher(ctx: &RequestContext) -> Result<(), ApiError> {
    if ctx.can_teach() {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
    }
}

fn is_org_admin(ctx: &RequestContext) -> bool {
    ctx.can_manage_organization()
}

async fn can_course_staff(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
) -> Result<bool, ApiError> {
    if ctx.can_manage_organization() {
        return Ok(true);
    }
    db::courses::caller_can_staff_course(
        pool,
        course_id,
        ctx.user_id,
        ctx.tenant_id,
        is_org_admin(ctx),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))
}

/// Course-scoped staff gate. `platform_admin` always bypasses; everyone else
/// must own the course or be an active teacher/ta member (org_admin via
/// `caller_can_staff_course`).
async fn require_course_staff(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
) -> Result<(), ApiError> {
    if !can_course_staff(pool, ctx, course_id).await? {
        return Err(ApiError::Forbidden);
    }
    Ok(())
}

async fn require_course_read(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
) -> Result<(), ApiError> {
    if !db::courses::caller_can_read_course(
        pool,
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

async fn set_tenant(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: Uuid,
    tenant: Uuid,
) -> sqlx::Result<()> {
    db::set_request_guc(tx, user_id, Some(tenant)).await
}

// --- shared inner functions ---

/// Reject unknown `grading_mode` / `release_mode` values before they reach the
/// Postgres `::assignment_grading_mode` / `::assignment_release_mode` casts —
/// an invalid label would otherwise fail the cast deep in the DB layer and
/// surface as an opaque 500 instead of a clean 400.
fn validate_assignment_modes(grading_mode: &str, release_mode: &str) -> Result<(), ApiError> {
    if !matches!(grading_mode, "numeric" | "pass_fail") {
        return Err(ApiError::Validation(format!(
            "invalid grading_mode '{grading_mode}' (expected 'numeric' or 'pass_fail')"
        )));
    }
    if !matches!(release_mode, "instant" | "manual") {
        return Err(ApiError::Validation(format!(
            "invalid release_mode '{release_mode}' (expected 'instant' or 'manual')"
        )));
    }
    Ok(())
}

/// Reject out-of-range late-submission / resubmission policy values with a
/// clean 422 (ApiError::Validation) before they hit the DB CHECK constraints
/// and surface as opaque 500s. `late_penalty_percent` must be 0..=100;
/// `max_resubmissions` must be >= 0.
fn validate_assignment_policy(
    late_penalty_percent: i32,
    max_resubmissions: i32,
) -> Result<(), ApiError> {
    if !(0..=100).contains(&late_penalty_percent) {
        return Err(ApiError::Validation(format!(
            "invalid late_penalty_percent {late_penalty_percent} (expected 0..=100)"
        )));
    }
    if max_resubmissions < 0 {
        return Err(ApiError::Validation(format!(
            "invalid max_resubmissions {max_resubmissions} (expected >= 0)"
        )));
    }
    Ok(())
}

async fn create_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    cid: Uuid,
    body: CreateAssignment,
) -> Result<(StatusCode, Json<AssignmentDto>), ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    require_teacher(ctx)?;
    require_course_staff(pool, ctx, cid).await?;
    validate_assignment_modes(&body.grading_mode, &body.release_mode)?;
    validate_assignment_policy(body.late_penalty_percent, body.max_resubmissions)?;
    if body.grading_mode == "numeric" && body.max_points.is_none() {
        return Err(ApiError::Validation(
            "numeric mode requires max_points".into(),
        ));
    }
    if body.grading_mode == "pass_fail" && body.max_points.is_some() {
        return Err(ApiError::Validation(
            "pass_fail mode forbids max_points".into(),
        ));
    }
    if !body.accepts_text && !body.accepts_files {
        return Err(ApiError::Validation(
            "at least one accepted submission type required".into(),
        ));
    }
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    set_tenant(&mut tx, ctx.user_id, tenant)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let row = db::assignments::insert(
        &mut tx,
        db::assignments::InsertAssignment {
            tenant_id: tenant,
            course_id: cid,
            lesson_id: body.lesson_id,
            title: &body.title,
            instructions_md: &body.instructions_md,
            grading_mode: &body.grading_mode,
            max_points: body.max_points,
            allow_late: body.allow_late,
            lock_on_submit: body.lock_on_submit,
            accepts_text: body.accepts_text,
            accepts_files: body.accepts_files,
            release_mode: &body.release_mode,
            late_penalty_percent: body.late_penalty_percent,
            max_resubmissions: body.max_resubmissions,
            due_at: body.due_at,
            created_by: ctx.user_id,
        },
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    db::audit::emit_audit_event(
        &mut tx,
        tenant,
        ctx.user_id,
        "assignment.create",
        "assignment",
        row.id,
        None,
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok((StatusCode::CREATED, Json(AssignmentDto::from(row))))
}

async fn get_one_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    id: Uuid,
) -> Result<Json<AssignmentDto>, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    set_tenant(&mut tx, ctx.user_id, tenant)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let row = db::assignments::fetch_by_id(&mut tx, id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::NotFound)?;
    require_course_read(pool, ctx, row.course_id).await?;
    if row.status == "draft" && !can_course_staff(pool, ctx, row.course_id).await? {
        return Err(ApiError::NotFound);
    }
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(AssignmentDto::from(row)))
}

async fn list_course_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    cid: Uuid,
    q: ListQuery,
) -> Result<Json<Vec<AssignmentDto>>, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    require_course_read(pool, ctx, cid).await?;
    let include_drafts = q.include_drafts && can_course_staff(pool, ctx, cid).await?;
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    set_tenant(&mut tx, ctx.user_id, tenant)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let rows = db::assignments::list_by_course(&mut tx, cid, include_drafts)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(rows.into_iter().map(AssignmentDto::from).collect()))
}

async fn list_lesson_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    lid: Uuid,
    q: ListQuery,
) -> Result<Json<Vec<AssignmentDto>>, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    set_tenant(&mut tx, ctx.user_id, tenant)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let course_id: Uuid =
        sqlx::query_scalar("SELECT course_id FROM lessons WHERE id = $1 AND tenant_id = $2")
            .bind(lid)
            .bind(tenant)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?
            .ok_or(ApiError::NotFound)?;
    require_course_read(pool, ctx, course_id).await?;
    let include_drafts = q.include_drafts && can_course_staff(pool, ctx, course_id).await?;
    let rows = db::assignments::list_by_lesson(&mut tx, lid, include_drafts)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(rows.into_iter().map(AssignmentDto::from).collect()))
}

async fn patch_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    id: Uuid,
    body: PatchAssignment,
) -> Result<Json<AssignmentDto>, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    require_teacher(ctx)?;
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    set_tenant(&mut tx, ctx.user_id, tenant)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let existing = db::assignments::fetch_by_id(&mut tx, id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::NotFound)?;
    require_course_staff(pool, ctx, existing.course_id).await?;
    if existing.status != "draft" {
        return Err(ApiError::Conflict("assignment_published".into()));
    }

    // Re-run invariants on the post-patch state.
    let new_grading_mode = body
        .grading_mode
        .as_deref()
        .unwrap_or(&existing.grading_mode);
    let new_max_points = match body.max_points {
        Some(opt) => opt,
        None => existing.max_points,
    };
    let new_accepts_text = body.accepts_text.unwrap_or(existing.accepts_text);
    let new_accepts_files = body.accepts_files.unwrap_or(existing.accepts_files);
    let new_release_mode = body
        .release_mode
        .as_deref()
        .unwrap_or(&existing.release_mode);
    let new_late_penalty_percent = body
        .late_penalty_percent
        .unwrap_or(existing.late_penalty_percent);
    let new_max_resubmissions = body.max_resubmissions.unwrap_or(existing.max_resubmissions);
    validate_assignment_modes(new_grading_mode, new_release_mode)?;
    validate_assignment_policy(new_late_penalty_percent, new_max_resubmissions)?;
    if new_grading_mode == "numeric" && new_max_points.is_none() {
        return Err(ApiError::Validation(
            "numeric mode requires max_points".into(),
        ));
    }
    if new_grading_mode == "pass_fail" && new_max_points.is_some() {
        return Err(ApiError::Validation(
            "pass_fail mode forbids max_points".into(),
        ));
    }
    if !new_accepts_text && !new_accepts_files {
        return Err(ApiError::Validation(
            "at least one accepted submission type required".into(),
        ));
    }

    let row = db::assignments::patch(
        &mut tx,
        id,
        db::assignments::PatchAssignment {
            title: body.title.as_deref(),
            instructions_md: body.instructions_md.as_deref(),
            grading_mode: body.grading_mode.as_deref(),
            max_points: body.max_points,
            allow_late: body.allow_late,
            lock_on_submit: body.lock_on_submit,
            accepts_text: body.accepts_text,
            accepts_files: body.accepts_files,
            release_mode: body.release_mode.as_deref(),
            late_penalty_percent: body.late_penalty_percent,
            max_resubmissions: body.max_resubmissions,
            attachment_asset_ids: body.attachment_asset_ids.as_deref(),
            due_at: body.due_at,
            lesson_id: body.lesson_id,
        },
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    db::audit::emit_audit_event(
        &mut tx,
        tenant,
        ctx.user_id,
        "assignment.update",
        "assignment",
        row.id,
        None,
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(AssignmentDto::from(row)))
}

async fn publish_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    id: Uuid,
) -> Result<Json<AssignmentDto>, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    require_teacher(ctx)?;
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    set_tenant(&mut tx, ctx.user_id, tenant)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let existing = db::assignments::fetch_by_id(&mut tx, id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::NotFound)?;
    require_course_staff(pool, ctx, existing.course_id).await?;
    let row = db::assignments::publish(&mut tx, id)
        .await
        .map_err(|e| match e {
            sqlx::Error::RowNotFound => ApiError::Conflict("not_in_draft".into()),
            other => ApiError::Internal(other.to_string()),
        })?;
    db::audit::emit_audit_event(
        &mut tx,
        tenant,
        ctx.user_id,
        "assignment.publish",
        "assignment",
        row.id,
        None,
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(AssignmentDto::from(row)))
}

async fn unpublish_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    id: Uuid,
) -> Result<Json<AssignmentDto>, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    require_teacher(ctx)?;
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    set_tenant(&mut tx, ctx.user_id, tenant)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let existing = db::assignments::fetch_by_id(&mut tx, id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::NotFound)?;
    require_course_staff(pool, ctx, existing.course_id).await?;
    let count = db::assignments::count_submissions(&mut tx, id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if count > 0 {
        return Err(ApiError::Conflict("assignment_has_submissions".into()));
    }
    let row = db::assignments::unpublish(&mut tx, id)
        .await
        .map_err(|e| match e {
            sqlx::Error::RowNotFound => ApiError::Conflict("not_published".into()),
            other => ApiError::Internal(other.to_string()),
        })?;
    db::audit::emit_audit_event(
        &mut tx,
        tenant,
        ctx.user_id,
        "assignment.unpublish",
        "assignment",
        row.id,
        None,
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(AssignmentDto::from(row)))
}

async fn delete_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    id: Uuid,
) -> Result<StatusCode, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    require_teacher(ctx)?;
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    set_tenant(&mut tx, ctx.user_id, tenant)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let existing = db::assignments::fetch_by_id(&mut tx, id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::NotFound)?;
    require_course_staff(pool, ctx, existing.course_id).await?;
    let count = db::assignments::count_submissions(&mut tx, id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if count > 0 {
        return Err(ApiError::Conflict("assignment_has_submissions".into()));
    }
    let n = db::assignments::delete(&mut tx, id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if n == 0 {
        return Err(ApiError::Conflict("not_in_draft_or_not_found".into()));
    }
    db::audit::emit_audit_event(
        &mut tx,
        tenant,
        ctx.user_id,
        "assignment.delete",
        "assignment",
        id,
        None,
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

// --- production handlers ---
async fn create(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(cid): Path<Uuid>,
    Json(body): Json<CreateAssignment>,
) -> Result<impl IntoResponse, ApiError> {
    create_inner(&s.pool, &ctx, cid, body).await
}
async fn get_one(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<AssignmentDto>, ApiError> {
    get_one_inner(&s.pool, &ctx, id).await
}
async fn list_for_course(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(cid): Path<Uuid>,
    Query(q): Query<ListQuery>,
) -> Result<Json<Vec<AssignmentDto>>, ApiError> {
    list_course_inner(&s.pool, &ctx, cid, q).await
}
async fn list_for_lesson(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(lid): Path<Uuid>,
    Query(q): Query<ListQuery>,
) -> Result<Json<Vec<AssignmentDto>>, ApiError> {
    list_lesson_inner(&s.pool, &ctx, lid, q).await
}
async fn patch(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    Json(body): Json<PatchAssignment>,
) -> Result<Json<AssignmentDto>, ApiError> {
    patch_inner(&s.pool, &ctx, id, body).await
}
async fn publish(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<AssignmentDto>, ApiError> {
    publish_inner(&s.pool, &ctx, id).await
}
async fn unpublish(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<AssignmentDto>, ApiError> {
    unpublish_inner(&s.pool, &ctx, id).await
}
async fn delete_one(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    delete_inner(&s.pool, &ctx, id).await
}

// --- test wrappers ---
async fn create_t(
    State(ts): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(cid): Path<Uuid>,
    Json(body): Json<CreateAssignment>,
) -> Result<impl IntoResponse, ApiError> {
    create_inner(&ts.pool, &ctx, cid, body).await
}
async fn get_one_t(
    State(ts): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<AssignmentDto>, ApiError> {
    get_one_inner(&ts.pool, &ctx, id).await
}
async fn list_for_course_t(
    State(ts): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(cid): Path<Uuid>,
    Query(q): Query<ListQuery>,
) -> Result<Json<Vec<AssignmentDto>>, ApiError> {
    list_course_inner(&ts.pool, &ctx, cid, q).await
}
async fn list_for_lesson_t(
    State(ts): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(lid): Path<Uuid>,
    Query(q): Query<ListQuery>,
) -> Result<Json<Vec<AssignmentDto>>, ApiError> {
    list_lesson_inner(&ts.pool, &ctx, lid, q).await
}
async fn patch_t(
    State(ts): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    Json(body): Json<PatchAssignment>,
) -> Result<Json<AssignmentDto>, ApiError> {
    patch_inner(&ts.pool, &ctx, id, body).await
}
async fn publish_t(
    State(ts): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<AssignmentDto>, ApiError> {
    publish_inner(&ts.pool, &ctx, id).await
}
async fn unpublish_t(
    State(ts): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<AssignmentDto>, ApiError> {
    unpublish_inner(&ts.pool, &ctx, id).await
}
async fn delete_one_t(
    State(ts): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    delete_inner(&ts.pool, &ctx, id).await
}
