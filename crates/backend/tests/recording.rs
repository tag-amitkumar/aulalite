// crates/backend/tests/recording.rs
mod fixtures;

use fixtures::*;
use serde_json::json;
use std::sync::Arc;

async fn course_with_session_recording_enabled(
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
    let series: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO live_session_series (tenant_id, course_id, title, starts_at,
                                          duration_minutes, frequency, end_kind,
                                          transport_mode, primary_teacher_id, recording_enabled)
         VALUES ($1, $2, 'S', $3, 60, 'none', 'open', 'webrtc', $4, true)
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
        "INSERT INTO live_sessions (tenant_id, course_id, series_id, occurrence_index, title,
                                    status, starts_at, duration_minutes, primary_teacher_id, mode,
                                    recording_enabled, transport_mode)
         VALUES ($1, $2, $3, 0, 'L', 'scheduled', $4, 60, $5, 'lecture', true, 'webrtc')
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

async fn set_recording_limit(pool: &sqlx::PgPool, tenant: uuid::Uuid, recording_gb: i32) {
    let plan_id = format!("recording-limit-{}", uuid::Uuid::new_v4().simple());
    sqlx::query(
        "INSERT INTO plans
            (id, name, monthly_price_cents, included_seats,
             included_class_minutes, included_recording_gb)
         VALUES ($1, $1, 0, 100, 10000, $2)",
    )
    .bind(&plan_id)
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
async fn recording_route_returns_404_when_no_recording() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (_, session) =
        course_with_session_recording_enabled(&pool, tenant, teacher, chrono::Utc::now()).await;

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
        "GET",
        &format!("/v1/sessions/{session}/recording"),
        None,
    )
    .await;
    assert_eq!(s, 404);
}

#[tokio::test]
async fn recording_route_returns_processing_state_when_pending() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (_, session) =
        course_with_session_recording_enabled(&pool, tenant, teacher, chrono::Utc::now()).await;

    sqlx::query(
        "INSERT INTO recordings (tenant_id, session_id, started_at, ended_at, duration_seconds)
         VALUES ($1, $2, now() - interval '10 minutes', now() - interval '5 minutes', 300)",
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
    let (s, body) = fire(
        &app,
        "GET",
        &format!("/v1/sessions/{session}/recording"),
        None,
    )
    .await;
    assert_eq!(s, 200, "{body}");
    assert_eq!(body["processing_status"], "pending");
    assert!(body["playback_url"].is_null());
}

#[tokio::test]
async fn recording_chat_window_returns_messages_in_range() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (_, session) =
        course_with_session_recording_enabled(&pool, tenant, teacher, chrono::Utc::now()).await;

    let started = chrono::Utc::now() - chrono::Duration::minutes(10);
    sqlx::query(
        "INSERT INTO recordings (tenant_id, session_id, started_at, ended_at, duration_seconds, processing_status)
         VALUES ($1, $2, $3, $3 + interval '5 minutes', 300, 'available')"
    ).bind(tenant).bind(session).bind(started).execute(&pool).await.unwrap();

    for offset_secs in [30i64, 90, 240] {
        sqlx::query(
            "INSERT INTO live_room_messages (tenant_id, session_id, sender_user_id, body, created_at)
             VALUES ($1, $2, $3, $4, $5)"
        ).bind(tenant).bind(session).bind(teacher)
            .bind(format!("msg @{offset_secs}s"))
            .bind(started + chrono::Duration::seconds(offset_secs))
            .execute(&pool).await.unwrap();
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
    // Window 0..120 should match first two messages (at 30s and 90s).
    let (s, body) = fire(
        &app,
        "GET",
        &format!("/v1/sessions/{session}/recording/chat?from_seconds=0&to_seconds=120"),
        None,
    )
    .await;
    assert_eq!(s, 200, "{body}");
    let arr = body["messages"].as_array().unwrap();
    assert_eq!(arr.len(), 2);
    assert!(arr[0]["video_offset_seconds"].as_f64().unwrap() < 60.0);
    assert!(arr[1]["video_offset_seconds"].as_f64().unwrap() < 120.0);
}

#[tokio::test]
async fn recording_retry_resets_failed_to_pending() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (_, session) =
        course_with_session_recording_enabled(&pool, tenant, teacher, chrono::Utc::now()).await;

    sqlx::query(
        "INSERT INTO recordings (tenant_id, session_id, started_at, ended_at, duration_seconds,
                                 processing_status, processing_error)
         VALUES ($1, $2, now() - interval '10 minutes', now() - interval '5 minutes', 300,
                 'failed', 'ffmpeg crash')",
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
    let (s, body) = fire(
        &app,
        "POST",
        &format!("/v1/sessions/{session}/recording/retry"),
        Some(json!({})),
    )
    .await;
    assert_eq!(s, 200, "{body}");
    assert_eq!(body["processing_status"], "pending");

    let row: (String, Option<String>) = sqlx::query_as(
        "SELECT processing_status, processing_error FROM recordings WHERE session_id = $1",
    )
    .bind(session)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(row.0, "pending");
    assert!(row.1.is_none());
}

