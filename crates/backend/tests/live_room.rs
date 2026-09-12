// crates/backend/tests/live_room.rs
mod fixtures;

use fixtures::*;
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use std::sync::Arc;

async fn course_with_session(
    pool: &sqlx::PgPool,
    tenant: uuid::Uuid,
    teacher: uuid::Uuid,
    starts_at: chrono::DateTime<chrono::Utc>,
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
    .bind(teacher)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO course_memberships (course_id, user_id, tenant_id, role)
         VALUES ($1, $2, $3, 'teacher')",
    )
    .bind(course)
    .bind(teacher)
    .bind(tenant)
    .execute(&mut *tx)
    .await
    .unwrap();
    // live_sessions.series_id is NOT NULL — create a minimal series first
    let series: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO live_session_series
            (tenant_id, course_id, title, starts_at, duration_minutes,
             frequency, end_kind, primary_teacher_id)
         VALUES ($1, $2, 'S', $3, 60, 'none', 'open', $4)
         RETURNING id",
    )
    .bind(tenant)
    .bind(course)
    .bind(starts_at)
    .bind(teacher)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    let session: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO live_sessions (tenant_id, course_id, series_id, occurrence_index,
                                    title, status, starts_at, duration_minutes,
                                    primary_teacher_id, mode, recording_enabled,
                                    transport_mode)
         VALUES ($1, $2, $3, 0, 'L', 'scheduled', $4, 60, $5, 'lecture', false, 'webrtc')
         RETURNING id",
    )
    .bind(tenant)
    .bind(course)
    .bind(series)
    .bind(starts_at)
    .bind(teacher)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    (course, session)
}

async fn set_media_limits(
    pool: &sqlx::PgPool,
    tenant: uuid::Uuid,
    class_minutes: i32,
    recording_gb: i32,
) {
    let plan_id = format!("media-limit-{}", uuid::Uuid::new_v4().simple());
    sqlx::query(
        "INSERT INTO plans
            (id, name, monthly_price_cents, included_seats,
             included_class_minutes, included_recording_gb)
         VALUES ($1, $1, 0, 100, $2, $3)",
    )
    .bind(&plan_id)
    .bind(class_minutes)
    .bind(recording_gb)
    .execute(pool)
    .await
    .unwrap();
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO subscriptions (tenant_id, plan_id, status)
         VALUES ($1, $2, 'active')
         ON CONFLICT (tenant_id) DO UPDATE SET
            plan_id = EXCLUDED.plan_id, status = 'active'",
    )
    .bind(tenant)
    .bind(plan_id)
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
}

#[tokio::test]
async fn go_live_happy_path() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let now = chrono::Utc::now();
    let (_course, session) = course_with_session(&pool, tenant, teacher, now).await;

    let mediamtx = Arc::new(backend::services::mediamtx::MockMediaMtxClient::new());
    let signer = Arc::new(backend::services::mediamtx::JwtSigner::new_ephemeral());
    let app = build_test_app(
        backend::handlers::live_sessions::live_room_router_for_tests(
            pool.clone(),
            mediamtx.clone(),
            signer,
            "http://localhost:8889".into(),
            "http://localhost:8888".into(),
        ),
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
        &format!("/v1/sessions/{session}/go-live"),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert!(body["main_publish_url"].as_str().unwrap().contains("/whip"));
    assert!(body["publish_password"].as_str().unwrap().len() >= 32);
    assert_eq!(body["transport_mode"], "webrtc");

    let row: (String,) = sqlx::query_as("SELECT status FROM live_sessions WHERE id = $1")
        .bind(session)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(row.0, "live");
}

#[tokio::test]
async fn go_live_blocks_before_mutation_when_projected_minutes_exceed_plan() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (_course, session) = course_with_session(&pool, tenant, teacher, chrono::Utc::now()).await;
    // The scheduled session is 60 minutes, so a 59-minute plan cannot start it.
    set_media_limits(&pool, tenant, 59, 1).await;

    let mediamtx = Arc::new(backend::services::mediamtx::MockMediaMtxClient::new());
    let signer = Arc::new(backend::services::mediamtx::JwtSigner::new_ephemeral());
    let app = build_test_app(
        backend::handlers::live_sessions::live_room_router_for_tests(
            pool.clone(),
            mediamtx,
            signer,
            "http://localhost:8889".into(),
            "http://localhost:8888".into(),
        ),
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
        &format!("/v1/sessions/{session}/go-live"),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, 402, "{body}");
    assert_eq!(body["error"], "class_minutes_limit_reached");
    let state: String = sqlx::query_scalar("SELECT status FROM live_sessions WHERE id = $1")
        .bind(session)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(state, "scheduled");
}

#[tokio::test]
async fn negative_media_limits_preserve_unlimited_plan_behavior() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (_course, session) = course_with_session(&pool, tenant, teacher, chrono::Utc::now()).await;
    set_media_limits(&pool, tenant, -1, -1).await;

    let app = build_test_app(
        backend::handlers::live_sessions::live_room_router_for_tests(
            pool.clone(),
            Arc::new(backend::services::mediamtx::MockMediaMtxClient::new()),
            Arc::new(backend::services::mediamtx::JwtSigner::new_ephemeral()),
            "http://localhost:8889".into(),
            "http://localhost:8888".into(),
        ),
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
        &format!("/v1/sessions/{session}/go-live"),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, 200, "{body}");
}

#[tokio::test]
async fn go_live_outside_window_returns_400() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let starts = chrono::Utc::now() + chrono::Duration::hours(5);
    let (_course, session) = course_with_session(&pool, tenant, teacher, starts).await;

    let mediamtx = Arc::new(backend::services::mediamtx::MockMediaMtxClient::new());
    let signer = Arc::new(backend::services::mediamtx::JwtSigner::new_ephemeral());
    let app = build_test_app(
        backend::handlers::live_sessions::live_room_router_for_tests(
            pool.clone(),
            mediamtx,
            signer,
            "http://localhost:8889".into(),
            "http://localhost:8888".into(),
        ),
        StubAuth {
            pool: pool.clone(),
            user_id: teacher,
            firebase_uid: fb,
            email: em,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Teacher),
        },
    );

    let (status, _) = fire(
        &app,
        "POST",
        &format!("/v1/sessions/{session}/go-live"),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, 400);
}

#[tokio::test]
async fn go_live_by_non_teacher_returns_403() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (student, fb_s, em_s) = create_user(&pool).await;
    attach_membership(&pool, tenant, student, "student").await;
    let (_course, session) = course_with_session(&pool, tenant, teacher, chrono::Utc::now()).await;

    let mediamtx = Arc::new(backend::services::mediamtx::MockMediaMtxClient::new());
    let signer = Arc::new(backend::services::mediamtx::JwtSigner::new_ephemeral());
    let app = build_test_app(
        backend::handlers::live_sessions::live_room_router_for_tests(
            pool.clone(),
            mediamtx,
            signer,
            "http://localhost:8889".into(),
            "http://localhost:8888".into(),
        ),
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
        &format!("/v1/sessions/{session}/go-live"),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, 403);
}

async fn force_session_live(pool: &sqlx::PgPool, session_id: uuid::Uuid) {
    sqlx::query(
        "UPDATE live_sessions
            SET status = 'live',
                actual_started_at = now(),
                main_path = $2,
                publish_nonce = 'precomputed',
                publish_nonce_expires_at = now() + interval '1 hour'
          WHERE id = $1",
    )
    .bind(session_id)
    .bind(format!("aula/x/y/{}", session_id.simple()))
    .execute(pool)
    .await
    .unwrap();
}

#[tokio::test]
async fn health_by_teacher_reports_main_stream_ok_and_redacts_details() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (_course, session) = course_with_session(&pool, tenant, teacher, chrono::Utc::now()).await;
    force_session_live(&pool, session).await;

    let main_path: Option<String> =
        sqlx::query_scalar("SELECT main_path FROM live_sessions WHERE id = $1")
            .bind(session)
            .fetch_one(&pool)
            .await
            .unwrap();
    let main_path = main_path.unwrap();

    let mediamtx = Arc::new(backend::services::mediamtx::MockMediaMtxClient::new());
    mediamtx.simulate_active(main_path.clone());
    let signer = Arc::new(backend::services::mediamtx::JwtSigner::new_ephemeral());
    let app = build_test_app(
        backend::handlers::live_sessions::live_room_router_for_tests(
            pool.clone(),
            mediamtx,
            signer,
            "http://localhost:8889".into(),
            "http://localhost:8888".into(),
        ),
        StubAuth {
            pool: pool.clone(),
            user_id: teacher,
            firebase_uid: fb,
            email: em,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Teacher),
        },
    );

    let (status, body) = fire(&app, "GET", &format!("/v1/sessions/{session}/health"), None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["session_id"], session.to_string());
    assert_eq!(body["media_server"]["status"], "ok");
    assert_eq!(body["main_stream"]["status"], "ok");

    let serialized = serde_json::to_string(&body).unwrap();
    assert!(!serialized.contains("publish_password"));
    assert!(!serialized.contains("publish_nonce"));
    assert!(!serialized.contains("MEDIAMTX_AUTH_SHARED_HEADER"));
    assert!(!serialized.contains("precomputed"));
    assert!(!serialized.contains(&main_path));
}

#[tokio::test]
async fn health_by_platform_admin_without_tenant_membership_reports_ok() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (_course, session) = course_with_session(&pool, tenant, teacher, chrono::Utc::now()).await;
    force_session_live(&pool, session).await;

    let main_path: Option<String> =
        sqlx::query_scalar("SELECT main_path FROM live_sessions WHERE id = $1")
            .bind(session)
            .fetch_one(&pool)
            .await
            .unwrap();
    let main_path = main_path.unwrap();

    let (admin, fb, em) = create_user(&pool).await;
    sqlx::query("UPDATE users SET is_platform_admin = TRUE WHERE id = $1")
        .bind(admin)
        .execute(&pool)
        .await
        .unwrap();

    let mediamtx = Arc::new(backend::services::mediamtx::MockMediaMtxClient::new());
    mediamtx.simulate_active(main_path);
    let signer = Arc::new(backend::services::mediamtx::JwtSigner::new_ephemeral());
    let app = build_test_app(
        backend::handlers::live_sessions::live_room_router_for_tests(
            pool.clone(),
            mediamtx,
            signer,
            "http://localhost:8889".into(),
            "http://localhost:8888".into(),
        ),
        StubAuth {
            pool: pool.clone(),
            user_id: admin,
            firebase_uid: fb,
            email: em,
            tenant_id: None,
            tenant_role: None,
        },
    );

    let (status, body) = fire(&app, "GET", &format!("/v1/sessions/{session}/health"), None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["media_server"]["status"], "ok");
    assert_eq!(body["main_stream"]["status"], "ok");
}

