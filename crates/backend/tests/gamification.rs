//! Integration tests for gamification (learning-suite Cycle 4): idempotent
//! awards through the lesson-completion flow, unlock surfacing + seen
//! marking, leaderboard ordering with opt-out, and tenant scoping.

mod fixtures;

use fixtures::*;

async fn published_course_with_lesson(
    pool: &sqlx::PgPool,
    tenant: uuid::Uuid,
    owner: uuid::Uuid,
) -> (uuid::Uuid, uuid::Uuid) {
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    let course: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO courses (tenant_id, slug, title, status, owner_user_id)
         VALUES ($1, $2, 'XP C', 'published', $3) RETURNING id",
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
    .bind(course)
    .bind(owner)
    .bind(tenant)
    .execute(&mut *tx)
    .await
    .unwrap();
    let module: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO modules (tenant_id, course_id, title, sort_order)
         VALUES ($1, $2, 'M', 10) RETURNING id",
    )
    .bind(tenant)
    .bind(course)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    let lesson: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO lessons (tenant_id, course_id, module_id, type, title, sort_order)
         VALUES ($1, $2, $3, 'rich_text', 'L', 10) RETURNING id",
    )
    .bind(tenant)
    .bind(course)
    .bind(module)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    (course, lesson)
}

async fn enroll(pool: &sqlx::PgPool, tenant: uuid::Uuid, course: uuid::Uuid, user: uuid::Uuid) {
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO course_memberships (course_id, user_id, tenant_id, role)
         VALUES ($1, $2, $3, 'student')",
    )
    .bind(course)
    .bind(user)
    .bind(tenant)
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
}

fn student_stub(
    pool: &sqlx::PgPool,
    tenant: uuid::Uuid,
    user: uuid::Uuid,
    fb: String,
    em: String,
) -> StubAuth {
    StubAuth {
        pool: pool.clone(),
        user_id: user,
        firebase_uid: fb,
        email: em,
        tenant_id: Some(tenant),
        tenant_role: Some(core_types::TenantRole::Student),
    }
}

#[tokio::test]
async fn lesson_completion_awards_xp_once_and_unlocks_first_lesson() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _f, _e) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (course, lesson) = published_course_with_lesson(&pool, tenant, teacher).await;
    let (student, sfb, sem) = create_user(&pool).await;
    attach_membership(&pool, tenant, student, "student").await;
    enroll(&pool, tenant, course, student).await;

    let progress_app = build_test_app(
        backend::handlers::progress::router_for_tests(pool.clone()),
        student_stub(&pool, tenant, student, sfb.clone(), sem.clone()),
    );
    let gam_app = build_test_app(
        backend::handlers::gamification::router_for_tests(pool.clone()),
        student_stub(&pool, tenant, student, sfb, sem),
    );

    // Mark complete → 10 XP + the first_lesson unlock, unseen.
    let (s, _b) = fire(
        &progress_app,
        "PUT",
        &format!("/v1/courses/{course}/lessons/{lesson}/completion"),
        None,
    )
    .await;
    assert_eq!(s, 200);

    let (s, b) = fire(&gam_app, "GET", "/v1/me/gamification", None).await;
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["total_xp"].as_i64().unwrap(), 10);
    assert_eq!(b["level"].as_i64().unwrap(), 1);
    assert_eq!(b["current_streak_days"].as_i64().unwrap(), 1);
    assert_eq!(b["streak_active_today"], true);
    let unlocks = b["unlocks"].as_array().unwrap();
    assert_eq!(unlocks.len(), 1);
    assert_eq!(unlocks[0]["id"], "first_lesson");
    assert_eq!(unlocks[0]["seen"], false);

    // Unmark + remark must NOT double-award (dedup key).
    let (_s, _b) = fire(
        &progress_app,
        "DELETE",
        &format!("/v1/courses/{course}/lessons/{lesson}/completion"),
        None,
    )
    .await;
    let (_s, _b) = fire(
        &progress_app,
        "PUT",
        &format!("/v1/courses/{course}/lessons/{lesson}/completion"),
        None,
    )
    .await;
    let (_s, b) = fire(&gam_app, "GET", "/v1/me/gamification", None).await;
    assert_eq!(b["total_xp"].as_i64().unwrap(), 10, "double award: {b}");

    // Mark celebrations seen.
    let (s, _b) = fire(&gam_app, "POST", "/v1/me/gamification/seen", None).await;
    assert_eq!(s, 200);
    let (_s, b) = fire(&gam_app, "GET", "/v1/me/gamification", None).await;
    assert_eq!(b["unlocks"][0]["seen"], true);
}

