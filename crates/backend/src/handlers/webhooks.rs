// crates/backend/src/handlers/webhooks.rs
//! Org-admin management of OUTBOUND webhook subscriptions + a delivery log.
//! These endpoints live INSIDE the `require_auth` router and are gated to
//! org-admin / platform-admin.
//!
//! Routes (authed router):
//!   * GET    /v1/admin/webhooks                       — list subscriptions
//!   * POST   /v1/admin/webhooks                       — create; returns secret ONCE
//!   * PATCH  /v1/admin/webhooks/{id}                   — update url/events/active
//!   * DELETE /v1/admin/webhooks/{id}                   — delete (cascades deliveries)
//!   * GET    /v1/admin/webhooks/deliveries            — recent deliveries (tenant)
//!   * GET    /v1/admin/webhooks/{id}/deliveries        — recent deliveries (one sub)
//!
//! On create we mint a per-subscription signing `secret` (`whsec_…`) and return
//! it in the POST response body exactly ONCE; thereafter only the public row is
//! returned (the secret never leaves the DB again). The delivery worker uses the
//! stored secret to sign each payload's `X-Aula-Signature` header
//! (`services::webhook_delivery::sign_payload`).
//!
//! The set of emittable events is fixed (`SUPPORTED_EVENTS`); a subscription may
//! only register events from that list, so a typo can't silently never fire.
use axum::extract::{Extension, Path, Query, State};
use axum::http::StatusCode;
use axum::{routing, Json, Router};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::context::RequestContext;
use crate::db;
use crate::error::ApiError;
use crate::AppState;

const MAX_URL_LEN: usize = 2048;
const MAX_EVENTS: usize = 50;
const DEFAULT_DELIVERY_LIMIT: i64 = 100;
const MAX_DELIVERY_LIMIT: i64 = 500;

/// Event names a subscription may register for. Emitters call
/// `services::webhook_delivery::emit_event(pool, tenant, <one of these>, payload)`.
/// Keep this list and the emitter call sites in sync.
pub const SUPPORTED_EVENTS: &[&str] = &[
    "course.created",
    "course.updated",
    "course.published",
    "enrollment.created",
    "assignment.published",
    "submission.graded",
    "announcement.created",
];

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/v1/admin/webhooks",
            routing::get(list_subscriptions).post(create_subscription),
        )
        .route(
            "/v1/admin/webhooks/{id}",
            routing::patch(update_subscription).delete(delete_subscription),
        )
        .route(
            "/v1/admin/webhooks/deliveries",
            routing::get(list_tenant_deliveries),
        )
        .route(
            "/v1/admin/webhooks/{id}/deliveries",
            routing::get(list_subscription_deliveries),
        )
}

