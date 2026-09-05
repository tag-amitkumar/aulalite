mod fixtures;

use fixtures::*;
use uuid::Uuid;

async fn seed_course(pool: &sqlx::PgPool) -> (Uuid, Uuid, Uuid, Uuid) {
    let tenant = create_tenant(pool).await;
    let (teacher, _, _) = create_user(pool).await;
    let (student, _, _) = create_user(pool).await;
    attach_membership(pool, tenant, teacher, "teacher").await;
    attach_membership(pool, tenant, student, "student").await;

    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();

    let course: Uuid = sqlx::query_scalar(
        "INSERT INTO courses (tenant_id, slug, title, owner_user_id, status)
         VALUES ($1, $2, 'Audit Course', $3, 'published') RETURNING id",
    )
    .bind(tenant)
    .bind(format!("audit-{}", Uuid::new_v4()))
    .bind(teacher)
    .fetch_one(&mut *tx)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO course_memberships (course_id, user_id, tenant_id, role, status)
         VALUES ($1, $2, $3, 'teacher', 'active'), ($1, $4, $3, 'student', 'active')",
    )
    .bind(course)
    .bind(teacher)
    .bind(tenant)
    .bind(student)
    .execute(&mut *tx)
    .await
    .unwrap();

    let module: Uuid = sqlx::query_scalar(
        "INSERT INTO modules (tenant_id, course_id, title, sort_order)
         VALUES ($1, $2, 'Week 1', 10) RETURNING id",
    )
    .bind(tenant)
    .bind(course)
    .fetch_one(&mut *tx)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO lessons (tenant_id, course_id, module_id, type, title, body_md, sort_order)
         VALUES ($1, $2, $3, 'rich_text', 'Welcome', 'Read this first', 10)",
    )
    .bind(tenant)
    .bind(course)
    .bind(module)
    .execute(&mut *tx)
    .await
    .unwrap();

    let series: Uuid = sqlx::query_scalar(
        "INSERT INTO live_session_series
            (tenant_id, course_id, title, starts_at, duration_minutes, frequency,
             end_kind, occurrence_count, primary_teacher_id, recording_enabled)
         VALUES
            ($1, $2, 'Weekly Class', now() + interval '1 day', 60, 'none',
             'count', 1, $3, true)
         RETURNING id",
    )
    .bind(tenant)
    .bind(course)
    .bind(teacher)
    .fetch_one(&mut *tx)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO live_sessions
            (tenant_id, course_id, series_id, occurrence_index, title, starts_at,
             duration_minutes, primary_teacher_id, recording_enabled)
         VALUES ($1, $2, $3, 0, 'Weekly Class', now() + interval '1 day', 60, $4, true)",
    )
    .bind(tenant)
    .bind(course)
    .bind(series)
    .bind(teacher)
    .execute(&mut *tx)
    .await
    .unwrap();

    tx.commit().await.unwrap();
    (tenant, teacher, student, course)
}

fn app_for(
    pool: sqlx::PgPool,
    tenant: Uuid,
    user: Uuid,
    role: core_types::TenantRole,
) -> axum::Router {
    build_test_app(
        backend::handlers::courses::router_for_tests(pool.clone()),
        StubAuth {
            pool,
            user_id: user,
            firebase_uid: format!("fb-{user}"),
            email: format!("{user}@example.test"),
            tenant_id: Some(tenant),
            tenant_role: Some(role),
        },
    )
}

#[test]
fn production_course_and_live_session_routes_can_merge() {
    let _ = axum::Router::<backend::AppState>::new()
        .merge(backend::handlers::courses::routes())
        .merge(backend::handlers::live_sessions::routes());
}

async fn add_removed_course_member(
    pool: &sqlx::PgPool,
    tenant: Uuid,
    course: Uuid,
) -> (Uuid, String) {
    let (removed_user, _, removed_email) = create_user(pool).await;
    attach_membership(pool, tenant, removed_user, "student").await;

    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO course_memberships (course_id, user_id, tenant_id, role, status)
         VALUES ($1, $2, $3, 'student', 'removed')",
    )
    .bind(course)
    .bind(removed_user)
    .bind(tenant)
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();

    (removed_user, removed_email)
}

#[tokio::test]
async fn course_outline_returns_modules_with_lessons_for_member() {
    let pool = pool().await;
    let (tenant, _teacher, student, course) = seed_course(&pool).await;
    let app = app_for(pool, tenant, student, core_types::TenantRole::Student);

    let (status, body) = fire(
        &app,
        "GET",
        &format!("/v1/courses/{course}/modules-with-lessons"),
        None,
    )
    .await;

    assert_eq!(status, 200, "{body}");
    assert_eq!(body[0]["title"], "Week 1");
    assert_eq!(body[0]["lessons"][0]["title"], "Welcome");
    assert_eq!(body[0]["lessons"][0]["type"], "rich_text");
}

#[tokio::test]
async fn course_members_returns_active_people_for_teacher() {
    let pool = pool().await;
    let (tenant, teacher, _student, course) = seed_course(&pool).await;
    let (removed_user, removed_email) = add_removed_course_member(&pool, tenant, course).await;
    let app = app_for(pool, tenant, teacher, core_types::TenantRole::Teacher);

    let (status, body) = fire(&app, "GET", &format!("/v1/courses/{course}/members"), None).await;

    assert_eq!(status, 200, "{body}");
    let members = body.as_array().unwrap();
    assert!(members.iter().all(|m| m["status"] == "active"));
    assert!(members.iter().any(|m| m["role"] == "teacher"));
    assert!(members.iter().any(|m| m["role"] == "student"));
    assert!(!members
        .iter()
        .any(|m| m["id"].as_str() == Some(&removed_user.to_string())));
    assert!(!members
        .iter()
        .any(|m| m["user_id"].as_str() == Some(&removed_user.to_string())));
    assert!(!members
        .iter()
        .any(|m| m["email"].as_str() == Some(removed_email.as_str())));
}

