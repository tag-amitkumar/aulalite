mod fixtures;

use fixtures::*;
use serde_json::json;

async fn course_with_module(
    pool: &sqlx::PgPool,
    tenant: uuid::Uuid,
    user: uuid::Uuid,
) -> (uuid::Uuid, uuid::Uuid) {
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
    .bind(user)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO course_memberships (course_id, user_id, tenant_id, role)
         VALUES ($1, $2, $3, 'teacher')",
    )
    .bind(course)
    .bind(user)
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
    tx.commit().await.unwrap();
    (course, module)
}

#[tokio::test]
async fn rich_text_lesson_create_and_reorder() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (user, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, user, "teacher").await;
    let (course, module) = course_with_module(&pool, tenant, user).await;

    let stub = StubAuth {
        pool: pool.clone(),
        user_id: user,
        firebase_uid: fb,
        email: em,
        tenant_id: Some(tenant),
        tenant_role: Some(core_types::TenantRole::Teacher),
    };
    let app = build_test_app(
        backend::handlers::lessons::router_for_tests(pool.clone()),
        stub,
    );

    let (s, b) = fire(
        &app,
        "POST",
        &format!("/v1/courses/{course}/modules/{module}/lessons"),
        Some(json!({
            "type": "rich_text",
            "title": "Welcome",
            "body_md": "# Hello\nThis is week one."
        })),
    )
    .await;
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["type"], "rich_text");
    assert_eq!(b["sort_order"].as_i64().unwrap(), 10);
}

#[tokio::test]
async fn video_lesson_creates_without_asset_id() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (user, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, user, "teacher").await;
    let (course, module) = course_with_module(&pool, tenant, user).await;

    let app = build_test_app(
        backend::handlers::lessons::router_for_tests(pool.clone()),
        StubAuth {
            pool: pool.clone(),
            user_id: user,
            firebase_uid: fb,
            email: em,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Teacher),
        },
    );

    let (s, b) = fire(
        &app,
        "POST",
        &format!("/v1/courses/{course}/modules/{module}/lessons"),
        Some(json!({ "type": "video", "title": "Week 1 video" })),
    )
    .await;
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["type"], "video");
    assert!(b["video_asset_id"].is_null());
}

#[tokio::test]
async fn file_bundle_lesson_creates_without_attachments() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (user, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, user, "teacher").await;
    let (course, module) = course_with_module(&pool, tenant, user).await;

    let app = build_test_app(
        backend::handlers::lessons::router_for_tests(pool.clone()),
        StubAuth {
            pool: pool.clone(),
            user_id: user,
            firebase_uid: fb,
            email: em,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Teacher),
        },
    );

    let (s, b) = fire(
        &app,
        "POST",
        &format!("/v1/courses/{course}/modules/{module}/lessons"),
        Some(json!({ "type": "file_bundle", "title": "Handouts" })),
    )
    .await;
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["type"], "file_bundle");
}

#[tokio::test]
async fn lesson_mutations_bind_every_nested_id_to_the_authorized_course() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (user, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, user, "teacher").await;
    let (authorized_course, authorized_module) = course_with_module(&pool, tenant, user).await;
    let (sibling_course, sibling_module) = course_with_module(&pool, tenant, user).await;

    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    let required_lesson: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO lessons
            (tenant_id, course_id, module_id, type, title, sort_order)
         VALUES ($1, $2, $3, 'rich_text', 'Required sibling', 10)
         RETURNING id",
    )
    .bind(tenant)
    .bind(sibling_course)
    .bind(sibling_module)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    let sibling_lesson: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO lessons
            (tenant_id, course_id, module_id, type, title, sort_order)
         VALUES ($1, $2, $3, 'rich_text', 'Sibling lesson', 20)
         RETURNING id",
    )
    .bind(tenant)
    .bind(sibling_course)
    .bind(sibling_module)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO lesson_prerequisites (tenant_id, lesson_id, required_lesson_id)
         VALUES ($1, $2, $3)",
    )
    .bind(tenant)
    .bind(sibling_lesson)
    .bind(required_lesson)
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let app = build_test_app(
        backend::handlers::lessons::router_for_tests(pool.clone()),
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
        "POST",
        &format!("/v1/courses/{authorized_course}/modules/{sibling_module}/lessons"),
        Some(json!({ "type": "rich_text", "title": "Wrong parent" })),
    )
    .await;
    assert_eq!(status, 404);

    let nested_url = format!(
        "/v1/courses/{authorized_course}/modules/{authorized_module}/lessons/{sibling_lesson}"
    );
    let (status, _) = fire(
        &app,
        "PATCH",
        &nested_url,
        Some(json!({ "title": "Taken over" })),
    )
    .await;
    assert_eq!(status, 404);
    let (status, _) = fire(&app, "DELETE", &nested_url, None).await;
    assert_eq!(status, 404);

    let (status, _) = fire(
        &app,
        "POST",
        &format!("/v1/courses/{authorized_course}/modules/{authorized_module}/lessons/reorder"),
        Some(json!({ "lesson_ids": [sibling_lesson] })),
    )
    .await;
    assert_eq!(status, 404);

    let (status, _) = fire(
        &app,
        "DELETE",
        &format!("/v1/courses/{authorized_course}/lessons/{sibling_lesson}/prerequisites"),
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
    let sibling: (String, i32) =
        sqlx::query_as("SELECT title, sort_order FROM lessons WHERE id = $1")
            .bind(sibling_lesson)
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    let prerequisites: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM lesson_prerequisites
          WHERE lesson_id = $1 AND required_lesson_id = $2",
    )
    .bind(sibling_lesson)
    .bind(required_lesson)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(sibling, ("Sibling lesson".into(), 20));
    assert_eq!(prerequisites, 1);
}

#[tokio::test]
async fn ta_cannot_replace_or_clear_lesson_prerequisites() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _, _) = create_user(&pool).await;
    let (ta, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    attach_membership(&pool, tenant, ta, "ta").await;
    let (course, module) = course_with_module(&pool, tenant, teacher).await;

    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO course_memberships (course_id, user_id, tenant_id, role, status)
         VALUES ($1, $2, $3, 'ta', 'active')",
    )
    .bind(course)
    .bind(ta)
    .bind(tenant)
    .execute(&mut *tx)
    .await
    .unwrap();
    let required: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO lessons (tenant_id, course_id, module_id, type, title, sort_order)
         VALUES ($1, $2, $3, 'rich_text', 'Required', 10) RETURNING id",
    )
    .bind(tenant)
    .bind(course)
    .bind(module)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    let lesson: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO lessons (tenant_id, course_id, module_id, type, title, sort_order)
         VALUES ($1, $2, $3, 'rich_text', 'Target', 20) RETURNING id",
    )
    .bind(tenant)
    .bind(course)
    .bind(module)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let app = build_test_app(
        backend::handlers::lessons::router_for_tests(pool.clone()),
        StubAuth {
            pool: pool.clone(),
            user_id: ta,
            firebase_uid: fb,
            email: em,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Ta),
        },
    );
    let url = format!("/v1/courses/{course}/lessons/{lesson}/prerequisites");
    let (status, _) = fire(
        &app,
        "PUT",
        &url,
        Some(json!({ "required_lesson_ids": [required] })),
    )
    .await;
    assert_eq!(status, 403);

    let (status, _) = fire(&app, "DELETE", &url, None).await;
    assert_eq!(status, 403);
}
