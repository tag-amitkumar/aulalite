//! Integration tests for flashcards (learning-suite Cycle 6): teacher deck/
//! card authoring (staff-gated), published-only student visibility, the
//! SM-2 review loop (server-authoritative scheduling), and the due-count.

mod fixtures;

use fixtures::*;
use serde_json::json;
use uuid::Uuid;

async fn published_course(pool: &sqlx::PgPool, tenant: Uuid, owner: Uuid) -> Uuid {
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    let course: Uuid = sqlx::query_scalar(
        "INSERT INTO courses (tenant_id, slug, title, status, owner_user_id)
         VALUES ($1, $2, 'FC', 'published', $3) RETURNING id",
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
    tx.commit().await.unwrap();
    course
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
async fn teacher_authors_deck_student_reviews_with_sm2() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, tfb, tem) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let course = published_course(&pool, tenant, teacher).await;
    let (student, sfb, sem) = create_user(&pool).await;
    attach_membership(&pool, tenant, student, "student").await;
    enroll(&pool, tenant, course, student).await;

    let teacher_app = build_test_app(
        backend::handlers::flashcards::router_for_tests(pool.clone()),
        stub(
            &pool,
            tenant,
            teacher,
            tfb,
            tem,
            core_types::TenantRole::Teacher,
        ),
    );
    let student_app = build_test_app(
        backend::handlers::flashcards::router_for_tests(pool.clone()),
        stub(
            &pool,
            tenant,
            student,
            sfb,
            sem,
            core_types::TenantRole::Student,
        ),
    );

    // Teacher creates a deck + two cards.
    let (s, b) = fire(
        &teacher_app,
        "POST",
        &format!("/v1/courses/{course}/decks"),
        Some(json!({"title": "Algebra basics", "description": "Week 1"})),
    )
    .await;
    assert_eq!(s, 200, "{b}");
    let deck_id = b["id"].as_str().unwrap().to_string();
    assert_eq!(b["status"], "draft");

    for (front, back) in [("2+2?", "4"), ("3*3?", "9")] {
        let (s, _b) = fire(
            &teacher_app,
            "POST",
            &format!("/v1/courses/{course}/decks/{deck_id}/cards"),
            Some(json!({"front": front, "back": back})),
        )
        .await;
        assert_eq!(s, 200);
    }

    // Students can't author and can't see draft decks.
    let (s, _b) = fire(
        &student_app,
        "POST",
        &format!("/v1/courses/{course}/decks"),
        Some(json!({"title": "nope"})),
    )
    .await;
    assert_eq!(s, 403);
    let (s, b) = fire(
        &student_app,
        "GET",
        &format!("/v1/courses/{course}/decks"),
        None,
    )
    .await;
    assert_eq!(s, 200, "{b}");
    assert_eq!(
        b.as_array().unwrap().len(),
        0,
        "draft leaked to student: {b}"
    );
    let (s, _b) = fire(
        &student_app,
        "GET",
        &format!("/v1/courses/{course}/decks/{deck_id}/review"),
        None,
    )
    .await;
    assert_eq!(s, 404, "draft deck must be invisible to students");

    // Publish → students see it; review queue holds both (new) cards.
    let (s, b) = fire(
        &teacher_app,
        "PATCH",
        &format!("/v1/courses/{course}/decks/{deck_id}"),
        Some(json!({"status": "published"})),
    )
    .await;
    assert_eq!(s, 200, "{b}");
    let (s, b) = fire(
        &student_app,
        "GET",
        &format!("/v1/courses/{course}/decks"),
        None,
    )
    .await;
    assert_eq!(s, 200);
    assert_eq!(b.as_array().unwrap().len(), 1);
    assert_eq!(b[0]["card_count"], 2);

    let (s, b) = fire(
        &student_app,
        "GET",
        &format!("/v1/courses/{course}/decks/{deck_id}/review"),
        None,
    )
    .await;
    assert_eq!(s, 200, "{b}");
    let queue = b.as_array().unwrap();
    assert_eq!(queue.len(), 2);
    let card_id = queue[0]["id"].as_str().unwrap().to_string();

    // Due count matches.
    let (s, b) = fire(
        &student_app,
        "GET",
        &format!("/v1/courses/{course}/flashcards/due-count"),
        None,
    )
    .await;
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["due"], 2);

    // Rate "good": first review graduates to a 1-day interval — the card
    // leaves today's queue.
    let (s, b) = fire(
        &student_app,
        "POST",
        &format!("/v1/courses/{course}/decks/{deck_id}/review/{card_id}"),
        Some(json!({"rating": "good"})),
    )
    .await;
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["interval_days"].as_f64().unwrap(), 1.0);
    assert_eq!(b["repetitions"], 1);

    let (s, b) = fire(
        &student_app,
        "GET",
        &format!("/v1/courses/{course}/decks/{deck_id}/review"),
        None,
    )
    .await;
    assert_eq!(s, 200);
    assert_eq!(b.as_array().unwrap().len(), 1, "rated card still due: {b}");

    // Rate the second card "again": it stays due today (interval 0).
    let second_id = b[0]["id"].as_str().unwrap().to_string();
    let (s, b) = fire(
        &student_app,
        "POST",
        &format!("/v1/courses/{course}/decks/{deck_id}/review/{second_id}"),
        Some(json!({"rating": "again"})),
    )
    .await;
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["interval_days"].as_f64().unwrap(), 0.0);
    assert_eq!(b["repetitions"], 0);
    let (s, b) = fire(
        &student_app,
        "GET",
        &format!("/v1/courses/{course}/decks/{deck_id}/review"),
        None,
    )
    .await;
    assert_eq!(s, 200);
    assert_eq!(
        b.as_array().unwrap().len(),
        1,
        "'again' card should stay due: {b}"
    );

    // Invalid ratings are rejected.
    let (s, _b) = fire(
        &student_app,
        "POST",
        &format!("/v1/courses/{course}/decks/{deck_id}/review/{second_id}"),
        Some(json!({"rating": "perfect"})),
    )
    .await;
    assert_eq!(s, 400);
}

