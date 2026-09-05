//! Structured search (pg_trgm) integration tests for `GET /v1/search`.
//!
//! Compile-only without a live Postgres; with `DATABASE_URL` set these run
//! against the migrated schema (the trigram indexes from
//! 20260529000027_search_indexes.sql). They assert the visibility scoping:
//! a student sees only courses they are enrolled in (not a course they aren't),
//! an assignment in a visible course is found, cross-tenant content never
//! leaks, and an empty query returns empty results.

mod fixtures;

use axum::http::StatusCode;
use uuid::Uuid;

use backend::handlers::search::router_for_tests as search_routes;
use fixtures::{
    attach_membership, build_test_app, create_tenant, create_user, fire, pool, StubAuth,
};

/// Insert a course with an explicit status; returns its id.
async fn insert_course(
    pool: &sqlx::PgPool,
    tenant_id: Uuid,
    owner_user_id: Uuid,
    slug: &str,
    title: &str,
    status: &str,
) -> Uuid {
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    let id = sqlx::query_scalar(
        "INSERT INTO courses (tenant_id, slug, title, owner_user_id, status)
         VALUES ($1, $2, $3, $4, $5) RETURNING id",
    )
    .bind(tenant_id)
    .bind(slug)
    .bind(title)
    .bind(owner_user_id)
    .bind(status)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    id
}

/// Give `user_id` an ACTIVE course membership with the given role.
async fn enroll(pool: &sqlx::PgPool, course_id: Uuid, user_id: Uuid, tenant_id: Uuid, role: &str) {
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO course_memberships (course_id, user_id, tenant_id, role, status)
         VALUES ($1, $2, $3, $4, 'active')",
    )
    .bind(course_id)
    .bind(user_id)
    .bind(tenant_id)
    .bind(role)
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
}

/// Insert an assignment with the given status; returns its id.
async fn insert_assignment(
    pool: &sqlx::PgPool,
    tenant_id: Uuid,
    course_id: Uuid,
    created_by: Uuid,
    title: &str,
    status: &str,
) -> Uuid {
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    let id = sqlx::query_scalar(
        "INSERT INTO assignments
            (tenant_id, course_id, title, grading_mode, max_points, created_by, status)
         VALUES ($1, $2, $3, 'numeric', 100, $4, $5::assignment_status) RETURNING id",
    )
    .bind(tenant_id)
    .bind(course_id)
    .bind(title)
    .bind(created_by)
    .bind(status)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    id
}

fn student_app(
    pool: sqlx::PgPool,
    user_id: Uuid,
    firebase_uid: String,
    email: String,
    tenant_id: Uuid,
) -> axum::Router {
    build_test_app(
        search_routes(pool.clone()),
        StubAuth {
            pool,
            user_id,
            firebase_uid,
            email,
            tenant_id: Some(tenant_id),
            tenant_role: Some(core_types::TenantRole::Student),
        },
    )
}

#[tokio::test]
async fn student_sees_only_enrolled_course() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _, _) = create_user(&pool).await;
    let (student, student_uid, student_email) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    attach_membership(&pool, tenant, student, "student").await;

    let enrolled = insert_course(
        &pool,
        tenant,
        teacher,
        &format!("algebra-{}", Uuid::new_v4()),
        "Algebra Fundamentals",
        "published",
    )
    .await;
    // A second course the student is NOT enrolled in (same title token).
    let _not_enrolled = insert_course(
        &pool,
        tenant,
        teacher,
        &format!("algebra2-{}", Uuid::new_v4()),
        "Algebra Advanced",
        "published",
    )
    .await;
    enroll(&pool, enrolled, student, tenant, "student").await;

    let app = student_app(pool, student, student_uid, student_email, tenant);
    let (status, body) = fire(&app, "GET", "/v1/search?q=Algebra", None).await;

    assert_eq!(status, StatusCode::OK);
    let courses = body["courses"].as_array().unwrap();
    assert_eq!(courses.len(), 1, "student should see only enrolled course");
    assert_eq!(courses[0]["id"], enrolled.to_string());
    assert_eq!(courses[0]["title"], "Algebra Fundamentals");
}

#[tokio::test]
async fn published_assignment_in_visible_course_is_found() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _, _) = create_user(&pool).await;
    let (student, student_uid, student_email) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    attach_membership(&pool, tenant, student, "student").await;

    let course = insert_course(
        &pool,
        tenant,
        teacher,
        &format!("bio-{}", Uuid::new_v4()),
        "Biology",
        "published",
    )
    .await;
    enroll(&pool, course, student, tenant, "student").await;
    let aid = insert_assignment(
        &pool,
        tenant,
        course,
        teacher,
        "Photosynthesis Essay",
        "published",
    )
    .await;
    // A draft in the same course must NOT surface for the student.
    let _draft = insert_assignment(
        &pool,
        tenant,
        course,
        teacher,
        "Photosynthesis Draft",
        "draft",
    )
    .await;

    let app = student_app(pool, student, student_uid, student_email, tenant);
    let (status, body) = fire(&app, "GET", "/v1/search?q=Photosynthesis", None).await;

    assert_eq!(status, StatusCode::OK);
    let assignments = body["assignments"].as_array().unwrap();
    assert_eq!(assignments.len(), 1, "only the published assignment");
    assert_eq!(assignments[0]["id"], aid.to_string());
    assert_eq!(assignments[0]["status"], "published");
}

#[tokio::test]
async fn cross_tenant_content_is_not_returned() {
    let pool = pool().await;
    let tenant_a = create_tenant(&pool).await;
    let tenant_b = create_tenant(&pool).await;
    let (admin, admin_uid, admin_email) = create_user(&pool).await;
    let (other, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant_a, admin, "org_admin").await;

    // Course + assignment live entirely in tenant B.
    let b_course = insert_course(
        &pool,
        tenant_b,
        other,
        &format!("chem-{}", Uuid::new_v4()),
        "Chemistry Secrets",
        "published",
    )
    .await;
    insert_assignment(
        &pool,
        tenant_b,
        b_course,
        other,
        "Chemistry Secrets Lab",
        "published",
    )
    .await;

    // org_admin in tenant A searches — must see nothing from tenant B.
    let app = build_test_app(
        search_routes(pool.clone()),
        StubAuth {
            pool,
            user_id: admin,
            firebase_uid: admin_uid,
            email: admin_email,
            tenant_id: Some(tenant_a),
            tenant_role: Some(core_types::TenantRole::OrgAdmin),
        },
    );
    let (status, body) = fire(&app, "GET", "/v1/search?q=Chemistry", None).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["courses"].as_array().unwrap().len(), 0);
    assert_eq!(body["assignments"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn empty_query_returns_empty_results() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (student, student_uid, student_email) = create_user(&pool).await;
    attach_membership(&pool, tenant, student, "student").await;

    let app = student_app(pool, student, student_uid, student_email, tenant);

    // Empty and whitespace-only queries must succeed with empty results.
    for uri in ["/v1/search?q=", "/v1/search?q=%20%20", "/v1/search"] {
        let (status, body) = fire(&app, "GET", uri, None).await;
        assert_eq!(status, StatusCode::OK, "uri={uri}");
        assert_eq!(body["courses"].as_array().unwrap().len(), 0, "uri={uri}");
        assert_eq!(
            body["assignments"].as_array().unwrap().len(),
            0,
            "uri={uri}"
        );
    }
}
