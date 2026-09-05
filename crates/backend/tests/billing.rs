// crates/backend/tests/billing.rs
//
// Integration tests for the billing HTTP API. These exercise:
//   * webhook idempotency (same event id twice -> second is a no-op),
//   * a `customer.subscription.updated` event writing the subscription row,
//   * `GET /v1/admin/billing` returning plan + subscription + usage,
//   * `GET /v1/admin/billing` being 403 for a student,
//   * a bad webhook signature -> 400 (valid header crafted with a known secret,
//     then tampered).
//
// They are COMPILE-ONLY in CI without a live Postgres: every test opens a real
// pool via `fixtures::pool()`, so they only execute when `DATABASE_URL` points
// at a migrated database. The harness mirrors the other integration tests.

mod fixtures;

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use fixtures::*;
use hmac::{Hmac, KeyInit, Mac};
use serde_json::{json, Value};
use sha2::Sha256;
use tower::ServiceExt;

type HmacSha256 = Hmac<Sha256>;

const TEST_WEBHOOK_SECRET: &str = "whsec_test_secret_billing";

/// Compute the hex `v1` signature Stripe would send for `payload` at time `t`,
/// matching `services::billing::verify_webhook_signature`'s scheme
/// (`HMAC-SHA256("{t}.{payload}")`).
fn stripe_sign(secret: &str, t: i64, payload: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(t.to_string().as_bytes());
    mac.update(b".");
    mac.update(payload);
    let bytes = mac.finalize().into_bytes();
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Build a billing test state with the mock Stripe client and the known
/// webhook secret.
fn billing_state(pool: &sqlx::PgPool) -> backend::handlers::billing::BillingTestState {
    backend::handlers::billing::BillingTestState {
        pool: pool.clone(),
        stripe: Arc::new(backend::services::billing::MockStripeClient::new()),
        stripe_webhook_secret: Some(TEST_WEBHOOK_SECRET.to_string()),
        app_origin: "https://app.test".to_string(),
    }
}

/// Fire a POST at the (no-auth) webhook router with a raw body + signature
/// header. Returns just the status (the webhook returns no JSON body).
async fn fire_webhook(app: &axum::Router, raw_body: &str, sig_header: Option<&str>) -> StatusCode {
    let mut builder = Request::builder()
        .method("POST")
        .uri("/v1/stripe/webhook")
        .header("content-type", "application/json");
    if let Some(sig) = sig_header {
        builder = builder.header("stripe-signature", sig);
    }
    let req = builder.body(Body::from(raw_body.to_string())).unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    resp.status()
}

/// Read the raw count of stripe_events rows for an id (for the idempotency
/// assertion).
async fn stripe_event_count(pool: &sqlx::PgPool, event_id: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM stripe_events WHERE id = $1")
        .bind(event_id)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// Read a tenant's subscription row (status, plan_id) directly, bypassing the
/// API. The webhook writes under the tenant GUC, so we set it here too.
async fn read_subscription(pool: &sqlx::PgPool, tenant: uuid::Uuid) -> Option<(String, String)> {
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    let row: Option<(String, String)> =
        sqlx::query_as("SELECT status, plan_id FROM subscriptions WHERE tenant_id = $1")
            .bind(tenant)
            .fetch_optional(&mut *tx)
            .await
            .unwrap();
    tx.commit().await.unwrap();
    row
}

/// Set a tenant's stripe_customer_id so webhook events can resolve it by
/// customer. Runs with the tenant GUC set for RLS.
async fn set_tenant_customer(pool: &sqlx::PgPool, tenant: uuid::Uuid, customer_id: &str) {
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("UPDATE tenants SET stripe_customer_id = $2 WHERE id = $1")
        .bind(tenant)
        .bind(customer_id)
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
}

async fn create_checkout_plan(pool: &sqlx::PgPool, label: &str) -> String {
    let plan_id = format!("{label}-{}", uuid::Uuid::new_v4().simple());
    let price_id = format!("price_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(
        "INSERT INTO plans
            (id, name, monthly_price_cents, included_seats,
             included_class_minutes, included_recording_gb, stripe_price_id)
         VALUES ($1, $1, 1000, 10, 100, 10, $2)",
    )
    .bind(&plan_id)
    .bind(price_id)
    .execute(pool)
    .await
    .unwrap();
    plan_id
}

/// Build a `customer.subscription.updated` event JSON for the given tenant,
/// plan, and status, resolving the tenant via metadata.tenant_id.
fn subscription_updated_event(
    event_id: &str,
    tenant: uuid::Uuid,
    plan_id: &str,
    status: &str,
) -> Value {
    let now = chrono::Utc::now().timestamp();
    json!({
        "id": event_id,
        "type": "customer.subscription.updated",
        "created": now,
        "data": {
            "object": {
                "id": "sub_test_123",
                "status": status,
                "customer": "cus_test_123",
                "current_period_start": now,
                "current_period_end": now + 2_592_000,
                "trial_end": Value::Null,
                "metadata": { "tenant_id": tenant.to_string(), "plan_id": plan_id },
                "items": {
                    "data": [ { "price": { "id": plan_id, "lookup_key": plan_id } } ]
                }
            }
        }
    })
}

#[tokio::test]
async fn webhook_is_idempotent_on_repeated_event_id() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    set_tenant_customer(&pool, tenant, "cus_test_123").await;

    let app = build_test_app_no_auth(backend::handlers::billing::webhook_router_for_tests(
        billing_state(&pool),
    ));

    let event_id = format!("evt_idem_{}", uuid::Uuid::new_v4());
    let event = subscription_updated_event(&event_id, tenant, "pro", "active");
    // Serialize once; sign EXACTLY those bytes so verification matches.
    let raw = serde_json::to_string(&event).unwrap();
    let t = chrono::Utc::now().timestamp();
    let sig = stripe_sign(TEST_WEBHOOK_SECRET, t, raw.as_bytes());
    let header = format!("t={t},v1={sig}");

    // First delivery: processed.
    let s1 = fire_webhook(&app, &raw, Some(&header)).await;
    assert_eq!(s1, 200);
    assert_eq!(stripe_event_count(&pool, &event_id).await, 1);

    // Second delivery (same id): no-op, still 200, still exactly one ledger row.
    let s2 = fire_webhook(&app, &raw, Some(&header)).await;
    assert_eq!(s2, 200);
    assert_eq!(
        stripe_event_count(&pool, &event_id).await,
        1,
        "duplicate event must not insert a second ledger row"
    );
}

#[tokio::test]
async fn subscription_updated_event_writes_subscription_row() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    set_tenant_customer(&pool, tenant, "cus_test_123").await;

    let app = build_test_app_no_auth(backend::handlers::billing::webhook_router_for_tests(
        billing_state(&pool),
    ));

    let event_id = format!("evt_upd_{}", uuid::Uuid::new_v4());
    let event = subscription_updated_event(&event_id, tenant, "pro", "active");
    let raw = serde_json::to_string(&event).unwrap();
    let t = chrono::Utc::now().timestamp();
    let sig = stripe_sign(TEST_WEBHOOK_SECRET, t, raw.as_bytes());
    let header = format!("t={t},v1={sig}");

    let status = fire_webhook(&app, &raw, Some(&header)).await;
    assert_eq!(status, 200);

    let row = read_subscription(&pool, tenant).await;
    assert_eq!(row, Some(("active".to_string(), "pro".to_string())));
}

