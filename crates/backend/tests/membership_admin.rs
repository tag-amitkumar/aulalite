mod fixtures;

use fixtures::*;
use serde_json::json;
use uuid::Uuid;

async fn app_for(
    pool: &sqlx::PgPool,
    tenant: Uuid,
    actor: Uuid,
    role: core_types::TenantRole,
) -> axum::Router {
    let (firebase_uid, email): (String, String) =
        sqlx::query_as("SELECT firebase_uid, email::text FROM users WHERE id = $1")
            .bind(actor)
            .fetch_one(pool)
            .await
            .unwrap();
    build_test_app(
        backend::handlers::admin::membership_router_for_tests(pool.clone()),
        StubAuth {
            pool: pool.clone(),
            user_id: actor,
            firebase_uid,
            email,
            tenant_id: Some(tenant),
            tenant_role: Some(role),
        },
    )
}

async fn role_and_status(pool: &sqlx::PgPool, tenant: Uuid, user: Uuid) -> (String, String) {
    sqlx::query_as(
        "SELECT role, status FROM tenant_memberships
          WHERE tenant_id = $1 AND user_id = $2",
    )
    .bind(tenant)
    .bind(user)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn initialized_tenant_with_owner(pool: &sqlx::PgPool) -> (Uuid, Uuid) {
    let tenant = create_tenant(pool).await;
    let (owner, _, _) = create_user(pool).await;
    attach_membership(pool, tenant, owner, "org_owner").await;
    sqlx::query("UPDATE tenants SET ownership_initialized_at = now() WHERE id = $1")
        .bind(tenant)
        .execute(pool)
        .await
        .unwrap();
    (tenant, owner)
}

#[tokio::test]
async fn ownership_cannot_be_changed_through_generic_membership_patch() {
    let pool = pool().await;
    let (tenant, owner) = initialized_tenant_with_owner(&pool).await;
    let app = app_for(&pool, tenant, owner, core_types::TenantRole::OrgOwner).await;

    for patch in [json!({"role": "org_admin"}), json!({"status": "suspended"})] {
        let (status, body) = fire(
            &app,
            "PATCH",
            &format!("/v1/admin/tenant/memberships/{owner}"),
            Some(patch),
        )
        .await;
        assert_eq!(status, 409, "{body}");
        assert_eq!(body["error"], "conflict: ownership_transfer_required");
    }
    assert_eq!(
        role_and_status(&pool, tenant, owner).await,
        ("org_owner".into(), "active".into())
    );
}

#[tokio::test]
async fn generic_patch_cannot_promote_any_member_to_owner() {
    let pool = pool().await;
    let (tenant, owner) = initialized_tenant_with_owner(&pool).await;
    let (admin, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, admin, "org_admin").await;
    let app = app_for(&pool, tenant, owner, core_types::TenantRole::OrgOwner).await;

    let (status, body) = fire(
        &app,
        "PATCH",
        &format!("/v1/admin/tenant/memberships/{admin}"),
        Some(json!({"role": "org_owner"})),
    )
    .await;
    assert_eq!(status, 409, "{body}");
    assert_eq!(body["error"], "conflict: ownership_transfer_required");
}

#[tokio::test]
async fn only_owner_can_appoint_or_modify_organization_admins() {
    let pool = pool().await;
    let (tenant, owner) = initialized_tenant_with_owner(&pool).await;
    let (first_admin, _, _) = create_user(&pool).await;
    let (second_admin, _, _) = create_user(&pool).await;
    let (teacher, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, first_admin, "org_admin").await;
    attach_membership(&pool, tenant, second_admin, "org_admin").await;
    attach_membership(&pool, tenant, teacher, "teacher").await;

    let admin_app = app_for(&pool, tenant, first_admin, core_types::TenantRole::OrgAdmin).await;
    for (target, patch) in [
        (second_admin, json!({"status": "suspended"})),
        (teacher, json!({"role": "org_admin"})),
    ] {
        let (status, _) = fire(
            &admin_app,
            "PATCH",
            &format!("/v1/admin/tenant/memberships/{target}"),
            Some(patch),
        )
        .await;
        assert_eq!(status, 403);
    }

    let owner_app = app_for(&pool, tenant, owner, core_types::TenantRole::OrgOwner).await;
    let (status, body) = fire(
        &owner_app,
        "PATCH",
        &format!("/v1/admin/tenant/memberships/{teacher}"),
        Some(json!({"role": "org_admin"})),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(role_and_status(&pool, tenant, teacher).await.0, "org_admin");
}

#[tokio::test]
async fn owner_can_atomically_transfer_to_active_admin() {
    let pool = pool().await;
    let (tenant, owner) = initialized_tenant_with_owner(&pool).await;
    let (admin, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, admin, "org_admin").await;
    let app = app_for(&pool, tenant, owner, core_types::TenantRole::OrgOwner).await;

    let (status, body) = fire(
        &app,
        "POST",
        "/v1/admin/tenant/transfer-ownership",
        Some(json!({
            "new_owner_user_id": admin,
            "confirmation": "TRANSFER OWNERSHIP"
        })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["previous_owner_user_id"], owner.to_string());
    assert_eq!(body["new_owner_user_id"], admin.to_string());
    assert_eq!(role_and_status(&pool, tenant, owner).await.0, "org_admin");
    assert_eq!(role_and_status(&pool, tenant, admin).await.0, "org_owner");

    let audit_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_events
          WHERE tenant_id = $1 AND action = 'tenant.ownership.transferred'",
    )
    .bind(tenant)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(audit_count, 1);
}

#[tokio::test]
async fn transfer_requires_exact_owner_confirmation_and_active_admin_target() {
    let pool = pool().await;
    let (tenant, owner) = initialized_tenant_with_owner(&pool).await;
    let (teacher, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let app = app_for(&pool, tenant, owner, core_types::TenantRole::OrgOwner).await;

    let (status, _) = fire(
        &app,
        "POST",
        "/v1/admin/tenant/transfer-ownership",
        Some(json!({
            "new_owner_user_id": teacher,
            "confirmation": "TRANSFER OWNERSHIP"
        })),
    )
    .await;
    assert_eq!(status, 409);

    let (status, _) = fire(
        &app,
        "POST",
        "/v1/admin/tenant/transfer-ownership",
        Some(json!({
            "new_owner_user_id": teacher,
            "confirmation": "transfer"
        })),
    )
    .await;
    assert_eq!(status, 422);
}

#[tokio::test]
async fn concurrent_transfers_leave_exactly_one_active_owner() {
    let pool = pool().await;
    let (tenant, owner) = initialized_tenant_with_owner(&pool).await;
    let (first, _, _) = create_user(&pool).await;
    let (second, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, first, "org_admin").await;
    attach_membership(&pool, tenant, second, "org_admin").await;
    let app = app_for(&pool, tenant, owner, core_types::TenantRole::OrgOwner).await;

    let request = |target| {
        fire(
            &app,
            "POST",
            "/v1/admin/tenant/transfer-ownership",
            Some(json!({
                "new_owner_user_id": target,
                "confirmation": "TRANSFER OWNERSHIP"
            })),
        )
    };
    let (one, two) = tokio::join!(request(first), request(second));
    let mut statuses = [one.0.as_u16(), two.0.as_u16()];
    statuses.sort_unstable();
    assert_eq!(statuses, [200, 403]);

    let owners: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM tenant_memberships
          WHERE tenant_id = $1 AND role = 'org_owner' AND status = 'active'",
    )
    .bind(tenant)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(owners, 1);
}

#[tokio::test]
async fn database_constraint_rejects_removing_initialized_owner() {
    let pool = pool().await;
    let (tenant, owner) = initialized_tenant_with_owner(&pool).await;
    let error = sqlx::query(
        "UPDATE tenant_memberships SET role = 'org_admin'
          WHERE tenant_id = $1 AND user_id = $2",
    )
    .bind(tenant)
    .bind(owner)
    .execute(&pool)
    .await
    .unwrap_err();
    let sqlx::Error::Database(database_error) = error else {
        panic!("expected database error");
    };
    assert_eq!(
        database_error.constraint(),
        Some("tenant_memberships_active_org_owner_required")
    );
}

#[tokio::test]
async fn runtime_role_cannot_directly_swap_ownership() {
    let pool = pool().await;
    let (tenant, owner) = initialized_tenant_with_owner(&pool).await;
    let (admin, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, admin, "org_admin").await;

    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET LOCAL ROLE aulalite_app")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("SELECT set_config('app.user_id', $1, true)")
        .bind(admin.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    let error = sqlx::query(
        "UPDATE tenant_memberships SET role = 'org_admin'
          WHERE tenant_id = $1 AND user_id = $2",
    )
    .bind(tenant)
    .bind(owner)
    .execute(&mut *tx)
    .await
    .unwrap_err();
    tx.rollback().await.unwrap();
    let sqlx::Error::Database(database_error) = error else {
        panic!("expected database authorization error");
    };
    assert_eq!(database_error.code().as_deref(), Some("42501"));
}

#[tokio::test]
async fn membership_status_cannot_be_reset_to_invited() {
    let pool = pool().await;
    let (tenant, owner) = initialized_tenant_with_owner(&pool).await;
    let (teacher, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let app = app_for(&pool, tenant, owner, core_types::TenantRole::OrgOwner).await;

    let (status, body) = fire(
        &app,
        "PATCH",
        &format!("/v1/admin/tenant/memberships/{teacher}"),
        Some(json!({"status": "invited"})),
    )
    .await;
    assert_eq!(status, 409, "{body}");
    assert_eq!(
        body["error"],
        "conflict: invalid_membership_status_transition"
    );
}
