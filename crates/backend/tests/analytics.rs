// crates/backend/tests/analytics.rs
//
// Integration tests for the read-only analytics aggregation layer:
//   * GET /v1/analytics/overview        (org-admin only)
//   * GET /v1/courses/:cid/analytics    (course-staff only)
//
// These require a live Postgres (DATABASE_URL) and so do not run in CI
// sandboxes without one; they are written to compile and be logically correct
// against the real schema.
mod fixtures;

use fixtures::*;
use uuid::Uuid;

/// Create a published course owned by `teacher`, with the owner teacher row in
/// course_memberships (mirrors handlers::courses create path).
async fn course(pool: &sqlx::PgPool, tenant: Uuid, teacher: Uuid) -> Uuid {
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    let course: Uuid = sqlx::query_scalar(
        "INSERT INTO courses (tenant_id, slug, title, owner_user_id, status)
         VALUES ($1, $2, 'C', $3, 'published') RETURNING id",
    )
    .bind(tenant)
    .bind(format!("c-{}", Uuid::new_v4()))
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
    course
}

/// Enroll `student` as an active course member with role='student'.
async fn enroll_student(pool: &sqlx::PgPool, tenant: Uuid, course_id: Uuid, student: Uuid) {
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO course_memberships (course_id, user_id, tenant_id, role)
         VALUES ($1, $2, $3, 'student')",
    )
    .bind(course_id)
    .bind(student)
    .bind(tenant)
    .execute(pool)
    .await
    .unwrap();
}

/// Create a scheduled live session (with its required series) for `course`.
async fn session(
    pool: &sqlx::PgPool,
    tenant: Uuid,
    course_id: Uuid,
    teacher: Uuid,
    status: &str,
    starts_at: chrono::DateTime<chrono::Utc>,
) -> Uuid {
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    let series: Uuid = sqlx::query_scalar(
        "INSERT INTO live_session_series
            (tenant_id, course_id, title, starts_at, duration_minutes,
             frequency, end_kind, primary_teacher_id)
         VALUES ($1, $2, 'S', $3, 60, 'none', 'open', $4)
         RETURNING id",
    )
    .bind(tenant)
    .bind(course_id)
    .bind(starts_at)
    .bind(teacher)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    let session: Uuid = sqlx::query_scalar(
        "INSERT INTO live_sessions (tenant_id, course_id, series_id, occurrence_index,
                                    title, status, starts_at, duration_minutes,
                                    primary_teacher_id, mode, recording_enabled,
                                    transport_mode)
         VALUES ($1, $2, $3, 0, 'L', $4, $5, 60, $6, 'lecture', false, 'webrtc')
         RETURNING id",
    )
    .bind(tenant)
    .bind(course_id)
    .bind(series)
    .bind(status)
    .bind(starts_at)
    .bind(teacher)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    session
}

/// Insert a published numeric assignment for the course.
async fn published_assignment(
    pool: &sqlx::PgPool,
    tenant: Uuid,
    course_id: Uuid,
    teacher: Uuid,
) -> Uuid {
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(pool)
        .await
        .unwrap();
    sqlx::query_scalar(
        "INSERT INTO assignments
            (tenant_id, course_id, title, grading_mode, max_points,
             status, published_at, created_by)
         VALUES ($1, $2, 'Essay', 'numeric', 100, 'published', now(), $3)
         RETURNING id",
    )
    .bind(tenant)
    .bind(course_id)
    .bind(teacher)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// Insert a graded submission for (assignment, student) with a numeric grade.
async fn graded_submission(
    pool: &sqlx::PgPool,
    tenant: Uuid,
    assignment_id: Uuid,
    course_id: Uuid,
    student: Uuid,
    grade: f64,
) -> Uuid {
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(pool)
        .await
        .unwrap();
    sqlx::query_scalar(
        "INSERT INTO submissions
            (tenant_id, assignment_id, course_id, student_user_id,
             status, submitted_at, numeric_grade, graded_at)
         VALUES ($1, $2, $3, $4, 'graded', now(), $5, now())
         RETURNING id",
    )
    .bind(tenant)
    .bind(assignment_id)
    .bind(course_id)
    .bind(student)
    .bind(grade)
    .fetch_one(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn overview_returns_sane_counts_for_org_admin() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (admin, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, admin, "org_admin").await;
    let (teacher, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (student, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, student, "student").await;

    let cid = course(&pool, tenant, teacher).await;
    enroll_student(&pool, tenant, cid, student).await;
    let _ended = session(
        &pool,
        tenant,
        cid,
        teacher,
        "ended",
        chrono::Utc::now() - chrono::Duration::days(1),
    )
    .await;
    let aid = published_assignment(&pool, tenant, cid, teacher).await;
    let _sub = graded_submission(&pool, tenant, aid, cid, student, 90.0).await;

    let app = build_test_app(
        backend::handlers::analytics::router_for_tests(pool.clone()),
        StubAuth {
            pool: pool.clone(),
            user_id: admin,
            firebase_uid: fb,
            email: em,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::OrgAdmin),
        },
    );

    let (s, body) = fire(&app, "GET", "/v1/analytics/overview", None).await;
    assert_eq!(s, 200, "{body}");
    assert_eq!(body["courses_total"], 1);
    assert_eq!(body["courses_published"], 1);
    assert_eq!(body["members_students"], 1);
    assert_eq!(body["members_teachers"], 1);
    assert_eq!(body["sessions_ended_total"], 1);
    assert_eq!(body["assignments_total"], 1);
    assert_eq!(body["submissions_total"], 1);
    assert_eq!(body["submissions_graded"], 1);
}

#[tokio::test]
async fn activity_returns_zero_filled_days_for_org_admin_and_403s_students() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (admin, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, admin, "org_admin").await;

    let app = build_test_app(
        backend::handlers::analytics::router_for_tests(pool.clone()),
        StubAuth {
            pool: pool.clone(),
            user_id: admin,
            firebase_uid: fb,
            email: em,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::OrgAdmin),
        },
    );

    // Default window: 30 zero-filled rows for a fresh tenant, oldest first.
    let (s, body) = fire(&app, "GET", "/v1/analytics/activity", None).await;
    assert_eq!(s, 200, "{body}");
    let rows = body.as_array().unwrap();
    assert_eq!(rows.len(), 30);
    assert_eq!(rows[0]["sessions"], 0);
    assert_eq!(rows[29]["lessons_completed"], 0);
    assert!(rows[0]["day"].as_str().unwrap() < rows[29]["day"].as_str().unwrap());

    // Window is validated.
    let (s, _b) = fire(&app, "GET", "/v1/analytics/activity?days=7", None).await;
    assert_eq!(s, 200);
    let (s, _b) = fire(&app, "GET", "/v1/analytics/activity?days=0", None).await;
    assert_eq!(s, 400);
    let (s, _b) = fire(&app, "GET", "/v1/analytics/activity?days=365", None).await;
    assert_eq!(s, 400);

    // Students are forbidden.
    let (student, sfb, sem) = create_user(&pool).await;
    attach_membership(&pool, tenant, student, "student").await;
    let student_app = build_test_app(
        backend::handlers::analytics::router_for_tests(pool.clone()),
        StubAuth {
            pool: pool.clone(),
            user_id: student,
            firebase_uid: sfb,
            email: sem,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Student),
        },
    );
    let (s, _) = fire(&student_app, "GET", "/v1/analytics/activity", None).await;
    assert_eq!(s, 403);
}

