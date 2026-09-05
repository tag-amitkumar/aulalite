//! Tenant branding storage + endpoints integration tests.
//!
//! Flows exercised (mirrors the notifications.rs / member_invitations.rs harness):
//!   * an org admin PATCHes logo_url + primary_color + accent_color, then GET
//!     /v1/admin/branding returns exactly those values;
//!   * a plain tenant member (student) sees the same branding via
//!     /v1/me/branding (the shell-theming endpoint);
//!   * an invalid hex color on PATCH => 422 (ApiError::Validation) and nothing
//!     is persisted;
//!   * a non-admin (student) is 403 on the admin GET + PATCH endpoints.
//!
//! These require a live Postgres (DATABASE_URL); they are compile-checked in CI
//! and run locally against the dev database.

mod fixtures;

use fixtures::*;
use serde_json::json;
use uuid::Uuid;

/// Build a branding test app scoped to `user` in `tenant` with `role`.
fn branding_app(
    pool: &sqlx::PgPool,
    user: Uuid,
    email: String,
    tenant: Uuid,
    role: core_types::TenantRole,
) -> axum::Router {
    build_test_app(
        backend::handlers::admin::branding_router_for_tests(pool.clone()),
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
async fn admin_patch_then_get_roundtrips_and_member_sees_it() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;

    // Org admin sets branding.
    let (admin, _, admin_email) = create_user(&pool).await;
    attach_membership(&pool, tenant, admin, "org_admin").await;
    let admin_app = branding_app(
        &pool,
        admin,
        admin_email,
        tenant,
        core_types::TenantRole::OrgAdmin,
    );

    // Initially empty.
    let (status, body) = fire(&admin_app, "GET", "/v1/admin/branding", None).await;
    assert_eq!(status, 200, "{body}");
    assert!(body["logo_url"].is_null());
    assert!(body["primary_color"].is_null());
    assert!(body["accent_color"].is_null());

    // PATCH sets all three.
    let (status, body) = fire(
        &admin_app,
        "PATCH",
        "/v1/admin/branding",
        Some(json!({
            "logo_url": "https://cdn.example.test/logo.png",
            "primary_color": "#1A2B3C",
            "accent_color": "#abc"
        })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["logo_url"], "https://cdn.example.test/logo.png");
    assert_eq!(body["primary_color"], "#1A2B3C");
    assert_eq!(body["accent_color"], "#abc");

    // GET reflects it.
    let (status, body) = fire(&admin_app, "GET", "/v1/admin/branding", None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["logo_url"], "https://cdn.example.test/logo.png");
    assert_eq!(body["primary_color"], "#1A2B3C");

    // A plain tenant member sees the same branding via /v1/me/branding.
    let (member, _, member_email) = create_user(&pool).await;
    attach_membership(&pool, tenant, member, "student").await;
    let member_app = branding_app(
        &pool,
        member,
        member_email,
        tenant,
        core_types::TenantRole::Student,
    );
    let (status, body) = fire(&member_app, "GET", "/v1/me/branding", None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["primary_color"], "#1A2B3C");
    assert_eq!(body["accent_color"], "#abc");
}

#[tokio::test]
async fn partial_merge_and_explicit_null_clears() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (admin, _, admin_email) = create_user(&pool).await;
    attach_membership(&pool, tenant, admin, "org_admin").await;
    let app = branding_app(
        &pool,
        admin,
        admin_email,
        tenant,
        core_types::TenantRole::OrgAdmin,
    );

    // Seed two fields.
    let (status, _) = fire(
        &app,
        "PATCH",
        "/v1/admin/branding",
        Some(json!({ "primary_color": "#fff", "accent_color": "#000" })),
    )
    .await;
    assert_eq!(status, 200);

    // PATCH only logo_url -> colors untouched (omitted != null).
    let (status, body) = fire(
        &app,
        "PATCH",
        "/v1/admin/branding",
        Some(json!({ "logo_url": "https://x.test/l.svg" })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["logo_url"], "https://x.test/l.svg");
    assert_eq!(body["primary_color"], "#fff");
    assert_eq!(body["accent_color"], "#000");

    // Explicit null clears primary_color; empty string clears accent_color.
    let (status, body) = fire(
        &app,
        "PATCH",
        "/v1/admin/branding",
        Some(json!({ "primary_color": null, "accent_color": "" })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert!(body["primary_color"].is_null());
    assert!(body["accent_color"].is_null());
    assert_eq!(body["logo_url"], "https://x.test/l.svg");
}

#[tokio::test]
async fn invalid_color_is_rejected_and_not_persisted() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (admin, _, admin_email) = create_user(&pool).await;
    attach_membership(&pool, tenant, admin, "org_admin").await;
    let app = branding_app(
        &pool,
        admin,
        admin_email,
        tenant,
        core_types::TenantRole::OrgAdmin,
    );

    // ApiError::Validation maps to 422 UNPROCESSABLE_ENTITY.
    let (status, _) = fire(
        &app,
        "PATCH",
        "/v1/admin/branding",
        Some(json!({ "primary_color": "blue" })),
    )
    .await;
    assert_eq!(status, 422);

    // A bad 4-digit hex is also rejected.
    let (status, _) = fire(
        &app,
        "PATCH",
        "/v1/admin/branding",
        Some(json!({ "accent_color": "#12g" })),
    )
    .await;
    assert_eq!(status, 422);

    // Bad logo URL is rejected too.
    let (status, _) = fire(
        &app,
        "PATCH",
        "/v1/admin/branding",
        Some(json!({ "logo_url": "ftp://nope" })),
    )
    .await;
    assert_eq!(status, 422);

    // Nothing was persisted.
    let (status, body) = fire(&app, "GET", "/v1/admin/branding", None).await;
    assert_eq!(status, 200, "{body}");
    assert!(body["primary_color"].is_null());
    assert!(body["accent_color"].is_null());
    assert!(body["logo_url"].is_null());
}

#[tokio::test]
async fn non_admin_is_forbidden_on_admin_endpoints() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (student, _, email) = create_user(&pool).await;
    attach_membership(&pool, tenant, student, "student").await;
    let app = branding_app(
        &pool,
        student,
        email,
        tenant,
        core_types::TenantRole::Student,
    );

    let (status, _) = fire(&app, "GET", "/v1/admin/branding", None).await;
    assert_eq!(status, 403);

    let (status, _) = fire(
        &app,
        "PATCH",
        "/v1/admin/branding",
        Some(json!({ "primary_color": "#fff" })),
    )
    .await;
    assert_eq!(status, 403);

    // But the member-facing shell endpoint is allowed.
    let (status, _) = fire(&app, "GET", "/v1/me/branding", None).await;
    assert_eq!(status, 200);
}