fn require_admin(ctx: &RequestContext) -> Result<Uuid, ApiError> {
    if !ctx.has_capability(core_types::Capability::IntegrationsManage) {
        return Err(ApiError::Forbidden);
    }
    ctx.tenant_id
        .ok_or_else(|| ApiError::BadRequest("no tenant".into()))
}

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct SubscriptionDto {
    pub id: Uuid,
    pub url: String,
    pub events: Vec<String>,
    pub active: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl From<db::webhooks::SubscriptionRow> for SubscriptionDto {
    fn from(r: db::webhooks::SubscriptionRow) -> Self {
        Self {
            id: r.id,
            url: r.url,
            events: r.events,
            active: r.active,
            created_at: r.created_at,
        }
    }
}

/// Create response. `secret` is the ONLY time the signing secret is returned.
#[derive(Serialize)]
pub struct CreatedSubscriptionDto {
    #[serde(flatten)]
    pub subscription: SubscriptionDto,
    pub secret: String,
}

#[derive(Serialize)]
pub struct DeliveryDto {
    pub id: Uuid,
    pub subscription_id: Uuid,
    pub event: String,
    pub payload_json: serde_json::Value,
    pub status: String,
    pub attempts: i32,
    pub last_attempt_at: Option<chrono::DateTime<chrono::Utc>>,
    pub response_code: Option<i32>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl From<db::webhooks::DeliveryRow> for DeliveryDto {
    fn from(r: db::webhooks::DeliveryRow) -> Self {
        Self {
            id: r.id,
            subscription_id: r.subscription_id,
            event: r.event,
            payload_json: r.payload_json,
            status: r.status,
            attempts: r.attempts,
            last_attempt_at: r.last_attempt_at,
            response_code: r.response_code,
            created_at: r.created_at,
        }
    }
}

#[derive(Deserialize)]
pub struct CreateSubscription {
    pub url: String,
    pub events: Vec<String>,
}

#[derive(Deserialize)]
pub struct UpdateSubscription {
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub events: Option<Vec<String>>,
    #[serde(default)]
    pub active: Option<bool>,
}

#[derive(Deserialize, Default)]
pub struct DeliveryQuery {
    #[serde(default)]
    pub limit: Option<i64>,
}

// ---------------------------------------------------------------------------
// Validation helpers
// ---------------------------------------------------------------------------

/// Validate the stable URL shape at write time. The delivery worker repeats the
/// check, resolves DNS, rejects non-public addresses, pins the approved address,
/// and disables redirects so DNS rebinding or a redirect cannot reach internal
/// services.
fn validate_url(url: &str) -> Result<String, ApiError> {
    let url = url.trim();
    if url.is_empty() {
        return Err(ApiError::Validation("url_required".into()));
    }
    if url.len() > MAX_URL_LEN {
        return Err(ApiError::Validation("url_too_long".into()));
    }
    crate::services::webhook_delivery::validate_target_url(url)
        .map(|parsed| parsed.to_string())
        .map_err(|reason| ApiError::Validation(reason.into()))
}

/// Validate the requested event list against `SUPPORTED_EVENTS` (non-empty,
/// deduped, all known). Returns the cleaned list.
fn validate_events(events: &[String]) -> Result<Vec<String>, ApiError> {
    let mut cleaned: Vec<String> = events
        .iter()
        .map(|e| e.trim().to_string())
        .filter(|e| !e.is_empty())
        .collect();
    cleaned.sort();
    cleaned.dedup();
    if cleaned.is_empty() {
        return Err(ApiError::Validation("events_required".into()));
    }
    if cleaned.len() > MAX_EVENTS {
        return Err(ApiError::Validation("too_many_events".into()));
    }
    for ev in &cleaned {
        if !SUPPORTED_EVENTS.contains(&ev.as_str()) {
            return Err(ApiError::Validation(format!("unknown_event:{ev}")));
        }
    }
    Ok(cleaned)
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn list_subscriptions(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<Vec<SubscriptionDto>>, ApiError> {
    let tenant = require_admin(&ctx)?;
    let rows = db::webhooks::list_subscriptions(&s.pool, tenant)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(rows.into_iter().map(SubscriptionDto::from).collect()))
}

async fn create_subscription(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Json(body): Json<CreateSubscription>,
) -> Result<Json<CreatedSubscriptionDto>, ApiError> {
    let tenant = require_admin(&ctx)?;
    let url = validate_url(&body.url)?;
    let events = validate_events(&body.events)?;

    let secret = format!("whsec_{}", random_b64(32));
    let row = db::webhooks::insert_subscription(&s.pool, tenant, &url, &secret, &events)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(CreatedSubscriptionDto {
        subscription: SubscriptionDto::from(row),
        secret,
    }))
}

async fn update_subscription(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateSubscription>,
) -> Result<Json<SubscriptionDto>, ApiError> {
    let tenant = require_admin(&ctx)?;

    let url = match body.url.as_deref() {
        Some(u) => Some(validate_url(u)?),
        None => None,
    };
    let events = match body.events.as_deref() {
        Some(e) => Some(validate_events(e)?),
        None => None,
    };

    let row = db::webhooks::update_subscription(
        &s.pool,
        tenant,
        id,
        url.as_deref(),
        events.as_deref(),
        body.active,
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?
    .ok_or(ApiError::NotFound)?;
    Ok(Json(SubscriptionDto::from(row)))
}

async fn delete_subscription(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let tenant = require_admin(&ctx)?;
    let deleted = db::webhooks::delete_subscription(&s.pool, tenant, id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !deleted {
        return Err(ApiError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn list_tenant_deliveries(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Query(q): Query<DeliveryQuery>,
) -> Result<Json<Vec<DeliveryDto>>, ApiError> {
    let tenant = require_admin(&ctx)?;
    let limit = clamp_limit(q.limit);
    let rows = db::webhooks::list_deliveries(&s.pool, tenant, None, limit)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(rows.into_iter().map(DeliveryDto::from).collect()))
}

async fn list_subscription_deliveries(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    Query(q): Query<DeliveryQuery>,
) -> Result<Json<Vec<DeliveryDto>>, ApiError> {
    let tenant = require_admin(&ctx)?;
    let limit = clamp_limit(q.limit);
    let rows = db::webhooks::list_deliveries(&s.pool, tenant, Some(id), limit)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(rows.into_iter().map(DeliveryDto::from).collect()))
}

fn clamp_limit(limit: Option<i64>) -> i64 {
    limit
        .unwrap_or(DEFAULT_DELIVERY_LIMIT)
        .clamp(1, MAX_DELIVERY_LIMIT)
}

/// `n` bytes of CSPRNG entropy as URL-safe base64 (no padding).
fn random_b64(n: usize) -> String {
    use base64::Engine;
    use rand::RngCore;
    let mut bytes = vec![0u8; n];
    rand::rng().fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_url_accepts_https_rejects_junk() {
        assert!(validate_url("https://example.com/hook").is_ok());
        assert!(validate_url("http://example.com/hook").is_err());
        assert!(validate_url("https://127.0.0.1/hook").is_err());
        assert!(validate_url("https://localhost/hook").is_err());
        assert!(validate_url("").is_err());
        assert!(validate_url("ftp://example.com").is_err());
        assert!(validate_url("not a url").is_err());
    }

    #[test]
    fn validate_events_dedups_and_rejects_unknown() {
        let ok = validate_events(&[
            "course.created".into(),
            "course.created".into(),
            "submission.graded".into(),
        ])
        .unwrap();
        assert_eq!(ok, vec!["course.created", "submission.graded"]);
        assert!(validate_events(&[]).is_err());
        assert!(validate_events(&["bogus.event".into()]).is_err());
    }

    #[test]
    fn clamp_limit_bounds() {
        assert_eq!(clamp_limit(None), DEFAULT_DELIVERY_LIMIT);
        assert_eq!(clamp_limit(Some(0)), 1);
        assert_eq!(clamp_limit(Some(10_000)), MAX_DELIVERY_LIMIT);
        assert_eq!(clamp_limit(Some(42)), 42);
    }
}
