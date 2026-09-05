//! Platform super-admin endpoint integration tests.
//!
//! Flow exercised (mirrors the member_invitations.rs harness):
//!   * a PLATFORM admin (is_platform_admin=true, NO tenant membership) lists
//!     tenants and sees MORE THAN ONE — proving the cross-tenant RLS bypass;
//!   * the platform admin creates a tenant; the new tenant is listed and a
//!     PENDING org_owner tenant_invitation row exists for admin_email;
//!   * the platform admin sets a tenant's status to 'suspended';
//!   * a NON-platform-admin (an org_admin of one tenant) gets 403 on all three
//!     endpoints.
//!
//! These require a live Postgres (DATABASE_URL); they are compile-checked in CI
//! and run locally against the dev database.

mod fixtures;

use fixtures::*;
use serde_json::json;
use uuid::Uuid;

/// Mark a user as a platform super-admin (the StubAuth middleware reads this
/// flag straight from the users row).
async fn make_platform_admin(pool: &sqlx::PgPool, user_id: Uuid) {
    sqlx::query("UPDATE users SET is_platform_admin = TRUE WHERE id = $1")
        .bind(user_id)
        .execute(pool)
        .await
        .unwrap();
}

/// Build a test app for the platform router under a PLATFORM admin: no tenant
/// membership (tenant_id = None, tenant_role = None), is_platform_admin read
/// from the users row.
fn platform_admin_app(pool: &sqlx::PgPool, admin: Uuid, email: String) -> axum::Router {
    build_test_app(
        backend::handlers::platform::router_for_tests(pool.clone()),
        StubAuth {
            pool: pool.clone(),
            user_id: admin,
            firebase_uid: format!("fb-{}", Uuid::new_v4()),
            email,
            tenant_id: None,
            tenant_role: None,
        },
    )
}

/// Build a test app under a plain ORG admin of `tenant` — a non-platform-admin.
fn org_admin_app(pool: &sqlx::PgPool, admin: Uuid, email: String, tenant: Uuid) -> axum::Router {
    build_test_app(
        backend::handlers::platform::router_for_tests(pool.clone()),
        StubAuth {
            pool: pool.clone(),
            user_id: admin,
            firebase_uid: format!("fb-{}", Uuid::new_v4()),
            email,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::OrgAdmin),
        },
    )
}

#[tokio::test]
async fn platform_admin_lists_tenants_cross_tenant() {
    let pool = pool().await;
    // At least two tenants must exist for the cross-tenant assertion.
    let _t1 = create_tenant(&pool).await;
    let _t2 = create_tenant(&pool).await;

    let (admin, _, em) = create_user(&pool).await;
    make_platform_admin(&pool, admin).await;
    let app = platform_admin_app(&pool, admin, em);

    let (status, body) = fire(&app, "GET", "/v1/platform/tenants", None).await;
    assert_eq!(status, 200, "{body}");
    let tenants = body.as_array().expect("array of tenants");
    assert!(
        tenants.len() >= 2,
        "platform admin should see >1 tenant cross-tenant, got {}",
        tenants.len()
    );
    // Shape check on the first row.
    let first = &tenants[0];
    assert!(first["id"].is_string());
    assert!(first["slug"].is_string());
    assert!(first["status"].is_string());
    assert!(first["member_count"].is_number());
}