#[tokio::test]
async fn card_mutations_reject_a_card_from_another_deck() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, fb, email) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let course = published_course(&pool, tenant, teacher).await;
    let app = build_test_app(
        backend::handlers::flashcards::router_for_tests(pool.clone()),
        stub(
            &pool,
            tenant,
            teacher,
            fb,
            email,
            core_types::TenantRole::Teacher,
        ),
    );

    let (_, first_deck) = fire(
        &app,
        "POST",
        &format!("/v1/courses/{course}/decks"),
        Some(json!({ "title": "First deck" })),
    )
    .await;
    let (_, second_deck) = fire(
        &app,
        "POST",
        &format!("/v1/courses/{course}/decks"),
        Some(json!({ "title": "Second deck" })),
    )
    .await;
    let first_deck = first_deck["id"].as_str().unwrap();
    let second_deck = second_deck["id"].as_str().unwrap();
    let (status, card) = fire(
        &app,
        "POST",
        &format!("/v1/courses/{course}/decks/{second_deck}/cards"),
        Some(json!({ "front": "Original front", "back": "Original back" })),
    )
    .await;
    assert_eq!(status, 200, "{card}");
    let card_id = card["id"].as_str().unwrap();

    let (status, _) = fire(
        &app,
        "PATCH",
        &format!("/v1/courses/{course}/decks/{first_deck}/cards/{card_id}"),
        Some(json!({ "front": "Taken over" })),
    )
    .await;
    assert_eq!(status, 404);
    let (status, _) = fire(
        &app,
        "DELETE",
        &format!("/v1/courses/{course}/decks/{first_deck}/cards/{card_id}"),
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
    let stored: (String, String) =
        sqlx::query_as("SELECT front, back FROM flashcards WHERE id = $1")
            .bind(card_id.parse::<Uuid>().unwrap())
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(stored, ("Original front".into(), "Original back".into()));
}
