// crates/backend/tests/attendance.rs
//
// Integration tests for durable live-room attendance (db + report endpoint).
// These require a live Postgres (testcontainers / DATABASE_URL) and so do not
// run in CI sandboxes without one; they are written to compile and be logically
// correct against the real schema.
mod fixtures;

use fixtures::*;
use std::sync::Arc;

/// Create a course + scheduled live session owned by `teacher`, mirroring the
/// helper in `live_room.rs` / `recording.rs`.
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
         VALUES ($1, $2, $3, 0, 'L', 'live', $4, 60, $5, 'lecture', false, 'webrtc')
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

#[tokio::test]
async fn join_then_leave_accrues_total_seconds() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (student, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, student, "student").await;
    let (_course, session) = course_with_session(&pool, tenant, teacher, chrono::Utc::now()).await;

    backend::db::attendance::record_join(&pool, tenant, session, student)
        .await
        .unwrap();

    // Backdate last_joined_at so the leave accrues a known, positive amount.
    sqlx::query(
        "UPDATE attendance SET last_joined_at = now() - interval '30 seconds'
          WHERE session_id = $1 AND user_id = $2",
    )
    .bind(session)
    .bind(student)
    .execute(&pool)
    .await
    .unwrap();

    backend::db::attendance::record_leave(&pool, tenant, session, student)
        .await
        .unwrap();

    let (total, open, left): (i32, bool, Option<chrono::DateTime<chrono::Utc>>) = sqlx::query_as(
        "SELECT total_seconds, open, last_left_at FROM attendance
          WHERE session_id = $1 AND user_id = $2",
    )
    .bind(session)
    .bind(student)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(total >= 25, "expected ~30s accrued, got {total}");
    assert!(!open, "row should be closed after leave");
    assert!(left.is_some(), "last_left_at should be stamped");
}

#[tokio::test]
async fn rejoin_increments_reconnect_count() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (student, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, student, "student").await;
    let (_course, session) = course_with_session(&pool, tenant, teacher, chrono::Utc::now()).await;

    backend::db::attendance::record_join(&pool, tenant, session, student)
        .await
        .unwrap();
    backend::db::attendance::record_leave(&pool, tenant, session, student)
        .await
        .unwrap();
    // Re-join: should bump reconnect_count and re-open.
    backend::db::attendance::record_join(&pool, tenant, session, student)
        .await
        .unwrap();

    let (reconnects, open): (i32, bool) = sqlx::query_as(
        "SELECT reconnect_count, open FROM attendance
          WHERE session_id = $1 AND user_id = $2",
    )
    .bind(session)
    .bind(student)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(reconnects, 1, "re-join should increment reconnect_count");
    assert!(open, "re-join should re-open the row");
}

#[tokio::test]
async fn finalize_open_for_session_closes_open_rows() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (student, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, student, "student").await;
    let (_course, session) = course_with_session(&pool, tenant, teacher, chrono::Utc::now()).await;

    // Student joined and never left (dropped socket): row stays open.
    backend::db::attendance::record_join(&pool, tenant, session, student)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE attendance SET last_joined_at = now() - interval '45 seconds'
          WHERE session_id = $1 AND user_id = $2",
    )
    .bind(session)
    .bind(student)
    .execute(&pool)
    .await
    .unwrap();

    let closed = backend::db::attendance::finalize_open_for_session(&pool, session, None)
        .await
        .unwrap();
    assert_eq!(closed, 1, "exactly one open row should be finalized");

    let (total, open): (i32, bool) = sqlx::query_as(
        "SELECT total_seconds, open FROM attendance
          WHERE session_id = $1 AND user_id = $2",
    )
    .bind(session)
    .bind(student)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(!open, "finalize should close the row");
    assert!(
        total >= 40,
        "finalize should accrue the open segment, got {total}"
    );
}

#[tokio::test]
async fn report_endpoint_returns_rows_for_staff() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (student, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, student, "student").await;
    let (_course, session) = course_with_session(&pool, tenant, teacher, chrono::Utc::now()).await;

    backend::db::attendance::record_join(&pool, tenant, session, student)
        .await
        .unwrap();
    backend::db::attendance::record_leave(&pool, tenant, session, student)
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
        &format!("/v1/sessions/{session}/attendance"),
        None,
    )
    .await;
    assert_eq!(s, 200, "{body}");
    let arr = body.as_array().unwrap();
    assert_eq!(arr.len(), 1, "one attendee expected");
    assert_eq!(arr[0]["user_id"], student.to_string());
    assert!(arr[0]["reconnect_count"].as_i64().unwrap() == 0);
}

#[tokio::test]
async fn report_endpoint_forbidden_for_non_staff() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (_course, session) = course_with_session(&pool, tenant, teacher, chrono::Utc::now()).await;

    // A plain student in the same tenant must NOT be able to read the report.
    let (student, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, student, "student").await;

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
        "GET",
        &format!("/v1/sessions/{session}/attendance"),
        None,
    )
    .await;
    assert_eq!(s, 403);
}
