//! Notifications backend integration tests.
//!
//! Flows exercised (mirrors the member_invitations.rs harness):
//!   * create -> list -> unread_count -> mark_read (single + read-all);
//!   * preferences default (all-true) + PATCH a single channel;
//!   * device-token register (upsert) + remove;
//!   * the `notify` facade with in_app_enabled=false suppresses the in-app row,
//!     while email/push mocks still record their best-effort sends.
//!
//! These require a live Postgres (DATABASE_URL); they are compile-checked in CI
//! and run locally against the dev database.

mod fixtures;

use fixtures::*;
use serde_json::json;
use std::ops::Not;
use std::sync::Arc;
use uuid::Uuid;

/// Build a notifications test app scoped to `user` in `tenant`.
fn notif_app(pool: &sqlx::PgPool, user: Uuid, email: String, tenant: Uuid) -> axum::Router {
    build_test_app(
        backend::handlers::notifications::router_for_tests(pool.clone()),
        StubAuth {
            pool: pool.clone(),
            user_id: user,
            firebase_uid: format!("fb-{}", Uuid::new_v4()),
            email,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Student),
        },
    )
}

fn admin_notif_app(
    pool: &sqlx::PgPool,
    user: Uuid,
    email: String,
    tenant: Uuid,
    role: core_types::TenantRole,
) -> axum::Router {
    build_test_app(
        backend::handlers::notifications::router_for_tests(pool.clone()),
        StubAuth {
            pool: pool.clone(),
            user_id: user,
            firebase_uid: format!("fb-{}", Uuid::new_v4()),
            email,
            tenant_id: Some(tenant),
            tenant_role: Some(role),
        },
    )
}

#[tokio::test]
async fn create_list_unread_count_mark_read_flow() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (user, _, email) = create_user(&pool).await;
    attach_membership(&pool, tenant, user, "student").await;

    // Seed two notifications directly via the db layer.
    let n1 = backend::db::notifications::create_notification(
        &pool,
        tenant,
        user,
        "grade_released",
        "Grade released",
        Some("Your grade for \"HW1\" has been released."),
        Some("https://app/x"),
    )
    .await
    .unwrap();
    let _n2 = backend::db::notifications::create_notification(
        &pool, tenant, user, "generic", "Hello", None, None,
    )
    .await
    .unwrap();

    let app = notif_app(&pool, user, email, tenant);

    // list -> newest first, both visible.
    let (status, body) = fire(&app, "GET", "/v1/me/notifications?limit=10", None).await;
    assert_eq!(status, 200, "{body}");
    let arr = body.as_array().unwrap();
    assert!(arr.len() >= 2, "expected >= 2, got {}", arr.len());
    assert_eq!(arr[0]["title"], "Hello"); // newest first

    // unread-count -> 2.
    let (status, body) = fire(&app, "GET", "/v1/me/notifications/unread-count", None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["count"].as_i64().unwrap(), 2);

    // mark one read -> 204.
    let (status, _) = fire(
        &app,
        "POST",
        &format!("/v1/me/notifications/{}/read", n1.id),
        None,
    )
    .await;
    assert_eq!(status, 204);

    // unread-count -> 1 now.
    let (_, body) = fire(&app, "GET", "/v1/me/notifications/unread-count", None).await;
    assert_eq!(body["count"].as_i64().unwrap(), 1);

    // marking the same one again -> 404 (already read).
    let (status, _) = fire(
        &app,
        "POST",
        &format!("/v1/me/notifications/{}/read", n1.id),
        None,
    )
    .await;
    assert_eq!(status, 404);

    // read-all -> updated == remaining unread (1).
    let (status, body) = fire(&app, "POST", "/v1/me/notifications/read-all", None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["updated"].as_i64().unwrap(), 1);

    let (_, body) = fire(&app, "GET", "/v1/me/notifications/unread-count", None).await;
    assert_eq!(body["count"].as_i64().unwrap(), 0);
}

