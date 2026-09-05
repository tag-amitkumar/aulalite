// crates/backend/tests/uploads.rs
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

#[tokio::test]
async fn begin_returns_presigned_url_and_pending_row() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (user, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, user, "teacher").await;
    let course = course_owned_by(&pool, tenant, user).await;

    let s3 = Arc::new(backend::storage::mock::MockS3Client::new());
    let app = build_test_app(
        backend::handlers::uploads::router_for_tests(pool.clone(), s3.clone(), "aulalite".into()),
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
        "/v1/uploads/begin",
        Some(json!({
            "filename": "cover.png",
            "content_type": "image/png",
            "size_bytes": 1_000_000,
            "linked_entity_type": "course",
            "linked_entity_id": course,
            "purpose": "cover"
        })),
    )
    .await;

    assert_eq!(status, 200, "{body}");
    assert!(body["presigned_put_url"]
        .as_str()
        .unwrap()
        .starts_with("https://mock.s3/put/"));
    let asset_id = body["asset_id"].as_str().unwrap();

    let row: (String,) = sqlx::query_as("SELECT status FROM file_assets WHERE id = $1::uuid")
        .bind(asset_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(row.0, "pending");
}

#[tokio::test]
async fn begin_rejects_oversized_request() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (user, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, user, "teacher").await;
    let course = course_owned_by(&pool, tenant, user).await;

    let s3 = Arc::new(backend::storage::mock::MockS3Client::new());
    let app = build_test_app(
        backend::handlers::uploads::router_for_tests(pool.clone(), s3, "aulalite".into()),
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
        "/v1/uploads/begin",
        Some(json!({
            "filename": "huge.png",
            "content_type": "image/png",
            "size_bytes": 6_000_000,
            "linked_entity_type": "course",
            "linked_entity_id": course,
            "purpose": "cover"
        })),
    )
    .await;
    assert_eq!(status, 400);
    assert!(body["error"].as_str().unwrap().contains("size"));
}

#[tokio::test]
async fn begin_rejects_bad_content_type() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (user, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, user, "teacher").await;
    let course = course_owned_by(&pool, tenant, user).await;

    let s3 = Arc::new(backend::storage::mock::MockS3Client::new());
    let app = build_test_app(
        backend::handlers::uploads::router_for_tests(pool.clone(), s3, "aulalite".into()),
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
        "/v1/uploads/begin",
        Some(json!({
            "filename": "evil.svg",
            "content_type": "image/svg+xml",
            "size_bytes": 1000,
            "linked_entity_type": "course",
            "linked_entity_id": course,
            "purpose": "cover"
        })),
    )
    .await;
    assert_eq!(status, 400);
    assert!(body["error"].as_str().unwrap().contains("content type"));
}

#[tokio::test]
async fn complete_marks_available_when_head_matches() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (user, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, user, "teacher").await;
    let course = course_owned_by(&pool, tenant, user).await;

    let s3 = Arc::new(backend::storage::mock::MockS3Client::new());
    let app = build_test_app(
        backend::handlers::uploads::router_for_tests(pool.clone(), s3.clone(), "aulalite".into()),
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
        "/v1/uploads/begin",
        Some(json!({
            "filename": "cover.png",
            "content_type": "image/png",
            "size_bytes": 1_000_000,
            "linked_entity_type": "course",
            "linked_entity_id": course,
            "purpose": "cover"
        })),
    )
    .await;
    let asset_id = body["asset_id"].as_str().unwrap().to_string();

    let object_key: String =
        sqlx::query_scalar("SELECT object_key FROM file_assets WHERE id = $1::uuid")
            .bind(&asset_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    s3.simulate_object(&object_key, 1_000_000, "image/png");

    let (status, body) = fire(
        &app,
        "POST",
        &format!("/v1/uploads/{asset_id}/complete"),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["status"], "available");
}

