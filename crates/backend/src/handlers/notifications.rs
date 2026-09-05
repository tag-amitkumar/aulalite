// crates/backend/src/handlers/notifications.rs
//! Authed self-service notification endpoints (the caller's OWN data only):
//!   * the in-app notification feed + unread count + read/read-all;
//!   * push device-token register/remove;
//!   * per-user notification preferences (read + patch).
//!
//! Every endpoint scopes strictly to `ctx.user_id`; tenant-scoped tables
//! (`notifications`, `device_tokens`) additionally require `ctx.tenant_id` and
//! run their writes under the RLS GUCs (handled in `db::notifications`). The
//! global `notification_preferences` table is keyed by `user_id` alone.
use axum::extract::{Extension, Path, Query, State};
use axum::http::StatusCode;
use axum::{routing, Json, Router};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::context::RequestContext;
use crate::db;
use crate::error::ApiError;
use crate::AppState;

// ===========================================================================
// DTOs
// ===========================================================================

#[derive(Serialize)]
pub struct NotificationDto {
    pub id: Uuid,
    pub kind: String,
    pub title: String,
    pub body: Option<String>,
    pub link: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub read_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl From<db::notifications::NotificationRow> for NotificationDto {
    fn from(r: db::notifications::NotificationRow) -> Self {
        Self {
            id: r.id,
            kind: r.kind,
            title: r.title,
            body: r.body,
            link: r.link,
            created_at: r.created_at,
            read_at: r.read_at,
        }
    }
}

#[derive(Serialize)]
pub struct UnreadCountDto {
    pub count: i64,
}

#[derive(Serialize)]
pub struct ReadAllDto {
    pub updated: i64,
}

#[derive(Serialize)]
pub struct DeviceTokenDto {
    pub id: Uuid,
    pub platform: String,
    pub label: Option<String>,
    pub user_agent: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub last_seen_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Serialize)]
pub struct DeviceTokenListDto {
    pub devices: Vec<DeviceTokenDto>,
}

impl From<db::notifications::DeviceTokenRow> for DeviceTokenDto {
    fn from(row: db::notifications::DeviceTokenRow) -> Self {
        Self {
            id: row.id,
            platform: row.platform,
            label: row.label,
            user_agent: row.user_agent,
            created_at: row.created_at,
            last_seen_at: row.last_seen_at,
        }
    }
}

#[derive(Serialize)]
pub struct DeliveryDto {
    pub id: Uuid,
    pub user_id: Uuid,
    pub notification_id: Option<Uuid>,
    pub channel: String,
    pub provider: String,
    pub target_hash: String,
    pub target_label: Option<String>,
    pub device_token_id: Option<Uuid>,
    pub kind: String,
    pub status: String,
    pub provider_message_id: Option<String>,
    pub provider_status: Option<String>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Serialize)]
pub struct DeliveryListDto {
    pub deliveries: Vec<DeliveryDto>,
}