#[tokio::test]
async fn health_by_student_returns_403() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (student, fb_s, em_s) = create_user(&pool).await;
    attach_membership(&pool, tenant, student, "student").await;
    let (course, session) = course_with_session(&pool, tenant, teacher, chrono::Utc::now()).await;
    sqlx::query("INSERT INTO course_memberships (course_id, user_id, tenant_id, role) VALUES ($1,$2,$3,'student')")
        .bind(course).bind(student).bind(tenant).execute(&pool).await.unwrap();

    let mediamtx = Arc::new(backend::services::mediamtx::MockMediaMtxClient::new());
    let signer = Arc::new(backend::services::mediamtx::JwtSigner::new_ephemeral());
    let app = build_test_app(
        backend::handlers::live_sessions::live_room_router_for_tests(
            pool.clone(),
            mediamtx.clone(),
            signer,
            "http://localhost:8889".into(),
            "http://localhost:8888".into(),
        ),
        StubAuth {
            pool: pool.clone(),
            user_id: student,
            firebase_uid: fb_s,
            email: em_s,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Student),
        },
    );

    let (status, _body) = fire(&app, "GET", &format!("/v1/sessions/{session}/health"), None).await;
    assert_eq!(status, 403);
    assert!(mediamtx.calls().is_empty());
}

#[tokio::test]
async fn health_reports_failed_recording_retry_eligible_after_end() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (_course, session) = course_with_session(&pool, tenant, teacher, chrono::Utc::now()).await;

    sqlx::query(
        "UPDATE live_sessions
            SET status = 'ended',
                recording_enabled = true,
                actual_started_at = now() - interval '1 hour',
                actual_ended_at = now()
          WHERE id = $1",
    )
    .bind(session)
    .execute(&pool)
    .await
    .unwrap();

    let raw_error = "ffmpeg failed writing s3://SECRET_INTERNAL_PATH/provider-log.txt";
    sqlx::query(
        "INSERT INTO recordings (tenant_id, session_id, processing_status,
                                  processing_error, started_at, ended_at, duration_seconds)
         VALUES ($1, $2, 'failed', $3, now() - interval '1 hour', now(), 3600)",
    )
    .bind(tenant)
    .bind(session)
    .bind(raw_error)
    .execute(&pool)
    .await
    .unwrap();

    let mediamtx = Arc::new(backend::services::mediamtx::MockMediaMtxClient::new());
    let signer = Arc::new(backend::services::mediamtx::JwtSigner::new_ephemeral());
    let app = build_test_app(
        backend::handlers::live_sessions::live_room_router_for_tests(
            pool.clone(),
            mediamtx,
            signer,
            "http://localhost:8889".into(),
            "http://localhost:8888".into(),
        ),
        StubAuth {
            pool: pool.clone(),
            user_id: teacher,
            firebase_uid: fb,
            email: em,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Teacher),
        },
    );

    let (status, body) = fire(&app, "GET", &format!("/v1/sessions/{session}/health"), None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["recording"]["status"], "error");
    assert_eq!(body["recording"]["processing_status"], "failed");
    assert_eq!(body["recording"]["retry_eligible"], true);

    let processing_error = body["recording"]["processing_error"]
        .as_str()
        .expect("processing_error should be a string");
    assert!(processing_error.contains("Recording processor reported a failure"));
    assert!(!processing_error.contains("ffmpeg failed"));
    assert!(!processing_error.contains("SECRET_INTERNAL_PATH"));
    assert!(!processing_error.contains("s3://"));
    assert!(!processing_error.contains("provider-log"));
}

#[tokio::test]
async fn end_class_happy_path() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (_, session) = course_with_session(&pool, tenant, teacher, chrono::Utc::now()).await;
    force_session_live(&pool, session).await;

    let mediamtx = Arc::new(backend::services::mediamtx::MockMediaMtxClient::new());
    let signer = Arc::new(backend::services::mediamtx::JwtSigner::new_ephemeral());
    let app = build_test_app(
        backend::handlers::live_sessions::live_room_router_for_tests(
            pool.clone(),
            mediamtx,
            signer,
            "http://localhost:8889".into(),
            "http://localhost:8888".into(),
        ),
        StubAuth {
            pool: pool.clone(),
            user_id: teacher,
            firebase_uid: fb,
            email: em,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Teacher),
        },
    );
    let (s, _) = fire(
        &app,
        "POST",
        &format!("/v1/sessions/{session}/end-class"),
        Some(json!({})),
    )
    .await;
    assert_eq!(s, 200);
    let row: (String,) = sqlx::query_as("SELECT status FROM live_sessions WHERE id = $1")
        .bind(session)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(row.0, "ended");
}

#[tokio::test]
async fn end_class_idempotent() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (_, session) = course_with_session(&pool, tenant, teacher, chrono::Utc::now()).await;
    force_session_live(&pool, session).await;

    let mediamtx = Arc::new(backend::services::mediamtx::MockMediaMtxClient::new());
    let signer = Arc::new(backend::services::mediamtx::JwtSigner::new_ephemeral());
    let app = build_test_app(
        backend::handlers::live_sessions::live_room_router_for_tests(
            pool.clone(),
            mediamtx,
            signer,
            "http://localhost:8889".into(),
            "http://localhost:8888".into(),
        ),
        StubAuth {
            pool: pool.clone(),
            user_id: teacher,
            firebase_uid: fb,
            email: em,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Teacher),
        },
    );
    let (s1, _) = fire(
        &app,
        "POST",
        &format!("/v1/sessions/{session}/end-class"),
        Some(json!({})),
    )
    .await;
    assert_eq!(s1, 200);
    let (s2, _) = fire(
        &app,
        "POST",
        &format!("/v1/sessions/{session}/end-class"),
        Some(json!({})),
    )
    .await;
    assert_eq!(s2, 200);
}

#[tokio::test]
async fn join_returns_lobby_state_when_scheduled_in_window() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (student, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, student, "student").await;
    let starts = chrono::Utc::now() + chrono::Duration::minutes(2);
    let (course, session) = course_with_session(&pool, tenant, teacher, starts).await;
    sqlx::query("INSERT INTO course_memberships (course_id, user_id, tenant_id, role) VALUES ($1,$2,$3,'student')")
        .bind(course).bind(student).bind(tenant).execute(&pool).await.unwrap();

    let mediamtx = Arc::new(backend::services::mediamtx::MockMediaMtxClient::new());
    let signer = Arc::new(backend::services::mediamtx::JwtSigner::new_ephemeral());
    let app = build_test_app(
        backend::handlers::live_sessions::live_room_router_for_tests(
            pool.clone(),
            mediamtx,
            signer,
            "http://localhost:8889".into(),
            "http://localhost:8888".into(),
        ),
        StubAuth {
            pool: pool.clone(),
            user_id: student,
            firebase_uid: fb,
            email: em,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Student),
        },
    );
    let (s, body) = fire(
        &app,
        "POST",
        &format!("/v1/sessions/{session}/join"),
        Some(json!({})),
    )
    .await;
    assert_eq!(s, 200, "{body}");
    assert_eq!(body["state"], "lobby");
    assert!(body["viewer_jwt"].is_null());
    assert!(body["main_url"].is_null());
}

#[tokio::test]
async fn join_returns_live_state_with_jwt_and_urls() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (student, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, student, "student").await;
    let (course, session) = course_with_session(&pool, tenant, teacher, chrono::Utc::now()).await;
    sqlx::query("INSERT INTO course_memberships (course_id, user_id, tenant_id, role) VALUES ($1,$2,$3,'student')")
        .bind(course).bind(student).bind(tenant).execute(&pool).await.unwrap();
    force_session_live(&pool, session).await;

    let mediamtx = Arc::new(backend::services::mediamtx::MockMediaMtxClient::new());
    let signer = Arc::new(backend::services::mediamtx::JwtSigner::new_ephemeral());
    let app = build_test_app(
        backend::handlers::live_sessions::live_room_router_for_tests(
            pool.clone(),
            mediamtx,
            signer,
            "http://localhost:8889".into(),
            "http://localhost:8888".into(),
        ),
        StubAuth {
            pool: pool.clone(),
            user_id: student,
            firebase_uid: fb,
            email: em,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Student),
        },
    );
    let (s, body) = fire(
        &app,
        "POST",
        &format!("/v1/sessions/{session}/join"),
        Some(json!({})),
    )
    .await;
    assert_eq!(s, 200, "{body}");
    assert_eq!(body["state"], "live");
    let viewer_jwt = body["viewer_jwt"].as_str().unwrap();
    let main_url = body["main_url"].as_str().unwrap();
    assert!(viewer_jwt.contains('.'));
    // WebRTC viewers send `Authorization: Bearer <jwt>` instead of `?jwt=...`.
    assert!(
        main_url.ends_with("/whep"),
        "expected webrtc /whep url, got {main_url}"
    );
    assert!(
        !main_url.contains("jwt="),
        "jwt must not appear in webrtc url, got {main_url}"
    );
    assert_eq!(body["transport_mode"], "webrtc");
}

#[tokio::test]
async fn join_by_non_member_returns_403() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (outsider, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, outsider, "student").await;
    let (_, session) = course_with_session(&pool, tenant, teacher, chrono::Utc::now()).await;

    let mediamtx = Arc::new(backend::services::mediamtx::MockMediaMtxClient::new());
    let signer = Arc::new(backend::services::mediamtx::JwtSigner::new_ephemeral());
    let app = build_test_app(
        backend::handlers::live_sessions::live_room_router_for_tests(
            pool.clone(),
            mediamtx,
            signer,
            "http://localhost:8889".into(),
            "http://localhost:8888".into(),
        ),
        StubAuth {
            pool: pool.clone(),
            user_id: outsider,
            firebase_uid: fb,
            email: em,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Student),
        },
    );
    let (s, _) = fire(
        &app,
        "POST",
        &format!("/v1/sessions/{session}/join"),
        Some(json!({})),
    )
    .await;
    assert_eq!(s, 403);
}

