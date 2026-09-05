// crates/backend/src/handlers/uploads.rs
use axum::extract::{Extension, Path, State};
use axum::{routing, Json, Router};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

use crate::context::RequestContext;
use crate::db;
use crate::error::ApiError;
use crate::services::file_assets as fa_svc;
use crate::storage::S3Client;
use crate::AppState;

const PUT_TTL: Duration = Duration::from_secs(15 * 60);

#[derive(Deserialize)]
pub struct BeginUpload {
    pub filename: String,
    pub content_type: String,
    pub size_bytes: i64,
    pub linked_entity_type: String,
    pub linked_entity_id: Uuid,
    pub purpose: String,
}

#[derive(Serialize)]
pub struct BeginUploadDto {
    pub asset_id: Uuid,
    pub presigned_put_url: String,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Serialize)]
pub struct AssetCompletedDto {
    pub asset_id: Uuid,
    pub status: String,
    pub size_bytes: i64,
    pub content_type: String,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/uploads/begin", routing::post(begin))
        .route("/v1/uploads/{asset_id}/complete", routing::post(complete))
}

#[doc(hidden)]
pub fn router_for_tests(pool: PgPool, storage: Arc<dyn S3Client>, bucket: String) -> Router {
    Router::new()
        .route("/v1/uploads/begin", routing::post(begin_t))
        .route("/v1/uploads/{asset_id}/complete", routing::post(complete_t))
        .with_state(TestState {
            pool,
            storage,
            bucket,
        })
}

#[derive(Clone)]
struct TestState {
    pool: PgPool,
    storage: Arc<dyn S3Client>,
    bucket: String,
}

fn is_org_admin(ctx: &RequestContext) -> bool {
    ctx.can_manage_organization()
}

async fn begin(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Json(b): Json<BeginUpload>,
) -> Result<Json<BeginUploadDto>, ApiError> {
    begin_inner(&s.pool, s.storage.as_ref(), &s.bucket_name, &ctx, b).await
}

async fn complete(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(asset_id): Path<Uuid>,
) -> Result<Json<AssetCompletedDto>, ApiError> {
    complete_inner(&s.pool, s.storage.as_ref(), &ctx, asset_id).await
}

async fn begin_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Json(b): Json<BeginUpload>,
) -> Result<Json<BeginUploadDto>, ApiError> {
    begin_inner(&s.pool, s.storage.as_ref(), &s.bucket, &ctx, b).await
}

async fn complete_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(asset_id): Path<Uuid>,
) -> Result<Json<AssetCompletedDto>, ApiError> {
    complete_inner(&s.pool, s.storage.as_ref(), &ctx, asset_id).await
}

async fn require_admin_for_course(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
) -> Result<(), ApiError> {
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
    Ok(())
}