#[tokio::test]
async fn course_sessions_returns_schedule_for_member() {
    let pool = pool().await;
    let (tenant, _teacher, student, course) = seed_course(&pool).await;
    let app = app_for(pool, tenant, student, core_types::TenantRole::Student);

    let (status, body) = fire(&app, "GET", &format!("/v1/courses/{course}/sessions"), None).await;

    assert_eq!(status, 200, "{body}");
    assert_eq!(body[0]["title"], "Weekly Class");
    assert_eq!(body[0]["status"], "scheduled");
    assert_eq!(body[0]["duration_minutes"], 60);
}

#[tokio::test]
async fn org_admin_can_read_all_course_detail_tabs_without_course_membership() {
    let pool = pool().await;
    let (tenant, _teacher, _student, course) = seed_course(&pool).await;
    let (admin, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, admin, "org_admin").await;
    let app = app_for(pool, tenant, admin, core_types::TenantRole::OrgAdmin);

    for path in [
        format!("/v1/courses/{course}/modules-with-lessons"),
        format!("/v1/courses/{course}/sessions"),
        format!("/v1/courses/{course}/members"),
    ] {
        let (status, body) = fire(&app, "GET", &path, None).await;
        assert_eq!(status, 200, "{path} returned {body}");
    }
}

#[tokio::test]
async fn org_admin_cannot_read_course_detail_tabs_from_another_tenant() {
    let pool = pool().await;
    let (_foreign_tenant, _teacher, _student, foreign_course) = seed_course(&pool).await;
    let admin_tenant = create_tenant(&pool).await;
    let (admin, _, _) = create_user(&pool).await;
    attach_membership(&pool, admin_tenant, admin, "org_admin").await;
    let app = app_for(pool, admin_tenant, admin, core_types::TenantRole::OrgAdmin);

    for path in [
        format!("/v1/courses/{foreign_course}/modules-with-lessons"),
        format!("/v1/courses/{foreign_course}/sessions"),
        format!("/v1/courses/{foreign_course}/members"),
    ] {
        let (status, _) = fire(&app, "GET", &path, None).await;
        assert_eq!(status, 404, "{path}");
    }
}

#[tokio::test]
async fn org_admin_cannot_read_missing_course_detail_tabs() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (admin, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, admin, "org_admin").await;
    let missing_course = Uuid::new_v4();
    let app = app_for(pool, tenant, admin, core_types::TenantRole::OrgAdmin);

    for path in [
        format!("/v1/courses/{missing_course}/modules-with-lessons"),
        format!("/v1/courses/{missing_course}/sessions"),
        format!("/v1/courses/{missing_course}/members"),
    ] {
        let (status, _) = fire(&app, "GET", &path, None).await;
        assert_eq!(status, 404, "{path}");
    }
}

#[tokio::test]
async fn non_member_cannot_read_course_outline() {
    let pool = pool().await;
    let (tenant, _teacher, _student, course) = seed_course(&pool).await;
    let (outsider, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, outsider, "student").await;
    let app = app_for(pool, tenant, outsider, core_types::TenantRole::Student);

    let (status, _) = fire(
        &app,
        "GET",
        &format!("/v1/courses/{course}/modules-with-lessons"),
        None,
    )
    .await;

    assert_eq!(status, 404);
}

#[tokio::test]
async fn tenant_student_non_member_cannot_read_course_sessions() {
    let pool = pool().await;
    let (tenant, _teacher, _student, course) = seed_course(&pool).await;
    let (outsider, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, outsider, "student").await;
    let app = app_for(pool, tenant, outsider, core_types::TenantRole::Student);

    let (status, _) = fire(&app, "GET", &format!("/v1/courses/{course}/sessions"), None).await;

    assert_eq!(status, 404);
}

#[tokio::test]
async fn tenant_student_non_member_cannot_read_course_members() {
    let pool = pool().await;
    let (tenant, _teacher, _student, course) = seed_course(&pool).await;
    let (outsider, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, outsider, "student").await;
    let app = app_for(pool, tenant, outsider, core_types::TenantRole::Student);

    let (status, _) = fire(&app, "GET", &format!("/v1/courses/{course}/members"), None).await;

    assert_eq!(status, 403);
}

#[tokio::test]
async fn tenant_teacher_non_member_cannot_read_course_detail_tabs() {
    let pool = pool().await;
    let (tenant, _teacher, _student, course) = seed_course(&pool).await;
    let (outsider, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, outsider, "teacher").await;
    let app = app_for(pool, tenant, outsider, core_types::TenantRole::Teacher);

    let (status, _) = fire(
        &app,
        "GET",
        &format!("/v1/courses/{course}/modules-with-lessons"),
        None,
    )
    .await;
    assert_eq!(status, 404);

    let (status, _) = fire(&app, "GET", &format!("/v1/courses/{course}/sessions"), None).await;
    assert_eq!(status, 404);

    let (status, _) = fire(&app, "GET", &format!("/v1/courses/{course}/members"), None).await;
    assert_eq!(status, 403);
}

#[tokio::test]
async fn student_cannot_read_people_tab_admin_data() {
    let pool = pool().await;
    let (tenant, _teacher, student, course) = seed_course(&pool).await;
    let app = app_for(pool, tenant, student, core_types::TenantRole::Student);

    let (status, _) = fire(&app, "GET", &format!("/v1/courses/{course}/members"), None).await;

    assert_eq!(status, 403);
}