#[tokio::test]
async fn portal_plan_change_uses_current_price_instead_of_checkout_metadata() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    set_tenant_customer(&pool, tenant, "cus_test_123").await;

    let app = build_test_app_no_auth(backend::handlers::billing::webhook_router_for_tests(
        billing_state(&pool),
    ));

    // Stripe retains the metadata stamped during the original Starter
    // checkout, while Customer Portal has changed the current item to Pro.
    let event_id = format!("evt_portal_plan_{}", uuid::Uuid::new_v4());
    let mut event = subscription_updated_event(&event_id, tenant, "pro", "active");
    event["data"]["object"]["metadata"]["plan_id"] = json!("starter");
    let raw = serde_json::to_string(&event).unwrap();
    let t = chrono::Utc::now().timestamp();
    let sig = stripe_sign(TEST_WEBHOOK_SECRET, t, raw.as_bytes());
    let header = format!("t={t},v1={sig}");

    assert_eq!(fire_webhook(&app, &raw, Some(&header)).await, 200);
    assert_eq!(
        read_subscription(&pool, tenant).await,
        Some(("active".to_string(), "pro".to_string()))
    );
}

#[tokio::test]
async fn older_subscription_event_cannot_overwrite_newer_state() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    set_tenant_customer(&pool, tenant, "cus_test_123").await;
    let app = build_test_app_no_auth(backend::handlers::billing::webhook_router_for_tests(
        billing_state(&pool),
    ));

    let now = chrono::Utc::now().timestamp();
    let mut newer = subscription_updated_event(
        &format!("evt_newer_{}", uuid::Uuid::new_v4()),
        tenant,
        "pro",
        "active",
    );
    newer["created"] = json!(now);
    let mut older = subscription_updated_event(
        &format!("evt_older_{}", uuid::Uuid::new_v4()),
        tenant,
        "starter",
        "past_due",
    );
    older["created"] = json!(now - 60);

    for event in [newer, older] {
        let raw = serde_json::to_string(&event).unwrap();
        let signed_at = chrono::Utc::now().timestamp();
        let sig = stripe_sign(TEST_WEBHOOK_SECRET, signed_at, raw.as_bytes());
        let header = format!("t={signed_at},v1={sig}");
        assert_eq!(fire_webhook(&app, &raw, Some(&header)).await, 200);
    }

    assert_eq!(
        read_subscription(&pool, tenant).await,
        Some(("active".to_string(), "pro".to_string()))
    );
}