#[tokio::test]
async fn overview_forbidden_for_student() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (student, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, student, "student").await;

    let app = build_test_app(
        backend::handlers::analytics::router_for_tests(pool.clone()),
        StubAuth {
            pool: pool.clone(),
            user_id: student,
            firebase_uid: fb,
            email: em,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Student),
        },
    );

    let (s, _) = fire(&app, "GET", "/v1/analytics/overview", None).await;
    assert_eq!(s, 403);
}

#[tokio::test]
async fn course_analytics_returns_enrolled_and_progress_for_staff() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (student, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, student, "student").await;

    let cid = course(&pool, tenant, teacher).await;
    enroll_student(&pool, tenant, cid, student).await;
    let _ended = session(
        &pool,
        tenant,
        cid,
        teacher,
        "ended",
        chrono::Utc::now() - chrono::Duration::days(1),
    )
    .await;
    let aid = published_assignment(&pool, tenant, cid, teacher).await;
    let _sub = graded_submission(&pool, tenant, aid, cid, student, 80.0).await;

    let app = build_test_app(
        backend::handlers::analytics::router_for_tests(pool.clone()),
        StubAuth {
            pool: pool.clone(),
            user_id: teacher,
            firebase_uid: fb,
            email: em,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Teacher),
        },
    );

    let (s, body) = fire(&app, "GET", &format!("/v1/courses/{cid}/analytics"), None).await;
    assert_eq!(s, 200, "{body}");
    assert_eq!(body["course_id"], cid.to_string());
    assert_eq!(body["enrolled_students"], 1);
    assert_eq!(body["sessions_total"], 1);
    assert_eq!(body["sessions_ended"], 1);

    let assignments = body["assignments"].as_array().unwrap();
    assert_eq!(assignments.len(), 1, "one assignment expected");
    assert_eq!(assignments[0]["assignment_id"], aid.to_string());
    assert_eq!(assignments[0]["submitted_count"], 1);
    assert_eq!(assignments[0]["graded_count"], 1);
    assert_eq!(assignments[0]["avg_numeric_grade"].as_f64().unwrap(), 80.0);

    // Cycle 7 additions: lesson-progress funnel + graded-quiz distribution.
    // This course has no lessons or quizzes, so the funnel counts enrollment
    // only and the distribution is all-zero.
    assert_eq!(body["funnel"]["enrolled"], 1);
    assert_eq!(body["funnel"]["started"], 0);
    assert_eq!(body["funnel"]["completed"], 0);
    assert_eq!(body["quiz_scores"]["from_90_up"], 0);
}

#[tokio::test]
async fn course_analytics_forbidden_for_non_staff() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let cid = course(&pool, tenant, teacher).await;

    // A plain student enrolled in the course is NOT staff and must be forbidden.
    let (student, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, student, "student").await;
    enroll_student(&pool, tenant, cid, student).await;

    let app = build_test_app(
        backend::handlers::analytics::router_for_tests(pool.clone()),
        StubAuth {
            pool: pool.clone(),
            user_id: student,
            firebase_uid: fb,
            email: em,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Student),
        },
    );

    let (s, _) = fire(&app, "GET", &format!("/v1/courses/{cid}/analytics"), None).await;
    assert_eq!(s, 403);
}
