mod fixtures;

use fixtures::*;
use serde_json::json;

async fn course_owned_by(pool: &sqlx::PgPool, tenant: uuid::Uuid, user: uuid::Uuid) -> uuid::Uuid {
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    let id: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO courses (tenant_id, slug, title, owner_user_id)
         VALUES ($1, $2, 'X', $3) RETURNING id",
    )
    .bind(tenant)
    .bind(format!("c-{}", uuid::Uuid::new_v4()))
    .bind(user)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO course_memberships (course_id, user_id, tenant_id, role)
         VALUES ($1, $2, $3, 'teacher')",
    )
    .bind(id)
    .bind(user)
    .bind(tenant)
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    id
}

#[tokio::test]
async fn modules_create_then_reorder() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (user, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, user, "teacher").await;
    let course = course_owned_by(&pool, tenant, user).await;

    let stub = StubAuth {
        pool: pool.clone(),
        user_id: user,
        firebase_uid: fb,
        email: em,
        tenant_id: Some(tenant),
        tenant_role: Some(core_types::TenantRole::Teacher),
    };
    let app = build_test_app(
        backend::handlers::modules::router_for_tests(pool.clone()),
        stub,
    );

    let (s1, b1) = fire(
        &app,
        "POST",
        &format!("/v1/courses/{course}/modules"),
        Some(json!({ "title": "Week 1" })),
    )
    .await;
    let (s2, b2) = fire(
        &app,
        "POST",
        &format!("/v1/courses/{course}/modules"),
        Some(json!({ "title": "Week 2" })),
    )
    .await;
    let (s3, b3) = fire(
        &app,
        "POST",
        &format!("/v1/courses/{course}/modules"),
        Some(json!({ "title": "Week 3" })),
    )
    .await;
    assert_eq!(s1, 200);
    assert_eq!(s2, 200);
    assert_eq!(s3, 200);
    let m1 = b1["id"].as_str().unwrap().to_string();
    let m2 = b2["id"].as_str().unwrap().to_string();
    let m3 = b3["id"].as_str().unwrap().to_string();
    assert_eq!(b1["sort_order"].as_i64().unwrap(), 10);
    assert_eq!(b2["sort_order"].as_i64().unwrap(), 20);
    assert_eq!(b3["sort_order"].as_i64().unwrap(), 30);

    // Reorder: m3, m1, m2
    let (s, _) = fire(
        &app,
        "POST",
        &format!("/v1/courses/{course}/modules/reorder"),
        Some(json!({ "module_ids": [m3, m1, m2] })),
    )
    .await;
    assert_eq!(s, 200);

    // Confirm new order via DB peek
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    let rows: Vec<(uuid::Uuid, i32)> = sqlx::query_as(
        "SELECT id, sort_order FROM modules
          WHERE course_id = $1 ORDER BY sort_order",
    )
    .bind(course)
    .fetch_all(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].1, 10);
    assert_eq!(rows[1].1, 20);
    assert_eq!(rows[2].1, 30);
}

#[tokio::test]
async fn module_mutations_reject_a_sibling_course_module_id() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (user, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, user, "teacher").await;
    let authorized_course = course_owned_by(&pool, tenant, user).await;
    let sibling_course = course_owned_by(&pool, tenant, user).await;

    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    let sibling_module: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO modules (tenant_id, course_id, title, sort_order)
         VALUES ($1, $2, 'Sibling module', 10) RETURNING id",
    )
    .bind(tenant)
    .bind(sibling_course)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let app = build_test_app(
        backend::handlers::modules::router_for_tests(pool.clone()),
        StubAuth {
            pool: pool.clone(),
            user_id: user,
            firebase_uid: fb,
            email: em,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Teacher),
        },
    );

    let (status, _) = fire(
        &app,
        "PATCH",
        &format!("/v1/courses/{authorized_course}/modules/{sibling_module}"),
        Some(json!({ "title": "Taken over" })),
    )
    .await;
    assert_eq!(status, 404);
    let (status, _) = fire(
        &app,
        "DELETE",
        &format!("/v1/courses/{authorized_course}/modules/{sibling_module}"),
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
    let title: String = sqlx::query_scalar("SELECT title FROM modules WHERE id = $1")
        .bind(sibling_module)
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(title, "Sibling module");
}
