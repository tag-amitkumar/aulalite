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

#[tokio::test]
async fn invite_then_accept_email_link() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, fb_t, em_t) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;

    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    let course: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO courses (tenant_id, slug, title, owner_user_id)
         VALUES ($1, $2, 'C', $3) RETURNING id",
    )
    .bind(tenant)
    .bind(format!("c-{}", uuid::Uuid::new_v4()))
    .bind(teacher)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO course_memberships (course_id, user_id, tenant_id, role)
         VALUES ($1,$2,$3,'teacher')",
    )
    .bind(course)
    .bind(teacher)
    .bind(tenant)
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let mock =
        std::sync::Arc::new(backend::services::invitations::mock::MockEmailLinkSender::new());
    let teacher_app = build_test_app(
        backend::handlers::enrollments::invitation_router_for_tests(
            pool.clone(),
            mock.clone(),
            "http://localhost:3000".into(),
        ),
        StubAuth {
            pool: pool.clone(),
            user_id: teacher,
            firebase_uid: fb_t,
            email: em_t,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Teacher),
        },
    );

    let invite_email = format!("newbie-{}@example.test", uuid::Uuid::new_v4());

    let (status, body) = fire(
        &teacher_app,
        "POST",
        &format!("/v1/courses/{course}/invitations"),
        Some(json!({ "email": invite_email, "role": "student" })),
    )
    .await;
    assert_eq!(status, 200, "{body}");

    let calls = mock.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, invite_email);
    assert!(calls[0]
        .1
        .starts_with("http://localhost:3000/accept-invite/"));

    let token: String = calls[0].1.rsplit('/').next().unwrap().to_string();

    let (newbie, fb_n, _) = create_user(&pool).await;
    sqlx::query("UPDATE users SET email = $1 WHERE id = $2")
        .bind(&invite_email)
        .bind(newbie)
        .execute(&pool)
        .await
        .unwrap();

    let newbie_app = build_test_app(
        backend::handlers::enrollments::invitation_router_for_tests(
            pool.clone(),
            std::sync::Arc::new(backend::services::invitations::mock::MockEmailLinkSender::new()),
            "http://localhost:3000".into(),
        ),
        StubAuth {
            pool: pool.clone(),
            user_id: newbie,
            firebase_uid: fb_n,
            email: invite_email.clone(),
            tenant_id: None,
            tenant_role: None,
        },
    );
    let (status, body) = fire(
        &newbie_app,
        "POST",
        &format!("/v1/invitations/{token}/accept"),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["course_id"].as_str().unwrap(), course.to_string());
}

#[tokio::test]
async fn accept_rejects_when_email_mismatch() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;

    let real_email = format!("real-{}@example.test", uuid::Uuid::new_v4());
    let other_email = format!("other-{}@example.test", uuid::Uuid::new_v4());

    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    let course: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO courses (tenant_id, slug, title, owner_user_id)
         VALUES ($1,$2,'C2',$3) RETURNING id",
    )
    .bind(tenant)
    .bind(format!("c2-{}", uuid::Uuid::new_v4()))
    .bind(teacher)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    let token = format!("tok-{}", uuid::Uuid::new_v4());
    sqlx::query(
        "INSERT INTO course_invitations
            (tenant_id, course_id, email, role, token, expires_at, created_by)
         VALUES ($1,$2,$3,'student',$4, now()+interval '14 days', $5)",
    )
    .bind(tenant)
    .bind(course)
    .bind(&real_email)
    .bind(&token)
    .bind(teacher)
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let (other_user, fb, _) = create_user(&pool).await;
    sqlx::query("UPDATE users SET email = $1 WHERE id = $2")
        .bind(&other_email)
        .bind(other_user)
        .execute(&pool)
        .await
        .unwrap();

    let mock =
        std::sync::Arc::new(backend::services::invitations::mock::MockEmailLinkSender::new());
    let app = build_test_app(
        backend::handlers::enrollments::invitation_router_for_tests(
            pool.clone(),
            mock,
            "http://localhost:3000".into(),
        ),
        StubAuth {
            pool: pool.clone(),
            user_id: other_user,
            firebase_uid: fb,
            email: other_email,
            tenant_id: None,
            tenant_role: None,
        },
    );
    let (status, _) = fire(
        &app,
        "POST",
        &format!("/v1/invitations/{token}/accept"),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, 400);
}

