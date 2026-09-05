// crates/backend/src/handlers/rubrics.rs
//! Rubric-based grading for assignments. Staff create/replace/read/delete a
//! single rubric per assignment; the rubric's criteria drive per-criterion
//! scoring in the grade flow (see `handlers::submissions::grade_inner`).
//!
//! Routes (all under the authed router):
//!   * GET    /v1/assignments/{aid}/rubric  — read the rubric + criteria (staff)
//!   * POST   /v1/assignments/{aid}/rubric  — create/replace the rubric (staff)
//!   * PUT    /v1/assignments/{aid}/rubric  — alias of POST (create/replace)
//!   * DELETE /v1/assignments/{aid}/rubric  — remove the rubric (staff)
//!
//! The rubric links to its assignment via `rubrics.assignment_id`; the
//! `assignments` table itself is never altered. Everything is tenant-scoped
//! under RLS (see `db::rubrics`).
use axum::extract::{Extension, Path, State};
use axum::{routing, Json, Router};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::context::RequestContext;
use crate::db;
use crate::error::ApiError;
use crate::AppState;

const MAX_TITLE_LEN: usize = 200;
const MAX_LABEL_LEN: usize = 200;
const MAX_CRITERIA: usize = 50;
const MAX_CRITERION_POINTS: i32 = 1_000_000;

#[derive(Deserialize)]
pub struct CriterionInput {
    pub label: String,
    pub max_points: i32,
}

#[derive(Deserialize)]
pub struct UpsertRubric {
    pub title: String,
    pub criteria: Vec<CriterionInput>,
}

#[derive(Serialize)]
pub struct CriterionDto {
    pub id: Uuid,
    pub label: String,
    pub max_points: i32,
    pub sort_order: i32,
}

impl From<db::rubrics::CriterionRow> for CriterionDto {
    fn from(c: db::rubrics::CriterionRow) -> Self {
        Self {
            id: c.id,
            label: c.label,
            max_points: c.max_points,
            sort_order: c.sort_order,
        }
    }
}

#[derive(Serialize)]
pub struct RubricDto {
    pub id: Uuid,
    pub course_id: Uuid,
    pub assignment_id: Uuid,
    pub title: String,
    pub criteria: Vec<CriterionDto>,
}

pub fn routes() -> Router<AppState> {
    Router::new().route(
        "/v1/assignments/{aid}/rubric",
        routing::get(get_rubric)
            .post(upsert_rubric)
            .put(upsert_rubric)
            .delete(delete_rubric),
    )
}

#[doc(hidden)]
pub fn router_for_tests(pool: PgPool) -> Router {
    Router::new()
        .route(
            "/v1/assignments/{aid}/rubric",
            routing::get(get_rubric_t)
                .post(upsert_rubric_t)
                .put(upsert_rubric_t)
                .delete(delete_rubric_t),
        )
        .with_state(TestState { pool })
}

#[derive(Clone)]
struct TestState {
    pool: PgPool,
}

fn is_org_admin(ctx: &RequestContext) -> bool {
    ctx.can_manage_organization()
}

/// Course-scoped staff gate (course owner / active teacher-ta member / org_admin
/// / platform_admin). `platform_admin` always passes.
async fn require_course_staff(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
) -> Result<(), ApiError> {
    if !ctx.can_grade() {
        return Err(ApiError::Forbidden);
    }
    if !db::courses::caller_can_staff_course(
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

/// Resolve the assignment (404 if missing) and its course id, then enforce
/// course-scoped staff authorization. Returns the assignment's course id.
async fn staff_assignment_course(
    pool: &PgPool,
    ctx: &RequestContext,
    tenant: Uuid,
    aid: Uuid,
) -> Result<Uuid, ApiError> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    set_tenant(&mut tx, ctx.user_id, tenant)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let assn = db::assignments::fetch_by_id(&mut tx, aid)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::NotFound)?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    require_course_staff(pool, ctx, assn.course_id).await?;
    Ok(assn.course_id)
}

async fn get_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    aid: Uuid,
) -> Result<Json<Option<RubricDto>>, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    staff_assignment_course(pool, ctx, tenant, aid).await?;

    let Some(rubric) = db::rubrics::fetch_for_assignment(pool, tenant, aid)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
    else {
        return Ok(Json(None));
    };
    let criteria = db::rubrics::list_criteria(pool, tenant, rubric.id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(Some(RubricDto {
        id: rubric.id,
        course_id: rubric.course_id,
        assignment_id: rubric.assignment_id,
        title: rubric.title,
        criteria: criteria.into_iter().map(CriterionDto::from).collect(),
    })))
}

