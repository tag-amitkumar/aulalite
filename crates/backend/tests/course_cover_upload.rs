// crates/backend/tests/course_cover_upload.rs
mod fixtures;

use fixtures::*;
use serde_json::json;
use std::sync::Arc;

async fn course_owned_by(pool: &sqlx::PgPool, tenant: uuid::Uuid, user: uuid::Uuid) -> uuid::Uuid {
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
    .bind(user)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO course_memberships (course_id, user_id, tenant_id, role)
         VALUES ($1,$2,$3,'teacher')",
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

async fn upload_cover(
    pool: &sqlx::PgPool,
    s3: &Arc<backend::storage::mock::MockS3Client>,
    stub: StubAuth,
    course: uuid::Uuid,
) -> uuid::Uuid {
    let app = build_test_app(
        backend::handlers::uploads::router_for_tests(pool.clone(), s3.clone(), "aulalite".into()),
        stub.clone(),
    );
    let (status, body) = fire(
        &app,
        "POST",
        "/v1/uploads/begin",
        Some(json!({
            "filename": "cover.png", "content_type": "image/png",
            "size_bytes": 1000,
            "linked_entity_type": "course", "linked_entity_id": course,
            "purpose": "cover"
        })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let asset_id_str = body["asset_id"].as_str().unwrap().to_string();
    let asset_id: uuid::Uuid = asset_id_str.parse().unwrap();
    let object_key: String = sqlx::query_scalar("SELECT object_key FROM file_assets WHERE id = $1")
        .bind(asset_id)
        .fetch_one(pool)
        .await
        .unwrap();
    s3.simulate_object(&object_key, 1000, "image/png");
    let (s, _) = fire(
        &app,
        "POST",
        &format!("/v1/uploads/{asset_id}/complete"),
        Some(json!({})),
    )
    .await;
    assert_eq!(s, 200);
    asset_id
}

#[tokio::test]
async fn teacher_uploads_cover_then_patches_course() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (user, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, user, "teacher").await;
    let course = course_owned_by(&pool, tenant, user).await;
    let s3 = Arc::new(backend::storage::mock::MockS3Client::new());

    let stub = StubAuth {
        pool: pool.clone(),
        user_id: user,
        firebase_uid: fb,
        email: em,
        tenant_id: Some(tenant),
        tenant_role: Some(core_types::TenantRole::Teacher),
    };
    let asset_id = upload_cover(&pool, &s3, stub.clone(), course).await;

    let courses_app = build_test_app(
        backend::handlers::courses::router_for_tests(pool.clone()),
        stub,
    );
    let (status, body) = fire(
        &courses_app,
        "PATCH",
        &format!("/v1/courses/{course}"),
        Some(json!({ "cover_asset_id": asset_id })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body["cover_asset_id"].as_str().unwrap(),
        asset_id.to_string()
    );
}

#[tokio::test]
async fn student_cannot_begin_cover_upload() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let course = course_owned_by(&pool, tenant, teacher).await;

    let (student, fb_s, em_s) = create_user(&pool).await;
    attach_membership(&pool, tenant, student, "student").await;

    let s3 = Arc::new(backend::storage::mock::MockS3Client::new());
    let app = build_test_app(
        backend::handlers::uploads::router_for_tests(pool.clone(), s3, "aulalite".into()),
        StubAuth {
            pool: pool.clone(),
            user_id: student,
            firebase_uid: fb_s,
            email: em_s,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Student),
        },
    );
    let (status, _) = fire(
        &app,
        "POST",
        "/v1/uploads/begin",
        Some(json!({
            "filename": "x.png", "content_type": "image/png",
            "size_bytes": 1000,
            "linked_entity_type": "course", "linked_entity_id": course,
            "purpose": "cover"
        })),
    )
    .await;
    assert_eq!(status, 403);
}

#[tokio::test]
async fn nullable_cover_asset_id_can_be_cleared() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (user, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, user, "teacher").await;
    let course = course_owned_by(&pool, tenant, user).await;
    let s3 = Arc::new(backend::storage::mock::MockS3Client::new());

    let stub = StubAuth {
        pool: pool.clone(),
        user_id: user,
        firebase_uid: fb,
        email: em,
        tenant_id: Some(tenant),
        tenant_role: Some(core_types::TenantRole::Teacher),
    };
    let asset_id = upload_cover(&pool, &s3, stub.clone(), course).await;

    let courses_app = build_test_app(
        backend::handlers::courses::router_for_tests(pool.clone()),
        stub,
    );
    fire(
        &courses_app,
        "PATCH",
        &format!("/v1/courses/{course}"),
        Some(json!({ "cover_asset_id": asset_id })),
    )
    .await;
    let (status, body) = fire(
        &courses_app,
        "PATCH",
        &format!("/v1/courses/{course}"),
        Some(json!({ "cover_asset_id": null })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert!(body["cover_asset_id"].is_null());
}
