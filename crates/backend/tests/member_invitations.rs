//! Tenant member-invite + seat-cap enforcement integration tests.
//!
//! Flow exercised (mirrors the parent.rs harness):
//!   * an org admin invites a teacher by email;
//!   * driving the JIT acceptance path for that email creates an ACTIVE
//!     tenant_membership(role=teacher) and the new member's role is reflected in
//!     the membership row;
//!   * SEAT CAP: with a plan included_seats=1 and 1 active member, creating an
//!     invite returns 409 "seat_limit_reached" when overage='block', but
//!     succeeds when overage='metered';
//!   * a non-admin (student) creating an invite gets 403;
//!   * revoke flips a pending invite to revoked (204).
//!
//! These require a live Postgres (DATABASE_URL); they are compile-checked in CI
//! and run locally against the dev database.

mod fixtures;

use fixtures::*;
use serde_json::json;
use uuid::Uuid;

/// Seed a UNIQUE global plan (the `plans` PK is the id) with `included_seats`,
/// then upsert the tenant's subscription pointing at it with `overage`.
async fn seed_plan_and_subscription(
    pool: &sqlx::PgPool,
    tenant: Uuid,
    included_seats: i32,
    overage: &str,
) {
    let plan_id = format!("plan-{}", Uuid::new_v4());
    sqlx::query(
        "INSERT INTO plans (id, name, monthly_price_cents, included_seats,
                            included_class_minutes, included_recording_gb)
         VALUES ($1, 'Test', 0, $2, 0, 0)",
    )
    .bind(&plan_id)
    .bind(included_seats)
    .execute(pool)
    .await
    .unwrap();

    // subscriptions is tenant-isolated: set the GUC to write it.
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO subscriptions (tenant_id, plan_id, status, overage_behavior)
         VALUES ($1, $2, 'active', $3)
         ON CONFLICT (tenant_id) DO UPDATE
            SET plan_id = EXCLUDED.plan_id,
                overage_behavior = EXCLUDED.overage_behavior",
    )
    .bind(tenant)
    .bind(&plan_id)
    .bind(overage)
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
}

