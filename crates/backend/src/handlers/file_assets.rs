// crates/backend/src/handlers/file_assets.rs
use axum::extract::{Extension, Path, State};
use axum::{routing, Json, Router};
use serde::Serialize;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

use crate::context::RequestContext;
use crate::db;
use crate::error::ApiError;
use crate::storage::S3Client;
use crate::AppState;

const GET_TTL: Duration = Duration::from_secs(15 * 60);

#[derive(Serialize)]
pub struct UrlDto {
    pub url: String,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Serialize)]
pub struct LessonFileDto {
    pub asset_id: Uuid,
    pub filename: String,
    pub content_type: String,
    pub size_bytes: i64,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/file-assets/{id}/url", routing::get(get_url))
        .route("/v1/file-assets/{id}", routing::delete(delete_asset))
        .route("/v1/lessons/{lid}/files", routing::get(list_lesson_files))
}

#[doc(hidden)]
pub fn router_for_tests(pool: PgPool, storage: Arc<dyn S3Client>) -> Router {
    Router::new()
        .route("/v1/file-assets/{id}/url", routing::get(get_url_t))
        .route("/v1/file-assets/{id}", routing::delete(delete_asset_t))
        .route("/v1/lessons/{lid}/files", routing::get(list_lesson_files_t))
        .with_state(TestState { pool, storage })
}

#[derive(Clone)]
struct TestState {
    pool: PgPool,
    storage: Arc<dyn S3Client>,
}

fn is_org_admin(ctx: &RequestContext) -> bool {
    ctx.can_manage_organization()
}

async fn caller_can_read_linked_entity(
    pool: &PgPool,
    ctx: &RequestContext,
    asset: &db::file_assets::FileAssetRow,
) -> Result<bool, ApiError> {
    let course_id = match (asset.linked_entity_type.as_deref(), asset.linked_entity_id) {
        (Some("course"), Some(id)) => id,
        (Some("lesson"), Some(id)) => {
            let mut tx = db::begin_with_context(pool, ctx.user_id, ctx.tenant_id)
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?;
            let course_id =
                sqlx::query_scalar::<_, Uuid>("SELECT course_id FROM lessons WHERE id = $1")
                    .bind(id)
                    .fetch_optional(&mut *tx)
                    .await
                    .map_err(|e| ApiError::Internal(e.to_string()))?
                    .ok_or(ApiError::FileAssetNotFound)?;
            tx.commit()
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?;
            course_id
        }
        _ => return Ok(false),
    };
    db::courses::caller_can_read_course(
        pool,
        course_id,
        ctx.user_id,
        ctx.tenant_id,
        is_org_admin(ctx),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))
}

async fn caller_can_admin_linked_entity(
    pool: &PgPool,
    ctx: &RequestContext,
    asset: &db::file_assets::FileAssetRow,
) -> Result<bool, ApiError> {
    let course_id = match (asset.linked_entity_type.as_deref(), asset.linked_entity_id) {
        (Some("course"), Some(id)) => id,
        (Some("lesson"), Some(id)) => {
            let mut tx = db::begin_with_context(pool, ctx.user_id, ctx.tenant_id)
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?;
            let course_id =
                sqlx::query_scalar::<_, Uuid>("SELECT course_id FROM lessons WHERE id = $1")
                    .bind(id)
                    .fetch_optional(&mut *tx)
                    .await
                    .map_err(|e| ApiError::Internal(e.to_string()))?
                    .ok_or(ApiError::FileAssetNotFound)?;
            tx.commit()
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?;
            course_id
        }
        _ => return Ok(false),
    };
    db::courses::caller_can_admin_course(
        pool,
        course_id,
        ctx.user_id,
        ctx.tenant_id,
        is_org_admin(ctx),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))
}

