// crates/backend/src/handlers/modules.rs
use axum::extract::{Extension, Path, State};
use axum::{routing, Json, Router};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::context::RequestContext;
use crate::db;
use crate::error::ApiError;
use crate::AppState;

#[derive(Deserialize)]
pub struct CreateModule {
    pub title: String,
}

#[derive(Deserialize)]
pub struct ReorderRequest {
    pub module_ids: Vec<Uuid>,
}

#[derive(Deserialize)]
pub struct PatchModule {
    pub title: Option<String>,
}

#[derive(Serialize)]
pub struct ModuleDto {
    pub id: Uuid,
    pub course_id: Uuid,
    pub title: String,
    pub sort_order: i32,
}

impl From<db::modules::ModuleRow> for ModuleDto {
    fn from(r: db::modules::ModuleRow) -> Self {
        Self {
            id: r.id,
            course_id: r.course_id,
            title: r.title,
            sort_order: r.sort_order,
        }
    }
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/courses/{cid}/modules", routing::post(create))
        .route("/v1/courses/{cid}/modules/reorder", routing::post(reorder))
        .route(
            "/v1/courses/{cid}/modules/{mid}",
            routing::patch(patch).delete(delete_one),
        )
}

#[doc(hidden)]
pub fn router_for_tests(pool: PgPool) -> Router {
    Router::new()
        .route("/v1/courses/{cid}/modules", routing::post(create_t))
        .route(
            "/v1/courses/{cid}/modules/reorder",
            routing::post(reorder_t),
        )
        .route(
            "/v1/courses/{cid}/modules/{mid}",
            routing::patch(patch_t).delete(delete_one_t),
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

// Production handlers
async fn create(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(course_id): Path<Uuid>,
    Json(body): Json<CreateModule>,
) -> Result<Json<ModuleDto>, ApiError> {
    create_inner(&s.pool, &ctx, course_id, body).await
}
async fn reorder(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(course_id): Path<Uuid>,
    Json(body): Json<ReorderRequest>,
) -> Result<axum::http::StatusCode, ApiError> {
    reorder_inner(&s.pool, &ctx, course_id, body).await
}
async fn patch(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path((cid, mid)): Path<(Uuid, Uuid)>,
    Json(body): Json<PatchModule>,
) -> Result<Json<ModuleDto>, ApiError> {
    patch_inner(&s.pool, &ctx, cid, mid, body).await
}
async fn delete_one(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path((cid, mid)): Path<(Uuid, Uuid)>,
) -> Result<axum::http::StatusCode, ApiError> {
    delete_inner(&s.pool, &ctx, cid, mid).await
}

// Test mirrors
async fn create_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(course_id): Path<Uuid>,
    Json(body): Json<CreateModule>,
) -> Result<Json<ModuleDto>, ApiError> {
    create_inner(&s.pool, &ctx, course_id, body).await
}
async fn reorder_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(course_id): Path<Uuid>,
    Json(body): Json<ReorderRequest>,
) -> Result<axum::http::StatusCode, ApiError> {
    reorder_inner(&s.pool, &ctx, course_id, body).await
}
async fn patch_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path((cid, mid)): Path<(Uuid, Uuid)>,
    Json(body): Json<PatchModule>,
) -> Result<Json<ModuleDto>, ApiError> {
    patch_inner(&s.pool, &ctx, cid, mid, body).await
}
async fn delete_one_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path((cid, mid)): Path<(Uuid, Uuid)>,
) -> Result<axum::http::StatusCode, ApiError> {
    delete_inner(&s.pool, &ctx, cid, mid).await
}

// Inner logic
async fn create_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
    body: CreateModule,
) -> Result<Json<ModuleDto>, ApiError> {
    let allowed = db::courses::caller_can_admin_course(
        pool,
        course_id,
        ctx.user_id,
        ctx.tenant_id,
        is_org_admin(ctx),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !allowed {
        return Err(ApiError::Forbidden);
    }
    let tenant_id = ctx
        .tenant_id
        .ok_or_else(|| ApiError::BadRequest("no tenant".into()))?;
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    db::set_request_guc(&mut tx, ctx.user_id, Some(tenant_id))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let so = db::modules::next_sort_order(&mut tx, course_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let row = db::modules::insert_module(&mut tx, tenant_id, course_id, &body.title, so)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    db::audit::emit_audit_event(
        &mut tx,
        tenant_id,
        ctx.user_id,
        "module.create",
        "module",
        row.id,
        None,
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(row.into()))
}

async fn reorder_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
    body: ReorderRequest,
) -> Result<axum::http::StatusCode, ApiError> {
    let allowed = db::courses::caller_can_admin_course(
        pool,
        course_id,
        ctx.user_id,
        ctx.tenant_id,
        is_org_admin(ctx),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !allowed {
        return Err(ApiError::Forbidden);
    }
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    db::set_request_guc(&mut tx, ctx.user_id, ctx.tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    db::modules::reorder(&mut tx, course_id, &body.module_ids)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(axum::http::StatusCode::OK)
}

async fn patch_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
    module_id: Uuid,
    body: PatchModule,
) -> Result<Json<ModuleDto>, ApiError> {
    let allowed = db::courses::caller_can_admin_course(
        pool,
        course_id,
        ctx.user_id,
        ctx.tenant_id,
        is_org_admin(ctx),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !allowed {
        return Err(ApiError::Forbidden);
    }
    let tenant_id = ctx
        .tenant_id
        .ok_or_else(|| ApiError::BadRequest("no tenant".into()))?;
    let title = body
        .title
        .ok_or_else(|| ApiError::BadRequest("nothing to update".into()))?;
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    db::set_request_guc(&mut tx, ctx.user_id, Some(tenant_id))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let row = db::modules::update_title(&mut tx, tenant_id, course_id, module_id, &title)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::NotFound)?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(row.into()))
}

async fn delete_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
    module_id: Uuid,
) -> Result<axum::http::StatusCode, ApiError> {
    let allowed = db::courses::caller_can_admin_course(
        pool,
        course_id,
        ctx.user_id,
        ctx.tenant_id,
        is_org_admin(ctx),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !allowed {
        return Err(ApiError::Forbidden);
    }
    let tenant_id = ctx
        .tenant_id
        .ok_or_else(|| ApiError::BadRequest("no tenant".into()))?;
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    db::set_request_guc(&mut tx, ctx.user_id, Some(tenant_id))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let deleted = db::modules::delete_module(&mut tx, tenant_id, course_id, module_id)
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