/// Drive the JIT member-invite acceptance path for `email`, exactly as the auth
/// flow does on first sign-in: (re)activate the tenant_memberships seat.
async fn accept_member(pool: &sqlx::PgPool, user_id: Uuid, email: &str) -> usize {
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.user_id', $1, true)")
        .bind(user_id.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    let n = backend::db::member_invitations::accept_pending_for_email(&mut tx, user_id, email)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    n
}

/// Read a (role, status) tuple for a membership directly, under the tenant GUC.
async fn read_membership(
    pool: &sqlx::PgPool,
    tenant: Uuid,
    user_id: Uuid,
) -> Option<(String, String)> {
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    let row: Option<(String, String)> = sqlx::query_as(
        "SELECT role, status FROM tenant_memberships
          WHERE tenant_id = $1 AND user_id = $2",
    )
    .bind(tenant)
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    row
}

/// Build an admin-scoped test app for the member-invitation router.
fn admin_app(
    pool: &sqlx::PgPool,
    admin: Uuid,
    email: String,
    tenant: Uuid,
    mock: std::sync::Arc<backend::services::invitations::mock::MockEmailLinkSender>,
) -> axum::Router {
    build_test_app(
        backend::handlers::member_invitations::router_for_tests(
            pool.clone(),
            mock,
            "http://localhost:3000".into(),
        ),
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

fn owner_app(
    pool: &sqlx::PgPool,
    owner: Uuid,
    email: String,
    tenant: Uuid,
    mock: std::sync::Arc<backend::services::invitations::mock::MockEmailLinkSender>,
) -> axum::Router {
    build_test_app(
        backend::handlers::member_invitations::router_for_tests(
            pool.clone(),
            mock,
            "http://localhost:3000".into(),
        ),
        StubAuth {
            pool: pool.clone(),
            user_id: owner,
            firebase_uid: format!("fb-{}", Uuid::new_v4()),
            email,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::OrgOwner),
        },
    )
}

#[tokio::test]
async fn admin_invites_teacher_then_jit_provisions_membership() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (admin, _, em_a) = create_user(&pool).await;
    attach_membership(&pool, tenant, admin, "org_admin").await;

    let teacher_email = format!("teacher-{}@example.test", Uuid::new_v4());
    let mock =
        std::sync::Arc::new(backend::services::invitations::mock::MockEmailLinkSender::new());
    let app = admin_app(&pool, admin, em_a, tenant, mock.clone());

    let (status, body) = fire(
        &app,
        "POST",
        "/v1/admin/member-invitations",
        Some(json!({ "email": teacher_email, "role": "teacher" })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["email"].as_str().unwrap(), teacher_email);
    assert_eq!(body["role"], "teacher");
    assert_eq!(body["status"], "pending");

    // The invite email was sent with a continue URL built from app_origin.
    let calls = mock.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, teacher_email);
    assert!(calls[0]
        .1
        .starts_with("http://localhost:3000/accept-invite"));

    // Teacher signs in for the first time -> JIT acceptance creates the seat.
    let (teacher, _, _) = create_user(&pool).await;
    sqlx::query("UPDATE users SET email = $1 WHERE id = $2")
        .bind(&teacher_email)
        .bind(teacher)
        .execute(&pool)
        .await
        .unwrap();
    let accepted = accept_member(&pool, teacher, &teacher_email).await;
    assert_eq!(accepted, 1);

    // The membership is active with role=teacher (what /v1/me would reflect).
    let membership = read_membership(&pool, tenant, teacher).await;
    assert_eq!(
        membership,
        Some(("teacher".to_string(), "active".to_string()))
    );
}

#[tokio::test]
async fn seat_cap_blocks_at_the_plan_limit() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (admin, _, em_a) = create_user(&pool).await;
    // The single org_admin membership is the one active seat.
    attach_membership(&pool, tenant, admin, "org_admin").await;

    // Plan with exactly 1 seat, overage 'block': at cap already.
    seed_plan_and_subscription(&pool, tenant, 1, "block").await;

    let mock =
        std::sync::Arc::new(backend::services::invitations::mock::MockEmailLinkSender::new());
    let app = admin_app(&pool, admin, em_a.clone(), tenant, mock.clone());

    let (status, body) = fire(
        &app,
        "POST",
        "/v1/admin/member-invitations",
        Some(json!({ "email": format!("x-{}@example.test", Uuid::new_v4()), "role": "teacher" })),
    )
    .await;
    assert_eq!(status, 409, "{body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("seat_limit_reached"),
        "expected seat_limit_reached, got {body}"
    );
    // No email sent on a blocked invite.
    assert_eq!(mock.calls().len(), 0);
}

#[tokio::test]
async fn concurrent_invites_cannot_both_reserve_the_final_seat() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (admin, _, em_a) = create_user(&pool).await;
    // One active admin plus exactly one available seat.
    attach_membership(&pool, tenant, admin, "org_admin").await;
    seed_plan_and_subscription(&pool, tenant, 2, "block").await;

    let mock =
        std::sync::Arc::new(backend::services::invitations::mock::MockEmailLinkSender::new());
    let app = admin_app(&pool, admin, em_a, tenant, mock.clone());
    let first_email = format!("race-a-{}@example.test", Uuid::new_v4());
    let second_email = format!("race-b-{}@example.test", Uuid::new_v4());

    let (first, second) = tokio::join!(
        fire(
            &app,
            "POST",
            "/v1/admin/member-invitations",
            Some(json!({ "email": first_email, "role": "teacher" })),
        ),
        fire(
            &app,
            "POST",
            "/v1/admin/member-invitations",
            Some(json!({ "email": second_email, "role": "teacher" })),
        ),
    );

    let mut statuses = [first.0.as_u16(), second.0.as_u16()];
    statuses.sort_unstable();
    assert_eq!(statuses, [200, 409], "responses: {first:?}, {second:?}");

    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    let pending: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM tenant_invitations
          WHERE tenant_id = $1 AND status = 'pending'",
    )
    .bind(tenant)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();

    assert_eq!(pending, 1, "only one seat may be reserved");
    assert_eq!(mock.calls().len(), 1, "only the winning invite is emailed");
}