#[tokio::test]
async fn webhook_rejects_bad_signature_with_400() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    set_tenant_customer(&pool, tenant, "cus_test_123").await;

    let app = build_test_app_no_auth(backend::handlers::billing::webhook_router_for_tests(
        billing_state(&pool),
    ));

    let event_id = format!("evt_badsig_{}", uuid::Uuid::new_v4());
    let event = subscription_updated_event(&event_id, tenant, "pro", "active");
    let raw = serde_json::to_string(&event).unwrap();
    let t = chrono::Utc::now().timestamp();

    // Valid header over the real body, then tamper the body so the HMAC no
    // longer matches -> verification fails -> 400.
    let sig = stripe_sign(TEST_WEBHOOK_SECRET, t, raw.as_bytes());
    let header = format!("t={t},v1={sig}");
    let tampered = raw.replace("active", "past_due");
    assert_ne!(tampered, raw, "tamper must change the body");

    let status = fire_webhook(&app, &tampered, Some(&header)).await;
    assert_eq!(status, 400, "tampered body must fail signature check");

    // The event must NOT have been recorded (we reject before dedupe).
    assert_eq!(stripe_event_count(&pool, &event_id).await, 0);

    // Sanity: the correctly-signed original is accepted.
    let ok = fire_webhook(&app, &raw, Some(&header)).await;
    assert_eq!(ok, 200);
}

#[tokio::test]
async fn get_admin_billing_returns_plan_subscription_and_usage() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (admin, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, admin, "org_owner").await;

    // Seed a subscription row on the 'pro' plan via the db layer (tenant GUC).
    backend::db::billing::upsert_subscription(
        &pool,
        tenant,
        "pro",
        "active",
        Some(chrono::Utc::now()),
        Some(chrono::Utc::now() + chrono::Duration::days(30)),
        None,
        Some("sub_seed_1"),
    )
    .await
    .unwrap();

    let app = build_test_app(
        backend::handlers::billing::router_for_tests(billing_state(&pool)),
        StubAuth {
            pool: pool.clone(),
            user_id: admin,
            firebase_uid: fb,
            email: em,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::OrgOwner),
        },
    );

    let (s, body) = fire(&app, "GET", "/v1/admin/billing", None).await;
    assert_eq!(s, 200, "{body}");
    assert_eq!(body["plan"]["id"], "pro");
    assert_eq!(body["subscription"]["plan_id"], "pro");
    assert_eq!(body["subscription"]["status"], "active");
    assert_eq!(body["overage_behavior"], "block");
    // Usage block is always present; the admin themselves is one active seat.
    assert!(body["usage"]["active_seats"].as_i64().unwrap() >= 1);
    assert_eq!(body["usage"]["included_seats"], 250);
    assert!(body["usage"]["class_minutes_used"].as_i64().is_some());
    assert!(body["usage"]["recording_gb_used"].as_f64().is_some());
}

#[tokio::test]
async fn get_admin_billing_is_403_for_student() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (student, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, student, "student").await;

    let app = build_test_app(
        backend::handlers::billing::router_for_tests(billing_state(&pool)),
        StubAuth {
            pool: pool.clone(),
            user_id: student,
            firebase_uid: fb,
            email: em,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Student),
        },
    );

    let (s, _) = fire(&app, "GET", "/v1/admin/billing", None).await;
    assert_eq!(s, 403);
}

