//! Phase 1c: submission state machine + late detection + lock_on_submit.

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
         VALUES ($1,$2,'C',$3,'published') RETURNING id",
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

async fn published_assignment(
    pool: &sqlx::PgPool,
    tenant: Uuid,
    course_id: Uuid,
    teacher: Uuid,
    due_at: Option<chrono::DateTime<chrono::Utc>>,
    allow_late: bool,
) -> Uuid {
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(pool)
        .await
        .unwrap();
    sqlx::query_scalar(
        "INSERT INTO assignments
            (tenant_id, course_id, title, grading_mode, max_points,
             status, published_at, due_at, allow_late, created_by)
         VALUES ($1,$2,'Essay','numeric',100,'published',now(),$3,$4,$5)
         RETURNING id",
    )
    .bind(tenant)
    .bind(course_id)
    .bind(due_at)
    .bind(allow_late)
    .bind(teacher)
    .fetch_one(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn student_creates_then_submits() {
    let pool = fixtures::pool().await;
    let tenant = fixtures::create_tenant(&pool).await;
    let (teacher, _, _) = fixtures::create_user(&pool).await;
    let (student, _, _) = fixtures::create_user(&pool).await;
    fixtures::attach_membership(&pool, tenant, teacher, "teacher").await;
    fixtures::attach_membership(&pool, tenant, student, "student").await;
    let cid = course(&pool, tenant, teacher).await;
    enroll_student(&pool, tenant, cid, student).await;
    let aid = published_assignment(&pool, tenant, cid, teacher, None, true).await;

    let stub = fixtures::StubAuth {
        pool: pool.clone(),
        user_id: student,
        firebase_uid: "fb".into(),
        email: "s".into(),
        tenant_id: Some(tenant),
        tenant_role: Some(core_types::TenantRole::Student),
    };
    let app = fixtures::build_test_app(
        backend::handlers::submissions::router_for_tests(pool.clone()),
        stub.clone(),
    );

    // Create-or-get.
    let req = Request::builder()
        .method("POST")
        .uri(format!("/v1/assignments/{aid}/submissions"))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let sid = body["id"].as_str().unwrap().to_string();
    assert_eq!(body["status"], "draft");

    // PATCH text_answer.
    let app = fixtures::build_test_app(
        backend::handlers::submissions::router_for_tests(pool.clone()),
        stub.clone(),
    );
    let req = Request::builder()
        .method("PATCH")
        .uri(format!("/v1/submissions/{sid}"))
        .header("content-type", "application/json")
        .body(Body::from(json!({"text_answer": "hello"}).to_string()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Submit.
    let app = fixtures::build_test_app(
        backend::handlers::submissions::router_for_tests(pool.clone()),
        stub,
    );
    let req = Request::builder()
        .method("POST")
        .uri(format!("/v1/submissions/{sid}/submit"))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["status"], "submitted");
    assert_eq!(body["is_late"], false);
}

#[tokio::test]
async fn submission_writes_require_current_student_course_membership() {
    let pool = fixtures::pool().await;
    let tenant = fixtures::create_tenant(&pool).await;
    let (teacher, _, _) = fixtures::create_user(&pool).await;
    let (student, _, _) = fixtures::create_user(&pool).await;
    fixtures::attach_membership(&pool, tenant, teacher, "teacher").await;
    fixtures::attach_membership(&pool, tenant, student, "student").await;
    let cid = course(&pool, tenant, teacher).await;
    let aid = published_assignment(&pool, tenant, cid, teacher, None, true).await;
    let stub = fixtures::StubAuth {
        pool: pool.clone(),
        user_id: student,
        firebase_uid: "student-membership-regression".into(),
        email: "student@example.test".into(),
        tenant_id: Some(tenant),
        tenant_role: Some(core_types::TenantRole::Student),
    };

    // A tenant-level student is not automatically enrolled in every course.
    let app = fixtures::build_test_app(
        backend::handlers::submissions::router_for_tests(pool.clone()),
        stub.clone(),
    );
    let req = Request::builder()
        .method("POST")
        .uri(format!("/v1/assignments/{aid}/submissions"))
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        app.oneshot(req).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );

    enroll_student(&pool, tenant, cid, student).await;
    let app = fixtures::build_test_app(
        backend::handlers::submissions::router_for_tests(pool.clone()),
        stub.clone(),
    );
    let req = Request::builder()
        .method("POST")
        .uri(format!("/v1/assignments/{aid}/submissions"))
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let sid = body["id"].as_str().unwrap();

    sqlx::query(
        "UPDATE course_memberships SET status = 'removed'
          WHERE course_id = $1 AND user_id = $2",
    )
    .bind(cid)
    .bind(student)
    .execute(&pool)
    .await
    .unwrap();

    for (method, path, body) in [
        (
            "PATCH",
            format!("/v1/submissions/{sid}"),
            Body::from(json!({ "text_answer": "no longer enrolled" }).to_string()),
        ),
        (
            "POST",
            format!("/v1/submissions/{sid}/submit"),
            Body::empty(),
        ),
    ] {
        let app = fixtures::build_test_app(
            backend::handlers::submissions::router_for_tests(pool.clone()),
            stub.clone(),
        );
        let mut request = Request::builder().method(method).uri(path);
        if method == "PATCH" {
            request = request.header("content-type", "application/json");
        }
        let response = app.oneshot(request.body(body).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
}

#[tokio::test]
async fn teacher_lists_and_student_cannot_see_others_submission() {
    let pool = fixtures::pool().await;
    let tenant = fixtures::create_tenant(&pool).await;
    let (teacher, _, _) = fixtures::create_user(&pool).await;
    let (alice, _, _) = fixtures::create_user(&pool).await;
    let (bob, _, _) = fixtures::create_user(&pool).await;
    fixtures::attach_membership(&pool, tenant, teacher, "teacher").await;
    fixtures::attach_membership(&pool, tenant, alice, "student").await;
    fixtures::attach_membership(&pool, tenant, bob, "student").await;
    let cid = course(&pool, tenant, teacher).await;
    enroll_student(&pool, tenant, cid, alice).await;
    enroll_student(&pool, tenant, cid, bob).await;
    let aid = published_assignment(&pool, tenant, cid, teacher, None, true).await;

    // Alice creates her submission.
    let alice_stub = fixtures::StubAuth {
        pool: pool.clone(),
        user_id: alice,
        firebase_uid: "fa".into(),
        email: "a".into(),
        tenant_id: Some(tenant),
        tenant_role: Some(core_types::TenantRole::Student),
    };
    let app = fixtures::build_test_app(
        backend::handlers::submissions::router_for_tests(pool.clone()),
        alice_stub,
    );
    let req = Request::builder()
        .method("POST")
        .uri(format!("/v1/assignments/{aid}/submissions"))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let body: Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let alice_sid = body["id"].as_str().unwrap().to_string();

    // Bob tries to GET Alice's submission — 404.
    let bob_stub = fixtures::StubAuth {
        pool: pool.clone(),
        user_id: bob,
        firebase_uid: "fb".into(),
        email: "b".into(),
        tenant_id: Some(tenant),
        tenant_role: Some(core_types::TenantRole::Student),
    };
    let app = fixtures::build_test_app(
        backend::handlers::submissions::router_for_tests(pool.clone()),
        bob_stub,
    );
    let req = Request::builder()
        .method("GET")
        .uri(format!("/v1/submissions/{alice_sid}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // Teacher lists — sees Alice's row.
    let teacher_stub = fixtures::StubAuth {
        pool: pool.clone(),
        user_id: teacher,
        firebase_uid: "ft".into(),
        email: "t".into(),
        tenant_id: Some(tenant),
        tenant_role: Some(core_types::TenantRole::Teacher),
    };
    let app = fixtures::build_test_app(
        backend::handlers::submissions::router_for_tests(pool.clone()),
        teacher_stub,
    );
    let req = Request::builder()
        .method("GET")
        .uri(format!("/v1/assignments/{aid}/submissions"))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let body: Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body.as_array().unwrap().len(), 1);
}

async fn assignment_with_release_mode(
    pool: &sqlx::PgPool,
    tenant: Uuid,
    course_id: Uuid,
    teacher: Uuid,
    release_mode: &str,
) -> Uuid {
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(pool)
        .await
        .unwrap();
    sqlx::query_scalar(
        "INSERT INTO assignments (tenant_id, course_id, title, grading_mode, max_points,
                                  status, published_at, release_mode, created_by)
         VALUES ($1,$2,'Q','numeric',100,'published',now(),$3::assignment_release_mode,$4)
         RETURNING id",
    )
    .bind(tenant)
    .bind(course_id)
    .bind(release_mode)
    .bind(teacher)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn make_submission(
    pool: &sqlx::PgPool,
    tenant: Uuid,
    aid: Uuid,
    course_id: Uuid,
    student: Uuid,
) -> Uuid {
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(pool)
        .await
        .unwrap();
    sqlx::query_scalar(
        "INSERT INTO submissions (tenant_id, assignment_id, course_id, student_user_id,
                                  status, submitted_at)
         VALUES ($1,$2,$3,$4,'submitted',now()) RETURNING id",
    )
    .bind(tenant)
    .bind(aid)
    .bind(course_id)
    .bind(student)
    .fetch_one(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn instant_release_grade_visible_to_student_immediately() {
    let pool = fixtures::pool().await;
    let tenant = fixtures::create_tenant(&pool).await;
    let (teacher, _, _) = fixtures::create_user(&pool).await;
    let (student, _, _) = fixtures::create_user(&pool).await;
    fixtures::attach_membership(&pool, tenant, teacher, "teacher").await;
    fixtures::attach_membership(&pool, tenant, student, "student").await;
    let cid = course(&pool, tenant, teacher).await;
    let aid = assignment_with_release_mode(&pool, tenant, cid, teacher, "instant").await;
    let sid = make_submission(&pool, tenant, aid, cid, student).await;

    let teacher_stub = fixtures::StubAuth {
        pool: pool.clone(),
        user_id: teacher,
        firebase_uid: "ft".into(),
        email: "t".into(),
        tenant_id: Some(tenant),
        tenant_role: Some(core_types::TenantRole::Teacher),
    };
    let app = fixtures::build_test_app(
        backend::handlers::submissions::router_for_tests(pool.clone()),
        teacher_stub,
    );
    let req = Request::builder()
        .method("POST")
        .uri(format!("/v1/submissions/{sid}/grade"))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "numeric_grade": 85.5,
                "student_visible_feedback": "good"
            })
            .to_string(),
        ))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let student_stub = fixtures::StubAuth {
        pool: pool.clone(),
        user_id: student,
        firebase_uid: "fs".into(),
        email: "s".into(),
        tenant_id: Some(tenant),
        tenant_role: Some(core_types::TenantRole::Student),
    };
    let app = fixtures::build_test_app(
        backend::handlers::submissions::router_for_tests(pool.clone()),
        student_stub,
    );
    let req = Request::builder()
        .method("GET")
        .uri(format!("/v1/submissions/{sid}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let body: Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["status"], "graded");
    assert_eq!(body["numeric_grade"].as_f64().unwrap(), 85.5);
    assert_eq!(body["student_visible_feedback"], "good");
}

#[tokio::test]
async fn manual_release_grade_hidden_from_student_until_release() {
    let pool = fixtures::pool().await;
    let tenant = fixtures::create_tenant(&pool).await;
    let (teacher, _, _) = fixtures::create_user(&pool).await;
    let (student, _, _) = fixtures::create_user(&pool).await;
    fixtures::attach_membership(&pool, tenant, teacher, "teacher").await;
    fixtures::attach_membership(&pool, tenant, student, "student").await;
    let cid = course(&pool, tenant, teacher).await;
    let aid = assignment_with_release_mode(&pool, tenant, cid, teacher, "manual").await;
    let sid = make_submission(&pool, tenant, aid, cid, student).await;

    let teacher_stub = fixtures::StubAuth {
        pool: pool.clone(),
        user_id: teacher,
        firebase_uid: "ft".into(),
        email: "t".into(),
        tenant_id: Some(tenant),
        tenant_role: Some(core_types::TenantRole::Teacher),
    };
    let app = fixtures::build_test_app(
        backend::handlers::submissions::router_for_tests(pool.clone()),
        teacher_stub.clone(),
    );
    let req = Request::builder()
        .method("POST")
        .uri(format!("/v1/submissions/{sid}/grade"))
        .header("content-type", "application/json")
        .body(Body::from(json!({"numeric_grade": 70}).to_string()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Student sees graded status but no grade fields (released_at NULL).
    let student_stub = fixtures::StubAuth {
        pool: pool.clone(),
        user_id: student,
        firebase_uid: "fs".into(),
        email: "s".into(),
        tenant_id: Some(tenant),
        tenant_role: Some(core_types::TenantRole::Student),
    };
    let app = fixtures::build_test_app(
        backend::handlers::submissions::router_for_tests(pool.clone()),
        student_stub.clone(),
    );
    let req = Request::builder()
        .method("GET")
        .uri(format!("/v1/submissions/{sid}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let body: Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert!(body["numeric_grade"].is_null());
    assert!(body["student_visible_feedback"].is_null());

    // Teacher releases.
    let app = fixtures::build_test_app(
        backend::handlers::submissions::router_for_tests(pool.clone()),
        teacher_stub,
    );
    let req = Request::builder()
        .method("POST")
        .uri(format!("/v1/submissions/{sid}/release"))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Student now sees grade.
    let app = fixtures::build_test_app(
        backend::handlers::submissions::router_for_tests(pool.clone()),
        student_stub,
    );
    let req = Request::builder()
        .method("GET")
        .uri(format!("/v1/submissions/{sid}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let body: Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["numeric_grade"].as_f64().unwrap(), 70.0);
}

#[tokio::test]
async fn grade_rejects_out_of_range_numeric() {
    let pool = fixtures::pool().await;
    let tenant = fixtures::create_tenant(&pool).await;
    let (teacher, _, _) = fixtures::create_user(&pool).await;
    let (student, _, _) = fixtures::create_user(&pool).await;
    fixtures::attach_membership(&pool, tenant, teacher, "teacher").await;
    fixtures::attach_membership(&pool, tenant, student, "student").await;
    let cid = course(&pool, tenant, teacher).await;
    let aid = assignment_with_release_mode(&pool, tenant, cid, teacher, "instant").await;
    let sid = make_submission(&pool, tenant, aid, cid, student).await;

    let stub = fixtures::StubAuth {
        pool: pool.clone(),
        user_id: teacher,
        firebase_uid: "ft".into(),
        email: "t".into(),
        tenant_id: Some(tenant),
        tenant_role: Some(core_types::TenantRole::Teacher),
    };
    let app = fixtures::build_test_app(
        backend::handlers::submissions::router_for_tests(pool.clone()),
        stub,
    );
    let req = Request::builder()
        .method("POST")
        .uri(format!("/v1/submissions/{sid}/grade"))
        .header("content-type", "application/json")
        .body(Body::from(json!({"numeric_grade": 150}).to_string()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn teacher_returns_graded_for_resubmit() {
    let pool = fixtures::pool().await;
    let tenant = fixtures::create_tenant(&pool).await;
    let (teacher, _, _) = fixtures::create_user(&pool).await;
    let (student, _, _) = fixtures::create_user(&pool).await;
    fixtures::attach_membership(&pool, tenant, teacher, "teacher").await;
    fixtures::attach_membership(&pool, tenant, student, "student").await;
    let cid = course(&pool, tenant, teacher).await;
    let aid = assignment_with_release_mode(&pool, tenant, cid, teacher, "instant").await;
    let sid = make_submission(&pool, tenant, aid, cid, student).await;

    let teacher_stub = fixtures::StubAuth {
        pool: pool.clone(),
        user_id: teacher,
        firebase_uid: "ft".into(),
        email: "t".into(),
        tenant_id: Some(tenant),
        tenant_role: Some(core_types::TenantRole::Teacher),
    };
    // Grade.
    let app = fixtures::build_test_app(
        backend::handlers::submissions::router_for_tests(pool.clone()),
        teacher_stub.clone(),
    );
    let req = Request::builder()
        .method("POST")
        .uri(format!("/v1/submissions/{sid}/grade"))
        .header("content-type", "application/json")
        .body(Body::from(json!({"numeric_grade": 50}).to_string()))
        .unwrap();
    assert_eq!(app.oneshot(req).await.unwrap().status(), StatusCode::OK);
    // Return.
    let app = fixtures::build_test_app(
        backend::handlers::submissions::router_for_tests(pool.clone()),
        teacher_stub,
    );
    let req = Request::builder()
        .method("POST")
        .uri(format!("/v1/submissions/{sid}/return"))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["status"], "returned");
    assert!(body["released_at"].is_null());
}

#[tokio::test]
async fn lock_on_submit_blocks_patch_after_submit() {
    let pool = fixtures::pool().await;
    let tenant = fixtures::create_tenant(&pool).await;
    let (teacher, _, _) = fixtures::create_user(&pool).await;
    let (student, _, _) = fixtures::create_user(&pool).await;
    fixtures::attach_membership(&pool, tenant, teacher, "teacher").await;
    fixtures::attach_membership(&pool, tenant, student, "student").await;
    let cid = course(&pool, tenant, teacher).await;
    enroll_student(&pool, tenant, cid, student).await;

    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&pool)
        .await
        .unwrap();
    let aid: Uuid = sqlx::query_scalar(
        "INSERT INTO assignments
            (tenant_id, course_id, title, grading_mode, max_points, status,
             published_at, lock_on_submit, created_by)
         VALUES ($1,$2,'L','numeric',100,'published',now(),true,$3)
         RETURNING id",
    )
    .bind(tenant)
    .bind(cid)
    .bind(teacher)
    .fetch_one(&pool)
    .await
    .unwrap();
    let sid: Uuid = sqlx::query_scalar(
        "INSERT INTO submissions (tenant_id, assignment_id, course_id, student_user_id,
                                  status, submitted_at, text_answer)
         VALUES ($1,$2,$3,$4,'submitted',now(),'answer') RETURNING id",
    )
    .bind(tenant)
    .bind(aid)
    .bind(cid)
    .bind(student)
    .fetch_one(&pool)
    .await
    .unwrap();

    let stub = fixtures::StubAuth {
        pool: pool.clone(),
        user_id: student,
        firebase_uid: "fs".into(),
        email: "s".into(),
        tenant_id: Some(tenant),
        tenant_role: Some(core_types::TenantRole::Student),
    };
    let app = fixtures::build_test_app(
        backend::handlers::submissions::router_for_tests(pool.clone()),
        stub,
    );
    let req = Request::builder()
        .method("PATCH")
        .uri(format!("/v1/submissions/{sid}"))
        .header("content-type", "application/json")
        .body(Body::from(json!({"text_answer": "edited"}).to_string()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn late_submit_rejected_when_allow_late_false() {
    let pool = fixtures::pool().await;
    let tenant = fixtures::create_tenant(&pool).await;
    let (teacher, _, _) = fixtures::create_user(&pool).await;
    let (student, _, _) = fixtures::create_user(&pool).await;
    fixtures::attach_membership(&pool, tenant, teacher, "teacher").await;
    fixtures::attach_membership(&pool, tenant, student, "student").await;
    let cid = course(&pool, tenant, teacher).await;
    enroll_student(&pool, tenant, cid, student).await;
    let past = chrono::Utc::now() - chrono::Duration::hours(1);
    let aid = published_assignment(&pool, tenant, cid, teacher, Some(past), false).await;

    let stub = fixtures::StubAuth {
        pool: pool.clone(),
        user_id: student,
        firebase_uid: "fs".into(),
        email: "s".into(),
        tenant_id: Some(tenant),
        tenant_role: Some(core_types::TenantRole::Student),
    };
    let app = fixtures::build_test_app(
        backend::handlers::submissions::router_for_tests(pool.clone()),
        stub.clone(),
    );
    // Create-or-get.
    let req = Request::builder()
        .method("POST")
        .uri(format!("/v1/assignments/{aid}/submissions"))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let body: Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let sid = body["id"].as_str().unwrap().to_string();

    // Submit — should 422.
    let app = fixtures::build_test_app(
        backend::handlers::submissions::router_for_tests(pool.clone()),
        stub,
    );
    let req = Request::builder()
        .method("POST")
        .uri(format!("/v1/submissions/{sid}/submit"))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
}
