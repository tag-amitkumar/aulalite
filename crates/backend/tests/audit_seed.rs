use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

mod fixtures;

use backend::auth::local_login::{LocalLoginConfig, LocalLoginProfile};
use backend::handlers::dev_seed::{routes as seed_routes, DevSeedState};

fn config(app_env: &str, enabled: bool) -> LocalLoginConfig {
    LocalLoginConfig::from_profiles(
        app_env,
        enabled,
        vec![
            LocalLoginProfile {
                name: "teacher".to_string(),
                token: "teacher-token".to_string(),
                password: "teacher-pass".to_string(),
                email: "local.teacher@example.test".to_string(),
                display_name: "Local Teacher".to_string(),
                firebase_uid: "local-login-local-teacher".to_string(),
            },
            LocalLoginProfile {
                name: "student".to_string(),
                token: "student-token".to_string(),
                password: "student-pass".to_string(),
                email: "local.student@example.test".to_string(),
                display_name: "Local Student".to_string(),
                firebase_uid: "local-login-local-student".to_string(),
            },
        ],
    )
}

async fn post_seed(pool: PgPool, app_env: &str, seed_enabled: bool) -> (StatusCode, Value) {
    let app = seed_routes(DevSeedState {
        pool,
        config: config(app_env, true),
        enabled: seed_enabled,
    });
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/dev/audit-seed")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    (status, json)
}

async fn set_tenant(pool: &PgPool, tenant_id: Uuid) {
    sqlx::query("SELECT set_config('app.tenant_id', $1, false)")
        .bind(tenant_id.to_string())
        .execute(pool)
        .await
        .unwrap();
}

async fn scalar_i64(pool: &PgPool, sql: &str, course_id: Uuid) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .bind(course_id)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn audit_seed_disabled_in_production() {
    let pool = fixtures::pool().await;
    let app = seed_routes(DevSeedState {
        pool,
        config: config("production", true),
        enabled: true,
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/dev/audit-seed")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn audit_seed_creates_course_for_teacher_and_student() {
    let pool = fixtures::pool().await;

    let (status, json) = post_seed(pool.clone(), "local", true).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["tenant_slug"], "local-audit");
    assert_eq!(json["course_slug"], "audit-course");
    assert_eq!(json["teacher_email"], "local.teacher@example.test");
    assert_eq!(json["student_email"], "local.student@example.test");
    assert!(json["enrollment_code"].as_str().unwrap().len() >= 8);

    let (status, second_json) = post_seed(pool.clone(), "local", true).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(second_json, json);

    let (tenant_id, course_id): (Uuid, Uuid) = sqlx::query_as(
        "SELECT t.id, c.id
         FROM tenants t
         JOIN courses c ON c.tenant_id = t.id
         WHERE t.slug = 'local-audit' AND c.slug = 'audit-course'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    set_tenant(&pool, tenant_id).await;

    let teacher_memberships = scalar_i64(
        &pool,
        "SELECT count(*)
         FROM course_memberships cm
         JOIN users u ON u.id = cm.user_id
         WHERE cm.course_id = $1
           AND cm.role = 'teacher'
           AND u.email = 'local.teacher@example.test'",
        course_id,
    )
    .await;
    assert_eq!(teacher_memberships, 1);

    let student_memberships = scalar_i64(
        &pool,
        "SELECT count(*)
         FROM course_memberships cm
         JOIN users u ON u.id = cm.user_id
         WHERE cm.course_id = $1
           AND cm.role = 'student'
           AND u.email = 'local.student@example.test'",
        course_id,
    )
    .await;
    assert_eq!(student_memberships, 1);

    assert_eq!(
        scalar_i64(
            &pool,
            "SELECT count(*) FROM modules WHERE course_id = $1",
            course_id
        )
        .await,
        1
    );
    assert_eq!(
        scalar_i64(
            &pool,
            "SELECT count(*) FROM lessons WHERE course_id = $1",
            course_id
        )
        .await,
        1
    );
    assert_eq!(
        scalar_i64(
            &pool,
            "SELECT count(*) FROM live_session_series WHERE course_id = $1",
            course_id,
        )
        .await,
        1
    );
    assert_eq!(
        scalar_i64(
            &pool,
            "SELECT count(*) FROM live_sessions WHERE course_id = $1",
            course_id
        )
        .await,
        1
    );
    let session_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM live_sessions WHERE course_id = $1 AND title = 'Audit Live Class'",
    )
    .bind(course_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query(
        "UPDATE live_sessions
            SET status = 'ended',
                actual_started_at = now() - interval '2 hours',
                actual_ended_at = now() - interval '1 hour',
                main_path = 'stale-main',
                screen_path = 'stale-screen',
                publish_nonce = 'stale-nonce',
                publish_nonce_expires_at = now() + interval '1 hour'
          WHERE id = $1",
    )
    .bind(session_id)
    .execute(&pool)
    .await
    .unwrap();

    let (status, _) = post_seed(pool.clone(), "local", true).await;
    assert_eq!(status, StatusCode::OK);
    let reset_session: (
        String,
        Option<chrono::DateTime<chrono::Utc>>,
        Option<chrono::DateTime<chrono::Utc>>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<chrono::DateTime<chrono::Utc>>,
    ) = sqlx::query_as(
        "SELECT status, actual_started_at, actual_ended_at, main_path,
                screen_path, publish_nonce, publish_nonce_expires_at
           FROM live_sessions
          WHERE id = $1",
    )
    .bind(session_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(reset_session.0, "scheduled");
    assert!(reset_session.1.is_none());
    assert!(reset_session.2.is_none());
    assert!(reset_session.3.is_none());
    assert!(reset_session.4.is_none());
    assert!(reset_session.5.is_none());
    assert!(reset_session.6.is_none());

    assert_eq!(
        scalar_i64(
            &pool,
            "SELECT count(*) FROM assignments WHERE course_id = $1",
            course_id
        )
        .await,
        1
    );
    let accepts_files: bool = sqlx::query_scalar(
        "SELECT accepts_files
         FROM assignments
         WHERE course_id = $1 AND title = 'Audit Assignment'",
    )
    .bind(course_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(accepts_files);
    assert_eq!(
        scalar_i64(
            &pool,
            "SELECT count(*) FROM enrollment_codes WHERE course_id = $1",
            course_id,
        )
        .await,
        1
    );
}

#[tokio::test]
async fn starts_at_consistent_between_series_and_session() {
    let pool = fixtures::pool().await;

    let (status, _) = post_seed(pool.clone(), "local", true).await;
    assert_eq!(status, StatusCode::OK);

    let (tenant_id, course_id): (Uuid, Uuid) = sqlx::query_as(
        "SELECT t.id, c.id
         FROM tenants t
         JOIN courses c ON c.tenant_id = t.id
         WHERE t.slug = 'local-audit' AND c.slug = 'audit-course'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    set_tenant(&pool, tenant_id).await;

    let (series_starts_at, session_starts_at): (
        chrono::DateTime<chrono::Utc>,
        chrono::DateTime<chrono::Utc>,
    ) = sqlx::query_as(
        "SELECT s.starts_at, ls.starts_at
           FROM live_session_series s
           JOIN live_sessions ls ON ls.series_id = s.id
          WHERE s.course_id = $1
            AND s.title = 'Audit Live Class'
            AND ls.occurrence_index = 0",
    )
    .bind(course_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(
        series_starts_at, session_starts_at,
        "live_session_series.starts_at must equal live_sessions.starts_at for the seeded row"
    );
}