#[tokio::test]
async fn recording_retry_remains_failed_when_storage_plan_is_full() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (_, session) =
        course_with_session_recording_enabled(&pool, tenant, teacher, chrono::Utc::now()).await;
    set_recording_limit(&pool, tenant, 0).await;
    sqlx::query(
        "INSERT INTO recordings
            (tenant_id, session_id, started_at, ended_at, duration_seconds,
             processing_status, processing_error)
         VALUES ($1, $2, now() - interval '10 minutes', now() - interval '5 minutes',
                 300, 'failed', 'recording_storage_limit_reached')",
    )
    .bind(tenant)
    .bind(session)
    .execute(&pool)
    .await
    .unwrap();

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
        &format!("/v1/sessions/{session}/recording/retry"),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, 402, "{body}");
    assert_eq!(body["error"], "recording_storage_limit_reached");
    let state: String =
        sqlx::query_scalar("SELECT processing_status FROM recordings WHERE session_id = $1")
            .bind(session)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(state, "failed");
}

#[tokio::test]
async fn recording_retry_by_non_admin_returns_403() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (_, session) =
        course_with_session_recording_enabled(&pool, tenant, teacher, chrono::Utc::now()).await;
    let (student, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, student, "student").await;

    sqlx::query(
        "INSERT INTO recordings (tenant_id, session_id, started_at, ended_at, duration_seconds,
                                 processing_status, processing_error)
         VALUES ($1, $2, now() - interval '10 minutes', now() - interval '5 minutes', 300,
                 'failed', 'ffmpeg crash')",
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
            user_id: student,
            firebase_uid: fb,
            email: em,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Student),
        },
    );
    let (s, _) = fire(
        &app,
        "POST",
        &format!("/v1/sessions/{session}/recording/retry"),
        Some(json!({})),
    )
    .await;
    assert_eq!(s, 403);
}

#[tokio::test]
async fn process_one_session_skips_under_5s() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (_, session) =
        course_with_session_recording_enabled(&pool, tenant, teacher, chrono::Utc::now()).await;
    let storage = Arc::new(backend::storage::mock::MockS3Client::new());
    let recorder = backend::services::recording::MockRecorderTool::new();
    let tmp = tempfile::tempdir().unwrap();
    let now = chrono::Utc::now();
    let res = backend::services::recording::process_one_session(
        &pool,
        storage.as_ref(),
        "aulalite",
        &recorder,
        tmp.path(),
        tenant,
        session,
        now,
        now, // duration 0
    )
    .await;
    assert!(matches!(res, Ok(None)));
    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM recordings WHERE session_id=$1")
        .bind(session)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count.0, 0);
}

#[tokio::test]
async fn process_one_session_succeeds_with_mock_segments() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let started = chrono::Utc::now() - chrono::Duration::minutes(10);
    let (course, session) =
        course_with_session_recording_enabled(&pool, tenant, teacher, started).await;
    let ended = started + chrono::Duration::minutes(2);

    // Build a fake segment tree matching the on-disk layout:
    //   <tmp>/aula/<tenant_simple>/<course_simple>/<session_simple>/seg.*
    let tmp = tempfile::tempdir().unwrap();
    let session_dir = tmp
        .path()
        .join("aula")
        .join(tenant.simple().to_string())
        .join(course.simple().to_string())
        .join(session.simple().to_string());
    std::fs::create_dir_all(&session_dir).unwrap();
    std::fs::write(session_dir.join("seg1.mp4"), b"\x00\x00\x00\x18ftypmp42").unwrap();

    let storage = Arc::new(backend::storage::mock::MockS3Client::new());
    let recorder = backend::services::recording::MockRecorderTool::new();
    recorder.set_duration_seconds(120);

    let res = backend::services::recording::process_one_session(
        &pool,
        storage.as_ref(),
        "aulalite",
        &recorder,
        tmp.path(),
        tenant,
        session,
        started,
        ended,
    )
    .await
    .unwrap();
    assert!(res.is_some());

    let row: (String, Option<uuid::Uuid>, i32) = sqlx::query_as(
        "SELECT processing_status, file_asset_id, duration_seconds FROM recordings WHERE session_id = $1"
    ).bind(session).fetch_one(&pool).await.unwrap();
    assert_eq!(row.0, "available");
    assert!(row.1.is_some());
    assert_eq!(row.2, 120);
}