async fn get_url(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<UrlDto>, ApiError> {
    get_url_inner(&s.pool, s.storage.as_ref(), &ctx, id).await
}
async fn delete_asset(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<axum::http::StatusCode, ApiError> {
    delete_inner(&s.pool, s.storage.as_ref(), &ctx, id).await
}
async fn list_lesson_files(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(lid): Path<Uuid>,
) -> Result<Json<Vec<LessonFileDto>>, ApiError> {
    list_lesson_files_inner(&s.pool, &ctx, lid).await
}

async fn get_url_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<UrlDto>, ApiError> {
    get_url_inner(&s.pool, s.storage.as_ref(), &ctx, id).await
}
async fn delete_asset_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<axum::http::StatusCode, ApiError> {
    delete_inner(&s.pool, s.storage.as_ref(), &ctx, id).await
}
async fn list_lesson_files_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(lid): Path<Uuid>,
) -> Result<Json<Vec<LessonFileDto>>, ApiError> {
    list_lesson_files_inner(&s.pool, &ctx, lid).await
}

async fn get_url_inner(
    pool: &PgPool,
    storage: &dyn S3Client,
    ctx: &RequestContext,
    id: Uuid,
) -> Result<Json<UrlDto>, ApiError> {
    let mut tx = db::begin_with_context(pool, ctx.user_id, ctx.tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let asset = db::file_assets::fetch(&mut *tx, id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::FileAssetNotFound)?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let tenant_id = ctx
        .tenant_id
        .ok_or_else(|| ApiError::BadRequest("no tenant".into()))?;
    if asset.tenant_id != tenant_id {
        return Err(ApiError::FileAssetNotFound);
    }
    if asset.status != "available" {
        return Err(ApiError::FileAssetNotFound);
    }
    if !caller_can_read_linked_entity(pool, ctx, &asset).await? {
        return Err(ApiError::FileAssetNotFound);
    }
    let url = storage
        .presigned_get_url(&asset.object_key, GET_TTL)
        .await
        .map_err(|e| ApiError::Internal(format!("presign get failed: {e}")))?;
    let expires_at = chrono::Utc::now() + chrono::Duration::seconds(GET_TTL.as_secs() as i64);
    Ok(Json(UrlDto { url, expires_at }))
}

async fn delete_inner(
    pool: &PgPool,
    storage: &dyn S3Client,
    ctx: &RequestContext,
    id: Uuid,
) -> Result<axum::http::StatusCode, ApiError> {
    let mut prefetch_tx = db::begin_with_context(pool, ctx.user_id, ctx.tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let asset = db::file_assets::fetch(&mut *prefetch_tx, id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::FileAssetNotFound)?;
    prefetch_tx
        .commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let tenant_id = ctx
        .tenant_id
        .ok_or_else(|| ApiError::BadRequest("no tenant".into()))?;
    if asset.tenant_id != tenant_id {
        return Err(ApiError::FileAssetNotFound);
    }
    if !caller_can_admin_linked_entity(pool, ctx, &asset).await? {
        return Err(ApiError::Forbidden);
    }

    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    db::set_request_guc(&mut tx, ctx.user_id, Some(tenant_id))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    db::file_assets::mark_pruned(&mut tx, id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    db::audit::emit_audit_event(
        &mut tx,
        tenant_id,
        ctx.user_id,
        "file_asset.delete",
        "file_asset",
        id,
        None,
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    if let Err(e) = storage.delete_object(&asset.object_key).await {
        tracing::warn!(?e, key = %asset.object_key, "object delete failed; row marked pruned anyway");
    }

    Ok(axum::http::StatusCode::NO_CONTENT)
}

async fn list_lesson_files_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    lid: Uuid,
) -> Result<Json<Vec<LessonFileDto>>, ApiError> {
    let mut lesson_tx = db::begin_with_context(pool, ctx.user_id, ctx.tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let course_id: Option<Uuid> = sqlx::query_scalar("SELECT course_id FROM lessons WHERE id = $1")
        .bind(lid)
        .fetch_optional(&mut *lesson_tx)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    lesson_tx
        .commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let course_id = course_id.ok_or(ApiError::NotFound)?;
    let allowed = db::courses::caller_can_read_course(
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

    let mut tx = db::begin_with_context(pool, ctx.user_id, ctx.tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let rows = db::file_assets::list_for_entity(&mut *tx, "lesson", lid)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(
        rows.into_iter()
            .map(|r| LessonFileDto {
                asset_id: r.id,
                filename: r
                    .object_key
                    .rsplit('/')
                    .next()
                    .unwrap_or("file")
                    .to_string(),
                content_type: r.content_type,
                size_bytes: r.size_bytes,
            })
            .collect(),
    ))
}
