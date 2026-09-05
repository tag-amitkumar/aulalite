//! Assignment-policy regression tests: late-submission penalties + the
//! resubmission cap. Mirrors `assignments_crud.rs` / `submissions_flow.rs`.
//! Requires a DB to run (the workspace test harness provisions one); these are
//! exercised the same way as the existing assignment suites.

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

/// Seed a published numeric assignment carrying explicit late-penalty +
/// resubmission policy values, with an optional `due_at`.
async fn published_assignment_with_policy(
    pool: &sqlx::PgPool,
    tenant: Uuid,
    course_id: Uuid,
    teacher: Uuid,
    due_at: Option<chrono::DateTime<chrono::Utc>>,
    late_penalty_percent: i32,
    max_resubmissions: i32,
) -> Uuid {
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(pool)
        .await
        .unwrap();
    sqlx::query_scalar(
        "INSERT INTO assignments
            (tenant_id, course_id, title, grading_mode, max_points,
             status, published_at, due_at, allow_late,
             late_penalty_percent, max_resubmissions, created_by)
         VALUES ($1,$2,'Essay','numeric',100,'published',now(),$3,true,$4,$5,$6)
         RETURNING id",
    )
    .bind(tenant)
    .bind(course_id)
    .bind(due_at)
    .bind(late_penalty_percent)
    .bind(max_resubmissions)
    .bind(teacher)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// Seed a submission row directly in a given status, optionally late.
async fn make_submission(
    pool: &sqlx::PgPool,
    tenant: Uuid,
    aid: Uuid,
    course_id: Uuid,
    student: Uuid,
    is_late: bool,
    submitted_at: chrono::DateTime<chrono::Utc>,
) -> Uuid {
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(pool)
        .await
        .unwrap();
    sqlx::query_scalar(
        "INSERT INTO submissions (tenant_id, assignment_id, course_id, student_user_id,
                                  status, submitted_at, is_late)
         VALUES ($1,$2,$3,$4,'submitted',$5,$6) RETURNING id",
    )
    .bind(tenant)
    .bind(aid)
    .bind(course_id)
    .bind(student)
    .bind(submitted_at)
    .bind(is_late)
    .fetch_one(pool)
    .await
    .unwrap()
}

fn teacher_stub(pool: &sqlx::PgPool, teacher: Uuid, tenant: Uuid) -> fixtures::StubAuth {
    fixtures::StubAuth {
        pool: pool.clone(),
        user_id: teacher,
        firebase_uid: "ft".into(),
        email: "t".into(),
        tenant_id: Some(tenant),
        tenant_role: Some(core_types::TenantRole::Teacher),
    }
}

#[tokio::test]
async fn create_assignment_round_trips_policy_values() {
    let pool = fixtures::pool().await;
    let tenant = fixtures::create_tenant(&pool).await;
    let (teacher, _, _) = fixtures::create_user(&pool).await;
    fixtures::attach_membership(&pool, tenant, teacher, "teacher").await;
    let cid = course(&pool, tenant, teacher).await;

    let app = fixtures::build_test_app(
        backend::handlers::assignments::router_for_tests(pool.clone()),
        teacher_stub(&pool, teacher, tenant),
    );
    let req = Request::builder()
        .method("POST")
        .uri(format!("/v1/courses/{cid}/assignments"))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "title": "Essay",
                "instructions_md": "x",
                "grading_mode": "numeric",
                "max_points": 100,
                "accepts_text": true,
                "accepts_files": false,
                "late_penalty_percent": 20,
                "max_resubmissions": 2
            })
            .to_string(),
        ))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body: Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["late_penalty_percent"], 20);
    assert_eq!(body["max_resubmissions"], 2);
}

#[tokio::test]
async fn create_assignment_defaults_policy_to_zero() {
    let pool = fixtures::pool().await;
    let tenant = fixtures::create_tenant(&pool).await;
    let (teacher, _, _) = fixtures::create_user(&pool).await;
    fixtures::attach_membership(&pool, tenant, teacher, "teacher").await;
    let cid = course(&pool, tenant, teacher).await;

    let app = fixtures::build_test_app(
        backend::handlers::assignments::router_for_tests(pool.clone()),
        teacher_stub(&pool, teacher, tenant),
    );
    let req = Request::builder()
        .method("POST")
        .uri(format!("/v1/courses/{cid}/assignments"))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "title": "No policy",
                "instructions_md": "x",
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
    assert_eq!(body["late_penalty_percent"], 0);
    assert_eq!(body["max_resubmissions"], 0);
}

#[tokio::test]
async fn rejects_out_of_range_policy_values_with_422() {
    let pool = fixtures::pool().await;
    let tenant = fixtures::create_tenant(&pool).await;
    let (teacher, _, _) = fixtures::create_user(&pool).await;
    fixtures::attach_membership(&pool, tenant, teacher, "teacher").await;
    let cid = course(&pool, tenant, teacher).await;

    // late_penalty_percent > 100 → 422 (not a DB 500).
    let app = fixtures::build_test_app(
        backend::handlers::assignments::router_for_tests(pool.clone()),
        teacher_stub(&pool, teacher, tenant),
    );
    let req = Request::builder()
        .method("POST")
        .uri(format!("/v1/courses/{cid}/assignments"))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "title": "Bad penalty",
                "instructions_md": "x",
                "grading_mode": "numeric",
                "max_points": 100,
                "accepts_text": true,
                "accepts_files": false,
                "late_penalty_percent": 150
            })
            .to_string(),
        ))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "late_penalty_percent > 100 must be 422, not 500"
    );

    // Negative max_resubmissions → 422.
    let app = fixtures::build_test_app(
        backend::handlers::assignments::router_for_tests(pool.clone()),
        teacher_stub(&pool, teacher, tenant),
    );
    let req = Request::builder()
        .method("POST")
        .uri(format!("/v1/courses/{cid}/assignments"))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "title": "Bad resubmits",
                "instructions_md": "x",
                "grading_mode": "numeric",
                "max_points": 100,
                "accepts_text": true,
                "accepts_files": false,
                "max_resubmissions": -1
            })
            .to_string(),
        ))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "negative max_resubmissions must be 422, not 500"
    );
}