impl From<db::notifications::DeliveryRow> for DeliveryDto {
    fn from(row: db::notifications::DeliveryRow) -> Self {
        Self {
            id: row.id,
            user_id: row.user_id,
            notification_id: row.notification_id,
            channel: row.channel,
            provider: row.provider,
            target_hash: row.target_hash,
            target_label: row.target_label,
            device_token_id: row.device_token_id,
            kind: row.kind,
            status: row.status,
            provider_message_id: row.provider_message_id,
            provider_status: row.provider_status,
            error_code: row.error_code,
            error_message: row.error_message,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

#[derive(Serialize)]
pub struct PrefDto {
    pub email_enabled: bool,
    pub push_enabled: bool,
    pub in_app_enabled: bool,
}

impl From<db::notifications::PrefRow> for PrefDto {
    fn from(r: db::notifications::PrefRow) -> Self {
        Self {
            email_enabled: r.email_enabled,
            push_enabled: r.push_enabled,
            in_app_enabled: r.in_app_enabled,
        }
    }
}

#[derive(Deserialize)]
pub struct ListQuery {
    pub limit: Option<i64>,
    pub before: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Deserialize)]
pub struct DeviceTokenBody {
    pub token: String,
    pub platform: String,
    pub label: Option<String>,
    pub user_agent: Option<String>,
}

#[derive(Deserialize)]
pub struct RemoveDeviceTokenBody {
    pub token: String,
}

#[derive(Deserialize)]
pub struct DeliveryQuery {
    pub channel: Option<String>,
    pub status: Option<String>,
    pub user_id: Option<Uuid>,
    pub kind: Option<String>,
    pub provider: Option<String>,
    pub before: Option<chrono::DateTime<chrono::Utc>>,
    pub limit: Option<i64>,
}

#[derive(Deserialize)]
pub struct PatchPrefBody {
    pub email_enabled: Option<bool>,
    pub push_enabled: Option<bool>,
    pub in_app_enabled: Option<bool>,
}

// ===========================================================================
// Helpers
// ===========================================================================

/// Clamp a caller-supplied page size to a sane 1..=100, defaulting to 50.
fn normalize_limit(limit: Option<i64>) -> i64 {
    limit.unwrap_or(50).clamp(1, 100)
}

fn valid_platform(p: &str) -> bool {
    matches!(p, "web" | "ios" | "android")
}

fn is_admin(ctx: &RequestContext) -> bool {
    ctx.can_manage_organization()
}

// ===========================================================================
// Routers
// ===========================================================================

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/me/notifications", routing::get(list))
        .route(
            "/v1/me/notifications/unread-count",
            routing::get(unread_count),
        )
        .route("/v1/me/notifications/{id}/read", routing::post(mark_read))
        .route(
            "/v1/me/notifications/read-all",
            routing::post(mark_all_read),
        )
        .route(
            "/v1/me/device-tokens",
            routing::get(list_device_tokens)
                .post(register_device_token)
                .delete(remove_device_token),
        )
        .route(
            "/v1/me/device-tokens/{id}",
            routing::delete(revoke_device_token),
        )
        .route(
            "/v1/me/notification-preferences",
            routing::get(get_preferences).patch(patch_preferences),
        )
        .route(
            "/v1/admin/notification-deliveries",
            routing::get(list_admin_deliveries),
        )
        .route(
            "/v1/admin/notification-deliveries/{id}",
            routing::get(get_admin_delivery),
        )
}

#[doc(hidden)]
pub fn router_for_tests(pool: PgPool) -> Router {
    Router::new()
        .route("/v1/me/notifications", routing::get(list_t))
        .route(
            "/v1/me/notifications/unread-count",
            routing::get(unread_count_t),
        )
        .route("/v1/me/notifications/{id}/read", routing::post(mark_read_t))
        .route(
            "/v1/me/notifications/read-all",
            routing::post(mark_all_read_t),
        )
        .route(
            "/v1/me/device-tokens",
            routing::get(list_device_tokens_t)
                .post(register_device_token_t)
                .delete(remove_device_token_t),
        )
        .route(
            "/v1/me/device-tokens/{id}",
            routing::delete(revoke_device_token_t),
        )
        .route(
            "/v1/me/notification-preferences",
            routing::get(get_preferences_t).patch(patch_preferences_t),
        )
        .route(
            "/v1/admin/notification-deliveries",
            routing::get(list_admin_deliveries_t),
        )
        .route(
            "/v1/admin/notification-deliveries/{id}",
            routing::get(get_admin_delivery_t),
        )
        .with_state(TestState { pool })
}

#[derive(Clone)]
struct TestState {
    pool: PgPool,
}

// ===========================================================================
// Inner logic
// ===========================================================================

async fn list_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    q: ListQuery,
) -> Result<Json<Vec<NotificationDto>>, ApiError> {
    let tenant_id = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    let limit = normalize_limit(q.limit);
    let rows = db::notifications::list_for_user(pool, tenant_id, ctx.user_id, limit, q.before)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(rows.into_iter().map(NotificationDto::from).collect()))
}

async fn unread_count_inner(
    pool: &PgPool,
    ctx: &RequestContext,
) -> Result<Json<UnreadCountDto>, ApiError> {
    let tenant_id = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    let count = db::notifications::unread_count(pool, tenant_id, ctx.user_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(UnreadCountDto { count }))
}