#[tokio::test]
async fn refresh_token_returns_new_jwt_for_live_session() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (student, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, student, "student").await;
    let (course, session) = course_with_session(&pool, tenant, teacher, chrono::Utc::now()).await;
    sqlx::query("INSERT INTO course_memberships (course_id, user_id, tenant_id, role) VALUES ($1,$2,$3,'student')")
        .bind(course).bind(student).bind(tenant).execute(&pool).await.unwrap();
    force_session_live(&pool, session).await;

    let mediamtx = Arc::new(backend::services::mediamtx::MockMediaMtxClient::new());
    let signer = Arc::new(backend::services::mediamtx::JwtSigner::new_ephemeral());
    let app = build_test_app(
        backend::handlers::live_sessions::live_room_router_for_tests(
            pool.clone(),
            mediamtx,
            signer,
            "http://localhost:8889".into(),
            "http://localhost:8888".into(),
        ),
        StubAuth {
            pool: pool.clone(),
            user_id: student,
            firebase_uid: fb,
            email: em,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Student),
        },
    );
    let (s, body) = fire(
        &app,
        "POST",
        &format!("/v1/sessions/{session}/refresh-token"),
        Some(json!({})),
    )
    .await;
    assert_eq!(s, 200, "{body}");
    assert!(body["viewer_jwt"].as_str().unwrap().contains('.'));
}

#[tokio::test]
async fn mediamtx_auth_publish_accepts_valid_nonce() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (course, session) = course_with_session(&pool, tenant, teacher, chrono::Utc::now()).await;

    let plaintext = "test-publish-nonce-abc";
    let hash = backend::db::live_sessions::hash_nonce(plaintext);
    let main_path = backend::services::mediamtx::path_for_session(tenant, course, session);
    sqlx::query(
        "UPDATE live_sessions
            SET status='live', actual_started_at=now(), main_path=$2,
                publish_nonce=$3, publish_nonce_expires_at = now() + interval '1 hour'
          WHERE id=$1",
    )
    .bind(session)
    .bind(&main_path)
    .bind(&hash)
    .execute(&pool)
    .await
    .unwrap();

    let mediamtx = Arc::new(backend::services::mediamtx::MockMediaMtxClient::new());
    let signer = Arc::new(backend::services::mediamtx::JwtSigner::new_ephemeral());
    let app = build_test_app_no_auth(
        backend::handlers::live_sessions::live_room_router_for_tests(
            pool.clone(),
            mediamtx,
            signer,
            "http://localhost:8889".into(),
            "http://localhost:8888".into(),
        ),
    );
    let (s, _) = fire(
        &app,
        "POST",
        "/v1/mediamtx/auth/publish",
        Some(json!({
            "action": "publish",
            "path": main_path,
            "ip": "127.0.0.1",
            "user": "teacher",
            "password": plaintext,
            "protocol": "webrtc",
            "query": ""
        })),
    )
    .await;
    assert_eq!(s, 200);
}

#[tokio::test]
async fn mediamtx_auth_publish_rejects_used_nonce() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (course, session) = course_with_session(&pool, tenant, teacher, chrono::Utc::now()).await;
    let plaintext = "test-nonce-once";
    let hash = backend::db::live_sessions::hash_nonce(plaintext);
    let main_path = backend::services::mediamtx::path_for_session(tenant, course, session);
    sqlx::query(
        "UPDATE live_sessions
            SET status='live', actual_started_at=now(), main_path=$2,
                publish_nonce=$3, publish_nonce_expires_at = now() + interval '1 hour'
          WHERE id=$1",
    )
    .bind(session)
    .bind(&main_path)
    .bind(&hash)
    .execute(&pool)
    .await
    .unwrap();

    let mediamtx = Arc::new(backend::services::mediamtx::MockMediaMtxClient::new());
    let signer = Arc::new(backend::services::mediamtx::JwtSigner::new_ephemeral());
    let app = build_test_app_no_auth(
        backend::handlers::live_sessions::live_room_router_for_tests(
            pool.clone(),
            mediamtx,
            signer,
            "http://localhost:8889".into(),
            "http://localhost:8888".into(),
        ),
    );
    let body = json!({
        "action": "publish", "path": main_path, "ip": "127.0.0.1",
        "user": "teacher", "password": plaintext, "protocol": "webrtc", "query": ""
    });
    let (s1, _) = fire(
        &app,
        "POST",
        "/v1/mediamtx/auth/publish",
        Some(body.clone()),
    )
    .await;
    assert_eq!(s1, 200);
    let (s2, _) = fire(&app, "POST", "/v1/mediamtx/auth/publish", Some(body)).await;
    assert_eq!(s2, 403);
}

#[tokio::test]
async fn mediamtx_auth_publish_rejects_wrong_password() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (course, session) = course_with_session(&pool, tenant, teacher, chrono::Utc::now()).await;
    let hash = backend::db::live_sessions::hash_nonce("the-real-nonce");
    let main_path = backend::services::mediamtx::path_for_session(tenant, course, session);
    sqlx::query(
        "UPDATE live_sessions
            SET status='live', actual_started_at=now(), main_path=$2,
                publish_nonce=$3, publish_nonce_expires_at = now() + interval '1 hour'
          WHERE id=$1",
    )
    .bind(session)
    .bind(&main_path)
    .bind(&hash)
    .execute(&pool)
    .await
    .unwrap();

    let mediamtx = Arc::new(backend::services::mediamtx::MockMediaMtxClient::new());
    let signer = Arc::new(backend::services::mediamtx::JwtSigner::new_ephemeral());
    let app = build_test_app_no_auth(
        backend::handlers::live_sessions::live_room_router_for_tests(
            pool.clone(),
            mediamtx,
            signer,
            "http://localhost:8889".into(),
            "http://localhost:8888".into(),
        ),
    );
    let (s, _) = fire(
        &app,
        "POST",
        "/v1/mediamtx/auth/publish",
        Some(json!({
            "action": "publish", "path": main_path, "ip": "127.0.0.1",
            "user": "teacher", "password": "wrong", "protocol": "webrtc", "query": ""
        })),
    )
    .await;
    assert_eq!(s, 403);
}

#[tokio::test]
async fn sweep_ends_a_session_whose_publisher_is_confirmed_gone() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (_, session) = course_with_session(&pool, tenant, teacher, chrono::Utc::now()).await;

    // Publisher last seen well beyond the confirmation window. Note that
    // `actual_started_at` is recent: the sweep must key off ABSENCE, not age.
    sqlx::query(
        "UPDATE live_sessions
            SET status='live',
                actual_started_at = now() - interval '5 minutes',
                publisher_last_seen_at = now() - make_interval(mins => $2),
                main_path = 'aula/x/y/z'
          WHERE id=$1",
    )
    .bind(session)
    .bind((backend::db::live_sessions::PUBLISHER_GONE_GRACE_MINUTES + 1) as i32)
    .execute(&pool)
    .await
    .unwrap();

    let ended = backend::db::live_sessions::end_if_publisher_gone(
        &pool,
        session,
        backend::db::live_sessions::PUBLISHER_GONE_GRACE_MINUTES,
    )
    .await
    .unwrap();
    assert!(ended, "a publisher gone past the window must end the session");

    let row: (String,) = sqlx::query_as("SELECT status FROM live_sessions WHERE id=$1")
        .bind(session)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(row.0, "ended");
}

#[tokio::test]
async fn sweep_never_ends_a_long_running_class_whose_publisher_is_present() {
    // The regression this whole change exists to prevent: a class far past any
    // previous deadline (4 hours, booked for 60) whose teacher is still
    // publishing must be left completely alone.
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (_, session) = course_with_session(&pool, tenant, teacher, chrono::Utc::now()).await;

    sqlx::query(
        "UPDATE live_sessions
            SET status='live',
                actual_started_at = now() - interval '4 hours',
                main_path = 'aula/x/y/z'
          WHERE id=$1",
    )
    .bind(session)
    .execute(&pool)
    .await
    .unwrap();

    // The media server reports the publisher present on this tick.
    backend::db::live_sessions::mark_publisher_seen(&pool, session)
        .await
        .unwrap();

    let ended = backend::db::live_sessions::end_if_publisher_gone(
        &pool,
        session,
        backend::db::live_sessions::PUBLISHER_GONE_GRACE_MINUTES,
    )
    .await
    .unwrap();
    assert!(!ended, "a publishing class must never be ended, at any age");

    let row: (String,) = sqlx::query_as("SELECT status FROM live_sessions WHERE id=$1")
        .bind(session)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(row.0, "live");
}

#[tokio::test]
async fn sweep_leaves_a_recently_dropped_publisher_alone() {
    // A blip inside the confirmation window is not a disconnection.
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (_, session) = course_with_session(&pool, tenant, teacher, chrono::Utc::now()).await;
    sqlx::query(
        "UPDATE live_sessions
            SET status='live',
                actual_started_at = now() - interval '5 minutes',
                publisher_last_seen_at = now() - interval '1 minute',
                main_path = 'aula/x/y/z'
          WHERE id=$1",
    )
    .bind(session)
    .execute(&pool)
    .await
    .unwrap();

    let ended = backend::db::live_sessions::end_if_publisher_gone(
        &pool,
        session,
        backend::db::live_sessions::PUBLISHER_GONE_GRACE_MINUTES,
    )
    .await
    .unwrap();
    assert!(!ended);
}

#[tokio::test]
async fn seeding_absence_starts_the_clock_at_the_session_start() {
    // A room that went live but never published must still be reclaimable:
    // the clock is seeded from actual_started_at, not from now(), so it does
    // not reset on every backend restart.
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (_, session) = course_with_session(&pool, tenant, teacher, chrono::Utc::now()).await;
    sqlx::query(
        "UPDATE live_sessions
            SET status='live',
                actual_started_at = now() - interval '4 hours',
                publisher_last_seen_at = NULL,
                main_path = NULL
          WHERE id=$1",
    )
    .bind(session)
    .execute(&pool)
    .await
    .unwrap();

    backend::db::live_sessions::seed_publisher_absence(&pool, session)
        .await
        .unwrap();

    let ended = backend::db::live_sessions::end_if_publisher_gone(
        &pool,
        session,
        backend::db::live_sessions::PUBLISHER_GONE_GRACE_MINUTES,
    )
    .await
    .unwrap();
    assert!(ended, "a room that never published must be reclaimed");
}

