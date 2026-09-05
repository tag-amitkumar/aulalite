// crates/backend/tests/lesson_attachments.rs
mod fixtures;

use fixtures::*;
use serde_json::json;
use std::sync::Arc;

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
         VALUES ($1,$2,$3,'teacher')",
    )
    .bind(course)
    .bind(user)
    .bind(tenant)
    .execute(&mut *tx)
    .await
    .unwrap();
    let module: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO modules (tenant_id, course_id, title, sort_order)
         VALUES ($1,$2,'M',10) RETURNING id",
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
async fn create_file_bundle_lesson_attach_two_files_list_returns_two() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (user, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, user, "teacher").await;
    let (course, module) = course_with_module(&pool, tenant, user).await;
    let s3 = Arc::new(backend::storage::mock::MockS3Client::new());

    let stub = StubAuth {
        pool: pool.clone(),
        user_id: user,
        firebase_uid: fb,
        email: em,
        tenant_id: Some(tenant),
        tenant_role: Some(core_types::TenantRole::Teacher),
    };

    let lessons_app = build_test_app(
        backend::handlers::lessons::router_for_tests(pool.clone()),
        stub.clone(),
    );
    let (_, body) = fire(
        &lessons_app,
        "POST",
        &format!("/v1/courses/{course}/modules/{module}/lessons"),
        Some(json!({ "type": "file_bundle", "title": "Handouts" })),
    )
    .await;
    let lesson_id_str = body["id"].as_str().unwrap().to_string();
    let lesson_id: uuid::Uuid = lesson_id_str.parse().unwrap();

    let uploads_app = build_test_app(
        backend::handlers::uploads::router_for_tests(pool.clone(), s3.clone(), "aulalite".into()),
        stub.clone(),
    );
    for i in 0..2 {
        let (_, body) = fire(
            &uploads_app,
            "POST",
            "/v1/uploads/begin",
            Some(json!({
                "filename": format!("handout_{i}.pdf"),
                "content_type": "application/pdf",
                "size_bytes": 1000,
                "linked_entity_type": "lesson", "linked_entity_id": lesson_id,
                "purpose": "attachment"
            })),
        )
        .await;
        let asset_id = body["asset_id"].as_str().unwrap().to_string();
        let asset_uuid: uuid::Uuid = asset_id.parse().unwrap();
        let object_key: String =
            sqlx::query_scalar("SELECT object_key FROM file_assets WHERE id = $1")
                .bind(asset_uuid)
                .fetch_one(&pool)
                .await
                .unwrap();
        s3.simulate_object(&object_key, 1000, "application/pdf");
        fire(
            &uploads_app,
            "POST",
            &format!("/v1/uploads/{asset_id}/complete"),
            Some(json!({})),
        )
        .await;
    }

    let fa_app = build_test_app(
        backend::handlers::file_assets::router_for_tests(pool.clone(), s3.clone()),
        stub,
    );
    let (status, body) = fire(
        &fa_app,
        "GET",
        &format!("/v1/lessons/{lesson_id}/files"),
        None,
    )
    .await;
    assert_eq!(status, 200);
    let arr = body.as_array().unwrap();
    assert_eq!(arr.len(), 2);
}

#[tokio::test]
async fn lesson_video_replaces_video_asset_id() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (user, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, user, "teacher").await;
    let (course, module) = course_with_module(&pool, tenant, user).await;
    let s3 = Arc::new(backend::storage::mock::MockS3Client::new());

    let stub = StubAuth {
        pool: pool.clone(),
        user_id: user,
        firebase_uid: fb,
        email: em,
        tenant_id: Some(tenant),
        tenant_role: Some(core_types::TenantRole::Teacher),
    };
    let lessons_app = build_test_app(
        backend::handlers::lessons::router_for_tests(pool.clone()),
        stub.clone(),
    );

    let (_, body) = fire(
        &lessons_app,
        "POST",
        &format!("/v1/courses/{course}/modules/{module}/lessons"),
        Some(json!({ "type": "video", "title": "Week 1" })),
    )
    .await;
    let lesson_id_str = body["id"].as_str().unwrap().to_string();
    let lesson_id: uuid::Uuid = lesson_id_str.parse().unwrap();

    let uploads_app = build_test_app(
        backend::handlers::uploads::router_for_tests(pool.clone(), s3.clone(), "aulalite".into()),
        stub.clone(),
    );
    let (_, body) = fire(
        &uploads_app,
        "POST",
        "/v1/uploads/begin",
        Some(json!({
            "filename": "lecture.mp4", "content_type": "video/mp4",
            "size_bytes": 50_000_000,
            "linked_entity_type": "lesson", "linked_entity_id": lesson_id,
            "purpose": "video"
        })),
    )
    .await;
    let asset_id_str = body["asset_id"].as_str().unwrap().to_string();
    let asset_id: uuid::Uuid = asset_id_str.parse().unwrap();
    let object_key: String = sqlx::query_scalar("SELECT object_key FROM file_assets WHERE id = $1")
        .bind(asset_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    s3.simulate_object(&object_key, 50_000_000, "video/mp4");
    fire(
        &uploads_app,
        "POST",
        &format!("/v1/uploads/{asset_id}/complete"),
        Some(json!({})),
    )
    .await;

    let (status, body) = fire(
        &lessons_app,
        "PATCH",
        &format!("/v1/courses/{course}/modules/{module}/lessons/{lesson_id}"),
        Some(json!({ "video_asset_id": asset_id })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body["video_asset_id"].as_str().unwrap(),
        asset_id.to_string()
    );
}

#[tokio::test]
async fn non_member_cannot_list_attachments() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (course, module) = course_with_module(&pool, tenant, teacher).await;

    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    let lesson_id: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO lessons (tenant_id, course_id, module_id, type, title, sort_order)
         VALUES ($1, $2, $3, 'file_bundle', 'H', 10) RETURNING id",
    )
    .bind(tenant)
    .bind(course)
    .bind(module)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let other_tenant = create_tenant(&pool).await;
    let (outsider, fb_o, em_o) = create_user(&pool).await;
    attach_membership(&pool, other_tenant, outsider, "student").await;

    let s3 = Arc::new(backend::storage::mock::MockS3Client::new());
    let fa_app = build_test_app(
        backend::handlers::file_assets::router_for_tests(pool.clone(), s3),
        StubAuth {
            pool: pool.clone(),
            user_id: outsider,
            firebase_uid: fb_o,
            email: em_o,
            tenant_id: Some(other_tenant),
            tenant_role: Some(core_types::TenantRole::Student),
        },
    );
    let (status, _) = fire(
        &fa_app,
        "GET",
        &format!("/v1/lessons/{lesson_id}/files"),
        None,
    )
    .await;
    // 404 (lesson not visible) or 403 (lesson not readable). Either is acceptable masking.
    assert!(status == 404 || status == 403);
}