async fn mark_read_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    id: Uuid,
) -> Result<StatusCode, ApiError> {
    let tenant_id = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    let ok = db::notifications::mark_read(pool, tenant_id, ctx.user_id, id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !ok {
        return Err(ApiError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn mark_all_read_inner(
    pool: &PgPool,
    ctx: &RequestContext,
) -> Result<Json<ReadAllDto>, ApiError> {
    let tenant_id = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    let updated = db::notifications::mark_all_read(pool, tenant_id, ctx.user_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(ReadAllDto { updated }))
}

async fn register_device_token_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    b: DeviceTokenBody,
) -> Result<StatusCode, ApiError> {
    let tenant_id = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    if b.token.trim().is_empty() {
        return Err(ApiError::BadRequest("token is required".into()));
    }
    if !valid_platform(&b.platform) {
        return Err(ApiError::BadRequest(format!(
            "invalid platform: {}",
            b.platform
        )));
    }
    db::notifications::register_device_token(
        pool,
        tenant_id,
        ctx.user_id,
        &b.token,
        &b.platform,
        b.label.as_deref(),
        b.user_agent.as_deref(),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn remove_device_token_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    b: RemoveDeviceTokenBody,
) -> Result<StatusCode, ApiError> {
    let tenant_id = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    db::notifications::remove_device_token(pool, tenant_id, ctx.user_id, &b.token)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_device_tokens_inner(
    pool: &PgPool,
    ctx: &RequestContext,
) -> Result<Json<DeviceTokenListDto>, ApiError> {
    let tenant_id = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    let rows = db::notifications::list_device_token_rows(pool, tenant_id, ctx.user_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(DeviceTokenListDto {
        devices: rows.into_iter().map(DeviceTokenDto::from).collect(),
    }))
}

async fn revoke_device_token_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    id: Uuid,
) -> Result<StatusCode, ApiError> {
    let tenant_id = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    let revoked = db::notifications::revoke_device_token_by_id(pool, tenant_id, ctx.user_id, id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !revoked {
        return Err(ApiError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn list_admin_deliveries_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    q: DeliveryQuery,
) -> Result<Json<DeliveryListDto>, ApiError> {
    if !is_admin(ctx) {
        return Err(ApiError::Forbidden);
    }
    let tenant_id = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    let filter = db::notifications::DeliveryFilter {
        channel: q.channel,
        status: q.status,
        user_id: q.user_id,
        kind: q.kind,
        provider: q.provider,
        before: q.before,
        limit: q.limit.unwrap_or(50).clamp(1, 200),
    };
    let rows = db::notifications::list_deliveries_for_admin(pool, tenant_id, ctx.user_id, filter)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(DeliveryListDto {
        deliveries: rows.into_iter().map(DeliveryDto::from).collect(),
    }))
}

async fn get_admin_delivery_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    id: Uuid,
) -> Result<Json<DeliveryDto>, ApiError> {
    if !is_admin(ctx) {
        return Err(ApiError::Forbidden);
    }
    let tenant_id = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    let row = db::notifications::get_delivery_for_admin(pool, tenant_id, ctx.user_id, id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(DeliveryDto::from(row)))
}

async fn get_preferences_inner(
    pool: &PgPool,
    ctx: &RequestContext,
) -> Result<Json<PrefDto>, ApiError> {
    let prefs = db::notifications::get_preferences(pool, ctx.user_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(PrefDto::from(prefs)))
}

async fn patch_preferences_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    b: PatchPrefBody,
) -> Result<Json<PrefDto>, ApiError> {
    // Read current (or defaults), apply the partial patch, upsert.
    let current = db::notifications::get_preferences(pool, ctx.user_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let email = b.email_enabled.unwrap_or(current.email_enabled);
    let push = b.push_enabled.unwrap_or(current.push_enabled);
    let in_app = b.in_app_enabled.unwrap_or(current.in_app_enabled);
    let updated = db::notifications::set_preferences(pool, ctx.user_id, email, push, in_app)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(PrefDto::from(updated)))
}

// ===========================================================================
// Production handlers
// ===========================================================================

async fn list(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Query(q): Query<ListQuery>,
) -> Result<Json<Vec<NotificationDto>>, ApiError> {
    list_inner(&s.pool, &ctx, q).await
}

async fn unread_count(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<UnreadCountDto>, ApiError> {
    unread_count_inner(&s.pool, &ctx).await
}

async fn mark_read(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    mark_read_inner(&s.pool, &ctx, id).await
}

async fn mark_all_read(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<ReadAllDto>, ApiError> {
    mark_all_read_inner(&s.pool, &ctx).await
}

async fn register_device_token(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Json(b): Json<DeviceTokenBody>,
) -> Result<StatusCode, ApiError> {
    register_device_token_inner(&s.pool, &ctx, b).await
}

async fn remove_device_token(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Json(b): Json<RemoveDeviceTokenBody>,
) -> Result<StatusCode, ApiError> {
    remove_device_token_inner(&s.pool, &ctx, b).await
}

async fn list_device_tokens(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<DeviceTokenListDto>, ApiError> {
    list_device_tokens_inner(&s.pool, &ctx).await
}

async fn revoke_device_token(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    revoke_device_token_inner(&s.pool, &ctx, id).await
}

async fn list_admin_deliveries(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Query(q): Query<DeliveryQuery>,
) -> Result<Json<DeliveryListDto>, ApiError> {
    list_admin_deliveries_inner(&s.pool, &ctx, q).await
}

async fn get_admin_delivery(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<DeliveryDto>, ApiError> {
    get_admin_delivery_inner(&s.pool, &ctx, id).await
}

async fn get_preferences(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<PrefDto>, ApiError> {
    get_preferences_inner(&s.pool, &ctx).await
}

async fn patch_preferences(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Json(b): Json<PatchPrefBody>,
) -> Result<Json<PrefDto>, ApiError> {
    patch_preferences_inner(&s.pool, &ctx, b).await
}

// ===========================================================================
// Test wrappers
// ===========================================================================

async fn list_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Query(q): Query<ListQuery>,
) -> Result<Json<Vec<NotificationDto>>, ApiError> {
    list_inner(&s.pool, &ctx, q).await
}

async fn unread_count_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<UnreadCountDto>, ApiError> {
    unread_count_inner(&s.pool, &ctx).await
}

async fn mark_read_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    mark_read_inner(&s.pool, &ctx, id).await
}

async fn mark_all_read_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<ReadAllDto>, ApiError> {
    mark_all_read_inner(&s.pool, &ctx).await
}

async fn register_device_token_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Json(b): Json<DeviceTokenBody>,
) -> Result<StatusCode, ApiError> {
    register_device_token_inner(&s.pool, &ctx, b).await
}

async fn remove_device_token_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Json(b): Json<RemoveDeviceTokenBody>,
) -> Result<StatusCode, ApiError> {
    remove_device_token_inner(&s.pool, &ctx, b).await
}

async fn list_device_tokens_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<DeviceTokenListDto>, ApiError> {
    list_device_tokens_inner(&s.pool, &ctx).await
}

async fn revoke_device_token_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    revoke_device_token_inner(&s.pool, &ctx, id).await
}

async fn list_admin_deliveries_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Query(q): Query<DeliveryQuery>,
) -> Result<Json<DeliveryListDto>, ApiError> {
    list_admin_deliveries_inner(&s.pool, &ctx, q).await
}

async fn get_admin_delivery_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<DeliveryDto>, ApiError> {
    get_admin_delivery_inner(&s.pool, &ctx, id).await
}

async fn get_preferences_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<PrefDto>, ApiError> {
    get_preferences_inner(&s.pool, &ctx).await
}

async fn patch_preferences_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Json(b): Json<PatchPrefBody>,
) -> Result<Json<PrefDto>, ApiError> {
    patch_preferences_inner(&s.pool, &ctx, b).await
}

#[cfg(test)]
mod tests {
    use super::{normalize_limit, valid_platform};

    #[test]
    fn limit_defaults_and_clamps() {
        assert_eq!(normalize_limit(None), 50);
        assert_eq!(normalize_limit(Some(0)), 1);
        assert_eq!(normalize_limit(Some(10)), 10);
        assert_eq!(normalize_limit(Some(1000)), 100);
        assert_eq!(normalize_limit(Some(-5)), 1);
    }

    #[test]
    fn platform_validation() {
        for p in ["web", "ios", "android"] {
            assert!(valid_platform(p));
        }
        assert!(!valid_platform("windows"));
        assert!(!valid_platform(""));
    }
}
