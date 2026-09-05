mod fixtures;

use fixtures::*;
use serde_json::json;

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
         VALUES ($1,$2,$3,'teacher')",
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
async fn weekly_series_creates_correct_occurrences() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (user, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, user, "teacher").await;
    let course = course_for(&pool, tenant, user).await;

    let app = build_test_app(
        backend::handlers::live_sessions::router_for_tests(pool.clone()),
        StubAuth {
            pool: pool.clone(),
            user_id: user,
            firebase_uid: fb,
            email: em,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Teacher),
        },
    );

    let (status, body) = fire(
        &app,
        "POST",
        &format!("/v1/courses/{course}/sessions"),
        Some(json!({
            "title": "Weekly Class",
            "starts_at": "2026-05-12T17:00:00Z",
            "duration_minutes": 60,
            "frequency": "weekly",
            "byweekday": ["mon", "wed", "fri"],
            "end_kind": "count",
            "occurrence_count": 6,
            "recording_enabled": true
        })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let occs = body["occurrences"].as_array().unwrap();
    assert_eq!(occs.len(), 6);
    assert_eq!(occs[0]["starts_at"], "2026-05-13T17:00:00Z"); // first Wed
}

#[tokio::test]
async fn occurrence_cancel_then_reschedule_sets_diverged() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (user, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, user, "teacher").await;
    let course = course_for(&pool, tenant, user).await;

    let app = build_test_app(
        backend::handlers::live_sessions::router_for_tests(pool.clone()),
        StubAuth {
            pool: pool.clone(),
            user_id: user,
            firebase_uid: fb,
            email: em,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Teacher),
        },
    );
    let (_, body) = fire(
        &app,
        "POST",
        &format!("/v1/courses/{course}/sessions"),
        Some(json!({
            "title": "S",
            "starts_at": "2026-05-12T17:00:00Z",
            "duration_minutes": 60,
            "frequency": "weekly",
            "byweekday": ["tue"],
            "end_kind": "count",
            "occurrence_count": 3
        })),
    )
    .await;
    let occs = body["occurrences"].as_array().unwrap();
    let first_id = occs[0]["id"].as_str().unwrap();
    let second_id = occs[1]["id"].as_str().unwrap();

    let (status, body) = fire(
        &app,
        "PATCH",
        &format!("/v1/sessions/{first_id}"),
        Some(json!({ "status": "cancelled" })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["status"], "cancelled");
    assert_eq!(body["diverged"], false);

    let (status, body) = fire(
        &app,
        "PATCH",
        &format!("/v1/sessions/{second_id}"),
        Some(json!({ "starts_at": "2026-05-19T18:00:00Z" })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["diverged"], true);
}
