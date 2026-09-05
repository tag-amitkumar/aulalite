//! Integration tests for quizzes (learning-suite Cycle 3): authoring,
//! publishing, the student taking flow with server-side grading, attempt
//! caps, answer-key stripping, and authorization edges.

mod fixtures;

use fixtures::*;
use serde_json::json;

async fn published_course(
    pool: &sqlx::PgPool,
    tenant: uuid::Uuid,
    owner: uuid::Uuid,
) -> uuid::Uuid {
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    let course: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO courses (tenant_id, slug, title, status, owner_user_id)
         VALUES ($1, $2, 'Quiz C', 'published', $3) RETURNING id",
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
    tx.commit().await.unwrap();
    course
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

fn stub(
    pool: &sqlx::PgPool,
    tenant: uuid::Uuid,
    user: uuid::Uuid,
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

fn sample_questions() -> serde_json::Value {
    json!([
        {
            "prompt_text": "Pick A",
            "prompt": {
                "kind": "single_choice",
                "choices": [
                    {"id": "a", "text": "Alpha"},
                    {"id": "b", "text": "Beta"}
                ],
                "correct": "a"
            },
            "points": 2
        },
        {
            "prompt_text": "Water is wet",
            "prompt": {"kind": "true_false", "correct": true},
            "points": 1
        },
        {
            "prompt_text": "Powerhouse of the cell?",
            "prompt": {"kind": "short_answer", "accepted": ["The Mitochondria"]},
            "points": 1
        }
    ])
}

/// Author a quiz with the sample questions and publish it. Returns quiz id.
async fn author_published_quiz(
    teacher_app: &axum::Router,
    course: uuid::Uuid,
    mode: &str,
    max_attempts: Option<i32>,
) -> String {
    let (s, b) = fire(
        teacher_app,
        "POST",
        &format!("/v1/courses/{course}/quizzes"),
        Some(json!({"title": "Unit quiz", "mode": mode, "max_attempts": max_attempts})),
    )
    .await;
    assert_eq!(s, 200, "{b}");
    let quiz_id = b["id"].as_str().unwrap().to_string();

    let (s, b) = fire(
        teacher_app,
        "PUT",
        &format!("/v1/courses/{course}/quizzes/{quiz_id}/questions"),
        Some(sample_questions()),
    )
    .await;
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["count"].as_i64().unwrap(), 3);

    let (s, b) = fire(
        teacher_app,
        "PATCH",
        &format!("/v1/courses/{course}/quizzes/{quiz_id}"),
        Some(json!({"status": "published"})),
    )
    .await;
    assert_eq!(s, 200, "{b}");
    quiz_id
}

#[tokio::test]
async fn publish_requires_questions() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let course = published_course(&pool, tenant, teacher).await;

    let app = build_test_app(
        backend::handlers::quizzes::router_for_tests(pool.clone()),
        stub(
            &pool,
            tenant,
            teacher,
            fb,
            em,
            core_types::TenantRole::Teacher,
        ),
    );
    let (s, b) = fire(
        &app,
        "POST",
        &format!("/v1/courses/{course}/quizzes"),
        Some(json!({"title": "Empty quiz"})),
    )
    .await;
    assert_eq!(s, 200, "{b}");
    let quiz_id = b["id"].as_str().unwrap();

    let (s, b) = fire(
        &app,
        "PATCH",
        &format!("/v1/courses/{course}/quizzes/{quiz_id}"),
        Some(json!({"status": "published"})),
    )
    .await;
    assert_eq!(s, 400, "{b}");
}

#[tokio::test]
async fn malformed_question_keys_are_rejected() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let course = published_course(&pool, tenant, teacher).await;
    let app = build_test_app(
        backend::handlers::quizzes::router_for_tests(pool.clone()),
        stub(
            &pool,
            tenant,
            teacher,
            fb,
            em,
            core_types::TenantRole::Teacher,
        ),
    );
    let (s, b) = fire(
        &app,
        "POST",
        &format!("/v1/courses/{course}/quizzes"),
        Some(json!({"title": "Q"})),
    )
    .await;
    assert_eq!(s, 200, "{b}");
    let quiz_id = b["id"].as_str().unwrap();

    // Correct id not among the choices.
    let (s, b) = fire(
        &app,
        "PUT",
        &format!("/v1/courses/{course}/quizzes/{quiz_id}/questions"),
        Some(json!([{
            "prompt_text": "Broken",
            "prompt": {
                "kind": "single_choice",
                "choices": [{"id": "a", "text": "A"}, {"id": "b", "text": "B"}],
                "correct": "zz"
            }
        }])),
    )
    .await;
    assert_eq!(s, 400, "{b}");
}