#[tokio::test]
async fn complete_marks_failed_when_size_mismatch() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (user, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, user, "teacher").await;
    let course = course_owned_by(&pool, tenant, user).await;

    let s3 = Arc::new(backend::storage::mock::MockS3Client::new());
    let app = build_test_app(
        backend::handlers::uploads::router_for_tests(pool.clone(), s3.clone(), "aulalite".into()),
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
        "/v1/uploads/begin",
        Some(json!({
            "filename": "x.png", "content_type": "image/png",
            "size_bytes": 1_000_000,
            "linked_entity_type": "course", "linked_entity_id": course,
            "purpose": "cover"
        })),
    )
    .await;
    let asset_id = body["asset_id"].as_str().unwrap().to_string();
    let object_key: String =
        sqlx::query_scalar("SELECT object_key FROM file_assets WHERE id = $1::uuid")
            .bind(&asset_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    s3.simulate_object(&object_key, 999_999, "image/png");

    let (status, _) = fire(
        &app,
        "POST",
        &format!("/v1/uploads/{asset_id}/complete"),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, 400);

    let row: (String,) = sqlx::query_as("SELECT status FROM file_assets WHERE id = $1::uuid")
        .bind(&asset_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(row.0, "failed");
}

#[tokio::test]
async fn complete_by_non_uploader_rejected() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (uploader, fb_u, em_u) = create_user(&pool).await;
    attach_membership(&pool, tenant, uploader, "teacher").await;
    let (other, fb_o, em_o) = create_user(&pool).await;
    attach_membership(&pool, tenant, other, "teacher").await;
    let course = course_owned_by(&pool, tenant, uploader).await;

    let s3 = Arc::new(backend::storage::mock::MockS3Client::new());

    let app_u = build_test_app(
        backend::handlers::uploads::router_for_tests(pool.clone(), s3.clone(), "aulalite".into()),
        StubAuth {
            pool: pool.clone(),
            user_id: uploader,
            firebase_uid: fb_u,
            email: em_u,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Teacher),
        },
    );
    let (_, body) = fire(
        &app_u,
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
    let asset_id = body["asset_id"].as_str().unwrap().to_string();

    let app_o = build_test_app(
        backend::handlers::uploads::router_for_tests(pool.clone(), s3, "aulalite".into()),
        StubAuth {
            pool: pool.clone(),
            user_id: other,
            firebase_uid: fb_o,
            email: em_o,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Teacher),
        },
    );
    let (status, _) = fire(
        &app_o,
        "POST",
        &format!("/v1/uploads/{asset_id}/complete"),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, 403);
}

#[tokio::test]
async fn complete_is_idempotent() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (user, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, user, "teacher").await;
    let course = course_owned_by(&pool, tenant, user).await;

    let s3 = Arc::new(backend::storage::mock::MockS3Client::new());
    let app = build_test_app(
        backend::handlers::uploads::router_for_tests(pool.clone(), s3.clone(), "aulalite".into()),
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
        "/v1/uploads/begin",
        Some(json!({
            "filename": "x.png", "content_type": "image/png",
            "size_bytes": 1000,
            "linked_entity_type": "course", "linked_entity_id": course,
            "purpose": "cover"
        })),
    )
    .await;
    let asset_id = body["asset_id"].as_str().unwrap().to_string();
    let object_key: String =
        sqlx::query_scalar("SELECT object_key FROM file_assets WHERE id = $1::uuid")
            .bind(&asset_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    s3.simulate_object(&object_key, 1000, "image/png");

    let (s1, b1) = fire(
        &app,
        "POST",
        &format!("/v1/uploads/{asset_id}/complete"),
        Some(json!({})),
    )
    .await;
    assert_eq!(s1, 200);
    assert_eq!(b1["status"], "available");

    let (s2, b2) = fire(
        &app,
        "POST",
        &format!("/v1/uploads/{asset_id}/complete"),
        Some(json!({})),
    )
    .await;
    assert_eq!(s2, 200);
    assert_eq!(b2["status"], "available");
}