#[tokio::test]
async fn preferences_default_then_patch() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (user, _, email) = create_user(&pool).await;
    attach_membership(&pool, tenant, user, "student").await;
    let app = notif_app(&pool, user, email, tenant);

    // Default (no row) -> all true.
    let (status, body) = fire(&app, "GET", "/v1/me/notification-preferences", None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["email_enabled"], true);
    assert_eq!(body["push_enabled"], true);
    assert_eq!(body["in_app_enabled"], true);

    // PATCH only email_enabled=false -> others unchanged.
    let (status, body) = fire(
        &app,
        "PATCH",
        "/v1/me/notification-preferences",
        Some(json!({ "email_enabled": false })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["email_enabled"], false);
    assert_eq!(body["push_enabled"], true);
    assert_eq!(body["in_app_enabled"], true);

    // Re-read persists.
    let (_, body) = fire(&app, "GET", "/v1/me/notification-preferences", None).await;
    assert_eq!(body["email_enabled"], false);
}

#[tokio::test]
async fn device_token_register_then_remove() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (user, _, email) = create_user(&pool).await;
    attach_membership(&pool, tenant, user, "student").await;
    let app = notif_app(&pool, user, email, tenant);

    let token = format!("tok-{}", Uuid::new_v4());

    // register -> 204.
    let (status, _) = fire(
        &app,
        "POST",
        "/v1/me/device-tokens",
        Some(json!({ "token": token, "platform": "web" })),
    )
    .await;
    assert_eq!(status, 204);

    // re-register (upsert) -> still 204, no duplicate.
    let (status, _) = fire(
        &app,
        "POST",
        "/v1/me/device-tokens",
        Some(json!({ "token": token, "platform": "android" })),
    )
    .await;
    assert_eq!(status, 204);
    let tokens = backend::db::notifications::list_device_tokens(&pool, tenant, user)
        .await
        .unwrap();
    assert_eq!(tokens.iter().filter(|t| **t == token).count(), 1);

    // invalid platform -> 400.
    let (status, _) = fire(
        &app,
        "POST",
        "/v1/me/device-tokens",
        Some(json!({ "token": "x", "platform": "blackberry" })),
    )
    .await;
    assert_eq!(status, 400);

    // remove -> 204, token gone.
    let (status, _) = fire(
        &app,
        "DELETE",
        "/v1/me/device-tokens",
        Some(json!({ "token": token })),
    )
    .await;
    assert_eq!(status, 204);
    let tokens = backend::db::notifications::list_device_tokens(&pool, tenant, user)
        .await
        .unwrap();
    assert!(!tokens.contains(&token));
}

#[tokio::test]
async fn device_tokens_can_be_listed_and_revoked_by_id() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (user, _, email) = create_user(&pool).await;
    attach_membership(&pool, tenant, user, "student").await;
    let app = notif_app(&pool, user, email, tenant);
    let token = format!("tok-{}", Uuid::new_v4());

    let (status, _) = fire(
        &app,
        "POST",
        "/v1/me/device-tokens",
        Some(json!({
            "token": token,
            "platform": "web",
            "label": "Chrome on Windows",
            "user_agent": "Mozilla/5.0"
        })),
    )
    .await;
    assert_eq!(status, 204);

    let rows = backend::db::notifications::list_device_token_rows(&pool, tenant, user)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].platform, "web");
    assert_eq!(rows[0].label.as_deref(), Some("Chrome on Windows"));
    assert_eq!(rows[0].user_agent.as_deref(), Some("Mozilla/5.0"));

    backend::db::notifications::revoke_device_token_by_id(&pool, tenant, user, rows[0].id)
        .await
        .unwrap();
    let rows = backend::db::notifications::list_device_token_rows(&pool, tenant, user)
        .await
        .unwrap();
    assert!(rows.is_empty());
}