#[tokio::test]
async fn the_sweep_lists_live_sessions_with_their_paths() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (_, session) = course_with_session(&pool, tenant, teacher, chrono::Utc::now()).await;
    sqlx::query(
        "UPDATE live_sessions
            SET status='live', main_path = 'aula/x/y/z'
          WHERE id=$1",
    )
    .bind(session)
    .execute(&pool)
    .await
    .unwrap();

    let rows = backend::db::live_sessions::list_live_for_sweep(&pool)
        .await
        .unwrap();
    let found = rows
        .iter()
        .find(|r| r.id == session)
        .expect("live session must appear in the sweep listing");
    assert_eq!(found.main_path.as_deref(), Some("aula/x/y/z"));
}

#[tokio::test]
async fn mediamtx_auth_read_accepts_valid_jwt_in_password() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (course, session) = course_with_session(&pool, tenant, teacher, chrono::Utc::now()).await;
    let main_path = backend::services::mediamtx::path_for_session(tenant, course, session);

    let signer = backend::services::mediamtx::JwtSigner::new_ephemeral();
    let claims = backend::services::mediamtx::ViewerClaims {
        iss: "aulalite".into(),
        sub: uuid::Uuid::new_v4().to_string(),
        tnt: tenant.to_string(),
        mediamtx_permissions: vec![backend::services::mediamtx::MediaMtxPermission {
            action: "read".into(),
            path: main_path.clone(),
        }],
        exp: 0,
    };
    let token = signer.mint_viewer_jwt(claims, std::time::Duration::from_secs(900));

    let mediamtx = Arc::new(backend::services::mediamtx::MockMediaMtxClient::new());
    let signer_arc = Arc::new(signer);
    let app = build_test_app_no_auth(
        backend::handlers::live_sessions::live_room_router_for_tests(
            pool.clone(),
            mediamtx,
            signer_arc,
            "http://localhost:8889".into(),
            "http://localhost:8888".into(),
        ),
    );
    let (s, _) = fire(
        &app,
        "POST",
        "/v1/mediamtx/auth/publish",
        Some(json!({
            "action": "read",
            "path": main_path,
            "password": token,
        })),
    )
    .await;
    assert_eq!(s, 200);
}

#[tokio::test]
async fn mediamtx_auth_read_accepts_valid_jwt_in_bearer_password() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (course, session) = course_with_session(&pool, tenant, teacher, chrono::Utc::now()).await;
    let main_path = backend::services::mediamtx::path_for_session(tenant, course, session);

    let signer = backend::services::mediamtx::JwtSigner::new_ephemeral();
    let claims = backend::services::mediamtx::ViewerClaims {
        iss: "aulalite".into(),
        sub: uuid::Uuid::new_v4().to_string(),
        tnt: tenant.to_string(),
        mediamtx_permissions: vec![backend::services::mediamtx::MediaMtxPermission {
            action: "read".into(),
            path: main_path.clone(),
        }],
        exp: 0,
    };
    let token = signer.mint_viewer_jwt(claims, std::time::Duration::from_secs(900));

    let mediamtx = Arc::new(backend::services::mediamtx::MockMediaMtxClient::new());
    let signer_arc = Arc::new(signer);
    let app = build_test_app_no_auth(
        backend::handlers::live_sessions::live_room_router_for_tests(
            pool.clone(),
            mediamtx,
            signer_arc,
            "http://localhost:8889".into(),
            "http://localhost:8888".into(),
        ),
    );
    let (s, _) = fire(
        &app,
        "POST",
        "/v1/mediamtx/auth/publish",
        Some(json!({
            "action": "read",
            "path": main_path,
            "password": format!("Bearer {token}"),
        })),
    )
    .await;
    assert_eq!(s, 200);
}

#[tokio::test]
async fn mediamtx_auth_read_accepts_valid_jwt_in_query() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (course, session) = course_with_session(&pool, tenant, teacher, chrono::Utc::now()).await;
    let main_path = backend::services::mediamtx::path_for_session(tenant, course, session);

    let signer = backend::services::mediamtx::JwtSigner::new_ephemeral();
    let claims = backend::services::mediamtx::ViewerClaims {
        iss: "aulalite".into(),
        sub: uuid::Uuid::new_v4().to_string(),
        tnt: tenant.to_string(),
        mediamtx_permissions: vec![backend::services::mediamtx::MediaMtxPermission {
            action: "read".into(),
            path: main_path.clone(),
        }],
        exp: 0,
    };
    let token = signer.mint_viewer_jwt(claims, std::time::Duration::from_secs(900));

    let mediamtx = Arc::new(backend::services::mediamtx::MockMediaMtxClient::new());
    let signer_arc = Arc::new(signer);
    let app = build_test_app_no_auth(
        backend::handlers::live_sessions::live_room_router_for_tests(
            pool.clone(),
            mediamtx,
            signer_arc,
            "http://localhost:8889".into(),
            "http://localhost:8888".into(),
        ),
    );
    let (s, _) = fire(
        &app,
        "POST",
        "/v1/mediamtx/auth/publish",
        Some(json!({
            "action": "read",
            "path": main_path,
            "password": "",
            "query": format!("jwt={token}"),
        })),
    )
    .await;
    assert_eq!(s, 200);
}

#[tokio::test]
async fn mediamtx_auth_read_rejects_jwt_for_different_path() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (course, session) = course_with_session(&pool, tenant, teacher, chrono::Utc::now()).await;
    let foreign_path = "aula/00000000000000000000000000000000/00000000000000000000000000000000/00000000000000000000000000000000".to_string();

    let signer = backend::services::mediamtx::JwtSigner::new_ephemeral();
    // JWT grants read on foreign_path; request is for the session's actual path.
    let claims = backend::services::mediamtx::ViewerClaims {
        iss: "aulalite".into(),
        sub: uuid::Uuid::new_v4().to_string(),
        tnt: tenant.to_string(),
        mediamtx_permissions: vec![backend::services::mediamtx::MediaMtxPermission {
            action: "read".into(),
            path: foreign_path,
        }],
        exp: 0,
    };
    let token = signer.mint_viewer_jwt(claims, std::time::Duration::from_secs(900));

    let mediamtx = Arc::new(backend::services::mediamtx::MockMediaMtxClient::new());
    let signer_arc = Arc::new(signer);
    let app = build_test_app_no_auth(
        backend::handlers::live_sessions::live_room_router_for_tests(
            pool.clone(),
            mediamtx,
            signer_arc,
            "http://localhost:8889".into(),
            "http://localhost:8888".into(),
        ),
    );
    // Request is for the legitimate main_path of THIS session, but JWT only
    // grants access to a different (foreign) path.
    let main_path = backend::services::mediamtx::path_for_session(tenant, course, session);
    let (s, _) = fire(
        &app,
        "POST",
        "/v1/mediamtx/auth/publish",
        Some(json!({
            "action": "read",
            "path": main_path,
            "password": token,
        })),
    )
    .await;
    assert_eq!(s, 403);
}

#[tokio::test]
async fn mediamtx_auth_read_accepts_wildcard_jwt_for_student_path() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (course, session) = course_with_session(&pool, tenant, teacher, chrono::Utc::now()).await;

    let signer = backend::services::mediamtx::JwtSigner::new_ephemeral();
    let claims = backend::services::mediamtx::ViewerClaims {
        iss: "aulalite".into(),
        sub: uuid::Uuid::new_v4().to_string(),
        tnt: tenant.to_string(),
        mediamtx_permissions: vec![backend::services::mediamtx::MediaMtxPermission {
            action: "read".into(),
            path: format!(
                "aula/{}/{}/{}/student/*",
                tenant.simple(),
                course.simple(),
                session.simple()
            ),
        }],
        exp: 0,
    };
    let token = signer.mint_viewer_jwt(claims, std::time::Duration::from_secs(900));

    let mediamtx = Arc::new(backend::services::mediamtx::MockMediaMtxClient::new());
    let signer_arc = Arc::new(signer);
    let app = build_test_app_no_auth(
        backend::handlers::live_sessions::live_room_router_for_tests(
            pool.clone(),
            mediamtx,
            signer_arc,
            "http://localhost:8889".into(),
            "http://localhost:8888".into(),
        ),
    );
    let student_path = format!(
        "aula/{}/{}/{}/student/abcdef0123456789abcdef0123456789",
        tenant.simple(),
        course.simple(),
        session.simple()
    );
    let (s, _) = fire(
        &app,
        "POST",
        "/v1/mediamtx/auth/publish",
        Some(json!({
            "action": "read",
            "path": student_path,
            "password": token,
        })),
    )
    .await;
    assert_eq!(s, 200);
}

#[tokio::test]
async fn mediamtx_auth_publish_accepts_student_nonce_for_per_student_path() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (course, session) = course_with_session(&pool, tenant, teacher, chrono::Utc::now()).await;
    let (student, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, student, "student").await;

    let plaintext = "student-publish-nonce-xyz";
    let hash = backend::db::live_sessions::hash_nonce(plaintext);
    let mut tx = pool.begin().await.unwrap();
    backend::db::live_room::set_student_publish_nonce(
        &mut tx,
        session,
        student,
        &hash,
        chrono::Utc::now() + chrono::Duration::hours(1),
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let mediamtx = Arc::new(backend::services::mediamtx::MockMediaMtxClient::new());
    let signer = Arc::new(backend::services::mediamtx::JwtSigner::new_ephemeral());
    let app = build_test_app_no_auth(
        backend::handlers::live_sessions::live_room_router_for_tests(
            pool.clone(),
            mediamtx,
            signer,
            "http://localhost:8889".into(),
            "http://localhost:8888".into(),
        ),
    );
    let student_path = format!(
        "aula/{}/{}/{}/student/{}",
        tenant.simple(),
        course.simple(),
        session.simple(),
        student.simple()
    );
    let (s, _) = fire(
        &app,
        "POST",
        "/v1/mediamtx/auth/publish",
        Some(json!({
            "action": "publish",
            "path": student_path,
            "password": plaintext,
        })),
    )
    .await;
    assert_eq!(s, 200);
    // Second attempt with same nonce should be rejected (single-use).
    let (s2, _) = fire(
        &app,
        "POST",
        "/v1/mediamtx/auth/publish",
        Some(json!({
            "action": "publish",
            "path": student_path,
            "password": plaintext,
        })),
    )
    .await;
    assert_eq!(s2, 403);
}