async fn upsert_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    aid: Uuid,
    body: UpsertRubric,
) -> Result<Json<RubricDto>, ApiError> {
    if !ctx.can_teach() {
        return Err(ApiError::Forbidden);
    }
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    let course_id = staff_assignment_course(pool, ctx, tenant, aid).await?;

    let title = body.title.trim();
    if title.is_empty() {
        return Err(ApiError::Validation("title_required".into()));
    }
    if title.chars().count() > MAX_TITLE_LEN {
        return Err(ApiError::Validation("title_too_long".into()));
    }
    if body.criteria.is_empty() {
        return Err(ApiError::Validation("criteria_required".into()));
    }
    if body.criteria.len() > MAX_CRITERIA {
        return Err(ApiError::Validation("too_many_criteria".into()));
    }

    // Normalize + validate each criterion, assigning sort_order by position.
    let mut prepared: Vec<(String, i32, i32)> = Vec::with_capacity(body.criteria.len());
    for (idx, c) in body.criteria.iter().enumerate() {
        let label = c.label.trim();
        if label.is_empty() {
            return Err(ApiError::Validation("criterion_label_required".into()));
        }
        if label.chars().count() > MAX_LABEL_LEN {
            return Err(ApiError::Validation("criterion_label_too_long".into()));
        }
        if c.max_points <= 0 || c.max_points > MAX_CRITERION_POINTS {
            return Err(ApiError::Validation(
                "criterion_max_points_out_of_range".into(),
            ));
        }
        prepared.push((label.to_string(), c.max_points, idx as i32));
    }

    let new_criteria: Vec<db::rubrics::NewCriterion<'_>> = prepared
        .iter()
        .map(
            |(label, max_points, sort_order)| db::rubrics::NewCriterion {
                label: label.as_str(),
                max_points: *max_points,
                sort_order: *sort_order,
            },
        )
        .collect();

    let (rubric, criteria) =
        db::rubrics::replace_for_assignment(pool, tenant, course_id, aid, title, &new_criteria)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(RubricDto {
        id: rubric.id,
        course_id: rubric.course_id,
        assignment_id: rubric.assignment_id,
        title: rubric.title,
        criteria: criteria.into_iter().map(CriterionDto::from).collect(),
    }))
}

async fn delete_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    aid: Uuid,
) -> Result<axum::http::StatusCode, ApiError> {
    if !ctx.can_teach() {
        return Err(ApiError::Forbidden);
    }
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    staff_assignment_course(pool, ctx, tenant, aid).await?;
    let deleted = db::rubrics::delete_for_assignment(pool, tenant, aid)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !deleted {
        return Err(ApiError::NotFound);
    }
    Ok(axum::http::StatusCode::NO_CONTENT)
}

// --- production handlers ---
async fn get_rubric(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(aid): Path<Uuid>,
) -> Result<Json<Option<RubricDto>>, ApiError> {
    get_inner(&s.pool, &ctx, aid).await
}
async fn upsert_rubric(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(aid): Path<Uuid>,
    Json(body): Json<UpsertRubric>,
) -> Result<Json<RubricDto>, ApiError> {
    upsert_inner(&s.pool, &ctx, aid, body).await
}
async fn delete_rubric(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(aid): Path<Uuid>,
) -> Result<axum::http::StatusCode, ApiError> {
    delete_inner(&s.pool, &ctx, aid).await
}

// --- test wrappers ---
async fn get_rubric_t(
    State(ts): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(aid): Path<Uuid>,
) -> Result<Json<Option<RubricDto>>, ApiError> {
    get_inner(&ts.pool, &ctx, aid).await
}
async fn upsert_rubric_t(
    State(ts): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(aid): Path<Uuid>,
    Json(body): Json<UpsertRubric>,
) -> Result<Json<RubricDto>, ApiError> {
    upsert_inner(&ts.pool, &ctx, aid, body).await
}
async fn delete_rubric_t(
    State(ts): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(aid): Path<Uuid>,
) -> Result<axum::http::StatusCode, ApiError> {
    delete_inner(&ts.pool, &ctx, aid).await
}