#[tokio::test]
async fn delivery_rows_are_tenant_scoped_and_sanitized() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (user, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, user, "student").await;
    let device_id = backend::db::notifications::register_device_token(
        &pool,
        tenant,
        user,
        "secret-push-token",
        "web",
        Some("Browser"),
        Some("Mozilla/5.0"),
    )
    .await
    .unwrap();

    let row = backend::db::notifications::record_delivery(
        &pool,
        backend::db::notifications::NewDelivery {
            tenant_id: tenant,
            user_id: user,
            notification_id: None,
            channel: "push",
            provider: "fcm",
            target_hash: &backend::db::notifications::target_hash("push", "secret-push-token"),
            target_label: Some("web - Browser"),
            device_token_id: Some(device_id),
            kind: "test",
            status: "sent",
            provider_message_id: Some("projects/p/messages/123"),
            provider_status: Some("200"),
            error_code: None,
            error_message: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(row.status, "sent");
    assert_ne!(row.target_hash, "secret-push-token");
    assert_eq!(row.target_label.as_deref(), Some("web - Browser"));

    let listed = backend::db::notifications::list_deliveries_for_admin(
        &pool,
        tenant,
        user,
        backend::db::notifications::DeliveryFilter {
            channel: Some("push".into()),
            status: Some("sent".into()),
            user_id: Some(user),
            kind: Some("test".into()),
            provider: Some("fcm".into()),
            before: None,
            limit: 20,
        },
    )
    .await
    .unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, row.id);
}

#[tokio::test]
async fn device_list_and_revoke_routes_are_owner_scoped() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (user, _, email) = create_user(&pool).await;
    attach_membership(&pool, tenant, user, "student").await;
    let app = notif_app(&pool, user, email, tenant);

    let token = format!("tok-{}", Uuid::new_v4());
    let (status, _) = fire(
        &app,
        "POST",
        "/v1/me/device-tokens",
        Some(json!({
            "token": token,
            "platform": "web",
            "label": "Edge",
            "user_agent": "Mozilla/5.0"
        })),
    )
    .await;
    assert_eq!(status, 204);

    let (status, body) = fire(&app, "GET", "/v1/me/device-tokens", None).await;
    assert_eq!(status, 200, "{body}");
    let devices = body["devices"].as_array().unwrap();
    assert_eq!(devices.len(), 1);
    let id = devices[0]["id"].as_str().unwrap();
    assert_eq!(devices[0]["label"], "Edge");
    assert_eq!(devices[0]["platform"], "web");
    assert!(body.to_string().contains("tok-").not());

    let (status, _) = fire(&app, "DELETE", &format!("/v1/me/device-tokens/{id}"), None).await;
    assert_eq!(status, 204);
    let (_, body) = fire(&app, "GET", "/v1/me/device-tokens", None).await;
    assert_eq!(body["devices"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn admin_delivery_routes_require_admin_role() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (student, _, student_email) = create_user(&pool).await;
    attach_membership(&pool, tenant, student, "student").await;
    let (admin, _, admin_email) = create_user(&pool).await;
    attach_membership(&pool, tenant, admin, "org_admin").await;

    let target_hash = backend::db::notifications::target_hash("push", "tok-secret");
    let row = backend::db::notifications::record_delivery(
        &pool,
        backend::db::notifications::NewDelivery {
            tenant_id: tenant,
            user_id: student,
            notification_id: None,
            channel: "push",
            provider: "fcm",
            target_hash: &target_hash,
            target_label: Some("web"),
            device_token_id: None,
            kind: "test",
            status: "failed",
            provider_message_id: None,
            provider_status: Some("404"),
            error_code: Some("UNREGISTERED"),
            error_message: Some("token is not registered"),
        },
    )
    .await
    .unwrap();

    let student_app = admin_notif_app(
        &pool,
        student,
        student_email,
        tenant,
        core_types::TenantRole::Student,
    );
    let (status, _) = fire(
        &student_app,
        "GET",
        "/v1/admin/notification-deliveries",
        None,
    )
    .await;
    assert_eq!(status, 403);

    let admin_app = admin_notif_app(
        &pool,
        admin,
        admin_email,
        tenant,
        core_types::TenantRole::OrgAdmin,
    );
    let (status, body) = fire(
        &admin_app,
        "GET",
        "/v1/admin/notification-deliveries?channel=push&status=failed",
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["deliveries"].as_array().unwrap().len(), 1);
    assert_eq!(body["deliveries"][0]["id"], row.id.to_string());
    assert!(!body.to_string().contains("tok-secret"));

    let (status, body) = fire(
        &admin_app,
        "GET",
        &format!("/v1/admin/notification-deliveries/{}", row.id),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["id"], row.id.to_string());
}

#[tokio::test]
async fn notify_facade_in_app_disabled_suppresses_row() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (user, _, _email) = create_user(&pool).await;
    attach_membership(&pool, tenant, user, "student").await;

    // Disable in_app, keep email + push on.
    backend::db::notifications::set_preferences(&pool, user, true, true, false)
        .await
        .unwrap();

    let email = Arc::new(backend::services::notifications::mock::MockEmailNotifier::new());
    let push = Arc::new(backend::services::notifications::mock::MockPushSender::new());

    backend::services::notifications::notify(
        &pool,
        email.as_ref(),
        push.as_ref(),
        tenant,
        user,
        "grade_released",
        "Grade released",
        Some("body text"),
        Some("https://app/x"),
    )
    .await;

    // No in-app row was created (in_app disabled).
    let rows = backend::db::notifications::list_for_user(&pool, tenant, user, 50, None)
        .await
        .unwrap();
    assert_eq!(rows.len(), 0, "in-app row should be suppressed");

    // Email still attempted (user has an email on file).
    assert_eq!(email.calls().len(), 1);
    assert_eq!(email.calls()[0].1, "Grade released");

    // Push: no device tokens registered -> facade skips send_push.
    assert_eq!(push.calls().len(), 0);
}