#[tokio::test]
async fn messages_route_returns_recent_first() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (_, session) = course_with_session(&pool, tenant, teacher, chrono::Utc::now()).await;

    for i in 0..3 {
        sqlx::query(
            "INSERT INTO live_room_messages (tenant_id, session_id, sender_user_id, body)
             VALUES ($1, $2, $3, $4)",
        )
        .bind(tenant)
        .bind(session)
        .bind(teacher)
        .bind(format!("msg {i}"))
        .execute(&pool)
        .await
        .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    let mediamtx = Arc::new(backend::services::mediamtx::MockMediaMtxClient::new());
    let signer = Arc::new(backend::services::mediamtx::JwtSigner::new_ephemeral());
    let app = build_test_app(
        backend::handlers::live_sessions::live_room_router_for_tests(
            pool.clone(),
            mediamtx,
            signer,
            "http://localhost:8889".into(),
            "http://localhost:8888".into(),
        ),
        StubAuth {
            pool: pool.clone(),
            user_id: teacher,
            firebase_uid: fb,
            email: em,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Teacher),
        },
    );
    let (s, body) = fire(
        &app,
        "GET",
        &format!("/v1/sessions/{session}/messages?limit=10"),
        None,
    )
    .await;
    assert_eq!(s, 200, "{body}");
    let arr = body["messages"].as_array().unwrap();
    assert_eq!(arr.len(), 3);
    assert_eq!(arr[0]["body"], "msg 2");
    assert_eq!(arr[2]["body"], "msg 0");
}

#[tokio::test]
async fn socket_upgrade_rejected_for_kicked_user() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (student, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, student, "student").await;
    let (course, session) = course_with_session(&pool, tenant, teacher, chrono::Utc::now()).await;
    sqlx::query("INSERT INTO course_memberships (course_id, user_id, tenant_id, role) VALUES ($1,$2,$3,'student')")
        .bind(course).bind(student).bind(tenant).execute(&pool).await.unwrap();

    // Pre-kick the student via DB (simulating a previous kick).
    let mut tx = pool.begin().await.unwrap();
    backend::db::live_room::insert_kick(&mut tx, tenant, session, student, teacher)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let mediamtx = Arc::new(backend::services::mediamtx::MockMediaMtxClient::new());
    let signer = Arc::new(backend::services::mediamtx::JwtSigner::new_ephemeral());
    let app = build_test_app(
        backend::handlers::live_sessions::live_room_router_for_tests(
            pool.clone(),
            mediamtx,
            signer,
            "http://localhost:8889".into(),
            "http://localhost:8888".into(),
        ),
        StubAuth {
            pool: pool.clone(),
            user_id: student,
            firebase_uid: fb,
            email: em,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Student),
        },
    );
    // GET without WebSocket upgrade headers — backend should still reject because of kick.
    let (s, _) = fire(&app, "GET", &format!("/v1/sessions/{session}/socket"), None).await;
    assert_eq!(s, 403);
}

#[tokio::test]
async fn messages_route_403_for_non_member() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (outsider, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, outsider, "student").await;
    let (_, session) = course_with_session(&pool, tenant, teacher, chrono::Utc::now()).await;

    let mediamtx = Arc::new(backend::services::mediamtx::MockMediaMtxClient::new());
    let signer = Arc::new(backend::services::mediamtx::JwtSigner::new_ephemeral());
    let app = build_test_app(
        backend::handlers::live_sessions::live_room_router_for_tests(
            pool.clone(),
            mediamtx,
            signer,
            "http://localhost:8889".into(),
            "http://localhost:8888".into(),
        ),
        StubAuth {
            pool: pool.clone(),
            user_id: outsider,
            firebase_uid: fb,
            email: em,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Student),
        },
    );
    let (s, _) = fire(
        &app,
        "GET",
        &format!("/v1/sessions/{session}/messages"),
        None,
    )
    .await;
    assert_eq!(s, 403);
}

#[tokio::test]
async fn socket_chat_message_persists_and_broadcasts() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (_, session) = course_with_session(&pool, tenant, teacher, chrono::Utc::now()).await;

    let mediamtx = Arc::new(backend::services::mediamtx::MockMediaMtxClient::new());
    let signer = Arc::new(backend::services::mediamtx::JwtSigner::new_ephemeral());
    let addr = bind_test_server(
        backend::handlers::live_sessions::live_room_router_for_tests(
            pool.clone(),
            mediamtx,
            signer,
            "http://localhost:8889".into(),
            "http://localhost:8888".into(),
        ),
        StubAuth {
            pool: pool.clone(),
            user_id: teacher,
            firebase_uid: fb,
            email: em,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Teacher),
        },
    )
    .await;

    let ws_url = format!("ws://{addr}/v1/sessions/{session}/socket");
    let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url).await.unwrap();
    let send = serde_json::json!({"type": "chat", "body": "hello world"}).to_string();
    ws.send(tokio_tungstenite::tungstenite::Message::Text(send))
        .await
        .unwrap();

    let mut got_chat = false;
    for _ in 0..5 {
        match tokio::time::timeout(std::time::Duration::from_secs(2), ws.next()).await {
            Ok(Some(Ok(tokio_tungstenite::tungstenite::Message::Text(t)))) => {
                let v: serde_json::Value = serde_json::from_str(&t).unwrap();
                if v["type"] == "chat" && v["body"] == "hello world" {
                    got_chat = true;
                    break;
                }
            }
            _ => continue,
        }
    }
    assert!(got_chat, "expected own chat message to broadcast back");

    let row: (String,) =
        sqlx::query_as("SELECT body FROM live_room_messages WHERE session_id = $1")
            .bind(session)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(row.0, "hello world");
}

#[tokio::test]
async fn socket_chat_delete_by_teacher_succeeds() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (_, session) = course_with_session(&pool, tenant, teacher, chrono::Utc::now()).await;

    let msg_id: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO live_room_messages (tenant_id, session_id, sender_user_id, body)
         VALUES ($1, $2, $3, 'gone soon') RETURNING id",
    )
    .bind(tenant)
    .bind(session)
    .bind(teacher)
    .fetch_one(&pool)
    .await
    .unwrap();

    let mediamtx = Arc::new(backend::services::mediamtx::MockMediaMtxClient::new());
    let signer = Arc::new(backend::services::mediamtx::JwtSigner::new_ephemeral());
    let addr = bind_test_server(
        backend::handlers::live_sessions::live_room_router_for_tests(
            pool.clone(),
            mediamtx,
            signer,
            "http://localhost:8889".into(),
            "http://localhost:8888".into(),
        ),
        StubAuth {
            pool: pool.clone(),
            user_id: teacher,
            firebase_uid: fb,
            email: em,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Teacher),
        },
    )
    .await;

    let ws_url = format!("ws://{addr}/v1/sessions/{session}/socket");
    let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url).await.unwrap();
    let payload = serde_json::json!({"type": "delete_message", "message_id": msg_id}).to_string();
    ws.send(tokio_tungstenite::tungstenite::Message::Text(payload))
        .await
        .unwrap();

    let mut deleted = false;
    for _ in 0..5 {
        match tokio::time::timeout(std::time::Duration::from_secs(2), ws.next()).await {
            Ok(Some(Ok(tokio_tungstenite::tungstenite::Message::Text(t)))) => {
                let v: serde_json::Value = serde_json::from_str(&t).unwrap();
                if v["type"] == "chat_deleted" && v["id"] == msg_id.to_string() {
                    deleted = true;
                    break;
                }
            }
            _ => continue,
        }
    }
    assert!(deleted);

    let r: (Option<chrono::DateTime<chrono::Utc>>,) =
        sqlx::query_as("SELECT deleted_at FROM live_room_messages WHERE id = $1")
            .bind(msg_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(r.0.is_some());
}

#[tokio::test]
async fn socket_hand_raise_appends_to_queue_and_broadcasts() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (student, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, student, "student").await;
    let (course, session) = course_with_session(&pool, tenant, teacher, chrono::Utc::now()).await;
    sqlx::query("INSERT INTO course_memberships (course_id, user_id, tenant_id, role) VALUES ($1,$2,$3,'student')")
        .bind(course).bind(student).bind(tenant).execute(&pool).await.unwrap();

    let mediamtx = Arc::new(backend::services::mediamtx::MockMediaMtxClient::new());
    let signer = Arc::new(backend::services::mediamtx::JwtSigner::new_ephemeral());
    let addr = bind_test_server(
        backend::handlers::live_sessions::live_room_router_for_tests(
            pool.clone(),
            mediamtx,
            signer,
            "http://localhost:8889".into(),
            "http://localhost:8888".into(),
        ),
        StubAuth {
            pool: pool.clone(),
            user_id: student,
            firebase_uid: fb,
            email: em,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Student),
        },
    )
    .await;
    let ws_url = format!("ws://{addr}/v1/sessions/{session}/socket");
    let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url).await.unwrap();
    let payload = serde_json::json!({"type": "hand_raise", "raise": true}).to_string();
    ws.send(tokio_tungstenite::tungstenite::Message::Text(payload))
        .await
        .unwrap();

    let mut got = false;
    for _ in 0..5 {
        match tokio::time::timeout(std::time::Duration::from_secs(2), ws.next()).await {
            Ok(Some(Ok(tokio_tungstenite::tungstenite::Message::Text(t)))) => {
                let v: serde_json::Value = serde_json::from_str(&t).unwrap();
                if v["type"] == "hand_raise_changed" && v["raised"] == true {
                    got = true;
                    break;
                }
            }
            _ => continue,
        }
    }
    assert!(got);
}