#[tokio::test]
async fn checkout_session_returns_url_for_admin() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let plan_id = create_checkout_plan(&pool, "checkout").await;
    let (admin, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, admin, "org_owner").await;

    let app = build_test_app(
        backend::handlers::billing::router_for_tests(billing_state(&pool)),
        StubAuth {
            pool: pool.clone(),
            user_id: admin,
            firebase_uid: fb,
            email: em,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::OrgOwner),
        },
    );

    let (s, body) = fire(
        &app,
        "POST",
        "/v1/admin/billing/checkout-session",
        Some(json!({ "plan_id": plan_id })),
    )
    .await;
    assert_eq!(s, 200, "{body}");
    // MockStripeClient returns a deterministic mock url.
    assert!(body["url"]
        .as_str()
        .unwrap()
        .starts_with("https://mock.stripe/checkout/"));
}

#[tokio::test]
async fn checkout_refuses_a_second_provider_subscription() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (admin, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, admin, "org_owner").await;
    backend::db::billing::upsert_subscription(
        &pool,
        tenant,
        "starter",
        "active",
        None,
        None,
        None,
        Some("sub_existing"),
    )
    .await
    .unwrap();

    let app = build_test_app(
        backend::handlers::billing::router_for_tests(billing_state(&pool)),
        StubAuth {
            pool: pool.clone(),
            user_id: admin,
            firebase_uid: fb,
            email: em,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::OrgOwner),
        },
    );

    let (status, body) = fire(
        &app,
        "POST",
        "/v1/admin/billing/checkout-session",
        Some(json!({ "plan_id": "pro" })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(body["error"].as_str().unwrap().contains("billing portal"));
}

#[tokio::test]
async fn concurrent_checkout_requests_reuse_one_tenant_intent() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let plan_id = create_checkout_plan(&pool, "concurrent-checkout").await;
    let other_plan_id = create_checkout_plan(&pool, "other-checkout").await;
    let (admin, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, admin, "org_owner").await;

    let app = build_test_app(
        backend::handlers::billing::router_for_tests(billing_state(&pool)),
        StubAuth {
            pool: pool.clone(),
            user_id: admin,
            firebase_uid: fb,
            email: em,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::OrgOwner),
        },
    );

    let request_body = json!({ "plan_id": plan_id });
    let (first, second) = tokio::join!(
        fire(
            &app,
            "POST",
            "/v1/admin/billing/checkout-session",
            Some(request_body.clone())
        ),
        fire(
            &app,
            "POST",
            "/v1/admin/billing/checkout-session",
            Some(request_body)
        )
    );
    assert_eq!(first.0, StatusCode::OK, "{}", first.1);
    assert_eq!(second.0, StatusCode::OK, "{}", second.1);
    assert_eq!(first.1["url"], second.1["url"]);

    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    let intent: (i64, String, Option<String>, Option<String>) = sqlx::query_as(
        "SELECT COUNT(*) OVER (), idempotency_key, stripe_session_id, checkout_url
           FROM stripe_checkout_intents
          WHERE tenant_id = $1",
    )
    .bind(tenant)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(intent.0, 1);
    assert!(intent.1.starts_with("aulalite-checkout-"));
    assert!(intent.2.is_some());
    assert_eq!(intent.3.as_deref(), first.1["url"].as_str());

    let (status, body) = fire(
        &app,
        "POST",
        "/v1/admin/billing/checkout-session",
        Some(json!({ "plan_id": other_plan_id })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(body["error"].as_str().unwrap().contains("already active"));
}

#[tokio::test]
async fn invoice_for_another_subscription_cannot_change_billing_status() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let customer_id = format!("cus_invoice_{}", uuid::Uuid::new_v4().simple());
    set_tenant_customer(&pool, tenant, &customer_id).await;
    backend::db::billing::upsert_subscription(
        &pool,
        tenant,
        "starter",
        "active",
        None,
        None,
        None,
        Some("sub_expected"),
    )
    .await
    .unwrap();
    let app = build_test_app_no_auth(backend::handlers::billing::webhook_router_for_tests(
        billing_state(&pool),
    ));

    let now = chrono::Utc::now().timestamp();
    let event_id = format!("evt_wrong_invoice_{}", uuid::Uuid::new_v4());
    let event = json!({
        "id": event_id,
        "type": "invoice.payment_failed",
        "created": now,
        "data": { "object": {
            "customer": customer_id.clone(),
            "subscription": "sub_different"
        }}
    });
    let raw = serde_json::to_string(&event).unwrap();
    let sig = stripe_sign(TEST_WEBHOOK_SECRET, now, raw.as_bytes());
    let header = format!("t={now},v1={sig}");
    assert_eq!(
        fire_webhook(&app, &raw, Some(&header)).await,
        StatusCode::OK
    );
    assert_eq!(
        read_subscription(&pool, tenant).await,
        Some(("active".to_string(), "starter".to_string()))
    );

    let matching_event_id = format!("evt_matching_invoice_{}", uuid::Uuid::new_v4());
    let matching_event = json!({
        "id": matching_event_id,
        "type": "invoice.payment_failed",
        "created": now + 1,
        "data": { "object": {
            "customer": customer_id,
            "subscription": { "id": "sub_expected" }
        }}
    });
    let matching_raw = serde_json::to_string(&matching_event).unwrap();
    let signed_at = chrono::Utc::now().timestamp();
    let matching_sig = stripe_sign(TEST_WEBHOOK_SECRET, signed_at, matching_raw.as_bytes());
    let matching_header = format!("t={signed_at},v1={matching_sig}");
    assert_eq!(
        fire_webhook(&app, &matching_raw, Some(&matching_header)).await,
        StatusCode::OK
    );
    assert_eq!(
        read_subscription(&pool, tenant).await,
        Some(("past_due".to_string(), "starter".to_string()))
    );
}