async fn begin_inner(
    pool: &PgPool,
    storage: &dyn S3Client,
    bucket: &str,
    ctx: &RequestContext,
    b: BeginUpload,
) -> Result<Json<BeginUploadDto>, ApiError> {
    // Wrap the validation lookups in a tx with RLS GUCs so they work under
    // the non-superuser `aulalite_app` role. `require_admin_for_course`
    // self-manages its own tx so it's fine to call with `pool` after we
    // commit this validation tx.
    let mut val_tx = db::begin_with_context(pool, ctx.user_id, ctx.tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    match (b.linked_entity_type.as_str(), b.purpose.as_str()) {
        ("course", "cover") => {
            val_tx
                .commit()
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?;
            require_admin_for_course(pool, ctx, b.linked_entity_id).await?;
        }
        ("course", "scorm") => {
            val_tx
                .commit()
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?;
            require_admin_for_course(pool, ctx, b.linked_entity_id).await?;
        }
        ("lesson", "video") => {
            let course_id: Option<Uuid> =
                sqlx::query_scalar("SELECT course_id FROM lessons WHERE id = $1")
                    .bind(b.linked_entity_id)
                    .fetch_optional(&mut *val_tx)
                    .await
                    .map_err(|e| ApiError::Internal(e.to_string()))?;
            let course_id = course_id.ok_or(ApiError::NotFound)?;
            let lesson_type: String = sqlx::query_scalar("SELECT type FROM lessons WHERE id = $1")
                .bind(b.linked_entity_id)
                .fetch_one(&mut *val_tx)
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?;
            val_tx
                .commit()
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?;
            if lesson_type != "video" {
                return Err(ApiError::BadRequest("lesson is not type 'video'".into()));
            }
            require_admin_for_course(pool, ctx, course_id).await?;
        }
        ("lesson", "attachment") => {
            let row: Option<(Uuid, String)> =
                sqlx::query_as("SELECT course_id, type FROM lessons WHERE id = $1")
                    .bind(b.linked_entity_id)
                    .fetch_optional(&mut *val_tx)
                    .await
                    .map_err(|e| ApiError::Internal(e.to_string()))?;
            val_tx
                .commit()
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?;
            let (course_id, lesson_type) = row.ok_or(ApiError::NotFound)?;
            if lesson_type != "file_bundle" {
                return Err(ApiError::BadRequest(
                    "lesson is not type 'file_bundle'".into(),
                ));
            }
            require_admin_for_course(pool, ctx, course_id).await?;
        }
        ("assignment_attachment", "attachment") => {
            let course_id: Option<Uuid> =
                sqlx::query_scalar("SELECT course_id FROM assignments WHERE id = $1")
                    .bind(b.linked_entity_id)
                    .fetch_optional(&mut *val_tx)
                    .await
                    .map_err(|e| ApiError::Internal(e.to_string()))?;
            val_tx
                .commit()
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?;
            let course_id = course_id.ok_or(ApiError::NotFound)?;
            require_admin_for_course(pool, ctx, course_id).await?;
        }
        ("submission_attachment", "attachment") => {
            let row: Option<(Uuid, Uuid)> =
                sqlx::query_as("SELECT student_user_id, course_id FROM submissions WHERE id = $1")
                    .bind(b.linked_entity_id)
                    .fetch_optional(&mut *val_tx)
                    .await
                    .map_err(|e| ApiError::Internal(e.to_string()))?;
            val_tx
                .commit()
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?;
            let (student_user_id, course_id) = row.ok_or(ApiError::NotFound)?;
            // Owner can upload to their submission OR a teacher/admin can upload (e.g., teacher annotations).
            if ctx.user_id != student_user_id {
                require_admin_for_course(pool, ctx, course_id).await?;
            }
        }
        ("live_session", "whiteboard") => {
            // Whiteboard paste/drop image: authorize anyone who can READ the
            // session's course (teachers + enrolled students; students may paste
            // only while the teacher has opened the board — read access is the
            // right coarse gate for the upload itself).
            let course_id: Option<Uuid> =
                sqlx::query_scalar("SELECT course_id FROM live_sessions WHERE id = $1")
                    .bind(b.linked_entity_id)
                    .fetch_optional(&mut *val_tx)
                    .await
                    .map_err(|e| ApiError::Internal(e.to_string()))?;
            val_tx
                .commit()
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?;
            let course_id = course_id.ok_or(ApiError::NotFound)?;
            let can_read = db::courses::caller_can_read_course(
                pool,
                course_id,
                ctx.user_id,
                ctx.tenant_id,
                is_org_admin(ctx),
            )
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
            if !can_read {
                return Err(ApiError::Forbidden);
            }
        }
        _ => {
            return Err(ApiError::BadRequest(format!(
                "unsupported (entity_type, purpose) pair: ({}, {})",
                b.linked_entity_type, b.purpose
            )));
        }
    }

    fa_svc::validate_request(&b.purpose, &b.content_type, b.size_bytes)
        .map_err(|e| ApiError::UploadValidationFailed(e.to_string()))?;

    let tenant_id = ctx
        .tenant_id
        .ok_or_else(|| ApiError::BadRequest("no tenant".into()))?;

    let asset_id = Uuid::new_v4();
    let now = chrono::Utc::now();
    let object_key = fa_svc::object_key(tenant_id, asset_id, &b.filename, now);

    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    db::set_request_guc(&mut tx, ctx.user_id, Some(tenant_id))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    // Insert pending row using the pre-generated UUID for object_key consistency.
    sqlx::query(
        "INSERT INTO file_assets
            (id, tenant_id, owner_user_id, bucket, object_key, content_type,
             size_bytes, status, visibility, linked_entity_type, linked_entity_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7, 'pending', 'private', $8, $9)",
    )
    .bind(asset_id)
    .bind(tenant_id)
    .bind(ctx.user_id)
    .bind(bucket)
    .bind(&object_key)
    .bind(&b.content_type)
    .bind(b.size_bytes)
    .bind(&b.linked_entity_type)
    .bind(b.linked_entity_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;

    db::audit::emit_audit_event(
        &mut tx,
        tenant_id,
        ctx.user_id,
        "file_asset.begin",
        "file_asset",
        asset_id,
        None,
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;

    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let presigned_put_url = storage
        .presigned_put_url(&object_key, &b.content_type, b.size_bytes, PUT_TTL)
        .await
        .map_err(|e| ApiError::Internal(format!("presign failed: {e}")))?;
    let expires_at = now + chrono::Duration::seconds(PUT_TTL.as_secs() as i64);

    Ok(Json(BeginUploadDto {
        asset_id,
        presigned_put_url,
        expires_at,
    }))
}

async fn complete_inner(
    pool: &PgPool,
    storage: &dyn S3Client,
    ctx: &RequestContext,
    asset_id: Uuid,
) -> Result<Json<AssetCompletedDto>, ApiError> {
    let mut prefetch_tx = db::begin_with_context(pool, ctx.user_id, ctx.tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let asset = db::file_assets::fetch(&mut *prefetch_tx, asset_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::FileAssetNotFound)?;
    prefetch_tx
        .commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    if asset.owner_user_id != ctx.user_id {
        return Err(ApiError::Forbidden);
    }

    if asset.status == "available" {
        return Ok(Json(AssetCompletedDto {
            asset_id: asset.id,
            status: asset.status,
            size_bytes: asset.size_bytes,
            content_type: asset.content_type,
        }));
    }
    if asset.status == "failed" {
        return Err(ApiError::UploadObjectMissing);
    }

    let head = storage.head_object(&asset.object_key).await;
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    db::set_request_guc(&mut tx, ctx.user_id, Some(asset.tenant_id))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    match head {
        Ok(h)
            if h.size_bytes == asset.size_bytes
                && content_type_matches(&h, &asset.content_type) =>
        {
            let updated = db::file_assets::mark_available(&mut tx, asset_id)
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?
                .ok_or(ApiError::FileAssetNotFound)?;
            db::audit::emit_audit_event(
                &mut tx,
                asset.tenant_id,
                ctx.user_id,
                "file_asset.complete",
                "file_asset",
                asset_id,
                None,
            )
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
            tx.commit()
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?;
            Ok(Json(AssetCompletedDto {
                asset_id: updated.id,
                status: updated.status,
                size_bytes: updated.size_bytes,
                content_type: updated.content_type,
            }))
        }
        // Object present but wrong size/content-type, or definitively absent
        // (NotFound): a terminal failure — mark the asset failed.
        Ok(_) | Err(crate::storage::StorageError::NotFound(_)) => {
            db::file_assets::mark_failed(&mut tx, asset_id)
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?;
            tx.commit()
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?;
            Err(ApiError::UploadObjectMissing)
        }
        // Transient storage error (network blip / 5xx): do NOT mark the asset
        // failed — leave it `pending` so the client can retry /complete. Roll
        // back the (otherwise empty) tx and return a retryable 503.
        Err(_) => {
            drop(tx);
            Err(ApiError::StorageUnavailable)
        }
    }
}

/// Returns true when the server-observed content-type for the uploaded
/// object matches what the client declared at `begin`. S3-compatible servers
/// may omit the header from HEAD responses; in that case we trust the
/// presigned-URL signature (which already binds the content-type) and
/// accept the upload. When the header is present we require a strict
/// case-insensitive match — clients cannot upload `application/x-sh` while
/// claiming `image/jpeg`.
fn content_type_matches(head: &crate::storage::ObjectHead, expected: &str) -> bool {
    match &head.content_type {
        Some(observed) => observed.eq_ignore_ascii_case(expected),
        None => true,
    }
}