#[tokio::test]
async fn begin_accepts_assignment_attachment_type() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let course = course_owned_by(&pool, tenant, teacher).await;

    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&pool)
        .await
        .unwrap();
    let aid: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO assignments
            (tenant_id, course_id, title, grading_mode, max_points,
             status, published_at, created_by)
         VALUES ($1,$2,'A','numeric',100,'published',now(),$3) RETURNING id",
    )
    .bind(tenant)
    .bind(course)
    .bind(teacher)
    .fetch_one(&pool)
    .await
    .unwrap();

    let s3 = Arc::new(backend::storage::mock::MockS3Client::new());
    let app = build_test_app(
        backend::handlers::uploads::router_for_tests(pool.clone(), s3, "aulalite".into()),
        StubAuth {
            pool: pool.clone(),
            user_id: teacher,
            firebase_uid: fb,
            email: em,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Teacher),
        },
    );

    let (status, body) = fire(
        &app,
        "POST",
        "/v1/uploads/begin",
        Some(json!({
            "filename": "rubric.pdf",
            "content_type": "application/pdf",
            "size_bytes": 50_000,
            "linked_entity_type": "assignment_attachment",
            "linked_entity_id": aid,
            "purpose": "attachment"
        })),
    )
    .await;

    assert_eq!(status, 200, "{body}");
    assert!(body["presigned_put_url"]
        .as_str()
        .unwrap()
        .starts_with("https://mock.s3/put/"));
    let asset_id = body["asset_id"].as_str().unwrap();
    let row: (String, String) =
        sqlx::query_as("SELECT status, linked_entity_type FROM file_assets WHERE id = $1::uuid")
            .bind(asset_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(row.0, "pending");
    assert_eq!(row.1, "assignment_attachment");
}

#[tokio::test]
async fn begin_accepts_submission_attachment_for_owner_student() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (student, fb_s, em_s) = create_user(&pool).await;
    attach_membership(&pool, tenant, student, "student").await;
    let course = course_owned_by(&pool, tenant, teacher).await;

    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&pool)
        .await
        .unwrap();
    let aid: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO assignments
            (tenant_id, course_id, title, grading_mode, max_points,
             status, published_at, created_by)
         VALUES ($1,$2,'A','numeric',100,'published',now(),$3) RETURNING id",
    )
    .bind(tenant)
    .bind(course)
    .bind(teacher)
    .fetch_one(&pool)
    .await
    .unwrap();
    let sid: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO submissions
            (tenant_id, assignment_id, course_id, student_user_id, status)
         VALUES ($1,$2,$3,$4,'draft') RETURNING id",
    )
    .bind(tenant)
    .bind(aid)
    .bind(course)
    .bind(student)
    .fetch_one(&pool)
    .await
    .unwrap();

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

    let (status, body) = fire(
        &app,
        "POST",
        "/v1/uploads/begin",
        Some(json!({
            "filename": "answer.pdf",
            "content_type": "application/pdf",
            "size_bytes": 25_000,
            "linked_entity_type": "submission_attachment",
            "linked_entity_id": sid,
            "purpose": "attachment"
        })),
    )
    .await;

    assert_eq!(status, 200, "{body}");
    let asset_id = body["asset_id"].as_str().unwrap();
    let row: (String, String) =
        sqlx::query_as("SELECT status, linked_entity_type FROM file_assets WHERE id = $1::uuid")
            .bind(asset_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(row.0, "pending");
    assert_eq!(row.1, "submission_attachment");
}

#[tokio::test]
async fn begin_object_key_embeds_asset_id() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (user, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, user, "teacher").await;
    let course = course_owned_by(&pool, tenant, user).await;

    let s3 = std::sync::Arc::new(backend::storage::mock::MockS3Client::new());
    let app = build_test_app(
        backend::handlers::uploads::router_for_tests(pool.clone(), s3, "aulalite".into()),
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
        "/v1/uploads/begin",
        Some(serde_json::json!({
            "filename": "x.png", "content_type": "image/png",
            "size_bytes": 1000,
            "linked_entity_type": "course", "linked_entity_id": course,
            "purpose": "cover"
        })),
    )
    .await;
    let asset_id = body["asset_id"].as_str().unwrap().to_string();
    let object_key: String =
        sqlx::query_scalar("SELECT object_key FROM file_assets WHERE id = $1::uuid")
            .bind(&asset_id)
            .fetch_one(&pool)
            .await
            .unwrap();

    // The simple (no-dash) form of asset_id MUST appear in object_key.
    let simple = asset_id.replace('-', "");
    assert!(
        object_key.contains(&simple),
        "asset_id {asset_id} (simple={simple}) not embedded in object_key={object_key}"
    );
}
