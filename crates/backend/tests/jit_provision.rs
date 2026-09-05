mod fixtures;

use backend::auth::jit_provision::{ensure_user, ensure_user_with_self_service, ProvisionError};
use backend::auth::verify::FirebaseClaims;
use chrono::Utc;

fn claims(uid: &str, email: &str, name: Option<&str>) -> FirebaseClaims {
    claims_with_verification(uid, email, name, Some(true))
}

fn claims_with_verification(
    uid: &str,
    email: &str,
    name: Option<&str>,
    email_verified: Option<bool>,
) -> FirebaseClaims {
    FirebaseClaims {
        sub: uid.into(),
        email: Some(email.into()),
        email_verified,
        name: name.map(String::from),
        picture: None,
        aud: "aulalite-dev".into(),
        iss: "https://securetoken.google.com/aulalite-dev".into(),
        exp: Utc::now().timestamp() + 600,
        iat: Utc::now().timestamp(),
        auth_time: None,
    }
}

#[tokio::test]
async fn first_call_creates_user() {
    let pool = fixtures::pool().await;
    let uid = format!("fbuid_{}", uuid::Uuid::new_v4());
    let email = format!("u_{}@example.test", uuid::Uuid::new_v4());

    let provisioned = ensure_user(&pool, &claims(&uid, &email, Some("U")))
        .await
        .unwrap();

    assert_eq!(provisioned.firebase_uid, uid);
    assert_eq!(provisioned.email.to_lowercase(), email.to_lowercase());
}

#[tokio::test]
async fn second_call_is_idempotent_and_updates_last_seen() {
    let pool = fixtures::pool().await;
    let uid = format!("fbuid_{}", uuid::Uuid::new_v4());
    let email = format!("u_{}@example.test", uuid::Uuid::new_v4());
    let user_claims = claims(&uid, &email, Some("U"));

    let first = ensure_user(&pool, &user_claims).await.unwrap();
    let second = ensure_user(&pool, &user_claims).await.unwrap();

    assert_eq!(first.user_id, second.user_id);
}

#[tokio::test]
async fn self_service_creates_exactly_one_personal_workspace_under_concurrency() {
    let pool = fixtures::pool().await;
    let uid = format!("fbuid_{}", uuid::Uuid::new_v4());
    let email = format!("owner_{}@example.test", uuid::Uuid::new_v4());
    let owner_claims = claims(&uid, &email, Some("Academy Owner"));

    let (first, second) = tokio::join!(
        ensure_user_with_self_service(&pool, &owner_claims, true),
        ensure_user_with_self_service(&pool, &owner_claims, true),
    );
    let first = first.unwrap();
    let second = second.unwrap();
    assert_eq!(first.user_id, second.user_id);

    let workspaces: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM tenants WHERE personal_owner_user_id = $1")
            .bind(first.user_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(workspaces, 1);

    let membership: (String, String) = sqlx::query_as(
        "SELECT tm.role, tm.status
           FROM tenant_memberships tm
           JOIN tenants t ON t.id = tm.tenant_id
          WHERE t.personal_owner_user_id = $1",
    )
    .bind(first.user_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(membership, ("org_owner".into(), "active".into()));
}

#[tokio::test]
async fn accepted_invitation_prevents_an_extra_personal_workspace() {
    let pool = fixtures::pool().await;
    let tenant_id = fixtures::create_tenant(&pool).await;
    let uid = format!("fbuid_{}", uuid::Uuid::new_v4());
    let email = format!("member_{}@example.test", uuid::Uuid::new_v4());

    sqlx::query(
        "INSERT INTO tenant_invitations (tenant_id, email, role, status)
         VALUES ($1, $2, 'student', 'pending')",
    )
    .bind(tenant_id)
    .bind(&email)
    .execute(&pool)
    .await
    .unwrap();

    let user =
        ensure_user_with_self_service(&pool, &claims(&uid, &email, Some("Invited Student")), true)
            .await
            .unwrap();

    let membership: (uuid::Uuid, String, String) = sqlx::query_as(
        "SELECT tenant_id, role, status
           FROM tenant_memberships
          WHERE user_id = $1",
    )
    .bind(user.user_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(membership, (tenant_id, "student".into(), "active".into()));

    let personal_workspaces: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM tenants WHERE personal_owner_user_id = $1")
            .bind(user.user_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(personal_workspaces, 0);
}

#[tokio::test]
async fn disabled_self_service_does_not_create_a_workspace() {
    let pool = fixtures::pool().await;
    let uid = format!("fbuid_{}", uuid::Uuid::new_v4());
    let email = format!("invite_only_{}@example.test", uuid::Uuid::new_v4());

    let user =
        ensure_user_with_self_service(&pool, &claims(&uid, &email, Some("Invite Only")), false)
            .await
            .unwrap();

    let memberships: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM tenant_memberships WHERE user_id = $1")
            .bind(user.user_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(memberships, 0);
}

#[tokio::test]
async fn unverified_email_is_rejected_before_user_or_invitation_mutation() {
    let pool = fixtures::pool().await;
    let tenant_id = fixtures::create_tenant(&pool).await;
    let uid = format!("fbuid_{}", uuid::Uuid::new_v4());
    let email = format!("invited_{}@example.test", uuid::Uuid::new_v4());

    let invitation_id: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO tenant_invitations (tenant_id, email, role, status)
         VALUES ($1, $2, 'student', 'pending')
         RETURNING id",
    )
    .bind(tenant_id)
    .bind(&email)
    .fetch_one(&pool)
    .await
    .unwrap();

    let err = ensure_user(
        &pool,
        &claims_with_verification(&uid, &email, Some("Unverified"), Some(false)),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, ProvisionError::EmailNotVerified));

    let user_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE firebase_uid = $1")
        .bind(&uid)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(user_count, 0, "rejected identity must not be provisioned");

    let invitation_status: String =
        sqlx::query_scalar("SELECT status FROM tenant_invitations WHERE id = $1")
            .bind(invitation_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(invitation_status, "pending");

    let membership_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
           FROM tenant_memberships tm
           JOIN users u ON u.id = tm.user_id
          WHERE tm.tenant_id = $1 AND lower(u.email::text) = lower($2)",
    )
    .bind(tenant_id)
    .bind(&email)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(membership_count, 0, "invite must not grant a membership");
}
