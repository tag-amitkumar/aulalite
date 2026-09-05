//! Phase 1c: assignments CRUD smoke tests.

mod fixtures;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

async fn course(pool: &sqlx::PgPool, tenant: Uuid, teacher: Uuid) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO courses (tenant_id, slug, title, owner_user_id, status)
         VALUES ($1, $2, 'C', $3, 'published') RETURNING id",
    )
    .bind(tenant)
    .bind(format!("c-{}", Uuid::new_v4()))
    .bind(teacher)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn enroll_student(pool: &sqlx::PgPool, tenant: Uuid, course_id: Uuid, student: Uuid) {
    sqlx::query(
        "INSERT INTO course_memberships (course_id, user_id, tenant_id, role, status)
         VALUES ($1, $2, $3, 'student', 'active')",
    )
    .bind(course_id)
    .bind(student)
    .bind(tenant)
    .execute(pool)
    .await
    .unwrap();
}

async fn create_assignment(pool: &sqlx::PgPool, tenant: Uuid, teacher: Uuid, cid: Uuid) -> Uuid {
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(pool)
        .await
        .unwrap();
    sqlx::query_scalar(
        "INSERT INTO assignments (tenant_id, course_id, title, grading_mode, max_points, created_by)
         VALUES ($1,$2,'A','numeric',100,$3) RETURNING id",
    ).bind(tenant).bind(cid).bind(teacher).fetch_one(pool).await.unwrap()
}

#[tokio::test]
async fn teacher_creates_draft_assignment() {
    let pool = fixtures::pool().await;
    let tenant = fixtures::create_tenant(&pool).await;
    let (teacher, _, _) = fixtures::create_user(&pool).await;
    fixtures::attach_membership(&pool, tenant, teacher, "teacher").await;
    let cid = course(&pool, tenant, teacher).await;

    let stub = fixtures::StubAuth {
        pool: pool.clone(),
        user_id: teacher,
        firebase_uid: "fb".into(),
        email: "t@x".into(),
        tenant_id: Some(tenant),
        tenant_role: Some(core_types::TenantRole::Teacher),
    };
    let app = fixtures::build_test_app(
        backend::handlers::assignments::router_for_tests(pool.clone()),
        stub,
    );

    let req = Request::builder()
        .method("POST")
        .uri(format!("/v1/courses/{cid}/assignments"))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "title": "Essay 1",
                "instructions_md": "Write 500 words.",
                "grading_mode": "numeric",
                "max_points": 100,
                "accepts_text": true,
                "accepts_files": false
            })
            .to_string(),
        ))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body: Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["status"], "draft");
    assert_eq!(body["grading_mode"], "numeric");
    assert_eq!(body["max_points"], 100);
}

#[tokio::test]
async fn rejects_unknown_grading_or_release_mode_with_422() {
    // Regression: an out-of-range grading_mode/release_mode must be rejected
    // with a clean 422 (ApiError::Validation) rather than reaching the Postgres
    // `::assignment_*_mode` enum cast and surfacing as an opaque 500.
    let pool = fixtures::pool().await;
    let tenant = fixtures::create_tenant(&pool).await;
    let (teacher, _, _) = fixtures::create_user(&pool).await;
    fixtures::attach_membership(&pool, tenant, teacher, "teacher").await;
    let cid = course(&pool, tenant, teacher).await;

    let stub = fixtures::StubAuth {
        pool: pool.clone(),
        user_id: teacher,
        firebase_uid: "fb".into(),
        email: "t@x".into(),
        tenant_id: Some(tenant),
        tenant_role: Some(core_types::TenantRole::Teacher),
    };

    // Invalid release_mode.
    let app = fixtures::build_test_app(
        backend::handlers::assignments::router_for_tests(pool.clone()),
        stub.clone(),
    );
    let req = Request::builder()
        .method("POST")
        .uri(format!("/v1/courses/{cid}/assignments"))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "title": "Bad release mode",
                "instructions_md": "x",
                "grading_mode": "numeric",
                "max_points": 100,
                "accepts_text": true,
                "accepts_files": false,
                "release_mode": "deferred"
            })
            .to_string(),
        ))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "invalid release_mode must be 422, not 500"
    );

    // Invalid grading_mode.
    let app = fixtures::build_test_app(
        backend::handlers::assignments::router_for_tests(pool.clone()),
        stub,
    );
    let req = Request::builder()
        .method("POST")
        .uri(format!("/v1/courses/{cid}/assignments"))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "title": "Bad grading mode",
                "instructions_md": "x",
                "grading_mode": "letter",
                "accepts_text": true,
                "accepts_files": false
            })
            .to_string(),
        ))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "invalid grading_mode must be 422, not 500"
    );
}

