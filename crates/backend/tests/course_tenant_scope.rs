use axum::http::StatusCode;
use uuid::Uuid;

mod fixtures;

use backend::handlers::courses::router_for_tests as course_routes;
use fixtures::{attach_membership, build_test_app, create_tenant, create_user, fire, StubAuth};

async fn insert_course(
    pool: &sqlx::PgPool,
    tenant_id: Uuid,
    owner_user_id: Uuid,
    slug: &str,
    title: &str,
) -> Uuid {
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    let course_id = sqlx::query_scalar(
        "INSERT INTO courses (tenant_id, slug, title, owner_user_id, status)
         VALUES ($1, $2, $3, $4, 'published')
         RETURNING id",
    )
    .bind(tenant_id)
    .bind(slug)
    .bind(title)
    .bind(owner_user_id)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    course_id
}

fn org_admin_app(
    pool: sqlx::PgPool,
    user_id: Uuid,
    firebase_uid: String,
    email: String,
    tenant_id: Uuid,
) -> axum::Router {
    build_test_app(
        course_routes(pool.clone()),
        StubAuth {
            pool,
            user_id,
            firebase_uid,
            email,
            tenant_id: Some(tenant_id),
            tenant_role: Some(core_types::TenantRole::OrgAdmin),
        },
    )
}

#[tokio::test]
async fn org_admin_lists_only_courses_in_active_tenant() {
    let pool = fixtures::pool().await;
    let tenant_a = create_tenant(&pool).await;
    let tenant_b = create_tenant(&pool).await;
    let (admin_id, admin_uid, admin_email) = create_user(&pool).await;
    let (owner_id, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant_a, admin_id, "org_admin").await;

    insert_course(
        &pool,
        tenant_a,
        owner_id,
        "tenant-a-course",
        "Tenant A Course",
    )
    .await;
    insert_course(
        &pool,
        tenant_b,
        owner_id,
        "tenant-b-course",
        "Tenant B Course",
    )
    .await;

    let app = org_admin_app(pool, admin_id, admin_uid, admin_email, tenant_a);
    let (status, body) = fire(&app, "GET", "/v1/courses", None).await;

    assert_eq!(status, StatusCode::OK);
    let courses = body.as_array().unwrap();
    assert_eq!(courses.len(), 1);
    assert_eq!(courses[0]["slug"], "tenant-a-course");
}

#[tokio::test]
async fn multi_workspace_admin_lists_courses_from_selected_workspace() {
    let pool = fixtures::pool().await;
    let tenant_a = create_tenant(&pool).await;
    let tenant_b = create_tenant(&pool).await;
    let (admin_id, admin_uid, admin_email) = create_user(&pool).await;
    let (owner_id, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant_a, admin_id, "org_admin").await;
    attach_membership(&pool, tenant_b, admin_id, "org_admin").await;

    // Keep tenant A as the oldest membership. Authorization must still honor
    // the explicitly selected tenant B instead of silently falling back to A.
    sqlx::query(
        "UPDATE tenant_memberships
            SET joined_at = CASE WHEN tenant_id = $1
                                 THEN now() - interval '1 day'
                                 ELSE now() END
          WHERE user_id = $2 AND tenant_id IN ($1, $3)",
    )
    .bind(tenant_a)
    .bind(admin_id)
    .bind(tenant_b)
    .execute(&pool)
    .await
    .unwrap();

    insert_course(&pool, tenant_a, owner_id, "old-workspace", "Old Workspace").await;
    insert_course(
        &pool,
        tenant_b,
        owner_id,
        "selected-workspace",
        "Selected Workspace",
    )
    .await;

    let app = org_admin_app(pool, admin_id, admin_uid, admin_email, tenant_b);
    let (status, body) = fire(&app, "GET", "/v1/courses", None).await;

    assert_eq!(status, StatusCode::OK);
    let courses = body.as_array().unwrap();
    assert_eq!(courses.len(), 1);
    assert_eq!(courses[0]["slug"], "selected-workspace");
}

#[tokio::test]
async fn org_admin_cannot_read_course_from_another_tenant() {
    let pool = fixtures::pool().await;
    let tenant_a = create_tenant(&pool).await;
    let tenant_b = create_tenant(&pool).await;
    let (admin_id, admin_uid, admin_email) = create_user(&pool).await;
    let (owner_id, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant_a, admin_id, "org_admin").await;

    let tenant_b_course = insert_course(
        &pool,
        tenant_b,
        owner_id,
        "tenant-b-course",
        "Tenant B Course",
    )
    .await;

    let app = org_admin_app(pool, admin_id, admin_uid, admin_email, tenant_a);
    let (status, _) = fire(&app, "GET", &format!("/v1/courses/{tenant_b_course}"), None).await;

    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn org_admin_cannot_patch_course_from_another_tenant() {
    let pool = fixtures::pool().await;
    let tenant_a = create_tenant(&pool).await;
    let tenant_b = create_tenant(&pool).await;
    let (admin_id, admin_uid, admin_email) = create_user(&pool).await;
    let (owner_id, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant_a, admin_id, "org_admin").await;

    let tenant_b_course = insert_course(
        &pool,
        tenant_b,
        owner_id,
        "tenant-b-course",
        "Tenant B Course",
    )
    .await;

    let app = org_admin_app(pool.clone(), admin_id, admin_uid, admin_email, tenant_a);
    let (status, _) = fire(
        &app,
        "PATCH",
        &format!("/v1/courses/{tenant_b_course}"),
        Some(serde_json::json!({ "title": "Cross Tenant Edit" })),
    )
    .await;

    assert!(
        matches!(status, StatusCode::FORBIDDEN | StatusCode::NOT_FOUND),
        "unexpected status: {status}"
    );
    let title: String = sqlx::query_scalar("SELECT title FROM courses WHERE id = $1")
        .bind(tenant_b_course)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(title, "Tenant B Course");
}