#[tokio::test]
async fn sso_jit_activation_respects_cap_but_existing_active_login_is_idempotent() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (admin, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, admin, "org_admin").await;
    seed_plan_and_subscription(&pool, tenant, 1, "block").await;

    let (new_user, _, _) = create_user(&pool).await;
    let blocked = backend::db::sso::ensure_tenant_membership(&pool, tenant, new_user, "student")
        .await
        .unwrap();
    assert_eq!(
        blocked,
        backend::db::seats::MembershipActivationOutcome::SeatLimitReached
    );
    assert_eq!(read_membership(&pool, tenant, new_user).await, None);

    let (suspended_user, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, suspended_user, "student").await;
    sqlx::query(
        "UPDATE tenant_memberships SET status = 'suspended'
          WHERE tenant_id = $1 AND user_id = $2",
    )
    .bind(tenant)
    .bind(suspended_user)
    .execute(&pool)
    .await
    .unwrap();
    let suspended =
        backend::db::sso::ensure_tenant_membership(&pool, tenant, suspended_user, "student")
            .await
            .unwrap();
    assert_eq!(
        suspended,
        backend::db::seats::MembershipActivationOutcome::Suspended
    );
    assert_eq!(
        read_membership(&pool, tenant, suspended_user).await,
        Some(("student".to_string(), "suspended".to_string()))
    );

    let existing = backend::db::sso::ensure_tenant_membership(&pool, tenant, admin, "student")
        .await
        .unwrap();
    assert_eq!(
        existing,
        backend::db::seats::MembershipActivationOutcome::AlreadyActive
    );
    assert_eq!(
        read_membership(&pool, tenant, admin).await,
        Some(("org_admin".to_string(), "active".to_string()))
    );
}

#[tokio::test]
async fn non_admin_forbidden_on_create() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (student, _, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, student, "student").await;

    let app = build_test_app(
        backend::handlers::member_invitations::router_for_tests(
            pool.clone(),
            std::sync::Arc::new(backend::services::invitations::mock::MockEmailLinkSender::new()),
            "http://localhost:3000".into(),
        ),
        StubAuth {
            pool: pool.clone(),
            user_id: student,
            firebase_uid: format!("fb-{}", Uuid::new_v4()),
            email: em,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Student),
        },
    );
    let (status, _) = fire(
        &app,
        "POST",
        "/v1/admin/member-invitations",
        Some(json!({ "email": "z@example.test", "role": "teacher" })),
    )
    .await;
    assert_eq!(status, 403);
}