#[tokio::test]
async fn student_take_flow_grades_server_side_and_strips_keys() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, tfb, tem) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let course = published_course(&pool, tenant, teacher).await;
    let teacher_app = build_test_app(
        backend::handlers::quizzes::router_for_tests(pool.clone()),
        stub(
            &pool,
            tenant,
            teacher,
            tfb,
            tem,
            core_types::TenantRole::Teacher,
        ),
    );
    let quiz_id = author_published_quiz(&teacher_app, course, "graded", Some(2)).await;

    let (student, sfb, sem) = create_user(&pool).await;
    attach_membership(&pool, tenant, student, "student").await;
    enroll_student(&pool, tenant, course, student).await;
    let student_app = build_test_app(
        backend::handlers::quizzes::router_for_tests(pool.clone()),
        stub(
            &pool,
            tenant,
            student,
            sfb,
            sem,
            core_types::TenantRole::Student,
        ),
    );

    // Detail for the student: questions present, keys stripped.
    let (s, b) = fire(
        &student_app,
        "GET",
        &format!("/v1/courses/{course}/quizzes/{quiz_id}"),
        None,
    )
    .await;
    assert_eq!(s, 200, "{b}");
    let body_text = b.to_string();
    assert!(!body_text.contains("correct"), "leaked key: {body_text}");
    assert!(
        !body_text.contains("Mitochondria"),
        "leaked key: {body_text}"
    );
    let questions = b["student_questions"].as_array().unwrap();
    assert_eq!(questions.len(), 3);
    let q_ids: Vec<&str> = questions
        .iter()
        .map(|q| q["id"].as_str().unwrap())
        .collect();

    // Start an attempt and submit: right, right, wrong → 3/4 points.
    let (s, b) = fire(
        &student_app,
        "POST",
        &format!("/v1/courses/{course}/quizzes/{quiz_id}/attempts"),
        None,
    )
    .await;
    assert_eq!(s, 200, "{b}");
    let attempt_id = b["id"].as_str().unwrap().to_string();

    let (s, b) = fire(
        &student_app,
        "POST",
        &format!("/v1/courses/{course}/quizzes/{quiz_id}/attempts/{attempt_id}/submit"),
        Some(json!({"answers": [
            {"question_id": q_ids[0], "answer": {"type": "choice", "value": "a"}},
            {"question_id": q_ids[1], "answer": {"type": "bool", "value": true}},
            {"question_id": q_ids[2], "answer": {"type": "text", "value": "ribosome"}}
        ]})),
    )
    .await;
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["score_points"].as_i64().unwrap(), 3);
    assert_eq!(b["max_points"].as_i64().unwrap(), 4);
    let per_q = b["per_question"].as_array().unwrap();
    assert_eq!(per_q[0]["correct"], true);
    assert_eq!(per_q[1]["correct"], true);
    assert_eq!(per_q[2]["correct"], false);

    // Double submit is rejected.
    let (s, _b) = fire(
        &student_app,
        "POST",
        &format!("/v1/courses/{course}/quizzes/{quiz_id}/attempts/{attempt_id}/submit"),
        Some(json!({"answers": []})),
    )
    .await;
    assert_eq!(s, 400);

    // Attempt cap: second attempt OK, third rejected.
    let (s, b) = fire(
        &student_app,
        "POST",
        &format!("/v1/courses/{course}/quizzes/{quiz_id}/attempts"),
        None,
    )
    .await;
    assert_eq!(s, 200, "{b}");
    let attempt2 = b["id"].as_str().unwrap().to_string();
    let (s, _b) = fire(
        &student_app,
        "POST",
        &format!("/v1/courses/{course}/quizzes/{quiz_id}/attempts/{attempt2}/submit"),
        Some(json!({"answers": []})),
    )
    .await;
    assert_eq!(s, 200);
    let (s, b) = fire(
        &student_app,
        "POST",
        &format!("/v1/courses/{course}/quizzes/{quiz_id}/attempts"),
        None,
    )
    .await;
    assert_eq!(s, 400, "{b}");

    // Teacher results rollup shows the best score.
    let (s, b) = fire(
        &teacher_app,
        "GET",
        &format!("/v1/courses/{course}/quizzes/{quiz_id}/results"),
        None,
    )
    .await;
    assert_eq!(s, 200, "{b}");
    let rows = b.as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["attempts"].as_i64().unwrap(), 2);
    assert_eq!(rows[0]["best_score"].as_i64().unwrap(), 3);

    // Students cannot read the staff rollup.
    let (s, _b) = fire(
        &student_app,
        "GET",
        &format!("/v1/courses/{course}/quizzes/{quiz_id}/results"),
        None,
    )
    .await;
    assert_eq!(s, 403);
}

