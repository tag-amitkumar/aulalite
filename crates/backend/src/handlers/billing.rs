// crates/backend/src/handlers/billing.rs
//
// Billing HTTP API: organization-owner billing dashboard + checkout controls,
// plus the PUBLIC Stripe webhook receiver.
//
// Routing split mirrors the rest of the codebase:
//   * `routes()`         — AUTHED admin endpoints, merged into the require_auth
//                          router in lib.rs (organization-owner gated).
//   * `webhook_routes()` — the PUBLIC `POST /v1/stripe/webhook`, merged like the
//                          mediamtx callbacks router WITHOUT the require_auth
//                          layer (Stripe carries no Firebase token).
//
// Each handler body lives in an `_inner` fn taking the bits it needs (pool +
// stripe client + secret + origin) so the production handlers and the
// `router_for_tests` mirrors share one implementation.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Extension, State};
use axum::http::{HeaderMap, StatusCode};
use axum::{routing, Json, Router};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::context::RequestContext;
use crate::db;
use crate::error::ApiError;
use crate::services::billing::StripeClient;
use crate::AppState;

// ============================================================================
// DTOs (the frontend mirrors these exact serde field names + types)
// ============================================================================

/// A billing plan as surfaced to the admin UI. Mirrors `db::billing::PlanRow`
/// minus the internal `stripe_price_id`.
#[derive(Debug, Clone, Serialize)]
pub struct PlanDto {
    pub id: String,
    pub name: String,
    pub monthly_price_cents: i32,
    pub included_seats: i32,
    pub included_class_minutes: i32,
    pub included_recording_gb: i32,
}

impl From<db::billing::PlanRow> for PlanDto {
    fn from(p: db::billing::PlanRow) -> Self {
        PlanDto {
            id: p.id,
            name: p.name,
            monthly_price_cents: p.monthly_price_cents,
            included_seats: p.included_seats,
            included_class_minutes: p.included_class_minutes,
            included_recording_gb: p.included_recording_gb,
        }
    }
}

/// The tenant's subscription row as surfaced to the admin UI.
#[derive(Debug, Clone, Serialize)]
pub struct SubscriptionDto {
    pub plan_id: String,
    pub status: String,
    pub current_period_start: Option<chrono::DateTime<chrono::Utc>>,
    pub current_period_end: Option<chrono::DateTime<chrono::Utc>>,
    pub trial_ends_at: Option<chrono::DateTime<chrono::Utc>>,
    pub stripe_subscription_id: Option<String>,
    pub overage_behavior: String,
}

impl From<db::billing::SubscriptionRow> for SubscriptionDto {
    fn from(s: db::billing::SubscriptionRow) -> Self {
        SubscriptionDto {
            plan_id: s.plan_id,
            status: s.status,
            current_period_start: s.current_period_start,
            current_period_end: s.current_period_end,
            trial_ends_at: s.trial_ends_at,
            stripe_subscription_id: s.stripe_subscription_id,
            overage_behavior: s.overage_behavior,
        }
    }
}

/// Usage-on-read paired with plan caps: class minutes are monthly, while seats
/// and recording storage are point-in-time totals.
/// `*_included` are `None` when the tenant has no plan/subscription resolved.
#[derive(Debug, Clone, Serialize)]
pub struct UsageDto {
    pub active_seats: i64,
    pub included_seats: Option<i32>,
    pub class_minutes_used: i64,
    pub included_class_minutes: Option<i32>,
    /// Recording bytes converted to GB (bytes / 1e9), rounded to 3 decimals.
    pub recording_gb_used: f64,
    pub included_recording_gb: Option<i32>,
}

/// Top-level billing dashboard payload for `GET /v1/admin/billing`.
#[derive(Debug, Clone, Serialize)]
pub struct BillingDto {
    pub plan: Option<PlanDto>,
    pub subscription: Option<SubscriptionDto>,
    pub usage: UsageDto,
    pub overage_behavior: String,
    /// The full plan catalogue, so the admin UI can offer upgrades without a
    /// second endpoint (cheapest first, matching `list_plans`).
    pub plans: Vec<PlanDto>,
}

// ============================================================================
// Routers
// ============================================================================

/// Authed admin billing endpoints. Merged into the require_auth router.
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/admin/billing", routing::get(get_billing))
        .route(
            "/v1/admin/billing/checkout-session",
            routing::post(checkout_session),
        )
        .route(
            "/v1/admin/billing/portal-session",
            routing::post(billing_portal_session),
        )
        .route("/v1/admin/billing/overage", routing::patch(patch_overage))
}

/// PUBLIC Stripe webhook router. Merged WITHOUT the require_auth layer (the
/// request carries a Stripe signature header, never a Firebase token).
pub fn webhook_routes() -> Router<AppState> {
    Router::new().route("/v1/stripe/webhook", routing::post(stripe_webhook))
}

// ----------------------------------------------------------------------------
// Test routers (mirror the live_sessions `router_for_tests` pattern).
// ----------------------------------------------------------------------------