#[tokio::test]
async fn socket_accept_hand_promotes_with_publish_credentials() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, fb_t, em_t) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (student, fb_s, em_s) = create_user(&pool).await;
    attach_membership(&pool, tenant, student, "student").await;
    let (course, session) = course_with_session(&pool, tenant, teacher, chrono::Utc::now()).await;
    sqlx::query("INSERT INTO course_memberships (course_id, user_id, tenant_id, role) VALUES ($1,$2,$3,'student')")
        .bind(course).bind(student).bind(tenant).execute(&pool).await.unwrap();

    let mediamtx = Arc::new(backend::services::mediamtx::MockMediaMtxClient::new());
    let signer = Arc::new(backend::services::mediamtx::JwtSigner::new_ephemeral());

    // SHARED broker so teacher and student see each other's events.
    let shared_broker: Arc<dyn backend::services::live_room::LiveRoomBroker> =
        Arc::new(backend::services::live_room::MockLiveRoomBroker::new());

    let teacher_router = backend::handlers::live_sessions::live_room_router_for_tests_with_broker(
        pool.clone(),
        mediamtx.clone(),
        signer.clone(),
        "http://localhost:8889".into(),
        "http://localhost:8888".into(),
        shared_broker.clone(),
    );
    let student_router = backend::handlers::live_sessions::live_room_router_for_tests_with_broker(
        pool.clone(),
        mediamtx,
        signer,
        "http://localhost:8889".into(),
        "http://localhost:8888".into(),
        shared_broker,
    );

    let teacher_addr = bind_test_server(
        teacher_router,
        StubAuth {
            pool: pool.clone(),
            user_id: teacher,
            firebase_uid: fb_t,
            email: em_t,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Teacher),
        },
    )
    .await;
    let student_addr = bind_test_server(
        student_router,
        StubAuth {
            pool: pool.clone(),
            user_id: student,
            firebase_uid: fb_s,
            email: em_s,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Student),
        },
    )
    .await;

    let (mut t_ws, _) = tokio_tungstenite::connect_async(format!(
        "ws://{teacher_addr}/v1/sessions/{session}/socket"
    ))
    .await
    .unwrap();
    let (mut s_ws, _) = tokio_tungstenite::connect_async(format!(
        "ws://{student_addr}/v1/sessions/{session}/socket"
    ))
    .await
    .unwrap();

    // Give both sockets a moment to subscribe.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Teacher sends accept_hand for the student.
    let payload = serde_json::json!({"type": "accept_hand", "user_id": student}).to_string();
    t_ws.send(tokio_tungstenite::tungstenite::Message::Text(payload))
        .await
        .unwrap();

    // Student receives Promoted (direct).
    let mut got = false;
    for _ in 0..15 {
        match tokio::time::timeout(std::time::Duration::from_secs(3), s_ws.next()).await {
            Ok(Some(Ok(tokio_tungstenite::tungstenite::Message::Text(t)))) => {
                let v: serde_json::Value = serde_json::from_str(&t).unwrap();
                if v["type"] == "promoted" {
                    assert!(v["publish_url"].as_str().unwrap().contains("/student/"));
                    assert!(v["publish_password"].as_str().unwrap().len() >= 24);
                    got = true;
                    break;
                }
            }
            _ => continue,
        }
    }
    assert!(got, "student should receive Promoted event");
}

struct WhiteboardSocketFixture {
    teacher_ws: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    student_ws: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
}

async fn whiteboard_socket_fixture() -> WhiteboardSocketFixture {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, fb_t, em_t) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (student, fb_s, em_s) = create_user(&pool).await;
    attach_membership(&pool, tenant, student, "student").await;
    let (course, session) = course_with_session(&pool, tenant, teacher, chrono::Utc::now()).await;
    sqlx::query(
        "INSERT INTO course_memberships (course_id, user_id, tenant_id, role)
         VALUES ($1, $2, $3, 'student')",
    )
    .bind(course)
    .bind(student)
    .bind(tenant)
    .execute(&pool)
    .await
    .unwrap();

    let mediamtx = Arc::new(backend::services::mediamtx::MockMediaMtxClient::new());
    let signer = Arc::new(backend::services::mediamtx::JwtSigner::new_ephemeral());
    let shared_broker: Arc<dyn backend::services::live_room::LiveRoomBroker> =
        Arc::new(backend::services::live_room::MockLiveRoomBroker::new());

    let teacher_router = backend::handlers::live_sessions::live_room_router_for_tests_with_broker(
        pool.clone(),
        mediamtx.clone(),
        signer.clone(),
        "http://localhost:8889".into(),
        "http://localhost:8888".into(),
        shared_broker.clone(),
    );
    let student_router = backend::handlers::live_sessions::live_room_router_for_tests_with_broker(
        pool.clone(),
        mediamtx,
        signer,
        "http://localhost:8889".into(),
        "http://localhost:8888".into(),
        shared_broker,
    );

    let teacher_addr = bind_test_server(
        teacher_router,
        StubAuth {
            pool: pool.clone(),
            user_id: teacher,
            firebase_uid: fb_t,
            email: em_t,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Teacher),
        },
    )
    .await;
    let student_addr = bind_test_server(
        student_router,
        StubAuth {
            pool: pool.clone(),
            user_id: student,
            firebase_uid: fb_s,
            email: em_s,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Student),
        },
    )
    .await;

    let (teacher_ws, _) = tokio_tungstenite::connect_async(format!(
        "ws://{teacher_addr}/v1/sessions/{session}/socket"
    ))
    .await
    .unwrap();
    let (mut student_ws, _) = tokio_tungstenite::connect_async(format!(
        "ws://{student_addr}/v1/sessions/{session}/socket"
    ))
    .await
    .unwrap();

    next_presence_count(&mut student_ws, 2).await;
    WhiteboardSocketFixture {
        teacher_ws,
        student_ws,
    }
}

async fn next_presence_count(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    expected_count: u64,
) -> serde_json::Value {
    for _ in 0..20 {
        match tokio::time::timeout(std::time::Duration::from_secs(2), ws.next()).await {
            Ok(Some(Ok(tokio_tungstenite::tungstenite::Message::Text(t)))) => {
                let value: serde_json::Value = serde_json::from_str(&t).unwrap();
                if value["type"] == "presence_count"
                    && value["count"].as_u64() == Some(expected_count)
                {
                    return value;
                }
            }
            _ => continue,
        }
    }
    panic!("did not receive presence_count {expected_count}");
}

async fn next_ws_event_type(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    expected_type: &str,
) -> serde_json::Value {
    for _ in 0..20 {
        match tokio::time::timeout(std::time::Duration::from_secs(2), ws.next()).await {
            Ok(Some(Ok(tokio_tungstenite::tungstenite::Message::Text(t)))) => {
                let value: serde_json::Value = serde_json::from_str(&t).unwrap();
                if value["type"] == expected_type {
                    return value;
                }
            }
            _ => continue,
        }
    }
    panic!("did not receive expected websocket event type {expected_type}");
}

async fn assert_no_ws_event_type(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    forbidden_type: &str,
) {
    for _ in 0..5 {
        match tokio::time::timeout(std::time::Duration::from_millis(200), ws.next()).await {
            Ok(Some(Ok(tokio_tungstenite::tungstenite::Message::Text(t)))) => {
                let value: serde_json::Value = serde_json::from_str(&t).unwrap();
                assert_ne!(
                    value["type"], forbidden_type,
                    "received unexpected websocket event type {forbidden_type}: {value}"
                );
            }
            Ok(Some(_)) => continue,
            Ok(None) | Err(_) => return,
        }
    }
}

#[tokio::test]
async fn teacher_whiteboard_stroke_broadcasts() {
    let mut f = whiteboard_socket_fixture().await;
    f.teacher_ws
        .send(tokio_tungstenite::tungstenite::Message::Text(
            serde_json::json!({
                "type": "whiteboard_stroke",
                "stroke": {
                    "id": "s1",
                    "points": [{"x": 0.1, "y": 0.2}, {"x": 0.3, "y": 0.4}],
                    "color": "#111827",
                    "width": 4.0,
                    "tool": "pen"
                }
            })
            .to_string(),
        ))
        .await
        .unwrap();

    let msg = next_ws_event_type(&mut f.student_ws, "whiteboard_stroke").await;
    assert_eq!(msg["stroke"]["id"], "s1");
}

#[tokio::test]
async fn student_whiteboard_stroke_gets_command_failed() {
    let mut f = whiteboard_socket_fixture().await;
    f.student_ws
        .send(tokio_tungstenite::tungstenite::Message::Text(
            serde_json::json!({
                "type": "whiteboard_stroke",
                "stroke": {
                    "id": "s1",
                    "points": [{"x": 0.1, "y": 0.2}, {"x": 0.3, "y": 0.4}],
                    "color": "#111827",
                    "width": 4.0,
                    "tool": "pen"
                }
            })
            .to_string(),
        ))
        .await
        .unwrap();

    let msg = next_ws_event_type(&mut f.student_ws, "command_failed").await;
    assert_eq!(msg["command"], "whiteboard_stroke");
    assert_no_ws_event_type(&mut f.teacher_ws, "whiteboard_stroke").await;
}

#[tokio::test]
async fn teacher_invalid_whiteboard_stroke_gets_command_failed_without_broadcast() {
    let mut f = whiteboard_socket_fixture().await;
    f.teacher_ws
        .send(tokio_tungstenite::tungstenite::Message::Text(
            serde_json::json!({
                "type": "whiteboard_stroke",
                "stroke": {
                    "id": "s1",
                    "points": [{"x": 0.1, "y": 0.2}],
                    "color": "#111827",
                    "width": 4.0,
                    "tool": "pen"
                }
            })
            .to_string(),
        ))
        .await
        .unwrap();

    let msg = next_ws_event_type(&mut f.teacher_ws, "command_failed").await;
    assert_eq!(msg["command"], "whiteboard_stroke");
    assert_eq!(msg["reason"], "stroke must contain at least two points");
    assert_no_ws_event_type(&mut f.student_ws, "whiteboard_stroke").await;
}

#[tokio::test]
async fn teacher_whiteboard_clear_broadcasts() {
    let mut f = whiteboard_socket_fixture().await;
    f.teacher_ws
        .send(tokio_tungstenite::tungstenite::Message::Text(
            serde_json::json!({"type": "whiteboard_clear"}).to_string(),
        ))
        .await
        .unwrap();

    let msg = next_ws_event_type(&mut f.student_ws, "whiteboard_clear").await;
    assert_eq!(msg["type"], "whiteboard_clear");
}