#[tokio::test]
async fn exact_recording_byte_reservation_blocks_upload_past_plan_limit() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    set_recording_limit(&pool, tenant, 1).await;

    // An available recording has left fewer bytes than the mock remux output.
    // Older-than-current-month storage still counts: this quota is total bytes
    // retained, not a monthly upload allowance.
    let prior_started = chrono::Utc::now() - chrono::Duration::days(60);
    let (_, prior_session) =
        course_with_session_recording_enabled(&pool, tenant, teacher, prior_started).await;
    let prior_recording: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO recordings
            (tenant_id, session_id, started_at, ended_at, duration_seconds,
             processing_status)
         VALUES ($1, $2, $3, $3 + interval '1 minute', 60, 'available')
         RETURNING id",
    )
    .bind(tenant)
    .bind(prior_session)
    .bind(prior_started)
    .fetch_one(&pool)
    .await
    .unwrap();
    let prior_asset: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO file_assets
            (tenant_id, owner_user_id, bucket, object_key, content_type,
             size_bytes, status, visibility, linked_entity_type, linked_entity_id)
         VALUES ($1, $2, 'aulalite', $3, 'video/mp4', 999999995,
                 'available', 'private', 'session_recording', $4)
         RETURNING id",
    )
    .bind(tenant)
    .bind(teacher)
    .bind(format!("quota-existing/{}.mp4", uuid::Uuid::new_v4()))
    .bind(prior_recording)
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query("UPDATE recordings SET file_asset_id = $2 WHERE id = $1")
        .bind(prior_recording)
        .bind(prior_asset)
        .execute(&pool)
        .await
        .unwrap();

    let started = chrono::Utc::now() - chrono::Duration::minutes(10);
    let (course, session) =
        course_with_session_recording_enabled(&pool, tenant, teacher, started).await;
    let tmp = tempfile::tempdir().unwrap();
    let session_dir = tmp
        .path()
        .join("aula")
        .join(tenant.simple().to_string())
        .join(course.simple().to_string())
        .join(session.simple().to_string());
    std::fs::create_dir_all(&session_dir).unwrap();
    std::fs::write(session_dir.join("seg1.mp4"), b"segment").unwrap();

    let storage = Arc::new(backend::storage::mock::MockS3Client::new());
    let recorder = backend::services::recording::MockRecorderTool::new();
    let result = backend::services::recording::process_one_session(
        &pool,
        storage.as_ref(),
        "aulalite",
        &recorder,
        tmp.path(),
        tenant,
        session,
        started,
        started + chrono::Duration::minutes(2),
    )
    .await;

    assert!(matches!(
        result,
        Err(backend::services::recording::RecorderError::RecordingStorageLimitReached)
    ));
    assert!(storage
        .calls()
        .iter()
        .all(|call| !matches!(call, backend::storage::mock::S3Call::PutObject { .. })));
    let state: (String, Option<String>) = sqlx::query_as(
        "SELECT processing_status, processing_error FROM recordings WHERE session_id = $1",
    )
    .bind(session)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(state.0, "failed");
    assert_eq!(state.1.as_deref(), Some("recording_storage_limit_reached"));
}

#[tokio::test]
async fn process_one_session_dedupes_via_unique_session_id() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let started = chrono::Utc::now() - chrono::Duration::minutes(10);
    let (_course, session) =
        course_with_session_recording_enabled(&pool, tenant, teacher, started).await;
    let ended = started + chrono::Duration::minutes(2);

    // Pre-insert a claimed row (simulating the first sweep already winning).
    sqlx::query(
        "INSERT INTO recordings
            (tenant_id, session_id, started_at, ended_at, duration_seconds,
             processing_status)
         VALUES ($1, $2, $3, $4, 120, 'remuxing')",
    )
    .bind(tenant)
    .bind(session)
    .bind(started)
    .bind(ended)
    .execute(&pool)
    .await
    .unwrap();

    let tmp = tempfile::tempdir().unwrap();
    let storage = Arc::new(backend::storage::mock::MockS3Client::new());
    let recorder = backend::services::recording::MockRecorderTool::new();

    // Second sweep tick: should return None due to ON CONFLICT.
    let res = backend::services::recording::process_one_session(
        &pool,
        storage.as_ref(),
        "aulalite",
        &recorder,
        tmp.path(),
        tenant,
        session,
        started,
        ended,
    )
    .await
    .unwrap();
    assert!(res.is_none());
}

