mod fixtures;

use std::sync::Once;

use axum::http::StatusCode;
use backend::services::totp;
use fixtures::{
    attach_membership, build_test_app, create_tenant, create_user, fire, pool, StubAuth,
};
use serde_json::json;

const TEST_SESSION_SECRET: &str = "test-session-secret-with-enough-length";
static SSO_SESSION_SECRET: Once = Once::new();

fn ensure_session_secret() {
    SSO_SESSION_SECRET.call_once(|| {
        std::env::set_var("SSO_SESSION_SECRET", TEST_SESSION_SECRET);
    });
}

fn app(
    pool: sqlx::PgPool,
    user_id: uuid::Uuid,
    firebase_uid: &str,
    email: &str,
    tenant_id: uuid::Uuid,
) -> axum::Router {
    ensure_session_secret();
    build_test_app(
        backend::handlers::mfa::router_for_tests(pool.clone()),
        StubAuth {
            pool,
            user_id,
            firebase_uid: firebase_uid.to_string(),
            email: email.to_string(),
            tenant_id: Some(tenant_id),
            tenant_role: Some(core_types::TenantRole::Student),
        },
    )
}

async fn enrolled_user() -> (
    sqlx::PgPool,
    uuid::Uuid,
    uuid::Uuid,
    String,
    String,
    Vec<u8>,
) {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (user, fb, email) = create_user(&pool).await;
    attach_membership(&pool, tenant, user, "student").await;
    let secret = totp::generate_secret();
    let hashes = vec![backend::db::api_keys::hash_secret("abcde-23456")];
    assert!(backend::db::mfa::start_enrollment(&pool, user, &secret)
        .await
        .unwrap());
    assert!(
        backend::db::mfa::confirm_enrollment(&pool, user, &secret, &hashes)
            .await
            .unwrap()
    );
    (pool, tenant, user, fb, email, secret)
}

#[tokio::test]
async fn enabled_enrollment_cannot_be_replaced_by_a_stale_enroll_request() {
    let (pool, _tenant, user, _fb, _email, original_secret) = enrolled_user().await;
    let replacement_secret = totp::generate_secret();

    assert!(
        !backend::db::mfa::start_enrollment(&pool, user, &replacement_secret)
            .await
            .unwrap()
    );

    let stored = backend::db::mfa::get(&pool, user).await.unwrap().unwrap();
    assert!(stored.enabled);
    assert_eq!(stored.secret, original_secret);
}

#[tokio::test]
async fn stale_verified_seed_cannot_enable_a_new_pending_enrollment() {
    let pool = pool().await;
    let (user, _fb, _email) = create_user(&pool).await;
    let first_secret = totp::generate_secret();
    let second_secret = totp::generate_secret();
    assert!(
        backend::db::mfa::start_enrollment(&pool, user, &first_secret)
            .await
            .unwrap()
    );
    assert!(
        backend::db::mfa::start_enrollment(&pool, user, &second_secret)
            .await
            .unwrap()
    );

    assert!(!backend::db::mfa::confirm_enrollment(
        &pool,
        user,
        &first_secret,
        &[backend::db::api_keys::hash_secret("stale-code")],
    )
    .await
    .unwrap());
    let stored = backend::db::mfa::get(&pool, user).await.unwrap().unwrap();
    assert!(!stored.enabled);
    assert_eq!(stored.secret, second_secret);
}

