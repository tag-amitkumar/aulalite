// crates/backend/tests/uploads_validation_matrix.rs
mod fixtures;

use fixtures::*;
use serde_json::json;
use std::sync::Arc;

async fn course_with_module_and_video_lesson_and_filebundle(
    pool: &sqlx::PgPool,
    tenant: uuid::Uuid,
    user: uuid::Uuid,
) -> (uuid::Uuid, uuid::Uuid, uuid::Uuid) {
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
    let video_lesson: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO lessons (tenant_id, course_id, module_id, type, title, sort_order)
         VALUES ($1, $2, $3, 'video', 'V', 10) RETURNING id",
    )
    .bind(tenant)
    .bind(course)
    .bind(module)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    let fb_lesson: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO lessons (tenant_id, course_id, module_id, type, title, sort_order)
         VALUES ($1, $2, $3, 'file_bundle', 'F', 20) RETURNING id",
    )
    .bind(tenant)
    .bind(course)
    .bind(module)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    (course, video_lesson, fb_lesson)
}

#[tokio::test]
async fn validation_matrix_end_to_end() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (user, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, user, "teacher").await;
    let (course, video_lesson, fb_lesson) =
        course_with_module_and_video_lesson_and_filebundle(&pool, tenant, user).await;

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

    // (purpose, content_type, size_bytes, linked_entity_type, linked_entity_id, expected_status)
    let cases: Vec<(&str, &str, i64, &str, uuid::Uuid, u16)> = vec![
        // cover: pass
        ("cover", "image/jpeg", 1_000_000, "course", course, 200),
        ("cover", "image/png", 5_242_880, "course", course, 200),
        ("cover", "image/webp", 1_000_000, "course", course, 200),
        // cover: fail (svg disallowed)
        ("cover", "image/svg+xml", 1000, "course", course, 400),
        // cover: fail (oversized)
        ("cover", "image/png", 5_242_881, "course", course, 400),
        // video: pass
        (
            "video",
            "video/mp4",
            100_000_000,
            "lesson",
            video_lesson,
            200,
        ),
        (
            "video",
            "video/webm",
            50_000_000,
            "lesson",
            video_lesson,
            200,
        ),
        // video: fail (oversized)
        (
            "video",
            "video/mp4",
            524_288_001,
            "lesson",
            video_lesson,
            400,
        ),
        // video: fail (bad content type)
        ("video", "image/png", 1000, "lesson", video_lesson, 400),
        // attachment: pass
        (
            "attachment",
            "application/pdf",
            50_000_000,
            "lesson",
            fb_lesson,
            200,
        ),
        (
            "attachment",
            "application/zip",
            100_000_000,
            "lesson",
            fb_lesson,
            200,
        ),
        // attachment: fail (executable)
        (
            "attachment",
            "application/x-msdownload",
            1000,
            "lesson",
            fb_lesson,
            400,
        ),
        // attachment: fail (oversized)
        (
            "attachment",
            "application/pdf",
            104_857_601,
            "lesson",
            fb_lesson,
            400,
        ),
    ];

    for (purpose, ct, size, etype, eid, expected) in cases {
        let (status, body) = fire(
            &app,
            "POST",
            "/v1/uploads/begin",
            Some(json!({
                "filename": format!("f-{}.bin", uuid::Uuid::new_v4()),
                "content_type": ct,
                "size_bytes": size,
                "linked_entity_type": etype,
                "linked_entity_id": eid,
                "purpose": purpose
            })),
        )
        .await;
        assert_eq!(
            status.as_u16(),
            expected,
            "case (purpose={purpose}, ct={ct}, size={size}, etype={etype}, eid={eid}): \
             expected {expected}, got {} body {body}",
            status.as_u16()
        );
    }
}
