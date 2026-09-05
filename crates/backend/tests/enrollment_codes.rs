mod fixtures;

use fixtures::*;
use serde_json::json;

async fn seed_blocking_seat_cap(pool: &sqlx::PgPool, tenant: uuid::Uuid, seats: i32) {
    let plan_id = format!("plan-{}", uuid::Uuid::new_v4());
    sqlx::query(
        "INSERT INTO plans (id, name, monthly_price_cents, included_seats,
                            included_class_minutes, included_recording_gb)
         VALUES ($1, 'Seat test', 0, $2, 0, 0)",
    )
    .bind(&plan_id)
    .bind(seats)
    .execute(pool)
    .await
    .unwrap();

    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO subscriptions (tenant_id, plan_id, status, overage_behavior)
         VALUES ($1, $2, 'active', 'block')",
    )
    .bind(tenant)
    .bind(plan_id)
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
}

async fn course_for(pool: &sqlx::PgPool, tenant: uuid::Uuid, owner: uuid::Uuid) -> uuid::Uuid {
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    let id: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO courses (tenant_id, slug, title, owner_user_id)
         VALUES ($1, $2, 'C', $3) RETURNING id",
    )
    .bind(tenant)
    .bind(format!("c-{}", uuid::Uuid::new_v4()))
    .bind(owner)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO course_memberships (course_id, user_id, tenant_id, role)
         VALUES ($1, $2, $3, 'teacher')",
    )
    .bind(id)
    .bind(owner)
    .bind(tenant)
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    id
}

#[tokio::test]
async fn teacher_generates_code_then_student_redeems() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, fb_t, em_t) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let course = course_for(&pool, tenant, teacher).await;

    let app_teacher = build_test_app(
        backend::handlers::enrollments::router_for_tests(pool.clone()),
        StubAuth {
            pool: pool.clone(),
            user_id: teacher,
            firebase_uid: fb_t,
            email: em_t,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Teacher),
        },
    );

    let (status, body) = fire(
        &app_teacher,
        "POST",
        &format!("/v1/courses/{course}/codes"),
        Some(json!({ "max_uses": 5 })),
    )
    .await;
    assert_eq!(status, 200);
    let code = body["code"].as_str().unwrap().to_string();
    assert_eq!(code.len(), 8);

    let (student, fb_s, em_s) = create_user(&pool).await;
    let app_student = build_test_app(
        backend::handlers::enrollments::router_for_tests(pool.clone()),
        StubAuth {
            pool: pool.clone(),
            user_id: student,
            firebase_uid: fb_s,
            email: em_s,
            tenant_id: None,
            tenant_role: None,
        },
    );
    let (status, body) = fire(
        &app_student,
        "POST",
        "/v1/codes/redeem",
        Some(json!({ "code": code })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["course_id"].as_str().unwrap(), course.to_string());

    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM tenant_memberships
          WHERE tenant_id=$1 AND user_id=$2 AND role='student' AND status='active'",
    )
    .bind(tenant)
    .bind(student)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 1);
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM course_memberships
          WHERE course_id=$1 AND user_id=$2 AND role='student' AND status='active'",
    )
    .bind(course)
    .bind(student)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn redemption_respects_seat_cap_without_consuming_code_and_active_retry_is_idempotent() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, fb_t, em_t) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    seed_blocking_seat_cap(&pool, tenant, 1).await;
    let course = course_for(&pool, tenant, teacher).await;

    let teacher_app = build_test_app(
        backend::handlers::enrollments::router_for_tests(pool.clone()),
        StubAuth {
            pool: pool.clone(),
            user_id: teacher,
            firebase_uid: fb_t,
            email: em_t,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Teacher),
        },
    );
    let (_, created) = fire(
        &teacher_app,
        "POST",
        &format!("/v1/courses/{course}/codes"),
        Some(json!({ "max_uses": 5 })),
    )
    .await;
    let code = created["code"].as_str().unwrap().to_string();

    let (student, fb_s, em_s) = create_user(&pool).await;
    let student_app = build_test_app(
        backend::handlers::enrollments::router_for_tests(pool.clone()),
        StubAuth {
            pool: pool.clone(),
            user_id: student,
            firebase_uid: fb_s,
            email: em_s,
            tenant_id: None,
            tenant_role: None,
        },
    );

    let (status, body) = fire(
        &student_app,
        "POST",
        "/v1/codes/redeem",
        Some(json!({ "code": code.clone() })),
    )
    .await;
    assert_eq!(status, 409, "{body}");
    assert_eq!(body["error"], "conflict: seat_limit_reached");

    let uses: i32 = sqlx::query_scalar("SELECT uses FROM enrollment_codes WHERE code = $1")
        .bind(&code)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(uses, 0, "blocked redemption must not consume the code");

    // Existing active members do not consume another seat, so a retry remains
    // idempotently valid even while the tenant is at/over its configured cap.
    attach_membership(&pool, tenant, student, "student").await;
    let (status, body) = fire(
        &student_app,
        "POST",
        "/v1/codes/redeem",
        Some(json!({ "code": code })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
}

#[tokio::test]
async fn redeem_rejects_when_max_uses_exhausted() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, fb_t, em_t) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let course = course_for(&pool, tenant, teacher).await;

    let app_t = build_test_app(
        backend::handlers::enrollments::router_for_tests(pool.clone()),
        StubAuth {
            pool: pool.clone(),
            user_id: teacher,
            firebase_uid: fb_t,
            email: em_t,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Teacher),
        },
    );
    let (_, body) = fire(
        &app_t,
        "POST",
        &format!("/v1/courses/{course}/codes"),
        Some(json!({ "max_uses": 1 })),
    )
    .await;
    let code = body["code"].as_str().unwrap().to_string();

    let (s1, fb1, em1) = create_user(&pool).await;
    let app_s1 = build_test_app(
        backend::handlers::enrollments::router_for_tests(pool.clone()),
        StubAuth {
            pool: pool.clone(),
            user_id: s1,
            firebase_uid: fb1,
            email: em1,
            tenant_id: None,
            tenant_role: None,
        },
    );
    let (status, _) = fire(
        &app_s1,
        "POST",
        "/v1/codes/redeem",
        Some(json!({ "code": code.clone() })),
    )
    .await;
    assert_eq!(status, 200);

    let (s2, fb2, em2) = create_user(&pool).await;
    let app_s2 = build_test_app(
        backend::handlers::enrollments::router_for_tests(pool.clone()),
        StubAuth {
            pool: pool.clone(),
            user_id: s2,
            firebase_uid: fb2,
            email: em2,
            tenant_id: None,
            tenant_role: None,
        },
    );
    let (status, body) = fire(
        &app_s2,
        "POST",
        "/v1/codes/redeem",
        Some(json!({ "code": code })),
    )
    .await;
    assert_eq!(status, 400);
    assert!(body["error"].as_str().unwrap().contains("invalid"));
}