#[tokio::test]
async fn publish_then_list_excludes_drafts_for_students() {
    let pool = fixtures::pool().await;
    let tenant = fixtures::create_tenant(&pool).await;
    let (teacher, _, _) = fixtures::create_user(&pool).await;
    let (student, _, _) = fixtures::create_user(&pool).await;
    fixtures::attach_membership(&pool, tenant, teacher, "teacher").await;
    fixtures::attach_membership(&pool, tenant, student, "student").await;
    let cid = course(&pool, tenant, teacher).await;
    enroll_student(&pool, tenant, cid, student).await;
    let aid = create_assignment(&pool, tenant, teacher, cid).await;

    // Teacher publishes.
    let teacher_stub = fixtures::StubAuth {
        pool: pool.clone(),
        user_id: teacher,
        firebase_uid: "fb".into(),
        email: "t".into(),
        tenant_id: Some(tenant),
        tenant_role: Some(core_types::TenantRole::Teacher),
    };
    let app = fixtures::build_test_app(
        backend::handlers::assignments::router_for_tests(pool.clone()),
        teacher_stub,
    );
    let req = Request::builder()
        .method("POST")
        .uri(format!("/v1/assignments/{aid}/publish"))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Student lists course assignments — sees published.
    let student_stub = fixtures::StubAuth {
        pool: pool.clone(),
        user_id: student,
        firebase_uid: "fb2".into(),
        email: "s".into(),
        tenant_id: Some(tenant),
        tenant_role: Some(core_types::TenantRole::Student),
    };
    let app = fixtures::build_test_app(
        backend::handlers::assignments::router_for_tests(pool.clone()),
        student_stub.clone(),
    );
    let req = Request::builder()
        .method("GET")
        .uri(format!("/v1/courses/{cid}/assignments"))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let body: Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body.as_array().unwrap().len(), 1);

    // Student GETs the assignment by id — visible.
    let app = fixtures::build_test_app(
        backend::handlers::assignments::router_for_tests(pool.clone()),
        student_stub,
    );
    let req = Request::builder()
        .method("GET")
        .uri(format!("/v1/assignments/{aid}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn student_cannot_see_draft_assignment() {
    let pool = fixtures::pool().await;
    let tenant = fixtures::create_tenant(&pool).await;
    let (teacher, _, _) = fixtures::create_user(&pool).await;
    let (student, _, _) = fixtures::create_user(&pool).await;
    fixtures::attach_membership(&pool, tenant, teacher, "teacher").await;
    fixtures::attach_membership(&pool, tenant, student, "student").await;
    let cid = course(&pool, tenant, teacher).await;
    enroll_student(&pool, tenant, cid, student).await;
    let aid = create_assignment(&pool, tenant, teacher, cid).await;

    let stub = fixtures::StubAuth {
        pool: pool.clone(),
        user_id: student,
        firebase_uid: "fb".into(),
        email: "s".into(),
        tenant_id: Some(tenant),
        tenant_role: Some(core_types::TenantRole::Student),
    };
    let app = fixtures::build_test_app(
        backend::handlers::assignments::router_for_tests(pool.clone()),
        stub.clone(),
    );
    let req = Request::builder()
        .method("GET")
        .uri(format!("/v1/courses/{cid}/assignments?include_drafts=true"))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert!(body.as_array().unwrap().is_empty());

    let app = fixtures::build_test_app(
        backend::handlers::assignments::router_for_tests(pool.clone()),
        stub,
    );
    let req = Request::builder()
        .method("GET")
        .uri(format!("/v1/assignments/{aid}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn assignment_reads_require_access_to_the_concrete_course() {
    let pool = fixtures::pool().await;
    let tenant = fixtures::create_tenant(&pool).await;
    let (teacher, _, _) = fixtures::create_user(&pool).await;
    let (student, _, _) = fixtures::create_user(&pool).await;
    fixtures::attach_membership(&pool, tenant, teacher, "teacher").await;
    fixtures::attach_membership(&pool, tenant, student, "student").await;
    let cid = course(&pool, tenant, teacher).await;
    let aid = create_assignment(&pool, tenant, teacher, cid).await;
    let stub = fixtures::StubAuth {
        pool: pool.clone(),
        user_id: student,
        firebase_uid: "unenrolled-student".into(),
        email: "unenrolled@example.test".into(),
        tenant_id: Some(tenant),
        tenant_role: Some(core_types::TenantRole::Student),
    };

    for uri in [
        format!("/v1/courses/{cid}/assignments?include_drafts=true"),
        format!("/v1/assignments/{aid}"),
    ] {
        let app = fixtures::build_test_app(
            backend::handlers::assignments::router_for_tests(pool.clone()),
            stub.clone(),
        );
        let req = Request::builder()
            .method("GET")
            .uri(uri)
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            app.oneshot(req).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );
    }
}