#[tokio::test]
async fn process_one_session_claims_existing_pending_retry() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let started = chrono::Utc::now() - chrono::Duration::minutes(10);
    let (course, session) =
        course_with_session_recording_enabled(&pool, tenant, teacher, started).await;
    let ended = started + chrono::Duration::minutes(2);
    sqlx::query(
        "INSERT INTO recordings
            (tenant_id, session_id, started_at, ended_at, duration_seconds,
             processing_status, processing_error)
         VALUES ($1, $2, $3, $4, 120, 'pending', NULL)",
    )
    .bind(tenant)
    .bind(session)
    .bind(started)
    .bind(ended)
    .execute(&pool)
    .await
    .unwrap();

    let tmp = tempfile::tempdir().unwrap();
    let session_dir = tmp
        .path()
        .join("aula")
        .join(tenant.simple().to_string())
        .join(course.simple().to_string())
        .join(session.simple().to_string());
    std::fs::create_dir_all(&session_dir).unwrap();
    std::fs::write(session_dir.join("seg1.mp4"), b"segment").unwrap();
    let storage = Arc::new(backend::storage::mock::MockS3Client::new());
    let recorder = backend::services::recording::MockRecorderTool::new();

    let result = backend::services::recording::process_one_session(
        &pool,
        storage.as_ref(),
        "aulalite",
        &recorder,
        tmp.path(),
        tenant,
        session,
        started,
        ended,
    )
    .await
    .unwrap();
    assert!(result.is_some());
    let state: String =
        sqlx::query_scalar("SELECT processing_status FROM recordings WHERE session_id = $1")
            .bind(session)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(state, "available");
}

#[tokio::test]
async fn retention_janitor_deletes_recordings_older_than_n_days() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    // Two sessions: session_a is far in the past (older than retention),
    // session_b is recent.
    let (_, session_a) = course_with_session_recording_enabled(
        &pool,
        tenant,
        teacher,
        chrono::Utc::now() - chrono::Duration::days(400),
    )
    .await;
    let (_, session_b) =
        course_with_session_recording_enabled(&pool, tenant, teacher, chrono::Utc::now()).await;

    // Insert two file_assets owned by the teacher (NOT tenant_id; FK to users).
    // UUID-suffixed object_keys to avoid UNIQUE(bucket, object_key) collisions
    // when this test re-runs against an already-seeded DB.
    let old_key = format!("old/{}.mp4", uuid::Uuid::new_v4());
    let fresh_key = format!("fresh/{}.mp4", uuid::Uuid::new_v4());
    let asset_old: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO file_assets (tenant_id, owner_user_id, bucket, object_key,
                                  content_type, size_bytes, status, visibility,
                                  linked_entity_type, linked_entity_id)
         VALUES ($1, $2, 'aulalite', $3, 'video/mp4', 100, 'available', 'private',
                 'session_recording', $4)
         RETURNING id",
    )
    .bind(tenant)
    .bind(teacher)
    .bind(&old_key)
    .bind(uuid::Uuid::new_v4())
    .fetch_one(&pool)
    .await
    .unwrap();
    let asset_fresh: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO file_assets (tenant_id, owner_user_id, bucket, object_key,
                                  content_type, size_bytes, status, visibility,
                                  linked_entity_type, linked_entity_id)
         VALUES ($1, $2, 'aulalite', $3, 'video/mp4', 100, 'available', 'private',
                 'session_recording', $4)
         RETURNING id",
    )
    .bind(tenant)
    .bind(teacher)
    .bind(&fresh_key)
    .bind(uuid::Uuid::new_v4())
    .fetch_one(&pool)
    .await
    .unwrap();

    // Insert two recordings: one stale, one fresh.
    sqlx::query(
        "INSERT INTO recordings (tenant_id, session_id, file_asset_id, started_at, ended_at,
                                 duration_seconds, processing_status, created_at)
         VALUES ($1, $2, $3, now() - interval '400 days', now() - interval '400 days' + interval '1 hour',
                 3600, 'available', now() - interval '400 days')"
    ).bind(tenant).bind(session_a).bind(asset_old).execute(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO recordings (tenant_id, session_id, file_asset_id, started_at, ended_at,
                                 duration_seconds, processing_status)
         VALUES ($1, $2, $3, now(), now() + interval '1 hour', 3600, 'available')",
    )
    .bind(tenant)
    .bind(session_b)
    .bind(asset_fresh)
    .execute(&pool)
    .await
    .unwrap();

    let storage = Arc::new(backend::storage::mock::MockS3Client::new());
    let pruned =
        backend::services::recording::run_retention_janitor(&pool, storage.as_ref(), 365, 100)
            .await
            .unwrap();
    assert!(pruned >= 1);

    // Old recording is gone.
    let old_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM recordings WHERE session_id=$1")
        .bind(session_a)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(old_count.0, 0);
    // Fresh recording survives.
    let fresh_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM recordings WHERE session_id=$1")
        .bind(session_b)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(fresh_count.0, 1);
    // Old file_asset is pruned.
    let old_status: (String,) = sqlx::query_as("SELECT status FROM file_assets WHERE id=$1")
        .bind(asset_old)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(old_status.0, "pruned");
}