#[tokio::test]
async fn platform_admin_creates_tenant_with_org_owner_invite() {
    let pool = pool().await;
    let (admin, _, em) = create_user(&pool).await;
    make_platform_admin(&pool, admin).await;
    let app = platform_admin_app(&pool, admin, em);

    let slug = format!("acme-{}", Uuid::new_v4().simple());
    let admin_email = format!("owner-{}@example.test", Uuid::new_v4());

    let (status, body) = fire(
        &app,
        "POST",
        "/v1/platform/tenants",
        Some(json!({ "slug": slug, "name": "Acme School", "admin_email": admin_email })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["slug"].as_str().unwrap(), slug);
    assert_eq!(body["name"], "Acme School");
    assert_eq!(body["status"], "trialing");
    assert_eq!(body["member_count"].as_i64().unwrap(), 0);

    let new_tenant_id: Uuid = body["id"].as_str().unwrap().parse().unwrap();

    // A PENDING org_owner tenant_invitation exists for admin_email (read under
    // the new tenant's GUC so RLS lets us see it).
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(new_tenant_id.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    let invite: Option<(String, String)> = sqlx::query_as(
        "SELECT role, status FROM tenant_invitations
          WHERE tenant_id = $1 AND lower(email::text) = lower($2)",
    )
    .bind(new_tenant_id)
    .bind(&admin_email)
    .fetch_optional(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        invite,
        Some(("org_owner".to_string(), "pending".to_string())),
        "a pending org_owner invite should exist for admin_email"
    );
}

#[tokio::test]
async fn platform_admin_can_replace_unclaimed_owner_email() {
    let pool = pool().await;
    let (platform_admin, _, platform_email) = create_user(&pool).await;
    make_platform_admin(&pool, platform_admin).await;
    let app = platform_admin_app(&pool, platform_admin, platform_email);
    let slug = format!("owner-fix-{}", Uuid::new_v4().simple());
    let original_email = format!("typo-{}@example.test", Uuid::new_v4());
    let corrected_email = format!("owner-{}@example.test", Uuid::new_v4());
    let (status, created) = fire(
        &app,
        "POST",
        "/v1/platform/tenants",
        Some(json!({
            "slug": slug,
            "name": "Owner Fix School",
            "admin_email": original_email
        })),
    )
    .await;
    assert_eq!(status, 200, "{created}");
    let tenant: Uuid = created["id"].as_str().unwrap().parse().unwrap();

    let (status, _) = fire(
        &app,
        "PATCH",
        &format!("/v1/platform/tenants/{tenant}/pending-owner-invitation"),
        Some(json!({
            "email": corrected_email.clone(),
            "confirmation": "replace"
        })),
    )
    .await;
    assert_eq!(status, 422);

    let (status, body) = fire(
        &app,
        "PATCH",
        &format!("/v1/platform/tenants/{tenant}/pending-owner-invitation"),
        Some(json!({
            "email": corrected_email.clone(),
            "confirmation": "REPLACE OWNER INVITATION"
        })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["email"], corrected_email);
    assert!(body["invitation_id"].is_string());
    assert!(body["created_at"].is_string());

    let stored: (String, String) = sqlx::query_as(
        "SELECT email::text, status FROM tenant_invitations
          WHERE tenant_id = $1 AND role = 'org_owner'",
    )
    .bind(tenant)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(stored, (corrected_email, "pending".into()));
}

#[tokio::test]
async fn platform_admin_can_invite_first_owner_for_empty_legacy_tenant() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (platform_admin, _, platform_email) = create_user(&pool).await;
    make_platform_admin(&pool, platform_admin).await;
    let app = platform_admin_app(&pool, platform_admin, platform_email);
    let owner_email = format!("legacy-owner-{}@example.test", Uuid::new_v4());

    let (status, body) = fire(
        &app,
        "PATCH",
        &format!("/v1/platform/tenants/{tenant}/pending-owner-invitation"),
        Some(json!({
            "email": owner_email.clone(),
            "confirmation": "REPLACE OWNER INVITATION"
        })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["email"], owner_email);

    let stored: (String, String, bool) = sqlx::query_as(
        "SELECT ti.email::text, ti.role,
                t.ownership_initialized_at IS NOT NULL
           FROM tenant_invitations ti
           JOIN tenants t ON t.id = ti.tenant_id
          WHERE ti.tenant_id = $1 AND ti.status = 'pending'",
    )
    .bind(tenant)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(stored, (owner_email, "org_owner".into(), true));
}

#[tokio::test]
async fn platform_admin_can_list_candidates_and_recover_ownership() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (owner, _, _) = create_user(&pool).await;
    let (candidate, _, candidate_email) = create_user(&pool).await;
    let (teacher, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, owner, "org_owner").await;
    attach_membership(&pool, tenant, candidate, "org_admin").await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    sqlx::query("UPDATE tenants SET ownership_initialized_at = now() WHERE id = $1")
        .bind(tenant)
        .execute(&pool)
        .await
        .unwrap();

    let (platform_admin, _, platform_email) = create_user(&pool).await;
    make_platform_admin(&pool, platform_admin).await;
    let app = platform_admin_app(&pool, platform_admin, platform_email);

    let (status, body) = fire(
        &app,
        "GET",
        &format!("/v1/platform/tenants/{tenant}/ownership-candidates"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let candidates = body.as_array().unwrap();
    assert_eq!(
        candidates.len(),
        2,
        "teacher must not be a recovery candidate"
    );
    assert_eq!(candidates[0]["user_id"], owner.to_string());
    assert_eq!(candidates[0]["role"], "org_owner");
    assert_eq!(candidates[1]["user_id"], candidate.to_string());
    assert_eq!(candidates[1]["email"], candidate_email);
    assert_eq!(candidates[1]["role"], "org_admin");

    let (status, body) = fire(
        &app,
        "POST",
        &format!("/v1/platform/tenants/{tenant}/recover-owner"),
        Some(json!({
            "new_owner_user_id": candidate,
            "confirmation": "RECOVER OWNERSHIP"
        })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["previous_owner_user_id"], owner.to_string());
    assert_eq!(body["new_owner_user_id"], candidate.to_string());

    let roles: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT user_id, role FROM tenant_memberships
          WHERE tenant_id = $1 AND user_id IN ($2, $3) ORDER BY user_id",
    )
    .bind(tenant)
    .bind(owner)
    .bind(candidate)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert!(roles.contains(&(owner, "org_admin".into())));
    assert!(roles.contains(&(candidate, "org_owner".into())));
}

#[tokio::test]
async fn platform_admin_can_initialize_legacy_tenant_from_active_member() {
    let pool = pool().await;
    // The shared fixture deliberately creates a legacy tenant with a NULL
    // ownership marker and no owner graph.
    let tenant = create_tenant(&pool).await;
    let (teacher, _, teacher_email) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;

    let (platform_admin, _, platform_email) = create_user(&pool).await;
    make_platform_admin(&pool, platform_admin).await;

    let unbound_directory = sqlx::query("SELECT * FROM platform_list_tenants()")
        .execute(&pool)
        .await
        .unwrap_err();
    assert!(unbound_directory
        .to_string()
        .contains("platform_admin_required"));

    // Supplying a real platform-admin UUID is not sufficient: the definer
    // boundary also requires the transaction-local authenticated actor GUC.
    let unbound = sqlx::query("SELECT * FROM platform_list_tenant_owner_candidates($1, $2)")
        .bind(tenant)
        .bind(platform_admin)
        .execute(&pool)
        .await
        .unwrap_err();
    assert!(unbound.to_string().contains("platform_admin_required"));

    let app = platform_admin_app(&pool, platform_admin, platform_email);
    let (status, body) = fire(
        &app,
        "GET",
        &format!("/v1/platform/tenants/{tenant}/ownership-candidates"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body.as_array().unwrap().len(), 1);
    assert_eq!(body[0]["user_id"], teacher.to_string());
    assert_eq!(body[0]["email"], teacher_email);
    assert_eq!(body[0]["role"], "teacher");

    let (status, body) = fire(
        &app,
        "POST",
        &format!("/v1/platform/tenants/{tenant}/recover-owner"),
        Some(json!({
            "new_owner_user_id": teacher,
            "confirmation": "RECOVER OWNERSHIP"
        })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert!(body["previous_owner_user_id"].is_null());
    assert_eq!(body["new_owner_user_id"], teacher.to_string());

    let stored: (String, bool) = sqlx::query_as(
        "SELECT tm.role, t.ownership_initialized_at IS NOT NULL
           FROM tenant_memberships tm
           JOIN tenants t ON t.id = tm.tenant_id
          WHERE tm.tenant_id = $1 AND tm.user_id = $2",
    )
    .bind(tenant)
    .bind(teacher)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(stored, ("org_owner".into(), true));
}

#[tokio::test]
async fn platform_admin_suspends_tenant() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (admin, _, em) = create_user(&pool).await;
    make_platform_admin(&pool, admin).await;
    let app = platform_admin_app(&pool, admin, em);

    let (status, body) = fire(
        &app,
        "PATCH",
        &format!("/v1/platform/tenants/{tenant}"),
        Some(json!({ "status": "suspended" })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["id"].as_str().unwrap(), tenant.to_string());
    assert_eq!(body["status"], "suspended");
}

#[tokio::test]
async fn non_platform_admin_forbidden_on_all_endpoints() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (org_admin, _, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, org_admin, "org_admin").await;
    // NOTE: not a platform admin (is_platform_admin defaults FALSE).
    let app = org_admin_app(&pool, org_admin, em, tenant);

    // GET list -> 403
    let (status, _) = fire(&app, "GET", "/v1/platform/tenants", None).await;
    assert_eq!(status, 403);

    // POST create -> 403
    let (status, _) = fire(
        &app,
        "POST",
        "/v1/platform/tenants",
        Some(json!({
            "slug": format!("x-{}", Uuid::new_v4().simple()),
            "name": "Nope",
            "admin_email": "nope@example.test"
        })),
    )
    .await;
    assert_eq!(status, 403);

    // PATCH status -> 403
    let (status, _) = fire(
        &app,
        "PATCH",
        &format!("/v1/platform/tenants/{tenant}"),
        Some(json!({ "status": "suspended" })),
    )
    .await;
    assert_eq!(status, 403);

    let (status, _) = fire(
        &app,
        "GET",
        &format!("/v1/platform/tenants/{tenant}/ownership-candidates"),
        None,
    )
    .await;
    assert_eq!(status, 403);

    let (status, _) = fire(
        &app,
        "POST",
        &format!("/v1/platform/tenants/{tenant}/recover-owner"),
        Some(json!({
            "new_owner_user_id": org_admin,
            "confirmation": "RECOVER OWNERSHIP"
        })),
    )
    .await;
    assert_eq!(status, 403);

    let (status, _) = fire(
        &app,
        "PATCH",
        &format!("/v1/platform/tenants/{tenant}/pending-owner-invitation"),
        Some(json!({
            "email": "replacement@example.test",
            "confirmation": "REPLACE OWNER INVITATION"
        })),
    )
    .await;
    assert_eq!(status, 403);
}