#[tokio::test]
async fn acceptance_respects_seat_cap_and_keeps_invitation_pending() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, fb_t, em_t) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    seed_blocking_seat_cap(&pool, tenant, 1).await;

    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    let course: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO courses (tenant_id, slug, title, owner_user_id)
         VALUES ($1, $2, 'Seat capped course', $3) RETURNING id",
    )
    .bind(tenant)
    .bind(format!("seat-course-{}", uuid::Uuid::new_v4()))
    .bind(teacher)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO course_memberships (course_id, user_id, tenant_id, role)
         VALUES ($1, $2, $3, 'teacher')",
    )
    .bind(course)
    .bind(teacher)
    .bind(tenant)
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let mock =
        std::sync::Arc::new(backend::services::invitations::mock::MockEmailLinkSender::new());
    let teacher_app = build_test_app(
        backend::handlers::enrollments::invitation_router_for_tests(
            pool.clone(),
            mock.clone(),
            "http://localhost:3000".into(),
        ),
        StubAuth {
            pool: pool.clone(),
            user_id: teacher,
            firebase_uid: fb_t,
            email: em_t,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Teacher),
        },
    );
    let invite_email = format!("seat-newbie-{}@example.test", uuid::Uuid::new_v4());
    let (status, body) = fire(
        &teacher_app,
        "POST",
        &format!("/v1/courses/{course}/invitations"),
        Some(json!({ "email": invite_email, "role": "student" })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let invitation_id = body["invitation_id"].as_str().unwrap();
    let token = mock.calls()[0].1.rsplit('/').next().unwrap().to_string();

    let (student, fb_s, _) = create_user(&pool).await;
    sqlx::query("UPDATE users SET email = $1 WHERE id = $2")
        .bind(&invite_email)
        .bind(student)
        .execute(&pool)
        .await
        .unwrap();
    let student_app = build_test_app(
        backend::handlers::enrollments::invitation_router_for_tests(
            pool.clone(),
            std::sync::Arc::new(backend::services::invitations::mock::MockEmailLinkSender::new()),
            "http://localhost:3000".into(),
        ),
        StubAuth {
            pool: pool.clone(),
            user_id: student,
            firebase_uid: fb_s,
            email: invite_email,
            tenant_id: None,
            tenant_role: None,
        },
    );

    let (status, body) = fire(
        &student_app,
        "POST",
        &format!("/v1/invitations/{token}/accept"),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, 409, "{body}");
    assert_eq!(body["error"], "conflict: seat_limit_reached");

    let invitation_status: String =
        sqlx::query_scalar("SELECT status FROM course_invitations WHERE id = $1")
            .bind(invitation_id.parse::<uuid::Uuid>().unwrap())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(invitation_status, "pending");
    let course_membership_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM course_memberships WHERE course_id = $1 AND user_id = $2",
    )
    .bind(course)
    .bind(student)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(course_membership_count, 0);
}

#[tokio::test]
async fn revoke_invitation_rejects_an_id_from_a_sibling_course() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, firebase_uid, email) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;

    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    let authorized_course: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO courses (tenant_id, slug, title, owner_user_id)
         VALUES ($1, $2, 'Authorized', $3) RETURNING id",
    )
    .bind(tenant)
    .bind(format!("authorized-{}", uuid::Uuid::new_v4()))
    .bind(teacher)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    let sibling_course: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO courses (tenant_id, slug, title, owner_user_id)
         VALUES ($1, $2, 'Sibling', $3) RETURNING id",
    )
    .bind(tenant)
    .bind(format!("sibling-{}", uuid::Uuid::new_v4()))
    .bind(teacher)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    for course in [authorized_course, sibling_course] {
        sqlx::query(
            "INSERT INTO course_memberships (course_id, user_id, tenant_id, role)
             VALUES ($1, $2, $3, 'teacher')",
        )
        .bind(course)
        .bind(teacher)
        .bind(tenant)
        .execute(&mut *tx)
        .await
        .unwrap();
    }
    tx.commit().await.unwrap();

    let app = build_test_app(
        backend::handlers::enrollments::invitation_router_for_tests(
            pool.clone(),
            std::sync::Arc::new(backend::services::invitations::mock::MockEmailLinkSender::new()),
            "http://localhost:3000".into(),
        ),
        StubAuth {
            pool: pool.clone(),
            user_id: teacher,
            firebase_uid,
            email,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Teacher),
        },
    );
    let invite_email = format!("nested-{}@example.test", uuid::Uuid::new_v4());
    let (status, created) = fire(
        &app,
        "POST",
        &format!("/v1/courses/{sibling_course}/invitations"),
        Some(json!({ "email": invite_email, "role": "student" })),
    )
    .await;
    assert_eq!(status, 200, "{created}");
    let invitation_id = created["invitation_id"].as_str().unwrap();
    let (status, _) = fire(
        &app,
        "DELETE",
        &format!("/v1/courses/{authorized_course}/invitations/{invitation_id}"),
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
    let status: String = sqlx::query_scalar("SELECT status FROM course_invitations WHERE id = $1")
        .bind(invitation_id.parse::<uuid::Uuid>().unwrap())
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(status, "pending");
}