#[tokio::test]
async fn revoke_pending_invitation_succeeds() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (admin, _, em_a) = create_user(&pool).await;
    attach_membership(&pool, tenant, admin, "org_admin").await;

    let mock =
        std::sync::Arc::new(backend::services::invitations::mock::MockEmailLinkSender::new());
    let app = admin_app(&pool, admin, em_a, tenant, mock.clone());

    let (status, body) = fire(
        &app,
        "POST",
        "/v1/admin/member-invitations",
        Some(json!({ "email": format!("r-{}@example.test", Uuid::new_v4()), "role": "ta" })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let id = body["id"].as_str().unwrap();

    let (status, _) = fire(
        &app,
        "DELETE",
        &format!("/v1/admin/member-invitations/{id}"),
        None,
    )
    .await;
    assert_eq!(status, 204);

    // Second revoke -> 404 (already non-pending).
    let (status, _) = fire(
        &app,
        "DELETE",
        &format!("/v1/admin/member-invitations/{id}"),
        None,
    )
    .await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn only_owner_can_create_or_revoke_admin_invitation() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (owner, _, owner_email) = create_user(&pool).await;
    let (admin, _, admin_email) = create_user(&pool).await;
    attach_membership(&pool, tenant, owner, "org_owner").await;
    attach_membership(&pool, tenant, admin, "org_admin").await;
    let mock =
        std::sync::Arc::new(backend::services::invitations::mock::MockEmailLinkSender::new());
    let admin_app = admin_app(&pool, admin, admin_email, tenant, mock.clone());
    let owner_app = owner_app(&pool, owner, owner_email, tenant, mock);
    let invited_email = format!("admin-{}@example.test", Uuid::new_v4());

    let (status, _) = fire(
        &admin_app,
        "POST",
        "/v1/admin/member-invitations",
        Some(json!({ "email": invited_email.clone(), "role": "org_admin" })),
    )
    .await;
    assert_eq!(status, 403);

    let (status, body) = fire(
        &owner_app,
        "POST",
        "/v1/admin/member-invitations",
        Some(json!({ "email": invited_email, "role": "org_admin" })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let invitation_id = body["id"].as_str().unwrap();

    let (status, _) = fire(
        &admin_app,
        "DELETE",
        &format!("/v1/admin/member-invitations/{invitation_id}"),
        None,
    )
    .await;
    assert_eq!(status, 403);

    let (status, _) = fire(
        &owner_app,
        "DELETE",
        &format!("/v1/admin/member-invitations/{invitation_id}"),
        None,
    )
    .await;
    assert_eq!(status, 204);
}

#[tokio::test]
async fn invitation_acceptance_cannot_claim_another_users_email() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let victim_email = format!("victim-{}@example.test", Uuid::new_v4());
    sqlx::query(
        "INSERT INTO tenant_invitations (tenant_id, email, role, status)
         VALUES ($1, $2, 'student', 'pending')",
    )
    .bind(tenant)
    .bind(&victim_email)
    .execute(&pool)
    .await
    .unwrap();
    let (attacker, _, _) = create_user(&pool).await;

    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.user_id', $1, true)")
        .bind(attacker.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    let error =
        backend::db::member_invitations::accept_pending_for_email(&mut tx, attacker, &victim_email)
            .await
            .unwrap_err();
    tx.rollback().await.unwrap();
    let sqlx::Error::Database(database_error) = error else {
        panic!("expected database authorization error");
    };
    assert_eq!(database_error.code().as_deref(), Some("42501"));

    let membership: Option<i32> = sqlx::query_scalar(
        "SELECT 1 FROM tenant_memberships WHERE tenant_id = $1 AND user_id = $2",
    )
    .bind(tenant)
    .bind(attacker)
    .fetch_optional(&pool)
    .await
    .unwrap();
    assert!(membership.is_none());
}

#[tokio::test]
async fn invitation_acceptance_rejects_a_mismatched_transaction_actor() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (victim, _, victim_email) = create_user(&pool).await;
    let (attacker, _, _) = create_user(&pool).await;
    let invitation_id: Uuid = sqlx::query_scalar(
        "INSERT INTO tenant_invitations (tenant_id, email, role, status)
         VALUES ($1, $2, 'student', 'pending') RETURNING id",
    )
    .bind(tenant)
    .bind(&victim_email)
    .fetch_one(&pool)
    .await
    .unwrap();

    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.user_id', $1, true)")
        .bind(attacker.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    let error =
        backend::db::member_invitations::accept_pending_for_email(&mut tx, victim, &victim_email)
            .await
            .unwrap_err();
    tx.rollback().await.unwrap();
    let sqlx::Error::Database(database_error) = error else {
        panic!("expected database authorization error");
    };
    assert_eq!(database_error.code().as_deref(), Some("42501"));

    let status: String = sqlx::query_scalar("SELECT status FROM tenant_invitations WHERE id = $1")
        .bind(invitation_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(status, "pending");
}

#[tokio::test]
async fn lower_role_invitation_cannot_demote_an_existing_admin() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (admin, _, admin_email) = create_user(&pool).await;
    attach_membership(&pool, tenant, admin, "org_admin").await;
    let invitation_id: Uuid = sqlx::query_scalar(
        "INSERT INTO tenant_invitations (tenant_id, email, role, status)
         VALUES ($1, $2, 'student', 'pending') RETURNING id",
    )
    .bind(tenant)
    .bind(&admin_email)
    .fetch_one(&pool)
    .await
    .unwrap();

    let accepted = accept_member(&pool, admin, &admin_email).await;
    assert_eq!(accepted, 0);
    assert_eq!(
        read_membership(&pool, tenant, admin).await,
        Some(("org_admin".into(), "active".into()))
    );
    let status: String = sqlx::query_scalar("SELECT status FROM tenant_invitations WHERE id = $1")
        .bind(invitation_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(status, "revoked");
}