#[tokio::test]
async fn concurrent_redemption_only_one_wins() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, fb_t, em_t) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let course = course_for(&pool, tenant, teacher).await;

    let app_t = build_test_app(
        backend::handlers::enrollments::router_for_tests(pool.clone()),
        StubAuth {
            pool: pool.clone(),
            user_id: teacher,
            firebase_uid: fb_t,
            email: em_t,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Teacher),
        },
    );
    let (_, body) = fire(
        &app_t,
        "POST",
        &format!("/v1/courses/{course}/codes"),
        Some(serde_json::json!({ "max_uses": 1 })),
    )
    .await;
    let code = body["code"].as_str().unwrap().to_string();

    let (s1, fb1, em1) = create_user(&pool).await;
    let (s2, fb2, em2) = create_user(&pool).await;

    let app1 = build_test_app(
        backend::handlers::enrollments::router_for_tests(pool.clone()),
        StubAuth {
            pool: pool.clone(),
            user_id: s1,
            firebase_uid: fb1,
            email: em1,
            tenant_id: None,
            tenant_role: None,
        },
    );
    let app2 = build_test_app(
        backend::handlers::enrollments::router_for_tests(pool.clone()),
        StubAuth {
            pool: pool.clone(),
            user_id: s2,
            firebase_uid: fb2,
            email: em2,
            tenant_id: None,
            tenant_role: None,
        },
    );

    let code1 = code.clone();
    let code2 = code.clone();
    let (r1, r2) = tokio::join!(
        fire(
            &app1,
            "POST",
            "/v1/codes/redeem",
            Some(serde_json::json!({ "code": code1 }))
        ),
        fire(
            &app2,
            "POST",
            "/v1/codes/redeem",
            Some(serde_json::json!({ "code": code2 }))
        ),
    );

    let statuses = (r1.0.as_u16(), r2.0.as_u16());
    assert!(
        statuses == (200, 400) || statuses == (400, 200),
        "expected exactly one winner, got {:?}",
        statuses
    );
}

#[tokio::test]
async fn revoke_code_rejects_an_id_from_a_sibling_course() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, firebase_uid, email) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let authorized_course = course_for(&pool, tenant, teacher).await;
    let sibling_course = course_for(&pool, tenant, teacher).await;
    let app = build_test_app(
        backend::handlers::enrollments::router_for_tests(pool.clone()),
        StubAuth {
            pool: pool.clone(),
            user_id: teacher,
            firebase_uid,
            email,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Teacher),
        },
    );

    let (status, created) = fire(
        &app,
        "POST",
        &format!("/v1/courses/{sibling_course}/codes"),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, 200, "{created}");
    let code_id = created["id"].as_str().unwrap();
    let (status, _) = fire(
        &app,
        "DELETE",
        &format!("/v1/courses/{authorized_course}/codes/{code_id}"),
        None,
    )
    .await;
    assert_eq!(status, 404);

    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    let expires_at: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT expires_at FROM enrollment_codes WHERE id = $1")
            .bind(code_id.parse::<uuid::Uuid>().unwrap())
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    tx.commit().await.unwrap();
    assert!(expires_at.is_none());
}
