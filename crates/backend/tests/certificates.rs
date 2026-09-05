//! Integration tests for certificates (learning-suite Cycle 5): automatic
//! eligibility on course completion (lessons + published graded quizzes),
//! teacher-only issue/revoke, the student's own list, and the PUBLIC
//! credential verification lookup.

mod fixtures;

use fixtures::*;
use uuid::Uuid;

async fn published_course_with_lesson(
    pool: &sqlx::PgPool,
    tenant: Uuid,
    owner: Uuid,
) -> (Uuid, Uuid) {
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    let course: Uuid = sqlx::query_scalar(
        "INSERT INTO courses (tenant_id, slug, title, status, owner_user_id)
         VALUES ($1, $2, 'Cert C', 'published', $3) RETURNING id",
    )
    .bind(tenant)
    .bind(format!("c-{}", Uuid::new_v4()))
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
    let module: Uuid = sqlx::query_scalar(
        "INSERT INTO modules (tenant_id, course_id, title, sort_order)
         VALUES ($1, $2, 'M', 10) RETURNING id",
    )
    .bind(tenant)
    .bind(course)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    let lesson: Uuid = sqlx::query_scalar(
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

async fn enroll(pool: &sqlx::PgPool, tenant: Uuid, course: Uuid, user: Uuid) {
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

fn stub(
    pool: &sqlx::PgPool,
    tenant: Uuid,
    user: Uuid,
    fb: String,
    em: String,
    role: core_types::TenantRole,
) -> StubAuth {
    StubAuth {
        pool: pool.clone(),
        user_id: user,
        firebase_uid: fb,
        email: em,
        tenant_id: Some(tenant),
        tenant_role: Some(role),
    }
}

#[tokio::test]
async fn completion_grants_eligibility_then_teacher_issues_and_public_verifies() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, tfb, tem) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (course, lesson) = published_course_with_lesson(&pool, tenant, teacher).await;
    let (student, sfb, sem) = create_user(&pool).await;
    attach_membership(&pool, tenant, student, "student").await;
    enroll(&pool, tenant, course, student).await;

    let student_progress = build_test_app(
        backend::handlers::progress::router_for_tests(pool.clone()),
        stub(
            &pool,
            tenant,
            student,
            sfb.clone(),
            sem.clone(),
            core_types::TenantRole::Student,
        ),
    );
    let student_certs = build_test_app(
        backend::handlers::certificates::router_for_tests(pool.clone()),
        stub(
            &pool,
            tenant,
            student,
            sfb,
            sem,
            core_types::TenantRole::Student,
        ),
    );
    let teacher_certs = build_test_app(
        backend::handlers::certificates::router_for_tests(pool.clone()),
        stub(
            &pool,
            tenant,
            teacher,
            tfb,
            tem,
            core_types::TenantRole::Teacher,
        ),
    );

    // Completing the only lesson completes the course → eligible row.
    let (s, _b) = fire(
        &student_progress,
        "PUT",
        &format!("/v1/courses/{course}/lessons/{lesson}/completion"),
        None,
    )
    .await;
    assert_eq!(s, 200);

    let (s, b) = fire(
        &teacher_certs,
        "GET",
        &format!("/v1/courses/{course}/certificates"),
        None,
    )
    .await;
    assert_eq!(s, 200, "{b}");
    let rows = b.as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["status"], "eligible");
    assert!(rows[0]["credential_id"].is_null());

    // Students cannot issue (not course staff).
    let (s, _b) = fire(
        &student_certs,
        "POST",
        &format!("/v1/courses/{course}/certificates/{student}/issue"),
        None,
    )
    .await;
    assert_eq!(s, 403);

    // Teacher issues: credential minted, snapshots taken.
    let (s, b) = fire(
        &teacher_certs,
        "POST",
        &format!("/v1/courses/{course}/certificates/{student}/issue"),
        None,
    )
    .await;
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["status"], "issued");
    let credential = b["credential_id"].as_str().unwrap().to_string();
    assert!(credential.starts_with("AULA-"), "{credential}");
    assert_eq!(b["course_title"], "Cert C");
    assert!(b["recipient_name"].as_str().is_some());

    // Student sees their issued certificate.
    let (s, b) = fire(&student_certs, "GET", "/v1/me/certificates", None).await;
    assert_eq!(s, 200, "{b}");
    assert_eq!(b.as_array().unwrap().len(), 1);
    assert_eq!(b[0]["status"], "issued");

    // Public verification resolves; unknown ids 404.
    let (s, b) = fire(
        &student_certs,
        "GET",
        &format!("/v1/verify/{credential}"),
        None,
    )
    .await;
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["status"], "issued");
    assert_eq!(b["course_title"], "Cert C");
    let (s, _b) = fire(&student_certs, "GET", "/v1/verify/AULA-0000-0000", None).await;
    assert_eq!(s, 404);

    // Revoke → verify shows revoked (explicitly, not 404).
    let (s, _b) = fire(
        &teacher_certs,
        "POST",
        &format!("/v1/courses/{course}/certificates/{student}/revoke"),
        None,
    )
    .await;
    assert_eq!(s, 204);
    let (s, b) = fire(
        &student_certs,
        "GET",
        &format!("/v1/verify/{credential}"),
        None,
    )
    .await;
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["status"], "revoked");

    // Re-issue keeps the SAME credential id (stable public links).
    let (s, b) = fire(
        &teacher_certs,
        "POST",
        &format!("/v1/courses/{course}/certificates/{student}/issue"),
        None,
    )
    .await;
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["credential_id"].as_str().unwrap(), credential);
}