#[tokio::test]
async fn notify_facade_in_app_enabled_persists_row() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (user, _, _email) = create_user(&pool).await;
    attach_membership(&pool, tenant, user, "student").await;

    // Defaults: all enabled (no preferences row written).
    let email = Arc::new(backend::services::notifications::mock::MockEmailNotifier::new());
    let push = Arc::new(backend::services::notifications::mock::MockPushSender::new());

    backend::services::notifications::notify(
        &pool,
        email.as_ref(),
        push.as_ref(),
        tenant,
        user,
        "grade_released",
        "Grade released",
        Some("body text"),
        Some("https://app/x"),
    )
    .await;

    let rows = backend::db::notifications::list_for_user(&pool, tenant, user, 50, None)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].kind, "grade_released");
    assert_eq!(rows[0].title, "Grade released");
    assert_eq!(email.calls().len(), 1);
}

#[tokio::test]
async fn notify_facade_records_delivery_rows() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (user, _, _email) = create_user(&pool).await;
    attach_membership(&pool, tenant, user, "student").await;
    let device_id = backend::db::notifications::register_device_token(
        &pool,
        tenant,
        user,
        "tok-for-delivery-log",
        "web",
        Some("Chrome"),
        None,
    )
    .await
    .unwrap();

    let email = Arc::new(backend::services::notifications::mock::MockEmailNotifier::new());
    let push = Arc::new(backend::services::notifications::mock::MockPushSender::new());

    backend::services::notifications::notify(
        &pool,
        email.as_ref(),
        push.as_ref(),
        tenant,
        user,
        "grade_released",
        "Grade released",
        Some("body text"),
        Some("/app/courses/math"),
    )
    .await;

    let rows = backend::db::notifications::list_deliveries_for_admin(
        &pool,
        tenant,
        user,
        backend::db::notifications::DeliveryFilter {
            channel: Some("push".into()),
            status: Some("sent".into()),
            limit: 20,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].device_token_id, Some(device_id));
    assert_eq!(rows[0].target_label.as_deref(), Some("web - Chrome"));
    assert!(!rows[0].target_hash.contains("tok-for-delivery-log"));

    let email_rows = backend::db::notifications::list_deliveries_for_admin(
        &pool,
        tenant,
        user,
        backend::db::notifications::DeliveryFilter {
            channel: Some("email".into()),
            status: Some("sent".into()),
            limit: 20,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(email_rows.len(), 1);
    assert_eq!(email_rows[0].target_label.as_deref(), Some("email"));
    assert!(!email_rows[0].target_hash.contains("@example.test"));
}
