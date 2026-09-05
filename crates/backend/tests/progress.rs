//! Integration tests for lesson progress (learning-suite Cycle 2):
//! mark/unmark completion, per-course progress with resume point, the
//! teacher's per-student rollup, and role/tenant authorization.

mod fixtures;

use fixtures::*;

async fn published_course_with_lessons(
    pool: &sqlx::PgPool,
    tenant: uuid::Uuid,
    owner: uuid::Uuid,
) -> (uuid::Uuid, uuid::Uuid, Vec<uuid::Uuid>) {
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    let course: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO courses (tenant_id, slug, title, status, owner_user_id)
         VALUES ($1, $2, 'Progress C', 'published', $3) RETURNING id",
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
         VALUES ($1, $2, 'M1', 10) RETURNING id",
    )
    .bind(tenant)
    .bind(course)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    let mut lessons = Vec::new();
    for (i, title) in ["L1", "L2", "L3"].iter().enumerate() {
        let lesson: uuid::Uuid = sqlx::query_scalar(
            "INSERT INTO lessons (tenant_id, course_id, module_id, type, title, sort_order)
             VALUES ($1, $2, $3, 'rich_text', $4, $5) RETURNING id",
        )
        .bind(tenant)
        .bind(course)
        .bind(module)
        .bind(title)
        .bind((i as i32 + 1) * 10)
        .fetch_one(&mut *tx)
        .await
        .unwrap();
        lessons.push(lesson);
    }
    tx.commit().await.unwrap();
    (course, module, lessons)
}

async fn enroll_student(
    pool: &sqlx::PgPool,
    tenant: uuid::Uuid,
    course: uuid::Uuid,
    user: uuid::Uuid,
) {
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
async fn student_marks_and_unmarks_completion_with_progress_rollup() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _tfb, _tem) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (course, _module, lessons) = published_course_with_lessons(&pool, tenant, teacher).await;

    let (student, sfb, sem) = create_user(&pool).await;
    attach_membership(&pool, tenant, student, "student").await;
    enroll_student(&pool, tenant, course, student).await;

    let app = build_test_app(
        backend::handlers::progress::router_for_tests(pool.clone()),
        student_stub(&pool, tenant, student, sfb, sem),
    );

    // Mark the first lesson complete.
    let (s, b) = fire(
        &app,
        "PUT",
        &format!("/v1/courses/{course}/lessons/{}/completion", lessons[0]),
        None,
    )
    .await;
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["completed"], true);

    // Progress reflects 1/3 and resumes at L2 (module/lesson order).
    let (s, b) = fire(&app, "GET", &format!("/v1/courses/{course}/progress"), None).await;
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["completed"].as_i64().unwrap(), 1);
    assert_eq!(b["total"].as_i64().unwrap(), 3);
    assert_eq!(
        b["resume_lesson_id"].as_str().unwrap(),
        lessons[1].to_string()
    );
    assert_eq!(b["resume_lesson_title"].as_str().unwrap(), "L2");

    // /v1/me/progress lists the course with the same counts.
    let (s, b) = fire(&app, "GET", "/v1/me/progress", None).await;
    assert_eq!(s, 200, "{b}");
    let rows = b.as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["completed"].as_i64().unwrap(), 1);
    assert_eq!(rows[0]["total"].as_i64().unwrap(), 3);

    // Unmark → back to 0 and resume at L1.
    let (s, _b) = fire(
        &app,
        "DELETE",
        &format!("/v1/courses/{course}/lessons/{}/completion", lessons[0]),
        None,
    )
    .await;
    assert_eq!(s, 200);
    let (_s, b) = fire(&app, "GET", &format!("/v1/courses/{course}/progress"), None).await;
    assert_eq!(b["completed"].as_i64().unwrap(), 0);
    assert_eq!(b["resume_lesson_title"].as_str().unwrap(), "L1");
}

#[tokio::test]
async fn non_member_cannot_write_or_read_progress() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _tfb, _tem) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (course, _module, lessons) = published_course_with_lessons(&pool, tenant, teacher).await;

    // A student in the SAME tenant but NOT enrolled in the course.
    let (outsider, ofb, oem) = create_user(&pool).await;
    attach_membership(&pool, tenant, outsider, "student").await;

    let app = build_test_app(
        backend::handlers::progress::router_for_tests(pool.clone()),
        student_stub(&pool, tenant, outsider, ofb, oem),
    );

    let (s, _b) = fire(
        &app,
        "PUT",
        &format!("/v1/courses/{course}/lessons/{}/completion", lessons[0]),
        None,
    )
    .await;
    assert_eq!(s, 403);
    let (s, _b) = fire(&app, "GET", &format!("/v1/courses/{course}/progress"), None).await;
    assert_eq!(s, 403);
}

#[tokio::test]
async fn lesson_from_another_course_is_rejected() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _tfb, _tem) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (course_a, _m, _lessons_a) = published_course_with_lessons(&pool, tenant, teacher).await;
    let (_course_b, _m2, lessons_b) = published_course_with_lessons(&pool, tenant, teacher).await;

    let (student, sfb, sem) = create_user(&pool).await;
    attach_membership(&pool, tenant, student, "student").await;
    enroll_student(&pool, tenant, course_a, student).await;

    let app = build_test_app(
        backend::handlers::progress::router_for_tests(pool.clone()),
        student_stub(&pool, tenant, student, sfb, sem),
    );

    // Lesson id belongs to course B; the course A path must 404.
    let (s, _b) = fire(
        &app,
        "PUT",
        &format!("/v1/courses/{course_a}/lessons/{}/completion", lessons_b[0]),
        None,
    )
    .await;
    assert_eq!(s, 404);
}

#[tokio::test]
async fn teacher_sees_per_student_progress_and_students_do_not() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, tfb, tem) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (course, _module, lessons) = published_course_with_lessons(&pool, tenant, teacher).await;

    let (student, sfb, sem) = create_user(&pool).await;
    attach_membership(&pool, tenant, student, "student").await;
    enroll_student(&pool, tenant, course, student).await;

    // Student completes two lessons.
    let student_app = build_test_app(
        backend::handlers::progress::router_for_tests(pool.clone()),
        student_stub(&pool, tenant, student, sfb, sem),
    );
    for lesson in &lessons[..2] {
        let (s, _b) = fire(
            &student_app,
            "PUT",
            &format!("/v1/courses/{course}/lessons/{lesson}/completion"),
            None,
        )
        .await;
        assert_eq!(s, 200);
    }

    // Teacher rollup shows 2/3 for the student.
    let teacher_app = build_test_app(
        backend::handlers::progress::router_for_tests(pool.clone()),
        StubAuth {
            pool: pool.clone(),
            user_id: teacher,
            firebase_uid: tfb,
            email: tem,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Teacher),
        },
    );
    let (s, _b) = fire(
        &teacher_app,
        "PUT",
        &format!("/v1/courses/{course}/lessons/{}/completion", lessons[2]),
        None,
    )
    .await;
    assert_eq!(s, 403, "staff must not create learner progress");

    let (s, b) = fire(
        &teacher_app,
        "GET",
        &format!("/v1/courses/{course}/progress/students"),
        None,
    )
    .await;
    assert_eq!(s, 200, "{b}");
    let rows = b.as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["completed"].as_i64().unwrap(), 2);
    assert_eq!(rows[0]["total"].as_i64().unwrap(), 3);

    // The student must not see the staff rollup.
    let (s, _b) = fire(
        &student_app,
        "GET",
        &format!("/v1/courses/{course}/progress/students"),
        None,
    )
    .await;
    assert_eq!(s, 403);
}