#[tokio::test]
async fn graded_quiz_gates_eligibility_until_submitted() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _tf, _te) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (course, lesson) = published_course_with_lesson(&pool, tenant, teacher).await;
    let (student, sfb, sem) = create_user(&pool).await;
    attach_membership(&pool, tenant, student, "student").await;
    enroll(&pool, tenant, course, student).await;

    // A published graded quiz blocks completion until submitted.
    let quiz_id: Uuid = {
        let mut tx = pool.begin().await.unwrap();
        sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
            .bind(tenant.to_string())
            .execute(&mut *tx)
            .await
            .unwrap();
        let qid: Uuid = sqlx::query_scalar(
            "INSERT INTO quizzes (tenant_id, course_id, title, mode, status, created_by)
             VALUES ($1, $2, 'Final', 'graded', 'published', $3) RETURNING id",
        )
        .bind(tenant)
        .bind(course)
        .bind(teacher)
        .fetch_one(&mut *tx)
        .await
        .unwrap();
        tx.commit().await.unwrap();
        qid
    };

    let student_progress = build_test_app(
        backend::handlers::progress::router_for_tests(pool.clone()),
        stub(
            &pool,
            tenant,
            student,
            sfb.clone(),
            sem.clone(),
            core_types::TenantRole::Student,
        ),
    );
    let (s, _b) = fire(
        &student_progress,
        "PUT",
        &format!("/v1/courses/{course}/lessons/{lesson}/completion"),
        None,
    )
    .await;
    assert_eq!(s, 200);

    // Lessons done but the quiz isn't → no eligibility yet.
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM certificates WHERE course_id = $1 AND user_id = $2",
    )
    .bind(course)
    .bind(student)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 0, "quiz should gate eligibility");

    // Submit the quiz (raw attempt row) and re-trigger via unmark/remark.
    {
        let mut tx = pool.begin().await.unwrap();
        sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
            .bind(tenant.to_string())
            .execute(&mut *tx)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO quiz_attempts (tenant_id, quiz_id, user_id, submitted_at, score_points, max_points)
             VALUES ($1, $2, $3, now(), 1, 1)",
        )
        .bind(tenant)
        .bind(quiz_id)
        .bind(student)
        .execute(&mut *tx)
        .await
        .unwrap();
        tx.commit().await.unwrap();
    }
    let (_s, _b) = fire(
        &student_progress,
        "DELETE",
        &format!("/v1/courses/{course}/lessons/{lesson}/completion"),
        None,
    )
    .await;
    let (s, _b) = fire(
        &student_progress,
        "PUT",
        &format!("/v1/courses/{course}/lessons/{lesson}/completion"),
        None,
    )
    .await;
    assert_eq!(s, 200);

    let status: String =
        sqlx::query_scalar("SELECT status FROM certificates WHERE course_id = $1 AND user_id = $2")
            .bind(course)
            .bind(student)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(status, "eligible");
}