#[tokio::test]
async fn late_submission_has_penalty_applied_to_numeric_grade() {
    let pool = fixtures::pool().await;
    let tenant = fixtures::create_tenant(&pool).await;
    let (teacher, _, _) = fixtures::create_user(&pool).await;
    let (student, _, _) = fixtures::create_user(&pool).await;
    fixtures::attach_membership(&pool, tenant, teacher, "teacher").await;
    fixtures::attach_membership(&pool, tenant, student, "student").await;
    let cid = course(&pool, tenant, teacher).await;
    // Due in the past, 20% penalty.
    let due = chrono::Utc::now() - chrono::Duration::hours(2);
    let aid = published_assignment_with_policy(&pool, tenant, cid, teacher, Some(due), 20, 0).await;
    // Submitted after the due date → late.
    let submitted = chrono::Utc::now() - chrono::Duration::hours(1);
    let sid = make_submission(&pool, tenant, aid, cid, student, true, submitted).await;

    // Teacher grades a raw 100; 20% penalty → effective 80, applied 20.
    let app = fixtures::build_test_app(
        backend::handlers::submissions::router_for_tests(pool.clone()),
        teacher_stub(&pool, teacher, tenant),
    );
    let req = Request::builder()
        .method("POST")
        .uri(format!("/v1/submissions/{sid}/grade"))
        .header("content-type", "application/json")
        .body(Body::from(json!({"numeric_grade": 100}).to_string()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["numeric_grade"].as_f64().unwrap(), 80.0);
    assert_eq!(body["applied_late_penalty_percent"], 20);
}

#[tokio::test]
async fn on_time_submission_has_no_penalty_applied() {
    let pool = fixtures::pool().await;
    let tenant = fixtures::create_tenant(&pool).await;
    let (teacher, _, _) = fixtures::create_user(&pool).await;
    let (student, _, _) = fixtures::create_user(&pool).await;
    fixtures::attach_membership(&pool, tenant, teacher, "teacher").await;
    fixtures::attach_membership(&pool, tenant, student, "student").await;
    let cid = course(&pool, tenant, teacher).await;
    let due = chrono::Utc::now() + chrono::Duration::hours(2);
    let aid = published_assignment_with_policy(&pool, tenant, cid, teacher, Some(due), 20, 0).await;
    let submitted = chrono::Utc::now() - chrono::Duration::hours(1);
    let sid = make_submission(&pool, tenant, aid, cid, student, false, submitted).await;

    let app = fixtures::build_test_app(
        backend::handlers::submissions::router_for_tests(pool.clone()),
        teacher_stub(&pool, teacher, tenant),
    );
    let req = Request::builder()
        .method("POST")
        .uri(format!("/v1/submissions/{sid}/grade"))
        .header("content-type", "application/json")
        .body(Body::from(json!({"numeric_grade": 90}).to_string()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["numeric_grade"].as_f64().unwrap(), 90.0);
    assert_eq!(body["applied_late_penalty_percent"], 0);
}

#[tokio::test]
async fn return_within_budget_increments_attempt_number() {
    let pool = fixtures::pool().await;
    let tenant = fixtures::create_tenant(&pool).await;
    let (teacher, _, _) = fixtures::create_user(&pool).await;
    let (student, _, _) = fixtures::create_user(&pool).await;
    fixtures::attach_membership(&pool, tenant, teacher, "teacher").await;
    fixtures::attach_membership(&pool, tenant, student, "student").await;
    let cid = course(&pool, tenant, teacher).await;
    // Allow up to 2 resubmissions.
    let aid = published_assignment_with_policy(&pool, tenant, cid, teacher, None, 0, 2).await;
    let submitted = chrono::Utc::now();
    let sid = make_submission(&pool, tenant, aid, cid, student, false, submitted).await;

    let app = fixtures::build_test_app(
        backend::handlers::submissions::router_for_tests(pool.clone()),
        teacher_stub(&pool, teacher, tenant),
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
    assert_eq!(body["attempt_number"], 2);
}

#[tokio::test]
async fn return_rejected_when_resubmissions_exhausted() {
    let pool = fixtures::pool().await;
    let tenant = fixtures::create_tenant(&pool).await;
    let (teacher, _, _) = fixtures::create_user(&pool).await;
    let (student, _, _) = fixtures::create_user(&pool).await;
    fixtures::attach_membership(&pool, tenant, teacher, "teacher").await;
    fixtures::attach_membership(&pool, tenant, student, "student").await;
    let cid = course(&pool, tenant, teacher).await;
    // max_resubmissions = 0 → no resubmission allowed at all.
    let aid = published_assignment_with_policy(&pool, tenant, cid, teacher, None, 0, 0).await;
    let submitted = chrono::Utc::now();
    let sid = make_submission(&pool, tenant, aid, cid, student, false, submitted).await;

    let app = fixtures::build_test_app(
        backend::handlers::submissions::router_for_tests(pool.clone()),
        teacher_stub(&pool, teacher, tenant),
    );
    let req = Request::builder()
        .method("POST")
        .uri(format!("/v1/submissions/{sid}/return"))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::CONFLICT,
        "return beyond max_resubmissions must be 409"
    );
}