/// State carrying just what the billing handlers need, for integration tests
/// that exercise the endpoints without constructing a full `AppState`.
#[derive(Clone)]
pub struct BillingTestState {
    pub pool: PgPool,
    pub stripe: Arc<dyn StripeClient>,
    pub stripe_webhook_secret: Option<String>,
    pub app_origin: String,
}

#[doc(hidden)]
pub fn router_for_tests(state: BillingTestState) -> Router {
    Router::new()
        .route("/v1/admin/billing", routing::get(get_billing_t))
        .route(
            "/v1/admin/billing/checkout-session",
            routing::post(checkout_session_t),
        )
        .route(
            "/v1/admin/billing/portal-session",
            routing::post(billing_portal_session_t),
        )
        .route("/v1/admin/billing/overage", routing::patch(patch_overage_t))
        .with_state(state)
}

#[doc(hidden)]
pub fn webhook_router_for_tests(state: BillingTestState) -> Router {
    Router::new()
        .route("/v1/stripe/webhook", routing::post(stripe_webhook_t))
        .with_state(state)
}

// ============================================================================
// GET /v1/admin/billing
// ============================================================================

fn is_admin(ctx: &RequestContext) -> bool {
    ctx.can_manage_billing()
}

async fn get_billing_inner(
    pool: &PgPool,
    ctx: &RequestContext,
) -> Result<Json<BillingDto>, ApiError> {
    if !is_admin(ctx) {
        return Err(ApiError::Forbidden);
    }
    let tenant_id = ctx
        .tenant_id
        .ok_or_else(|| ApiError::BadRequest("no tenant".into()))?;

    let subscription = db::billing::get_subscription(pool, tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    // The plan is whichever the subscription points at; `None` if the tenant
    // has no subscription row yet (never started checkout / pre-provisioned).
    let plan = match subscription.as_ref() {
        Some(sub) => db::billing::get_plan(pool, &sub.plan_id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?,
        None => None,
    };

    let usage_totals = db::billing::compute_usage(pool, tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let usage = UsageDto {
        active_seats: usage_totals.active_seats,
        included_seats: plan.as_ref().map(|p| p.included_seats),
        class_minutes_used: usage_totals.class_minutes_used,
        included_class_minutes: plan
            .as_ref()
            .and_then(|p| (p.included_class_minutes >= 0).then_some(p.included_class_minutes)),
        // bytes / 1e9, rounded to milli-GB so the UI shows a stable figure.
        recording_gb_used: (usage_totals.recording_bytes_used as f64 / 1e9 * 1000.0).round()
            / 1000.0,
        included_recording_gb: plan
            .as_ref()
            .and_then(|p| (p.included_recording_gb >= 0).then_some(p.included_recording_gb)),
    };

    // `overage_behavior` defaults to the schema default when no subscription
    // exists yet, so the UI always has a value to render.
    let overage_behavior = subscription
        .as_ref()
        .map(|s| s.overage_behavior.clone())
        .unwrap_or_else(|| "block".to_string());

    let plans = db::billing::list_plans(pool)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .into_iter()
        .map(PlanDto::from)
        .collect();

    Ok(Json(BillingDto {
        plan: plan.map(PlanDto::from),
        subscription: subscription.map(SubscriptionDto::from),
        usage,
        overage_behavior,
        plans,
    }))
}

async fn get_billing(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<BillingDto>, ApiError> {
    get_billing_inner(&s.pool, &ctx).await
}

async fn get_billing_t(
    State(s): State<BillingTestState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<BillingDto>, ApiError> {
    get_billing_inner(&s.pool, &ctx).await
}

// ============================================================================
// POST /v1/admin/billing/checkout-session
// ============================================================================

#[derive(Deserialize)]
pub struct CheckoutRequest {
    pub plan_id: String,
}

#[derive(Serialize)]
pub struct CheckoutResponse {
    pub url: String,
}

/// Resolve the success/cancel urls: prefer explicit `STRIPE_SUCCESS_URL` /
/// `STRIPE_CANCEL_URL` env, otherwise fall back to `<app_origin>/admin/billing`.
fn checkout_urls(app_origin: &str) -> (String, String) {
    let default = format!("{}/admin/billing", app_origin.trim_end_matches('/'));
    let success = std::env::var("STRIPE_SUCCESS_URL").unwrap_or_else(|_| default.clone());
    let cancel = std::env::var("STRIPE_CANCEL_URL").unwrap_or(default);
    (success, cancel)
}

/// Resolve a deploy-time Stripe price without requiring the runtime app role
/// to update the global plan catalog. The seeded catalog intentionally carries
/// no environment-specific Stripe ids; production provides, for example,
/// `STRIPE_PRICE_ID_STARTER` and `STRIPE_PRICE_ID_PRO`.
fn configured_price_id(plan_id: &str) -> Option<String> {
    let suffix: String = plan_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    std::env::var(format!("STRIPE_PRICE_ID_{suffix}"))
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

async fn checkout_session_inner(
    pool: &PgPool,
    stripe: &dyn StripeClient,
    app_origin: &str,
    ctx: &RequestContext,
    body: CheckoutRequest,
) -> Result<Json<CheckoutResponse>, ApiError> {
    if !is_admin(ctx) {
        return Err(ApiError::Forbidden);
    }
    let tenant_id = ctx
        .tenant_id
        .ok_or_else(|| ApiError::BadRequest("no tenant".into()))?;

    // Checkout creates a new subscription. Once Stripe has attached a
    // provider subscription, every plan/cancellation/payment change must go
    // through the Customer Portal; otherwise a crafted API call could create
    // a second concurrent subscription for the same workspace.
    let existing_subscription = db::billing::get_subscription(pool, tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if existing_subscription
        .as_ref()
        .and_then(|subscription| subscription.stripe_subscription_id.as_deref())
        .is_some_and(|id| !id.trim().is_empty())
    {
        return Err(ApiError::Conflict(
            "manage the existing subscription in the billing portal".into(),
        ));
    }

    let mut plan = db::billing::get_plan(pool, &body.plan_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::BadRequest(format!("unknown plan: {}", body.plan_id)))?;
    plan.stripe_price_id = plan
        .stripe_price_id
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    if plan.stripe_price_id.is_none() {
        plan.stripe_price_id = configured_price_id(&plan.id);
    }
    if plan.stripe_price_id.is_none() {
        tracing::error!(plan_id = %plan.id, "Stripe price id is not configured for plan");
        return Err(ApiError::Internal(
            "Stripe price id is not configured for the selected plan".into(),
        ));
    }

    // Persist one active intent before making any provider call. Same-plan
    // retries share the key (and eventually the URL); another plan conflicts
    // until this Checkout Session expires.
    let candidate_expires_at = chrono::Utc::now() + chrono::Duration::hours(1);
    let candidate_key = format!(
        "aulalite-checkout-{}-{}",
        tenant_id.simple(),
        Uuid::new_v4().simple()
    );
    let intent = db::billing::claim_checkout_intent(
        pool,
        tenant_id,
        &plan.id,
        &ctx.email,
        &candidate_key,
        candidate_expires_at,
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    if intent.plan_id != plan.id {
        return Err(ApiError::Conflict(format!(
            "checkout for plan '{}' is already active until {}",
            intent.plan_id, intent.expires_at
        )));
    }
    if let Some(url) = intent.checkout_url.as_deref().filter(|url| !url.is_empty()) {
        return Ok(Json(CheckoutResponse {
            url: url.to_string(),
        }));
    }

    // Create the customer before the Session and always pass it explicitly.
    // Concurrent requests use the intent-derived customer idempotency key, and
    // the compare-and-set keeps the first canonical tenant/customer binding.
    let existing_customer = db::billing::get_tenant_stripe_customer(pool, tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .filter(|id| !id.trim().is_empty());
    let customer_id = match existing_customer {
        Some(customer_id) => customer_id,
        None => {
            let customer_key = format!("{}-customer", intent.idempotency_key);
            let created = stripe
                .ensure_customer(tenant_id, &intent.customer_email, &customer_key)
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?;
            db::billing::remember_tenant_stripe_customer(pool, tenant_id, &created)
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?
                .filter(|id| !id.trim().is_empty())
                .ok_or_else(|| ApiError::Internal("could not persist the Stripe customer".into()))?
        }
    };

    let (success_url, cancel_url) = checkout_urls(app_origin);

    let session = stripe
        .create_checkout_session(
            tenant_id,
            &plan,
            Some(&customer_id),
            &success_url,
            &cancel_url,
            &intent.idempotency_key,
            intent.expires_at.timestamp(),
        )
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let provider_expires_at =
        chrono::DateTime::<chrono::Utc>::from_timestamp(session.expires_at, 0)
            .filter(|expires_at| *expires_at > chrono::Utc::now())
            .ok_or_else(|| {
                ApiError::Internal("Stripe returned an invalid session expiry".into())
            })?;
    let completed = db::billing::complete_checkout_intent(
        pool,
        tenant_id,
        &intent.idempotency_key,
        &session.id,
        &session.url,
        provider_expires_at,
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?
    .ok_or_else(|| ApiError::Conflict("checkout intent expired; start checkout again".into()))?;
    if completed.stripe_session_id.as_deref() != Some(session.id.as_str()) {
        return Err(ApiError::Conflict(
            "checkout intent was completed by another request".into(),
        ));
    }
    let checkout_url = completed
        .checkout_url
        .filter(|url| !url.is_empty())
        .ok_or_else(|| ApiError::Internal("checkout intent is missing its URL".into()))?;

    // Audit-log the intent. Best-effort metadata records the target plan.
    let mut tx = db::begin_with_context(pool, ctx.user_id, Some(tenant_id))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    db::audit::emit_audit_event(
        &mut tx,
        tenant_id,
        ctx.user_id,
        "billing.checkout_started",
        "subscription",
        tenant_id,
        Some(serde_json::json!({ "plan_id": plan.id })),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(CheckoutResponse { url: checkout_url }))
}

async fn checkout_session(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Json(body): Json<CheckoutRequest>,
) -> Result<Json<CheckoutResponse>, ApiError> {
    checkout_session_inner(&s.pool, s.stripe.as_ref(), &s.app_origin, &ctx, body).await
}

async fn checkout_session_t(
    State(s): State<BillingTestState>,
    Extension(ctx): Extension<RequestContext>,
    Json(body): Json<CheckoutRequest>,
) -> Result<Json<CheckoutResponse>, ApiError> {
    checkout_session_inner(&s.pool, s.stripe.as_ref(), &s.app_origin, &ctx, body).await
}

// ============================================================================
// POST /v1/admin/billing/portal-session
// ============================================================================

#[derive(Serialize)]
pub struct BillingPortalResponse {
    pub url: String,
}

/// Construct a Stripe return URL from the configured application origin only.
/// The endpoint intentionally accepts no browser-supplied URL, closing the
/// usual Customer Portal open-redirect footgun. APP_ORIGIN is required to be
/// a bare HTTP(S) origin; configuration errors fail closed.
fn billing_portal_return_url(app_origin: &str) -> Result<String, ApiError> {
    let parsed = reqwest::Url::parse(app_origin.trim()).map_err(|_| {
        ApiError::Internal("APP_ORIGIN is not a valid URL for Stripe billing portal".into())
    })?;
    let is_http = matches!(parsed.scheme(), "http" | "https");
    let is_bare_origin = parsed.host_str().is_some()
        && parsed.username().is_empty()
        && parsed.password().is_none()
        && matches!(parsed.path(), "" | "/")
        && parsed.query().is_none()
        && parsed.fragment().is_none();
    if !is_http || !is_bare_origin {
        return Err(ApiError::Internal(
            "APP_ORIGIN must be a bare HTTP(S) origin for Stripe billing portal".into(),
        ));
    }
    Ok(format!(
        "{}/admin/billing",
        parsed.origin().ascii_serialization()
    ))
}

async fn billing_portal_session_inner(
    pool: &PgPool,
    stripe: &dyn StripeClient,
    app_origin: &str,
    ctx: &RequestContext,
) -> Result<Json<BillingPortalResponse>, ApiError> {
    if !is_admin(ctx) {
        return Err(ApiError::Forbidden);
    }
    let tenant_id = ctx
        .tenant_id
        .ok_or_else(|| ApiError::BadRequest("no tenant".into()))?;
    let customer_id = db::billing::get_tenant_stripe_customer(pool, tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .filter(|id| !id.trim().is_empty())
        .ok_or_else(|| {
            ApiError::Conflict(
                "billing profile is not ready; complete subscription checkout first".into(),
            )
        })?;
    let return_url = billing_portal_return_url(app_origin)?;
    let session = stripe
        .create_billing_portal_session(&customer_id, &return_url)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    // Never persist the short-lived portal URL or the Stripe customer id. The
    // audit event records the administrator's action without provider secrets.
    let mut tx = db::begin_with_context(pool, ctx.user_id, Some(tenant_id))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    db::audit::emit_audit_event(
        &mut tx,
        tenant_id,
        ctx.user_id,
        "billing.portal_session_created",
        "subscription",
        tenant_id,
        Some(serde_json::json!({ "provider": "stripe" })),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(BillingPortalResponse { url: session.url }))
}

async fn billing_portal_session(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<BillingPortalResponse>, ApiError> {
    billing_portal_session_inner(&s.pool, s.stripe.as_ref(), &s.app_origin, &ctx).await
}

async fn billing_portal_session_t(
    State(s): State<BillingTestState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<BillingPortalResponse>, ApiError> {
    billing_portal_session_inner(&s.pool, s.stripe.as_ref(), &s.app_origin, &ctx).await
}

// ============================================================================
// PATCH /v1/admin/billing/overage
// ============================================================================

#[derive(Deserialize)]
pub struct OverageRequest {
    pub overage_behavior: String,
}

async fn patch_overage_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    body: OverageRequest,
) -> Result<StatusCode, ApiError> {
    if !is_admin(ctx) {
        return Err(ApiError::Forbidden);
    }
    let tenant_id = ctx
        .tenant_id
        .ok_or_else(|| ApiError::BadRequest("no tenant".into()))?;

    if body.overage_behavior != "block" {
        return Err(ApiError::BadRequest(
            "metered_overage_unavailable; usage over plan limits must remain blocked".into(),
        ));
    }

    let updated = db::billing::set_overage_behavior(pool, tenant_id, &body.overage_behavior)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !updated {
        // No subscription row to update — surface a clean 404 so the UI can
        // prompt the admin to start a subscription first.
        return Err(ApiError::NotFound);
    }

    let mut tx = db::begin_with_context(pool, ctx.user_id, Some(tenant_id))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    db::audit::emit_audit_event(
        &mut tx,
        tenant_id,
        ctx.user_id,
        "billing.overage_changed",
        "subscription",
        tenant_id,
        Some(serde_json::json!({ "overage_behavior": body.overage_behavior })),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(StatusCode::OK)
}

async fn patch_overage(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Json(body): Json<OverageRequest>,
) -> Result<StatusCode, ApiError> {
    patch_overage_inner(&s.pool, &ctx, body).await
}

async fn patch_overage_t(
    State(s): State<BillingTestState>,
    Extension(ctx): Extension<RequestContext>,
    Json(body): Json<OverageRequest>,
) -> Result<StatusCode, ApiError> {
    patch_overage_inner(&s.pool, &ctx, body).await
}

// ============================================================================
// POST /v1/stripe/webhook  (PUBLIC)
// ============================================================================

/// Map a Stripe subscription status string to our `subscriptions.status` enum
/// (`'trialing' | 'active' | 'past_due' | 'canceled'`). Unknown Stripe states
/// (`incomplete`, `unpaid`, …) collapse to the closest safe value.
fn map_stripe_status(stripe_status: &str) -> &'static str {
    match stripe_status {
        "trialing" => "trialing",
        "active" => "active",
        "past_due" => "past_due",
        "canceled" | "incomplete_expired" => "canceled",
        // `unpaid` is effectively a dunning-failed state; treat as past_due so
        // the tenant keeps (degraded) access while we chase payment.
        "unpaid" => "past_due",
        // `incomplete`, `paused`, and any future-additive states map to
        // trialing as the least-privileged "not yet active" bucket.
        _ => "trialing",
    }
}

/// Convert a Stripe unix timestamp (seconds) into a chrono UTC datetime.
fn ts_to_dt(v: &serde_json::Value, key: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    v.get(key)
        .and_then(|x| x.as_i64())
        .and_then(|secs| chrono::DateTime::<chrono::Utc>::from_timestamp(secs, 0))
}

/// Return every Stripe price id currently configured for a catalog plan.
///
/// The database value is useful for installations that manage the catalog at
/// runtime, while the deploy-time value keeps immutable environment-specific
/// Stripe ids out of seed migrations. Accepting both also makes price-id
/// rotations safe while the database and deployment configuration converge.
fn configured_price_ids_for_plan(
    plan: &db::billing::PlanRow,
    deploy_price_id: Option<String>,
) -> Vec<String> {
    let mut ids = Vec::with_capacity(2);
    if let Some(id) = plan
        .stripe_price_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
    {
        ids.push(id.to_string());
    }
    if let Some(id) = deploy_price_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
    {
        if !ids.iter().any(|existing| existing == id) {
            ids.push(id.to_string());
        }
    }
    ids
}

/// Resolve the internal plan represented by the subscription's CURRENT item.
///
/// Stripe subscription metadata is copied from the original checkout and is
/// not rewritten by Customer Portal plan changes. Consequently the current
/// item's price id / lookup key is authoritative. Metadata is used only when
/// Stripe omitted all current-price identity, and only for a catalog plan that
/// has an actual Stripe price configured.
fn resolve_plan_id_from_catalog<F>(
    sub_obj: &serde_json::Value,
    plans: &[db::billing::PlanRow],
    mut deploy_price_id: F,
) -> Result<Option<String>, String>
where
    F: FnMut(&str) -> Option<String>,
{
    let configured: Vec<_> = plans
        .iter()
        .map(|plan| {
            let price_ids = configured_price_ids_for_plan(plan, deploy_price_id(&plan.id));
            (plan, price_ids)
        })
        .collect();

    let price = sub_obj
        .get("items")
        .and_then(|items| items.get("data"))
        .and_then(serde_json::Value::as_array)
        .and_then(|items| items.first())
        .and_then(|item| item.get("price"));

    let (price_id, lookup_key) = match price {
        Some(serde_json::Value::String(id)) => (Some(id.trim()), None),
        Some(serde_json::Value::Object(price)) => (
            price
                .get("id")
                .and_then(serde_json::Value::as_str)
                .map(str::trim),
            price
                .get("lookup_key")
                .and_then(serde_json::Value::as_str)
                .map(str::trim),
        ),
        _ => (None, None),
    };
    let price_id = price_id.filter(|id| !id.is_empty());
    let lookup_key = lookup_key.filter(|key| !key.is_empty());

    // Price ids must map uniquely. A duplicate provider id across catalog
    // plans is a deployment error, not a reason to pick whichever row happens
    // to be returned first.
    let by_price_id = if let Some(price_id) = price_id {
        let mut matches = configured
            .iter()
            .filter(|(_, ids)| ids.iter().any(|id| id == price_id))
            .map(|(plan, _)| plan.id.as_str());
        let first = matches.next();
        if matches.any(|candidate| Some(candidate) != first) {
            return Err(format!(
                "Stripe price {price_id} is configured for multiple plans"
            ));
        }
        first
    } else {
        None
    };

    // A lookup key is itself current Stripe price configuration. We therefore
    // require an exact catalog id, but it need not duplicate the deploy price
    // id mapping (Stripe commonly supplies one or the other).
    let by_lookup_key = lookup_key.and_then(|key| {
        configured
            .iter()
            .find(|(plan, _)| plan.id == key)
            .map(|(plan, _)| plan.id.as_str())
    });

    match (by_price_id, by_lookup_key) {
        (Some(price_plan), Some(lookup_plan)) if price_plan != lookup_plan => {
            return Err(format!(
                "Stripe price id resolves to plan {price_plan}, but lookup_key resolves to {lookup_plan}"
            ));
        }
        (Some(plan_id), _) | (_, Some(plan_id)) => return Ok(Some(plan_id.to_string())),
        (None, None) if price_id.is_some() || lookup_key.is_some() => {
            // Never fall back to checkout metadata for an unknown current
            // price: doing so would silently retain the pre-portal plan.
            return Err("current Stripe subscription price is not in the plan catalog".into());
        }
        (None, None) => {}
    }

    let metadata_plan_id = sub_obj
        .get("metadata")
        .and_then(|metadata| metadata.get("plan_id"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|plan_id| !plan_id.is_empty());

    let Some(metadata_plan_id) = metadata_plan_id else {
        return Ok(None);
    };
    let Some((plan, _)) = configured
        .iter()
        .find(|(plan, price_ids)| plan.id == metadata_plan_id && !price_ids.is_empty())
    else {
        return Err(format!(
            "metadata plan {metadata_plan_id} is missing or has no configured Stripe price"
        ));
    };
    Ok(Some(plan.id.clone()))
}

/// Extract the Stripe customer id from an event object (`customer` is either a
/// string id or an expanded object with an `id`).
fn customer_id_of(obj: &serde_json::Value) -> Option<String> {
    match obj.get("customer") {
        Some(serde_json::Value::String(s)) if !s.is_empty() => Some(s.clone()),
        Some(serde_json::Value::Object(o)) => {
            o.get("id").and_then(|v| v.as_str()).map(str::to_string)
        }
        _ => None,
    }
}

/// Stripe invoices identify the subscription either by id or by an expanded
/// subscription object. Invoice status must never be applied using customer id
/// alone because one customer can have historical or accidental subscriptions.
fn invoice_subscription_id_of(obj: &serde_json::Value) -> Option<String> {
    match obj.get("subscription") {
        Some(serde_json::Value::String(id)) if !id.trim().is_empty() => Some(id.trim().to_string()),
        Some(serde_json::Value::Object(subscription)) => subscription
            .get("id")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(str::to_string),
        _ => None,
    }
}

/// Resolve the tenant a Stripe object belongs to. Preference order:
///   1. `metadata.tenant_id` on the object (set when we create checkout).
///   2. `tenants.stripe_customer_id` matching the object's `customer`.
async fn resolve_tenant(pool: &PgPool, obj: &serde_json::Value) -> Result<Option<Uuid>, ApiError> {
    if let Some(tid) = obj
        .get("metadata")
        .and_then(|m| m.get("tenant_id"))
        .and_then(|t| t.as_str())
    {
        if let Ok(uuid) = Uuid::parse_str(tid) {
            return Ok(Some(uuid));
        }
    }
    if let Some(cid) = customer_id_of(obj) {
        return db::billing::tenant_for_stripe_customer(pool, &cid)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()));
    }
    Ok(None)
}

/// Apply a subscription object to our `subscriptions` row, resolving tenant +
/// plan. `forced_status` overrides the object's own status (used by the
/// deleted/payment events). Resolution failures are retryable webhook errors;
/// silently accepting one would leave local entitlements stale.
async fn apply_subscription(
    pool: &PgPool,
    sub_obj: &serde_json::Value,
    forced_status: Option<&str>,
    event_created_at: chrono::DateTime<chrono::Utc>,
) -> Result<(), ApiError> {
    let Some(tenant_id) = resolve_tenant(pool, sub_obj).await? else {
        return Err(ApiError::Internal(
            "stripe webhook could not resolve a tenant for the subscription".into(),
        ));
    };
    let plans = db::billing::list_plans(pool)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let plan_id =
        resolve_plan_id_from_catalog(sub_obj, &plans, configured_price_id).map_err(|detail| {
            ApiError::Internal(format!(
                "stripe webhook could not safely resolve a plan for tenant {tenant_id}: {detail}"
            ))
        })?;
    let Some(plan_id) = plan_id else {
        return Err(ApiError::Internal(format!(
            "stripe webhook could not resolve a plan for tenant {tenant_id}"
        )));
    };

    let status = forced_status.map(str::to_string).unwrap_or_else(|| {
        map_stripe_status(sub_obj.get("status").and_then(|v| v.as_str()).unwrap_or("")).to_string()
    });
    let period_start = ts_to_dt(sub_obj, "current_period_start");
    let period_end = ts_to_dt(sub_obj, "current_period_end");
    let trial_end = ts_to_dt(sub_obj, "trial_end");
    let stripe_sub_id = sub_obj.get("id").and_then(|v| v.as_str());

    let applied = db::billing::apply_stripe_subscription_event(
        pool,
        tenant_id,
        &plan_id,
        &status,
        period_start,
        period_end,
        trial_end,
        stripe_sub_id,
        event_created_at,
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !applied {
        tracing::info!(%tenant_id, %event_created_at, "stripe webhook: ignored stale subscription event");
    }
    Ok(())
}

/// Update only the status on an existing subscription row, resolving tenant by
/// the object's customer/metadata. Used by invoice.* events whose object is an
/// invoice (not a subscription), so we don't have plan/period info to upsert.
async fn force_status_for_customer(
    pool: &PgPool,
    obj: &serde_json::Value,
    status: &str,
    event_created_at: chrono::DateTime<chrono::Utc>,
) -> Result<(), ApiError> {
    let Some(tenant_id) = resolve_tenant(pool, obj).await? else {
        return Err(ApiError::Internal(
            "stripe webhook could not resolve a tenant for the invoice".into(),
        ));
    };
    let Some(invoice_subscription_id) = invoice_subscription_id_of(obj) else {
        // One-off invoices can share the customer but are not authoritative
        // for the tenant's recurring subscription state.
        tracing::warn!(%tenant_id, "stripe webhook: ignored invoice without a subscription id");
        return Ok(());
    };
    match db::billing::apply_stripe_status_event(
        pool,
        tenant_id,
        &invoice_subscription_id,
        status,
        event_created_at,
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?
    {
        db::billing::StripeStatusEventResult::Applied => Ok(()),
        db::billing::StripeStatusEventResult::Stale => {
            tracing::info!(%tenant_id, %event_created_at, "stripe webhook: ignored stale invoice event");
            Ok(())
        }
        db::billing::StripeStatusEventResult::SubscriptionNotFound => Err(ApiError::Internal(
            format!("stripe subscription has not arrived yet for tenant {tenant_id}"),
        )),
        db::billing::StripeStatusEventResult::SubscriptionMismatch => {
            // A historical subscription on the same Stripe customer is a
            // valid event, but cannot mutate the current entitlement.
            tracing::warn!(%tenant_id, "stripe webhook: ignored invoice for a different subscription");
            Ok(())
        }
    }
}

async fn stripe_webhook_inner(
    pool: &PgPool,
    webhook_secret: Option<&str>,
    headers: &HeaderMap,
    body: Bytes,
) -> Result<StatusCode, ApiError> {
    // 1. Verify signature (or skip with a warn outside production).
    match webhook_secret {
        Some(secret) => {
            let sig = headers
                .get("stripe-signature")
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| ApiError::BadRequest("missing Stripe-Signature header".into()))?;
            let now = chrono::Utc::now().timestamp();
            if let Err(e) =
                crate::services::billing::verify_webhook_signature(&body, sig, secret, 300, now)
            {
                tracing::warn!(?e, "stripe webhook signature verification failed");
                return Err(ApiError::BadRequest("invalid signature".into()));
            }
        }
        None => {
            tracing::warn!(
                "STRIPE_WEBHOOK_SECRET unset (non-prod): skipping webhook signature verification"
            );
        }
    }

    // 2. Parse the event envelope.
    let event: serde_json::Value = serde_json::from_slice(&body)
        .map_err(|e| ApiError::BadRequest(format!("invalid json: {e}")))?;
    let event_id = event
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ApiError::BadRequest("event missing id".into()))?;
    let event_type = event
        .get("type")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ApiError::BadRequest("event missing type".into()))?;
    let event_created_at = event
        .get("created")
        .and_then(|value| value.as_i64())
        .and_then(|seconds| chrono::DateTime::<chrono::Utc>::from_timestamp(seconds, 0))
        .ok_or_else(|| ApiError::BadRequest("event missing valid created timestamp".into()))?;
    let object = event
        .get("data")
        .and_then(|d| d.get("object"))
        .cloned()
        .unwrap_or(serde_json::Value::Null);

    // 3. Idempotency: claim the event. Completed events no-op; failed or
    //    abandoned attempts remain retryable instead of being poisoned merely
    //    because their id was inserted before the business mutation finished.
    let claimed = db::billing::claim_stripe_event(pool, event_id, event_type)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !claimed {
        tracing::info!(%event_id, %event_type, "stripe webhook: completed or in-flight event, no-op");
        return Ok(StatusCode::OK);
    }

    // 4. Dispatch on event type. Only mark the claim complete after every
    //    required mutation succeeds. On an error, release it so Stripe's next
    //    delivery can retry immediately.
    let dispatch_result: Result<(), ApiError> = async {
        match event_type {
            "customer.subscription.created" | "customer.subscription.updated" => {
                apply_subscription(pool, &object, None, event_created_at).await?;
            }
            "customer.subscription.deleted" => {
                apply_subscription(pool, &object, Some("canceled"), event_created_at).await?;
            }
            "invoice.payment_failed" => {
                force_status_for_customer(pool, &object, "past_due", event_created_at).await?;
            }
            "invoice.payment_succeeded" => {
                force_status_for_customer(pool, &object, "active", event_created_at).await?;
            }
            "checkout.session.completed" => {
                let tenant_id = resolve_tenant(pool, &object).await?.ok_or_else(|| {
                    ApiError::Internal(
                        "stripe checkout completed without a resolvable tenant".into(),
                    )
                })?;
                let customer_id = customer_id_of(&object).ok_or_else(|| {
                    ApiError::Internal("stripe checkout completed without a customer".into())
                })?;
                db::billing::set_tenant_stripe_customer(pool, tenant_id, &customer_id)
                    .await
                    .map_err(|e| ApiError::Internal(e.to_string()))?;
            }
            other => {
                tracing::info!(%event_id, event_type = %other, "stripe webhook: unhandled event type, no-op");
            }
        }
        Ok(())
    }
    .await;

    if let Err(err) = dispatch_result {
        let error_detail = err.to_string();
        if let Err(release_err) =
            db::billing::release_stripe_event(pool, event_id, &error_detail).await
        {
            tracing::error!(?release_err, %event_id, "stripe webhook: failed to release event claim");
        }
        return Err(err);
    }

    db::billing::complete_stripe_event(pool, event_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(StatusCode::OK)
}

async fn stripe_webhook(
    State(s): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, ApiError> {
    stripe_webhook_inner(&s.pool, s.stripe_webhook_secret.as_deref(), &headers, body).await
}

async fn stripe_webhook_t(
    State(s): State<BillingTestState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, ApiError> {
    stripe_webhook_inner(&s.pool, s.stripe_webhook_secret.as_deref(), &headers, body).await
}

#[cfg(test)]
mod tests {
    use super::{billing_portal_return_url, resolve_plan_id_from_catalog};
    use crate::db::billing::PlanRow;
    use serde_json::json;

    fn plan(id: &str, stripe_price_id: Option<&str>) -> PlanRow {
        PlanRow {
            id: id.to_string(),
            name: id.to_string(),
            monthly_price_cents: 1_000,
            included_seats: 10,
            included_class_minutes: 100,
            included_recording_gb: 10,
            stripe_price_id: stripe_price_id.map(str::to_string),
        }
    }

    #[test]
    fn portal_return_url_is_derived_from_bare_app_origin() {
        assert_eq!(
            billing_portal_return_url("https://app.aulalite.test/").unwrap(),
            "https://app.aulalite.test/admin/billing"
        );
        assert_eq!(
            billing_portal_return_url("http://localhost:3000").unwrap(),
            "http://localhost:3000/admin/billing"
        );
    }

    #[test]
    fn portal_return_url_rejects_non_origin_configuration() {
        for invalid in [
            "javascript:alert(1)",
            "https://user:pass@app.test",
            "https://app.test/path",
            "https://app.test?next=https://evil.test",
            "//app.test",
        ] {
            assert!(
                billing_portal_return_url(invalid).is_err(),
                "accepted {invalid}"
            );
        }
    }

    #[test]
    fn current_subscription_lookup_key_wins_over_checkout_metadata() {
        let plans = [plan("starter", None), plan("pro", None)];
        let subscription = json!({
            "metadata": { "plan_id": "starter" },
            "items": { "data": [{
                "price": { "id": "price_portal_pro", "lookup_key": "pro" }
            }] }
        });

        assert_eq!(
            resolve_plan_id_from_catalog(&subscription, &plans, |_| None).unwrap(),
            Some("pro".to_string())
        );
    }

    #[test]
    fn price_ids_map_from_database_and_deploy_configuration() {
        let plans = [plan("starter", Some("price_db_starter")), plan("pro", None)];
        let deploy_price = |plan_id: &str| match plan_id {
            "pro" => Some("price_env_pro".to_string()),
            _ => None,
        };

        for (price_id, expected_plan) in [("price_db_starter", "starter"), ("price_env_pro", "pro")]
        {
            let subscription = json!({
                "metadata": { "plan_id": "stale-plan" },
                "items": { "data": [{ "price": { "id": price_id } }] }
            });
            assert_eq!(
                resolve_plan_id_from_catalog(&subscription, &plans, deploy_price).unwrap(),
                Some(expected_plan.to_string())
            );
        }
    }

    #[test]
    fn metadata_fallback_requires_a_stripe_configured_catalog_plan() {
        let plans = [plan("starter", None), plan("pro", Some("price_db_pro"))];
        let configured_metadata = json!({ "metadata": { "plan_id": "pro" } });
        assert_eq!(
            resolve_plan_id_from_catalog(&configured_metadata, &plans, |_| None).unwrap(),
            Some("pro".to_string())
        );

        let unconfigured_metadata = json!({ "metadata": { "plan_id": "starter" } });
        assert!(resolve_plan_id_from_catalog(&unconfigured_metadata, &plans, |_| None).is_err());
    }

    #[test]
    fn unknown_current_price_never_falls_back_to_stale_metadata() {
        let plans = [plan("starter", Some("price_starter"))];
        let subscription = json!({
            "metadata": { "plan_id": "starter" },
            "items": { "data": [{ "price": { "id": "price_unknown" } }] }
        });

        assert!(resolve_plan_id_from_catalog(&subscription, &plans, |_| None).is_err());
    }
}