#[tokio::test]
async fn parent_sees_linked_child_gamification_and_403s_on_unlinked() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _f, _e) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (course, lesson) = published_course_with_lesson(&pool, tenant, teacher).await;
    let (student, sfb, sem) = create_user(&pool).await;
    attach_membership(&pool, tenant, student, "student").await;
    enroll(&pool, tenant, course, student).await;

    // Student earns 10 XP via lesson completion.
    let progress_app = build_test_app(
        backend::handlers::progress::router_for_tests(pool.clone()),
        student_stub(&pool, tenant, student, sfb, sem),
    );
    let (s, _b) = fire(
        &progress_app,
        "PUT",
        &format!("/v1/courses/{course}/lessons/{lesson}/completion"),
        None,
    )
    .await;
    assert_eq!(s, 200);

    // Link a parent to the student (invitation + JIT acceptance path).
    let (admin, _afb, _aem) = create_user(&pool).await;
    attach_membership(&pool, tenant, admin, "org_admin").await;
    let parent_email = format!("parent-{}@example.test", uuid::Uuid::new_v4());
    backend::db::parent::create_invitation(&pool, tenant, &parent_email, student, None, admin)
        .await
        .unwrap();
    let (parent, _pfb, _pem) = create_user(&pool).await;
    sqlx::query("UPDATE users SET email = $1 WHERE id = $2")
        .bind(&parent_email)
        .bind(parent)
        .execute(&pool)
        .await
        .unwrap();
    {
        let mut tx = pool.begin().await.unwrap();
        sqlx::query("SELECT set_config('app.user_id', $1, true)")
            .bind(parent.to_string())
            .execute(&mut *tx)
            .await
            .unwrap();
        let n = backend::db::parent::accept_pending_for_email(&mut tx, parent, &parent_email)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        assert_eq!(n.accepted, 1);
        assert_eq!(n.blocked_tenants, 0);
    }

    let parent_app = build_test_app(
        backend::handlers::parent::router_for_tests(
            pool.clone(),
            std::sync::Arc::new(backend::services::invitations::mock::MockEmailLinkSender::new()),
            "http://localhost:3000".into(),
        ),
        StubAuth {
            pool: pool.clone(),
            user_id: parent,
            firebase_uid: format!("fb-{}", uuid::Uuid::new_v4()),
            email: parent_email,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Parent),
        },
    );

    let (s, b) = fire(
        &parent_app,
        "GET",
        &format!("/v1/parent/children/{student}/gamification"),
        None,
    )
    .await;
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["total_xp"].as_i64().unwrap(), 10);
    assert_eq!(b["level"].as_i64().unwrap(), 1);
    assert_eq!(b["current_streak_days"].as_i64().unwrap(), 1);
    assert_eq!(b["streak_active_today"], true);

    // An UNLINKED student must be Forbidden.
    let (other, _of, _oe) = create_user(&pool).await;
    attach_membership(&pool, tenant, other, "student").await;
    let (s, _b) = fire(
        &parent_app,
        "GET",
        &format!("/v1/parent/children/{other}/gamification"),
        None,
    )
    .await;
    assert_eq!(s, 403);
}

#[tokio::test]
async fn leaderboard_orders_by_course_xp_and_honors_opt_out() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _f, _e) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (course, lesson) = published_course_with_lesson(&pool, tenant, teacher).await;

    let (alice, afb, aem) = create_user(&pool).await;
    attach_membership(&pool, tenant, alice, "student").await;
    enroll(&pool, tenant, course, alice).await;
    let (bob, bfb, bem) = create_user(&pool).await;
    attach_membership(&pool, tenant, bob, "student").await;
    enroll(&pool, tenant, course, bob).await;

    // Alice completes the lesson (10 course XP); Bob does nothing.
    let alice_progress = build_test_app(
        backend::handlers::progress::router_for_tests(pool.clone()),
        student_stub(&pool, tenant, alice, afb.clone(), aem.clone()),
    );
    let (s, _b) = fire(
        &alice_progress,
        "PUT",
        &format!("/v1/courses/{course}/lessons/{lesson}/completion"),
        None,
    )
    .await;
    assert_eq!(s, 200);

    let bob_gam = build_test_app(
        backend::handlers::gamification::router_for_tests(pool.clone()),
        student_stub(&pool, tenant, bob, bfb, bem),
    );
    let (s, b) = fire(
        &bob_gam,
        "GET",
        &format!("/v1/courses/{course}/leaderboard"),
        None,
    )
    .await;
    assert_eq!(s, 200, "{b}");
    let rows = b.as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["course_xp"].as_i64().unwrap(), 10);
    assert_eq!(rows[1]["course_xp"].as_i64().unwrap(), 0);
    assert_eq!(rows[1]["is_me"], true);

    // Alice opts out → drops off the board.
    let alice_gam = build_test_app(
        backend::handlers::gamification::router_for_tests(pool.clone()),
        student_stub(&pool, tenant, alice, afb, aem),
    );
    let (s, _b) = fire(
        &alice_gam,
        "PUT",
        "/v1/me/leaderboard-opt-out",
        Some(serde_json::json!({"opted_out": true})),
    )
    .await;
    assert_eq!(s, 200);
    let (_s, b) = fire(
        &bob_gam,
        "GET",
        &format!("/v1/courses/{course}/leaderboard"),
        None,
    )
    .await;
    let rows = b.as_array().unwrap();
    assert_eq!(rows.len(), 1, "opted-out learner still listed: {b}");
}