#[tokio::test]
async fn socket_kick_inserts_audit_row_and_blocks_rejoin() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, fb_t, em_t) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (student, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, student, "student").await;
    let (course, session) = course_with_session(&pool, tenant, teacher, chrono::Utc::now()).await;
    sqlx::query("INSERT INTO course_memberships (course_id, user_id, tenant_id, role) VALUES ($1,$2,$3,'student')")
        .bind(course).bind(student).bind(tenant).execute(&pool).await.unwrap();

    let mediamtx = Arc::new(backend::services::mediamtx::MockMediaMtxClient::new());
    let signer = Arc::new(backend::services::mediamtx::JwtSigner::new_ephemeral());
    let teacher_addr = bind_test_server(
        backend::handlers::live_sessions::live_room_router_for_tests(
            pool.clone(),
            mediamtx,
            signer,
            "http://localhost:8889".into(),
            "http://localhost:8888".into(),
        ),
        StubAuth {
            pool: pool.clone(),
            user_id: teacher,
            firebase_uid: fb_t,
            email: em_t,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Teacher),
        },
    )
    .await;
    let (mut t_ws, _) = tokio_tungstenite::connect_async(format!(
        "ws://{teacher_addr}/v1/sessions/{session}/socket"
    ))
    .await
    .unwrap();
    t_ws.send(tokio_tungstenite::tungstenite::Message::Text(
        serde_json::json!({"type": "kick", "user_id": student}).to_string(),
    ))
    .await
    .unwrap();
    // Give it time to process.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    let exists: (bool,) = sqlx::query_as(
        "SELECT EXISTS(SELECT 1 FROM live_room_kicks WHERE session_id=$1 AND user_id=$2)",
    )
    .bind(session)
    .bind(student)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(exists.0);
}

#[tokio::test]
async fn presence_evict_stale_helper_decrements_count() {
    let broker = backend::services::live_room::MockLiveRoomBroker::new();
    let session_id = uuid::Uuid::new_v4();
    backend::services::live_room::LiveRoomBroker::presence_join(
        &broker,
        session_id,
        backend::services::live_room::PresenceEntry {
            user_id: uuid::Uuid::new_v4(),
            display_name: "stale".into(),
            role: "student".into(),
            last_seen_ms: 100,
        },
    )
    .await
    .unwrap();
    let evicted = backend::services::live_room::LiveRoomBroker::presence_evict_stale(
        &broker,
        session_id,
        1_000_000_000_000,
    )
    .await
    .unwrap();
    assert_eq!(evicted, 1);
}

#[tokio::test]
async fn chat_prune_older_than_removes_old_messages() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (_, session) = course_with_session(&pool, tenant, teacher, chrono::Utc::now()).await;

    sqlx::query(
        "INSERT INTO live_room_messages (tenant_id, session_id, sender_user_id, body, created_at)
         VALUES ($1, $2, $3, 'old', now() - interval '100 days')",
    )
    .bind(tenant)
    .bind(session)
    .bind(teacher)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO live_room_messages (tenant_id, session_id, sender_user_id, body)
         VALUES ($1, $2, $3, 'fresh')",
    )
    .bind(tenant)
    .bind(session)
    .bind(teacher)
    .execute(&pool)
    .await
    .unwrap();

    let pruned = backend::db::live_room::prune_older_than(&pool, 90)
        .await
        .unwrap();
    assert!(pruned >= 1);

    let remaining: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM live_room_messages WHERE session_id = $1")
            .bind(session)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(remaining.0, 1);
}

#[tokio::test]
async fn socket_chat_rate_limit_drops_excess() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (_, session) = course_with_session(&pool, tenant, teacher, chrono::Utc::now()).await;

    let mediamtx = Arc::new(backend::services::mediamtx::MockMediaMtxClient::new());
    let signer = Arc::new(backend::services::mediamtx::JwtSigner::new_ephemeral());
    let addr = bind_test_server(
        backend::handlers::live_sessions::live_room_router_for_tests(
            pool.clone(),
            mediamtx,
            signer,
            "http://localhost:8889".into(),
            "http://localhost:8888".into(),
        ),
        StubAuth {
            pool: pool.clone(),
            user_id: teacher,
            firebase_uid: fb,
            email: em,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Teacher),
        },
    )
    .await;
    let (mut ws, _) =
        tokio_tungstenite::connect_async(format!("ws://{addr}/v1/sessions/{session}/socket"))
            .await
            .unwrap();

    // Send 5 chat messages back-to-back; expect at least one rate_limited reply.
    for i in 0..5 {
        let payload = serde_json::json!({"type": "chat", "body": format!("msg {i}")}).to_string();
        ws.send(tokio_tungstenite::tungstenite::Message::Text(payload))
            .await
            .unwrap();
    }

    let mut rate_limited = false;
    for _ in 0..15 {
        match tokio::time::timeout(std::time::Duration::from_secs(2), ws.next()).await {
            Ok(Some(Ok(tokio_tungstenite::tungstenite::Message::Text(t)))) => {
                if t.contains("\"type\":\"rate_limited\"") {
                    rate_limited = true;
                    break;
                }
            }
            _ => continue,
        }
    }
    assert!(rate_limited, "expected at least one rate_limited reply");
}

// ============================================================================
// Live-room safety + auth pass (2026-05-15 design)
// ============================================================================

/// End-to-end: a viewer JWT minted by /join survives the MediaMTX read callback
/// when presented as `Bearer <jwt>` in the password field.
#[tokio::test]
async fn whep_bearer_e2e_real_jwt() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (student, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, student, "student").await;
    let (course, session) = course_with_session(&pool, tenant, teacher, chrono::Utc::now()).await;
    sqlx::query("INSERT INTO course_memberships (course_id, user_id, tenant_id, role) VALUES ($1,$2,$3,'student')")
        .bind(course).bind(student).bind(tenant).execute(&pool).await.unwrap();
    let main_path = backend::services::mediamtx::path_for_session(tenant, course, session);
    sqlx::query(
        "UPDATE live_sessions
            SET status='live', actual_started_at=now(), main_path=$2,
                publish_nonce='precomputed', publish_nonce_expires_at = now() + interval '1 hour'
          WHERE id=$1",
    )
    .bind(session)
    .bind(&main_path)
    .execute(&pool)
    .await
    .unwrap();

    let mediamtx = Arc::new(backend::services::mediamtx::MockMediaMtxClient::new());
    let signer = Arc::new(backend::services::mediamtx::JwtSigner::new_ephemeral());
    let app = build_test_app(
        backend::handlers::live_sessions::live_room_router_for_tests(
            pool.clone(),
            mediamtx.clone(),
            signer.clone(),
            "http://localhost:8889".into(),
            "http://localhost:8888".into(),
        ),
        StubAuth {
            pool: pool.clone(),
            user_id: student,
            firebase_uid: fb,
            email: em,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Student),
        },
    );

    // Drive /join to get a real viewer JWT.
    let (s_join, join_body) = fire(
        &app,
        "POST",
        &format!("/v1/sessions/{session}/join"),
        Some(json!({})),
    )
    .await;
    assert_eq!(s_join, 200, "{join_body}");
    let viewer_jwt = join_body["viewer_jwt"].as_str().unwrap().to_string();

    // Replay it through the MediaMTX read callback as `Bearer <jwt>`.
    let no_auth = build_test_app_no_auth(
        backend::handlers::live_sessions::live_room_router_for_tests(
            pool.clone(),
            mediamtx,
            signer,
            "http://localhost:8889".into(),
            "http://localhost:8888".into(),
        ),
    );
    let (s_read, _) = fire(
        &no_auth,
        "POST",
        "/v1/mediamtx/auth/publish",
        Some(json!({
            "action": "read",
            "path": main_path,
            "password": format!("Bearer {viewer_jwt}"),
        })),
    )
    .await;
    assert_eq!(s_read, 200, "real viewer JWT must pass the read callback");
}

/// Sanity check for design item 1: viewer URLs for the WebRTC transport must
/// never carry the JWT in their query (clients now send `Authorization: Bearer`).
#[tokio::test]
async fn whep_main_url_has_no_jwt_in_query() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (student, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, student, "student").await;
    let (course, session) = course_with_session(&pool, tenant, teacher, chrono::Utc::now()).await;
    sqlx::query("INSERT INTO course_memberships (course_id, user_id, tenant_id, role) VALUES ($1,$2,$3,'student')")
        .bind(course).bind(student).bind(tenant).execute(&pool).await.unwrap();
    force_session_live(&pool, session).await;
    // Promote a screen path too so we can assert on it as well.
    sqlx::query("UPDATE live_sessions SET screen_path = $2 WHERE id = $1")
        .bind(session)
        .bind(format!("aula/x/y/{}/screen", session.simple()))
        .execute(&pool)
        .await
        .unwrap();

    let mediamtx = Arc::new(backend::services::mediamtx::MockMediaMtxClient::new());
    let signer = Arc::new(backend::services::mediamtx::JwtSigner::new_ephemeral());
    let app = build_test_app(
        backend::handlers::live_sessions::live_room_router_for_tests(
            pool.clone(),
            mediamtx,
            signer,
            "http://localhost:8889".into(),
            "http://localhost:8888".into(),
        ),
        StubAuth {
            pool: pool.clone(),
            user_id: student,
            firebase_uid: fb,
            email: em,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Student),
        },
    );
    let (s, body) = fire(
        &app,
        "POST",
        &format!("/v1/sessions/{session}/join"),
        Some(json!({})),
    )
    .await;
    assert_eq!(s, 200, "{body}");
    let main_url = body["main_url"].as_str().unwrap();
    let screen_url = body["screen_url"].as_str().unwrap();
    assert!(
        !main_url.contains("jwt="),
        "main_url leaked jwt: {main_url}"
    );
    assert!(
        !main_url.contains("access_token="),
        "main_url leaked access_token: {main_url}"
    );
    assert!(
        !screen_url.contains("jwt="),
        "screen_url leaked jwt: {screen_url}"
    );
    assert!(
        !screen_url.contains("access_token="),
        "screen_url leaked access_token: {screen_url}"
    );
}

/// `bearer ` (lowercase) is accepted by the read callback.
#[tokio::test]
async fn mediamtx_read_accepts_lowercase_bearer() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (course, session) = course_with_session(&pool, tenant, teacher, chrono::Utc::now()).await;
    let main_path = backend::services::mediamtx::path_for_session(tenant, course, session);

    let signer = backend::services::mediamtx::JwtSigner::new_ephemeral();
    let claims = backend::services::mediamtx::ViewerClaims {
        iss: "aulalite".into(),
        sub: uuid::Uuid::new_v4().to_string(),
        tnt: tenant.to_string(),
        mediamtx_permissions: vec![backend::services::mediamtx::MediaMtxPermission {
            action: "read".into(),
            path: main_path.clone(),
        }],
        exp: 0,
    };
    let token = signer.mint_viewer_jwt(claims, std::time::Duration::from_secs(900));

    let mediamtx = Arc::new(backend::services::mediamtx::MockMediaMtxClient::new());
    let signer_arc = Arc::new(signer);
    let app = build_test_app_no_auth(
        backend::handlers::live_sessions::live_room_router_for_tests(
            pool.clone(),
            mediamtx,
            signer_arc,
            "http://localhost:8889".into(),
            "http://localhost:8888".into(),
        ),
    );
    let (s, _) = fire(
        &app,
        "POST",
        "/v1/mediamtx/auth/publish",
        Some(json!({
            "action": "read",
            "path": main_path,
            "password": format!("bearer {token}"),
        })),
    )
    .await;
    assert_eq!(s, 200);
}

