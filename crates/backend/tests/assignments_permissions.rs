//! Phase 1c: assignments + submissions role permissions matrix.

mod fixtures;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::json;
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

#[tokio::test]
async fn student_cannot_create_assignment() {
    let pool = fixtures::pool().await;
    let tenant = fixtures::create_tenant(&pool).await;
    let (teacher, _, _) = fixtures::create_user(&pool).await;
    let (student, _, _) = fixtures::create_user(&pool).await;
    fixtures::attach_membership(&pool, tenant, teacher, "teacher").await;
    fixtures::attach_membership(&pool, tenant, student, "student").await;
    let cid = course(&pool, tenant, teacher).await;

    let stub = fixtures::StubAuth {
        pool: pool.clone(),
        user_id: student,
        firebase_uid: "fs".into(),
        email: "s".into(),
        tenant_id: Some(tenant),
        tenant_role: Some(core_types::TenantRole::Student),
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
                "title": "x", "grading_mode": "numeric", "max_points": 100
            })
            .to_string(),
        ))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn student_cannot_grade_submission() {
    let pool = fixtures::pool().await;
    let tenant = fixtures::create_tenant(&pool).await;
    let (teacher, _, _) = fixtures::create_user(&pool).await;
    let (student, _, _) = fixtures::create_user(&pool).await;
    fixtures::attach_membership(&pool, tenant, teacher, "teacher").await;
    fixtures::attach_membership(&pool, tenant, student, "student").await;
    let cid = course(&pool, tenant, teacher).await;

    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&pool)
        .await
        .unwrap();
    let aid: Uuid = sqlx::query_scalar(
        "INSERT INTO assignments (tenant_id, course_id, title, grading_mode, max_points,
                                  status, published_at, created_by)
         VALUES ($1,$2,'A','numeric',100,'published',now(),$3) RETURNING id",
    )
    .bind(tenant)
    .bind(cid)
    .bind(teacher)
    .fetch_one(&pool)
    .await
    .unwrap();
    let sid: Uuid = sqlx::query_scalar(
        "INSERT INTO submissions (tenant_id, assignment_id, course_id, student_user_id,
                                  status, submitted_at)
         VALUES ($1,$2,$3,$4,'submitted',now()) RETURNING id",
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
        .method("POST")
        .uri(format!("/v1/submissions/{sid}/grade"))
        .header("content-type", "application/json")
        .body(Body::from(json!({"numeric_grade": 100}).to_string()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn delete_published_assignment_with_submissions_409() {
    let pool = fixtures::pool().await;
    let tenant = fixtures::create_tenant(&pool).await;
    let (teacher, _, _) = fixtures::create_user(&pool).await;
    let (student, _, _) = fixtures::create_user(&pool).await;
    fixtures::attach_membership(&pool, tenant, teacher, "teacher").await;
    fixtures::attach_membership(&pool, tenant, student, "student").await;
    let cid = course(&pool, tenant, teacher).await;

    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&pool)
        .await
        .unwrap();
    let aid: Uuid = sqlx::query_scalar(
        "INSERT INTO assignments (tenant_id, course_id, title, grading_mode, max_points,
                                  status, published_at, created_by)
         VALUES ($1,$2,'A','numeric',100,'published',now(),$3) RETURNING id",
    )
    .bind(tenant)
    .bind(cid)
    .bind(teacher)
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO submissions (tenant_id, assignment_id, course_id, student_user_id, status)
         VALUES ($1,$2,$3,$4,'draft')",
    )
    .bind(tenant)
    .bind(aid)
    .bind(cid)
    .bind(student)
    .execute(&pool)
    .await
    .unwrap();

    let stub = fixtures::StubAuth {
        pool: pool.clone(),
        user_id: teacher,
        firebase_uid: "ft".into(),
        email: "t".into(),
        tenant_id: Some(tenant),
        tenant_role: Some(core_types::TenantRole::Teacher),
    };
    let app = fixtures::build_test_app(
        backend::handlers::assignments::router_for_tests(pool.clone()),
        stub,
    );
    let req = Request::builder()
        .method("DELETE")
        .uri(format!("/v1/assignments/{aid}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn unpublish_with_submissions_409() {
    let pool = fixtures::pool().await;
    let tenant = fixtures::create_tenant(&pool).await;
    let (teacher, _, _) = fixtures::create_user(&pool).await;
    let (student, _, _) = fixtures::create_user(&pool).await;
    fixtures::attach_membership(&pool, tenant, teacher, "teacher").await;
    fixtures::attach_membership(&pool, tenant, student, "student").await;
    let cid = course(&pool, tenant, teacher).await;

    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&pool)
        .await
        .unwrap();
    let aid: Uuid = sqlx::query_scalar(
        "INSERT INTO assignments (tenant_id, course_id, title, grading_mode, max_points,
                                  status, published_at, created_by)
         VALUES ($1,$2,'A','numeric',100,'published',now(),$3) RETURNING id",
    )
    .bind(tenant)
    .bind(cid)
    .bind(teacher)
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO submissions (tenant_id, assignment_id, course_id, student_user_id, status)
         VALUES ($1,$2,$3,$4,'draft')",
    )
    .bind(tenant)
    .bind(aid)
    .bind(cid)
    .bind(student)
    .execute(&pool)
    .await
    .unwrap();

    let stub = fixtures::StubAuth {
        pool: pool.clone(),
        user_id: teacher,
        firebase_uid: "ft".into(),
        email: "t".into(),
        tenant_id: Some(tenant),
        tenant_role: Some(core_types::TenantRole::Teacher),
    };
    let app = fixtures::build_test_app(
        backend::handlers::assignments::router_for_tests(pool.clone()),
        stub,
    );
    let req = Request::builder()
        .method("POST")
        .uri(format!("/v1/assignments/{aid}/unpublish"))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
}