#[tokio::test]
async fn portal_session_returns_url_and_audits_for_admin() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (admin, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, admin, "org_owner").await;
    set_tenant_customer(&pool, tenant, "cus_portal_test").await;

    let app = build_test_app(
        backend::handlers::billing::router_for_tests(billing_state(&pool)),
        StubAuth {
            pool: pool.clone(),
            user_id: admin,
            firebase_uid: fb,
            email: em,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::OrgOwner),
        },
    );

    let (s, body) = fire(&app, "POST", "/v1/admin/billing/portal-session", None).await;
    assert_eq!(s, 200, "{body}");
    assert_eq!(body["url"], "https://mock.stripe/portal/cus_portal_test");

    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM audit_events
         WHERE tenant_id = $1 AND actor_user_id = $2
           AND action = 'billing.portal_session_created'",
    )
    .bind(tenant)
    .bind(admin)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn portal_session_requires_billing_capability() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (student, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, student, "student").await;
    set_tenant_customer(&pool, tenant, "cus_forbidden_test").await;

    let app = build_test_app(
        backend::handlers::billing::router_for_tests(billing_state(&pool)),
        StubAuth {
            pool: pool.clone(),
            user_id: student,
            firebase_uid: fb,
            email: em,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Student),
        },
    );

    let (s, _) = fire(&app, "POST", "/v1/admin/billing/portal-session", None).await;
    assert_eq!(s, 403);

    // Organization administrators operate the workspace but do not control
    // its commercial contract; that remains owner-only.
    let (admin, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, admin, "org_admin").await;
    let admin_app = build_test_app(
        backend::handlers::billing::router_for_tests(billing_state(&pool)),
        StubAuth {
            pool: pool.clone(),
            user_id: admin,
            firebase_uid: fb,
            email: em,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::OrgAdmin),
        },
    );
    let (s, _) = fire(&admin_app, "POST", "/v1/admin/billing/portal-session", None).await;
    assert_eq!(s, 403);
}

#[tokio::test]
async fn portal_session_requires_existing_stripe_customer() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (admin, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, admin, "org_owner").await;

    let app = build_test_app(
        backend::handlers::billing::router_for_tests(billing_state(&pool)),
        StubAuth {
            pool: pool.clone(),
            user_id: admin,
            firebase_uid: fb,
            email: em,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::OrgOwner),
        },
    );

    let (s, body) = fire(&app, "POST", "/v1/admin/billing/portal-session", None).await;
    assert_eq!(s, 409, "{body}");
    assert!(body["error"]
        .as_str()
        .unwrap()
        .contains("complete subscription checkout first"));
}

#[tokio::test]
async fn metered_overage_is_rejected_until_usage_billing_exists() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (admin, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, admin, "org_owner").await;

    // Need an existing subscription row for the overage UPDATE to affect a row.
    backend::db::billing::upsert_subscription(
        &pool, tenant, "starter", "active", None, None, None, None,
    )
    .await
    .unwrap();

    let app = build_test_app(
        backend::handlers::billing::router_for_tests(billing_state(&pool)),
        StubAuth {
            pool: pool.clone(),
            user_id: admin,
            firebase_uid: fb,
            email: em,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::OrgOwner),
        },
    );

    let (s, _) = fire(
        &app,
        "PATCH",
        "/v1/admin/billing/overage",
        Some(json!({ "overage_behavior": "metered" })),
    )
    .await;
    assert_eq!(s, 400);

    // Verify the row changed.
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    let behavior: String =
        sqlx::query_scalar("SELECT overage_behavior FROM subscriptions WHERE tenant_id = $1")
            .bind(tenant)
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(behavior, "block");
}