#[tokio::test]
async fn drafts_are_invisible_to_students_and_practice_is_unlimited() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, tfb, tem) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let course = published_course(&pool, tenant, teacher).await;
    let teacher_app = build_test_app(
        backend::handlers::quizzes::router_for_tests(pool.clone()),
        stub(
            &pool,
            tenant,
            teacher,
            tfb,
            tem,
            core_types::TenantRole::Teacher,
        ),
    );

    // One draft + one published practice quiz.
    let (s, _b) = fire(
        &teacher_app,
        "POST",
        &format!("/v1/courses/{course}/quizzes"),
        Some(json!({"title": "Draft quiz"})),
    )
    .await;
    assert_eq!(s, 200);
    let practice_id = author_published_quiz(&teacher_app, course, "practice", None).await;

    let (student, sfb, sem) = create_user(&pool).await;
    attach_membership(&pool, tenant, student, "student").await;
    enroll_student(&pool, tenant, course, student).await;
    let student_app = build_test_app(
        backend::handlers::quizzes::router_for_tests(pool.clone()),
        stub(
            &pool,
            tenant,
            student,
            sfb,
            sem,
            core_types::TenantRole::Student,
        ),
    );

    // Student list shows only the published quiz; teacher list shows both.
    let (s, b) = fire(
        &student_app,
        "GET",
        &format!("/v1/courses/{course}/quizzes"),
        None,
    )
    .await;
    assert_eq!(s, 200, "{b}");
    assert_eq!(b.as_array().unwrap().len(), 1);
    let (_s, b) = fire(
        &teacher_app,
        "GET",
        &format!("/v1/courses/{course}/quizzes"),
        None,
    )
    .await;
    assert_eq!(b.as_array().unwrap().len(), 2);

    // Practice mode: three attempts in a row all succeed.
    for _ in 0..3 {
        let (s, b) = fire(
            &student_app,
            "POST",
            &format!("/v1/courses/{course}/quizzes/{practice_id}/attempts"),
            None,
        )
        .await;
        assert_eq!(s, 200, "{b}");
        let aid = b["id"].as_str().unwrap().to_string();
        let (s, _b) = fire(
            &student_app,
            "POST",
            &format!("/v1/courses/{course}/quizzes/{practice_id}/attempts/{aid}/submit"),
            Some(json!({"answers": []})),
        )
        .await;
        assert_eq!(s, 200);
    }
}

#[tokio::test]
async fn staff_cannot_create_learner_quiz_attempts() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, tfb, tem) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let course = published_course(&pool, tenant, teacher).await;
    let teacher_app = build_test_app(
        backend::handlers::quizzes::router_for_tests(pool.clone()),
        stub(
            &pool,
            tenant,
            teacher,
            tfb,
            tem,
            core_types::TenantRole::Teacher,
        ),
    );
    let quiz_id = author_published_quiz(&teacher_app, course, "graded", Some(1)).await;

    let (status, _) = fire(
        &teacher_app,
        "POST",
        &format!("/v1/courses/{course}/quizzes/{quiz_id}/attempts"),
        None,
    )
    .await;
    assert_eq!(status, 403);
}