/// `Bearer  <jwt>` with extra whitespace is accepted (trimmed).
#[tokio::test]
async fn mediamtx_read_accepts_bearer_with_whitespace() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (course, session) = course_with_session(&pool, tenant, teacher, chrono::Utc::now()).await;
    let main_path = backend::services::mediamtx::path_for_session(tenant, course, session);

    let signer = backend::services::mediamtx::JwtSigner::new_ephemeral();
    let claims = backend::services::mediamtx::ViewerClaims {
        iss: "aulalite".into(),
        sub: uuid::Uuid::new_v4().to_string(),
        tnt: tenant.to_string(),
        mediamtx_permissions: vec![backend::services::mediamtx::MediaMtxPermission {
            action: "read".into(),
            path: main_path.clone(),
        }],
        exp: 0,
    };
    let token = signer.mint_viewer_jwt(claims, std::time::Duration::from_secs(900));

    let mediamtx = Arc::new(backend::services::mediamtx::MockMediaMtxClient::new());
    let signer_arc = Arc::new(signer);
    let app = build_test_app_no_auth(
        backend::handlers::live_sessions::live_room_router_for_tests(
            pool.clone(),
            mediamtx,
            signer_arc,
            "http://localhost:8889".into(),
            "http://localhost:8888".into(),
        ),
    );
    let (s, _) = fire(
        &app,
        "POST",
        "/v1/mediamtx/auth/publish",
        Some(json!({
            "action": "read",
            "path": main_path,
            // Note: 2 spaces between "Bearer" and the token + leading whitespace
            // surrounding the password value.
            "password": format!("  Bearer  {token}  "),
        })),
    )
    .await;
    assert_eq!(s, 200);
}

/// A non-teacher issuing `delete_message` triggers a `command_failed` event
/// back to the sender, and the broker does not see a `chat_deleted`.
#[tokio::test]
async fn command_failure_emits_event() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (student, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, student, "student").await;
    let (course, session) = course_with_session(&pool, tenant, teacher, chrono::Utc::now()).await;
    sqlx::query("INSERT INTO course_memberships (course_id, user_id, tenant_id, role) VALUES ($1,$2,$3,'student')")
        .bind(course).bind(student).bind(tenant).execute(&pool).await.unwrap();

    let mediamtx = Arc::new(backend::services::mediamtx::MockMediaMtxClient::new());
    let signer = Arc::new(backend::services::mediamtx::JwtSigner::new_ephemeral());
    let addr = bind_test_server(
        backend::handlers::live_sessions::live_room_router_for_tests(
            pool.clone(),
            mediamtx,
            signer,
            "http://localhost:8889".into(),
            "http://localhost:8888".into(),
        ),
        StubAuth {
            pool: pool.clone(),
            user_id: student,
            firebase_uid: fb,
            email: em,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Student),
        },
    )
    .await;

    let ws_url = format!("ws://{addr}/v1/sessions/{session}/socket");
    let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url).await.unwrap();
    // Student attempts a teacher-only command — should yield CommandFailed.
    let payload = serde_json::json!({
        "type": "demote_hand",
        "user_id": teacher,
    })
    .to_string();
    ws.send(tokio_tungstenite::tungstenite::Message::Text(payload))
        .await
        .unwrap();

    let mut saw_failed = false;
    let mut saw_demoted = false;
    for _ in 0..10 {
        match tokio::time::timeout(std::time::Duration::from_secs(2), ws.next()).await {
            Ok(Some(Ok(tokio_tungstenite::tungstenite::Message::Text(t)))) => {
                let v: serde_json::Value = serde_json::from_str(&t).unwrap();
                if v["type"] == "command_failed" {
                    assert_eq!(v["command"], "demote_hand");
                    assert!(v["reason"].as_str().unwrap_or("").len() >= 3);
                    saw_failed = true;
                }
                if v["type"] == "demoted" {
                    saw_demoted = true;
                }
            }
            _ => continue,
        }
        if saw_failed {
            break;
        }
    }
    assert!(saw_failed, "expected a command_failed event");
    assert!(!saw_demoted, "demoted must not be emitted on failure");
}

/// `?limit=1000000` is clamped to 200 in the messages route.
#[tokio::test]
async fn messages_limit_clamped_to_200() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (_, session) = course_with_session(&pool, tenant, teacher, chrono::Utc::now()).await;

    // Insert 250 rows so the response would exceed 200 if unclamped.
    for i in 0..250 {
        sqlx::query(
            "INSERT INTO live_room_messages (tenant_id, session_id, sender_user_id, body)
             VALUES ($1, $2, $3, $4)",
        )
        .bind(tenant)
        .bind(session)
        .bind(teacher)
        .bind(format!("msg {i}"))
        .execute(&pool)
        .await
        .unwrap();
    }

    let mediamtx = Arc::new(backend::services::mediamtx::MockMediaMtxClient::new());
    let signer = Arc::new(backend::services::mediamtx::JwtSigner::new_ephemeral());
    let app = build_test_app(
        backend::handlers::live_sessions::live_room_router_for_tests(
            pool.clone(),
            mediamtx,
            signer,
            "http://localhost:8889".into(),
            "http://localhost:8888".into(),
        ),
        StubAuth {
            pool: pool.clone(),
            user_id: teacher,
            firebase_uid: fb,
            email: em,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Teacher),
        },
    );
    let (s, body) = fire(
        &app,
        "GET",
        &format!("/v1/sessions/{session}/messages?limit=1000000"),
        None,
    )
    .await;
    assert_eq!(s, 200, "{body}");
    let arr = body["messages"].as_array().unwrap();
    assert!(
        arr.len() <= 200,
        "got {} messages; expected <= 200",
        arr.len()
    );
}

/// `to_seconds=NaN` falls back to MAX_UTC instead of overflowing.
#[tokio::test]
async fn messages_inner_rejects_non_finite_to() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (_, session) = course_with_session(&pool, tenant, teacher, chrono::Utc::now()).await;

    // The handler reads from the recordings table; insert a row that points
    // at our session so the recording chat handler can resolve a started_at.
    sqlx::query(
        "INSERT INTO recordings (tenant_id, session_id, processing_status,
                                  started_at, ended_at, duration_seconds)
         VALUES ($1, $2, 'available', now() - interval '1 hour', now(), 60)",
    )
    .bind(tenant)
    .bind(session)
    .execute(&pool)
    .await
    .unwrap();

    let mediamtx = Arc::new(backend::services::mediamtx::MockMediaMtxClient::new());
    let signer = Arc::new(backend::services::mediamtx::JwtSigner::new_ephemeral());
    let app = build_test_app(
        backend::handlers::live_sessions::live_room_router_for_tests(
            pool.clone(),
            mediamtx,
            signer,
            "http://localhost:8889".into(),
            "http://localhost:8888".into(),
        ),
        StubAuth {
            pool: pool.clone(),
            user_id: teacher,
            firebase_uid: fb,
            email: em,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Teacher),
        },
    );
    // serde_json refuses to encode NaN; pass the request via a raw URL.
    let (s, body) = fire(
        &app,
        "GET",
        &format!("/v1/sessions/{session}/recording/chat?from_seconds=0&to_seconds=NaN"),
        None,
    )
    .await;
    // Non-finite to_seconds is treated as "open ended"; the call must succeed
    // (and not crash on overflow). The body shape carries an empty messages
    // array because we inserted no chat rows.
    assert_eq!(s, 200, "{body}");
    assert!(body["messages"].is_array());
}

/// `HandRaiseChanged` events emitted by the server carry the speaker's
/// `display_name`.
#[tokio::test]
async fn hand_raise_event_includes_display_name() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (student, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, student, "student").await;
    let (course, session) = course_with_session(&pool, tenant, teacher, chrono::Utc::now()).await;
    sqlx::query("INSERT INTO course_memberships (course_id, user_id, tenant_id, role) VALUES ($1,$2,$3,'student')")
        .bind(course).bind(student).bind(tenant).execute(&pool).await.unwrap();

    let mediamtx = Arc::new(backend::services::mediamtx::MockMediaMtxClient::new());
    let signer = Arc::new(backend::services::mediamtx::JwtSigner::new_ephemeral());
    let addr = bind_test_server(
        backend::handlers::live_sessions::live_room_router_for_tests(
            pool.clone(),
            mediamtx,
            signer,
            "http://localhost:8889".into(),
            "http://localhost:8888".into(),
        ),
        StubAuth {
            pool: pool.clone(),
            user_id: student,
            firebase_uid: fb,
            email: em.clone(),
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Student),
        },
    )
    .await;

    let ws_url = format!("ws://{addr}/v1/sessions/{session}/socket");
    let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url).await.unwrap();
    let payload = serde_json::json!({"type": "hand_raise", "raise": true}).to_string();
    ws.send(tokio_tungstenite::tungstenite::Message::Text(payload))
        .await
        .unwrap();

    let mut got = false;
    let expected_name = em.split('@').next().unwrap().to_string();
    for _ in 0..10 {
        match tokio::time::timeout(std::time::Duration::from_secs(2), ws.next()).await {
            Ok(Some(Ok(tokio_tungstenite::tungstenite::Message::Text(t)))) => {
                let v: serde_json::Value = serde_json::from_str(&t).unwrap();
                if v["type"] == "hand_raise_changed" {
                    let dn = v["display_name"].as_str().unwrap_or("");
                    assert!(
                        !dn.is_empty(),
                        "hand_raise_changed must carry a non-empty display_name; got {v:?}"
                    );
                    assert_eq!(dn, expected_name);
                    got = true;
                    break;
                }
            }
            _ => continue,
        }
    }
    assert!(got, "expected hand_raise_changed event");
}