async fn trusted_device_rows_for_token(
    pool: &sqlx::PgPool,
    user_id: uuid::Uuid,
    token: &str,
) -> i64 {
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.user_id', $1, true)")
        .bind(user_id.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();

    let rows = sqlx::query_scalar(
        "SELECT COUNT(*) FROM user_mfa_trusted_devices
          WHERE user_id = $1 AND token_hash = $2",
    )
    .bind(user_id)
    .bind(backend::db::mfa::trusted_device_hash(token))
    .fetch_one(&mut *tx)
    .await
    .unwrap();

    tx.commit().await.unwrap();
    rows
}

async fn audit_reset_events_for(
    pool: &sqlx::PgPool,
    tenant_id: uuid::Uuid,
    actor_user_id: uuid::Uuid,
    target_user_id: uuid::Uuid,
) -> i64 {
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();

    let rows = sqlx::query_scalar(
        "SELECT COUNT(*)
           FROM audit_events
          WHERE tenant_id = $1
            AND actor_user_id = $2
            AND action = 'user_mfa.recovery_codes.reset'
            AND resource_type = 'user'
            AND resource_id = $3",
    )
    .bind(tenant_id)
    .bind(actor_user_id)
    .bind(target_user_id)
    .fetch_one(&mut *tx)
    .await
    .unwrap();

    tx.commit().await.unwrap();
    rows
}

fn assert_stepup_token(body: &serde_json::Value, firebase_uid: &str, email: &str) {
    let token = body["stepup_token"].as_str().expect("stepup token");
    let session = backend::services::oidc::verify_session_token(TEST_SESSION_SECRET, token)
        .expect("valid step-up token");
    let claims = session.claims;
    assert_eq!(claims.sub, firebase_uid);
    assert_eq!(claims.email.as_deref(), Some(email));
    assert_eq!(claims.email_verified, Some(true));
    assert_eq!(claims.iss, backend::services::oidc::SSO_SESSION_ISS);
    assert_eq!(claims.aud, backend::services::oidc::SSO_SESSION_AUD);
}

#[tokio::test]
async fn totp_challenge_can_create_trusted_device() {
    let (pool, tenant, user, fb, email, secret) = enrolled_user().await;
    let app = app(pool.clone(), user, &fb, &email, tenant);
    let code = totp::code_at(&secret, chrono::Utc::now().timestamp());

    let (status, body) = fire(
        &app,
        "POST",
        "/v1/auth/mfa/challenge",
        Some(json!({
            "code": code,
            "remember_device": true,
            "device_label": "Work laptop"
        })),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["stepped_up"], true);
    assert_eq!(body["used_recovery_code"], false);
    assert_stepup_token(&body, &fb, &email);
    let token = body["trusted_device_token"].as_str().unwrap();
    assert!(token.len() >= 43);
    assert!(body["trusted_device_expires_at"].as_str().is_some());

    let rows = trusted_device_rows_for_token(&pool, user, token).await;
    assert_eq!(rows, 1);
}

#[tokio::test]
async fn recovery_code_challenge_consumes_code_once() {
    let (pool, tenant, user, fb, email, _secret) = enrolled_user().await;
    let app = app(pool, user, &fb, &email, tenant);

    let (status, body) = fire(
        &app,
        "POST",
        "/v1/auth/mfa/challenge",
        Some(json!({ "code": "abcde-23456" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["used_recovery_code"], true);
    assert_stepup_token(&body, &fb, &email);

    let (status, body) = fire(
        &app,
        "POST",
        "/v1/auth/mfa/challenge",
        Some(json!({ "code": "abcde-23456" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"], "invalid_code");
}

#[tokio::test]
async fn trusted_device_challenge_succeeds_then_revoke_blocks_it() {
    let (pool, tenant, user, fb, email, secret) = enrolled_user().await;
    let app = app(pool.clone(), user, &fb, &email, tenant);
    let code = totp::code_at(&secret, chrono::Utc::now().timestamp());
    let (status, body) = fire(
        &app,
        "POST",
        "/v1/auth/mfa/challenge",
        Some(json!({ "code": code, "remember_device": true })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_stepup_token(&body, &fb, &email);
    let token = body["trusted_device_token"].as_str().unwrap().to_string();

    let (status, body) = fire(
        &app,
        "POST",
        "/v1/auth/mfa/challenge",
        Some(json!({ "trusted_device_token": token })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_stepup_token(&body, &fb, &email);

    let (status, body) = fire(&app, "GET", "/v1/me/mfa/trusted-devices", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let devices = body["devices"].as_array().expect("devices");
    assert_eq!(devices.len(), 1, "{body}");
    assert_eq!(devices[0]["label"], "This device");
    assert!(devices[0]["last_used_at"].as_str().is_some(), "{body}");
    let id = devices[0]["id"].as_str().unwrap().to_string();

    let (status, body) = fire(
        &app,
        "DELETE",
        &format!("/v1/me/mfa/trusted-devices/{id}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, body) = fire(
        &app,
        "POST",
        "/v1/auth/mfa/challenge",
        Some(json!({ "trusted_device_token": token })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"], "trusted_device_revoked");
}

#[tokio::test]
async fn org_admin_can_reset_recovery_codes_without_disabling_mfa() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (admin, admin_fb, admin_email) = create_user(&pool).await;
    let (target, target_fb, target_email) = create_user(&pool).await;
    attach_membership(&pool, tenant, admin, "org_admin").await;
    attach_membership(&pool, tenant, target, "student").await;

    let secret = totp::generate_secret();
    assert!(backend::db::mfa::start_enrollment(&pool, target, &secret)
        .await
        .unwrap());
    assert!(backend::db::mfa::confirm_enrollment(
        &pool,
        target,
        &secret,
        &[backend::db::api_keys::hash_secret("old-code")]
    )
    .await
    .unwrap());

    let admin_app = build_test_app(
        backend::handlers::admin::mfa_admin_router_for_tests(pool.clone()),
        StubAuth {
            pool: pool.clone(),
            user_id: admin,
            firebase_uid: admin_fb,
            email: admin_email,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::OrgAdmin),
        },
    );

    let (status, body) = fire(
        &admin_app,
        "POST",
        &format!("/v1/admin/tenant/memberships/{target}/mfa/recovery-codes"),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let recovery_codes = body["recovery_codes"].as_array().unwrap();
    assert_eq!(recovery_codes.len(), 10);
    let replacement_code = recovery_codes[0].as_str().unwrap().to_string();
    assert!(backend::db::mfa::is_enabled(&pool, target).await.unwrap());

    let target_app = app(pool.clone(), target, &target_fb, &target_email, tenant);
    let (status, body) = fire(
        &target_app,
        "POST",
        "/v1/auth/mfa/challenge",
        Some(json!({ "code": "old-code" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"], "invalid_code");

    let (status, body) = fire(
        &target_app,
        "POST",
        "/v1/auth/mfa/challenge",
        Some(json!({ "code": replacement_code })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["used_recovery_code"], true);
    assert_stepup_token(&body, &target_fb, &target_email);

    let audit_count = audit_reset_events_for(&pool, tenant, admin, target).await;
    assert_eq!(audit_count, 1);
}

#[tokio::test]
async fn student_cannot_reset_another_users_recovery_codes() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (student, student_fb, student_email) = create_user(&pool).await;
    let (target, _target_fb, _target_email) = create_user(&pool).await;
    attach_membership(&pool, tenant, student, "student").await;
    attach_membership(&pool, tenant, target, "student").await;

    let secret = totp::generate_secret();
    assert!(backend::db::mfa::start_enrollment(&pool, target, &secret)
        .await
        .unwrap());
    assert!(backend::db::mfa::confirm_enrollment(
        &pool,
        target,
        &secret,
        &[backend::db::api_keys::hash_secret("old-code")]
    )
    .await
    .unwrap());

    let app = build_test_app(
        backend::handlers::admin::mfa_admin_router_for_tests(pool.clone()),
        StubAuth {
            pool: pool.clone(),
            user_id: student,
            firebase_uid: student_fb,
            email: student_email,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Student),
        },
    );

    let (status, _body) = fire(
        &app,
        "POST",
        &format!("/v1/admin/tenant/memberships/{target}/mfa/recovery-codes"),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(backend::db::mfa::consume_recovery_code(
        &pool,
        target,
        &backend::db::api_keys::hash_secret("old-code")
    )
    .await
    .unwrap());
}
