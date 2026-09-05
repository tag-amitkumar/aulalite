# Live-Class Health and Recovery Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add teacher-facing live-class health, diagnostics, and recovery affordances so a teacher can see whether devices, publishing, MediaMTX, room socket, student viewing, and recording are healthy.

**Architecture:** Extend the existing live-room system rather than adding a new subsystem. The backend exposes server-side health facts through `GET /v1/sessions/{id}/health`; the frontend merges those facts with browser-side room state and renders a compact health strip plus diagnostics sheet.

**Tech Stack:** Rust 2021, Axum, SQLx/Postgres, Dioxus 0.7, existing `features-courses` live-room modules, MediaMTX client trait, existing design-system CSS assets.

---

## Scope Check

This plan implements one product slice: live-class health and recovery. It does not implement load testing, media failover, saved whiteboard replay, or a platform operations dashboard.

## File Structure

- Modify: `crates/backend/src/handlers/live_sessions.rs`
  - Add server health DTOs, pure status-mapping helpers, health endpoint, production route, and test route.
- Modify: `crates/backend/tests/live_room.rs`
  - Add DB-backed endpoint tests for teacher access, student denial, and failed-recording retry eligibility.
- Create: `crates/features-courses/src/live_room_health.rs`
  - Own frontend health DTOs, browser-health model, merge helpers, teacher health strip, diagnostics sheet, and student stream notice component.
- Modify: `crates/features-courses/src/lib.rs`
  - Export `live_room_health`.
- Modify: `crates/features-courses/src/live_room_broadcast.rs`
  - Track server health, socket status, diagnostics open state, and render teacher health UI.
- Modify: `crates/features-courses/src/live_room_view.rs`
  - Replace known blank/error video surfaces with `StreamStateNotice`.
- Modify: `crates/design-system/assets/components.css`
  - Add health strip, diagnostics, health row, and stream notice CSS.
- Modify: `crates/shell-web/public/assets/components.css`
  - Mirror the same CSS so editorial asset sync tests keep passing.

## Task 1: Backend Health Contract and Pure Mapping

**Files:**
- Modify: `crates/backend/src/handlers/live_sessions.rs`

- [ ] **Step 1: Add failing backend health mapping tests**

Append this test module near the bottom of `crates/backend/src/handlers/live_sessions.rs`:

```rust
#[cfg(test)]
mod health_tests {
    use super::*;
    use crate::services::mediamtx::{MediaMtxError, PathStatus};

    #[test]
    fn media_server_health_maps_probe_result() {
        let ok = media_server_health(Ok(()));
        assert_eq!(ok.status, LiveHealthStatus::Ok);
        assert_eq!(ok.detail, "Media server API is reachable");

        let err = media_server_health(Err(MediaMtxError::Api("503".into())));
        assert_eq!(err.status, LiveHealthStatus::Error);
        assert!(err.detail.contains("Media server API check failed"), "{err:?}");
    }

    #[test]
    fn main_stream_health_uses_session_lifecycle() {
        let inactive_live = main_stream_health("live", Some(Ok(PathStatus::Inactive)));
        assert_eq!(inactive_live.status, LiveHealthStatus::Error);

        let inactive_scheduled = main_stream_health("scheduled", None);
        assert_eq!(inactive_scheduled.status, LiveHealthStatus::NotApplicable);

        let active_live = main_stream_health("live", Some(Ok(PathStatus::Active)));
        assert_eq!(active_live.status, LiveHealthStatus::Ok);
    }

    #[test]
    fn screen_stream_health_does_not_infer_browser_sharing() {
        let no_path = screen_stream_health(None);
        assert_eq!(no_path.status, LiveHealthStatus::NotApplicable);

        let inactive = screen_stream_health(Some(Ok(PathStatus::Inactive)));
        assert_eq!(inactive.status, LiveHealthStatus::NotApplicable);

        let active = screen_stream_health(Some(Ok(PathStatus::Active)));
        assert_eq!(active.status, LiveHealthStatus::Ok);
    }

    #[test]
    fn recording_health_maps_processing_states_and_retry() {
        let disabled = recording_health_from_parts(false, "live", None, None);
        assert_eq!(disabled.status, LiveHealthStatus::NotApplicable);
        assert!(!disabled.retry_eligible);

        let no_row_live = recording_health_from_parts(true, "live", None, None);
        assert_eq!(no_row_live.status, LiveHealthStatus::Warning);
        assert!(!no_row_live.retry_eligible);

        let remuxing = recording_health_from_parts(true, "ended", Some("remuxing"), None);
        assert_eq!(remuxing.status, LiveHealthStatus::Warning);
        assert!(!remuxing.retry_eligible);

        let available = recording_health_from_parts(true, "ended", Some("available"), None);
        assert_eq!(available.status, LiveHealthStatus::Ok);
        assert!(!available.retry_eligible);

        let failed = recording_health_from_parts(
            true,
            "ended",
            Some("failed"),
            Some("ffmpeg failed with a long provider log"),
        );
        assert_eq!(failed.status, LiveHealthStatus::Error);
        assert!(failed.retry_eligible);
        assert_eq!(failed.processing_status.as_deref(), Some("failed"));
        assert!(failed.processing_error.unwrap().contains("ffmpeg failed"));
    }
}
```

- [ ] **Step 2: Run the mapping tests to verify they fail**

Run:

```powershell
cargo test -p backend --lib health_tests -- --nocapture
```

Expected: compile fails because `LiveHealthStatus`, `media_server_health`, `main_stream_health`, `screen_stream_health`, and `recording_health_from_parts` do not exist.

- [ ] **Step 3: Add health DTOs and pure mapping helpers**

In `crates/backend/src/handlers/live_sessions.rs`, add this code after `JoinResponse`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LiveHealthStatus {
    Ok,
    Warning,
    Error,
    Unknown,
    NotApplicable,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct LiveHealthCheckDto {
    pub status: LiveHealthStatus,
    pub label: String,
    pub detail: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct LiveSessionLifecycleHealthDto {
    pub status: LiveHealthStatus,
    pub lifecycle: String,
    pub scheduled_starts_at: chrono::DateTime<chrono::Utc>,
    pub actual_started_at: Option<chrono::DateTime<chrono::Utc>>,
    pub actual_ended_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct LiveRecordingHealthDto {
    pub enabled: bool,
    pub status: LiveHealthStatus,
    pub processing_status: Option<String>,
    pub processing_error: Option<String>,
    pub retry_eligible: bool,
    pub detail: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct LiveSessionHealthDto {
    pub session_id: Uuid,
    pub checked_at: chrono::DateTime<chrono::Utc>,
    pub session: LiveSessionLifecycleHealthDto,
    pub media_server: LiveHealthCheckDto,
    pub main_stream: LiveHealthCheckDto,
    pub screen_stream: LiveHealthCheckDto,
    pub recording: LiveRecordingHealthDto,
}

fn health_check(
    status: LiveHealthStatus,
    label: impl Into<String>,
    detail: impl Into<String>,
) -> LiveHealthCheckDto {
    LiveHealthCheckDto {
        status,
        label: label.into(),
        detail: detail.into(),
    }
}

fn media_server_health(
    result: Result<(), crate::services::mediamtx::MediaMtxError>,
) -> LiveHealthCheckDto {
    match result {
        Ok(()) => health_check(
            LiveHealthStatus::Ok,
            "Media server",
            "Media server API is reachable",
        ),
        Err(err) => health_check(
            LiveHealthStatus::Error,
            "Media server",
            format!("Media server API check failed: {err}"),
        ),
    }
}

fn main_stream_health(
    session_status: &str,
    path_status: Option<Result<crate::services::mediamtx::PathStatus, crate::services::mediamtx::MediaMtxError>>,
) -> LiveHealthCheckDto {
    use crate::services::mediamtx::PathStatus;

    if session_status != "live" {
        return health_check(
            LiveHealthStatus::NotApplicable,
            "Main stream",
            "Main stream is checked after the session is live",
        );
    }

    match path_status {
        Some(Ok(PathStatus::Active)) => health_check(
            LiveHealthStatus::Ok,
            "Main stream",
            "Teacher stream is active on the media server",
        ),
        Some(Ok(PathStatus::Inactive)) => health_check(
            LiveHealthStatus::Error,
            "Main stream",
            "Session is live but the teacher stream is not active on the media server",
        ),
        Some(Err(err)) => health_check(
            LiveHealthStatus::Unknown,
            "Main stream",
            format!("Could not check teacher stream path: {err}"),
        ),
        None => health_check(
            LiveHealthStatus::Unknown,
            "Main stream",
            "Session is live but no main media path is recorded yet",
        ),
    }
}

fn screen_stream_health(
    path_status: Option<Result<crate::services::mediamtx::PathStatus, crate::services::mediamtx::MediaMtxError>>,
) -> LiveHealthCheckDto {
    use crate::services::mediamtx::PathStatus;

    match path_status {
        Some(Ok(PathStatus::Active)) => health_check(
            LiveHealthStatus::Ok,
            "Screen share",
            "Screen share path is active on the media server",
        ),
        Some(Ok(PathStatus::Inactive)) | None => health_check(
            LiveHealthStatus::NotApplicable,
            "Screen share",
            "No active screen share is visible on the media server",
        ),
        Some(Err(err)) => health_check(
            LiveHealthStatus::Unknown,
            "Screen share",
            format!("Could not check screen share path: {err}"),
        ),
    }
}

fn summarize_processing_error(error: Option<&str>) -> Option<String> {
    let raw = error?.trim();
    if raw.is_empty() {
        return None;
    }
    let mut end = raw.len().min(240);
    while end > 0 && !raw.is_char_boundary(end) {
        end -= 1;
    }
    Some(raw[..end].to_string())
}

fn recording_health_from_parts(
    enabled: bool,
    session_status: &str,
    processing_status: Option<&str>,
    processing_error: Option<&str>,
) -> LiveRecordingHealthDto {
    if !enabled {
        return LiveRecordingHealthDto {
            enabled,
            status: LiveHealthStatus::NotApplicable,
            processing_status: None,
            processing_error: None,
            retry_eligible: false,
            detail: "Recording is disabled for this session".into(),
        };
    }

    match processing_status {
        None if session_status == "live" => LiveRecordingHealthDto {
            enabled,
            status: LiveHealthStatus::Warning,
            processing_status: None,
            processing_error: None,
            retry_eligible: false,
            detail: "Recording is expected; processing starts after class ends".into(),
        },
        None => LiveRecordingHealthDto {
            enabled,
            status: LiveHealthStatus::Unknown,
            processing_status: None,
            processing_error: None,
            retry_eligible: false,
            detail: "No recording row exists for this session yet".into(),
        },
        Some("pending") | Some("remuxing") | Some("uploading") => LiveRecordingHealthDto {
            enabled,
            status: LiveHealthStatus::Warning,
            processing_status: processing_status.map(str::to_string),
            processing_error: None,
            retry_eligible: false,
            detail: "Recording is still processing".into(),
        },
        Some("available") => LiveRecordingHealthDto {
            enabled,
            status: LiveHealthStatus::Ok,
            processing_status: Some("available".into()),
            processing_error: None,
            retry_eligible: false,
            detail: "Recording is available".into(),
        },
        Some("failed") => LiveRecordingHealthDto {
            enabled,
            status: LiveHealthStatus::Error,
            processing_status: Some("failed".into()),
            processing_error: summarize_processing_error(processing_error),
            retry_eligible: session_status == "ended",
            detail: "Recording processing failed".into(),
        },
        Some(other) => LiveRecordingHealthDto {
            enabled,
            status: LiveHealthStatus::Unknown,
            processing_status: Some(other.to_string()),
            processing_error: summarize_processing_error(processing_error),
            retry_eligible: false,
            detail: "Recording is in an unknown processing state".into(),
        },
    }
}

fn recording_health(
    enabled: bool,
    session_status: &str,
    row: Option<&db::recordings::RecordingRow>,
) -> LiveRecordingHealthDto {
    recording_health_from_parts(
        enabled,
        session_status,
        row.map(|r| r.processing_status.as_str()),
        row.and_then(|r| r.processing_error.as_deref()),
    )
}
```

- [ ] **Step 4: Run the mapping tests**

Run:

```powershell
cargo test -p backend --lib health_tests -- --nocapture
```

Expected: all tests in `health_tests` pass.

- [ ] **Step 5: Commit backend mapping contract**

Run:

```powershell
git add crates/backend/src/handlers/live_sessions.rs
git commit -m "feat(live): add health status mapping"
```

Expected: commit succeeds with only `crates/backend/src/handlers/live_sessions.rs` staged.

## Task 2: Backend Health Endpoint and Integration Coverage

**Files:**
- Modify: `crates/backend/src/handlers/live_sessions.rs`
- Modify: `crates/backend/tests/live_room.rs`

- [ ] **Step 1: Add failing endpoint tests**

Append these tests to `crates/backend/tests/live_room.rs` after `go_live_happy_path`:

```rust
#[tokio::test]
async fn health_by_teacher_reports_main_stream_ok_and_redacts_details() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (_course, session) = course_with_session(&pool, tenant, teacher, chrono::Utc::now()).await;
    force_session_live(&pool, session).await;

    let main_path: String =
        sqlx::query_scalar("SELECT main_path FROM live_sessions WHERE id = $1")
            .bind(session)
            .fetch_one(&pool)
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

    let raw = body.to_string();
    assert!(!raw.contains("publish_password"), "{raw}");
    assert!(!raw.contains("publish_nonce"), "{raw}");
    assert!(!raw.contains("MEDIAMTX_AUTH_SHARED_HEADER"), "{raw}");
}

#[tokio::test]
async fn health_by_student_returns_403() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (student, fb, em) = create_user(&pool).await;
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

    let (status, _) = fire(&app, "GET", &format!("/v1/sessions/{session}/health"), None).await;
    assert_eq!(status, 403);
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

    sqlx::query(
        "INSERT INTO recordings
            (tenant_id, session_id, started_at, ended_at, duration_seconds,
             processing_status, processing_error)
         VALUES ($1, $2, now() - interval '1 hour', now(), 3600, 'failed',
                 'ffmpeg failed: sample failure')",
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

    let (status, body) = fire(&app, "GET", &format!("/v1/sessions/{session}/health"), None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["recording"]["status"], "error");
    assert_eq!(body["recording"]["processing_status"], "failed");
    assert_eq!(body["recording"]["retry_eligible"], true);
    assert!(
        body["recording"]["processing_error"]
            .as_str()
            .unwrap()
            .contains("ffmpeg failed"),
        "{body}"
    );
}
```

- [ ] **Step 2: Run the new endpoint tests to verify they fail**

Run:

```powershell
cargo test -p backend --features db-tests --test live_room health_ -- --nocapture
```

Expected: tests fail with `404 Not Found` because `/v1/sessions/{id}/health` is not routed yet.

- [ ] **Step 3: Add the health endpoint implementation**

In `crates/backend/src/handlers/live_sessions.rs`, add this implementation after `join_t`:

```rust
async fn health_inner(
    pool: &PgPool,
    mediamtx_client: &dyn MediaMtxClient,
    ctx: &RequestContext,
    session_id: Uuid,
) -> Result<Json<LiveSessionHealthDto>, ApiError> {
    let session =
        db::live_sessions::load_for_join_with_context(pool, ctx.user_id, ctx.tenant_id, session_id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?
            .ok_or(ApiError::NotFound)?;
    let tenant_id = ctx
        .tenant_id
        .ok_or_else(|| ApiError::BadRequest("no tenant".into()))?;
    if session.tenant_id != tenant_id {
        return Err(ApiError::NotFound);
    }

    let allowed = if ctx.is_platform_admin {
        true
    } else {
        db::courses::caller_can_staff_course(
            pool,
            session.course_id,
            ctx.user_id,
            is_org_admin(ctx),
        )
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
    };
    if !allowed {
        return Err(ApiError::Forbidden);
    }

    let media_server = media_server_health(mediamtx_client.healthz().await);

    let main_status = match session.main_path.as_deref() {
        Some(path) => Some(mediamtx_client.path_status(path).await),
        None => None,
    };
    let main_stream = main_stream_health(&session.status, main_status);

    let screen_status = match session.screen_path.as_deref() {
        Some(path) => Some(mediamtx_client.path_status(path).await),
        None => None,
    };
    let screen_stream = screen_stream_health(screen_status);

    let recording_row = {
        let mut tx = db::begin_with_context(pool, ctx.user_id, ctx.tenant_id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        let row = db::recordings::fetch_by_session(&mut *tx, session_id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        tx.commit()
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        row
    };

    Ok(Json(LiveSessionHealthDto {
        session_id,
        checked_at: chrono::Utc::now(),
        session: LiveSessionLifecycleHealthDto {
            status: if session.status == "live" {
                LiveHealthStatus::Ok
            } else {
                LiveHealthStatus::NotApplicable
            },
            lifecycle: session.status.clone(),
            scheduled_starts_at: session.starts_at,
            actual_started_at: session.actual_started_at,
            actual_ended_at: session.actual_ended_at,
        },
        media_server,
        main_stream,
        screen_stream,
        recording: recording_health(session.recording_enabled, &session.status, recording_row.as_ref()),
    }))
}

async fn health(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(session_id): Path<Uuid>,
) -> Result<Json<LiveSessionHealthDto>, ApiError> {
    health_inner(&s.pool, s.mediamtx.as_ref(), &ctx, session_id).await
}

async fn health_t(
    State(s): State<LiveRoomTestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(session_id): Path<Uuid>,
) -> Result<Json<LiveSessionHealthDto>, ApiError> {
    health_inner(&s.pool, s.mediamtx.as_ref(), &ctx, session_id).await
}
```

- [ ] **Step 4: Add the route to production and test routers**

In `live_room_routes`, add this route next to `/join`:

```rust
.route("/v1/sessions/{id}/health", routing::get(health))
```

In `live_room_router_for_tests`, add this route next to `/join`:

```rust
.route("/v1/sessions/{id}/health", routing::get(health_t))
```

- [ ] **Step 5: Run backend tests**

Run:

```powershell
cargo test -p backend --lib health_tests -- --nocapture
cargo test -p backend --features db-tests --test live_room health_ -- --nocapture
```

Expected: all health mapping and endpoint tests pass.

- [ ] **Step 6: Commit backend endpoint**

Run:

```powershell
git add crates/backend/src/handlers/live_sessions.rs crates/backend/tests/live_room.rs
git commit -m "feat(live): expose session health endpoint"
```

Expected: commit succeeds with the handler and live-room integration test changes.

## Task 3: Frontend Health Model and UI Components

**Files:**
- Create: `crates/features-courses/src/live_room_health.rs`
- Modify: `crates/features-courses/src/lib.rs`

- [ ] **Step 1: Create failing frontend health module tests**

Create `crates/features-courses/src/live_room_health.rs` with this initial test-first content:

```rust
use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

#[cfg(test)]
mod tests {
    use super::*;

    fn check(status: HealthStatus, label: &str) -> HealthCheckDto {
        HealthCheckDto {
            status,
            label: label.into(),
            detail: format!("{label} detail"),
        }
    }

    fn server_health(main: HealthStatus, recording: HealthStatus) -> LiveSessionHealthDto {
        LiveSessionHealthDto {
            session_id: "s1".into(),
            checked_at: "2026-06-29T00:00:00Z".into(),
            session: SessionLifecycleHealthDto {
                status: HealthStatus::Ok,
                lifecycle: "live".into(),
            },
            media_server: check(HealthStatus::Ok, "Media server"),
            main_stream: check(main, "Main stream"),
            screen_stream: check(HealthStatus::NotApplicable, "Screen share"),
            recording: RecordingHealthDto {
                enabled: true,
                status: recording,
                processing_status: Some("failed".into()),
                processing_error: Some("ffmpeg failed".into()),
                retry_eligible: recording == HealthStatus::Error,
                detail: "Recording detail".into(),
            },
        }
    }

    #[test]
    fn merge_promotes_main_stream_error_over_local_ok() {
        let server = server_health(HealthStatus::Error, HealthStatus::Ok);
        let browser = BrowserRoomHealth {
            devices_captured: true,
            publish_active: true,
            screen_publish_active: false,
            socket_status: SocketHealthStatus::Connected,
            quality: crate::live_room_stats::NetQuality::Good,
            video_error: None,
        };

        let model = merge_live_room_health(Some(&server), browser);
        assert_eq!(model.stream.status, HealthStatus::Error);
        assert_eq!(model.overall_status, HealthStatus::Error);
    }

    #[test]
    fn merge_marks_inactive_screen_warning_only_when_browser_is_sharing() {
        let server = server_health(HealthStatus::Ok, HealthStatus::Ok);
        let browser = BrowserRoomHealth {
            devices_captured: true,
            publish_active: true,
            screen_publish_active: true,
            socket_status: SocketHealthStatus::Connected,
            quality: crate::live_room_stats::NetQuality::Good,
            video_error: None,
        };

        let model = merge_live_room_health(Some(&server), browser);
        assert_eq!(model.screen.status, HealthStatus::Warning);
    }

    #[test]
    fn health_strip_renders_statuses() {
        fn app() -> Element {
            let server = server_health(HealthStatus::Ok, HealthStatus::Warning);
            let browser = BrowserRoomHealth {
                devices_captured: true,
                publish_active: true,
                screen_publish_active: false,
                socket_status: SocketHealthStatus::Connected,
                quality: crate::live_room_stats::NetQuality::Fair,
                video_error: None,
            };
            let model = merge_live_room_health(Some(&server), browser);
            rsx! {
                LiveRoomHealthStrip {
                    model,
                    on_open_diagnostics: |_| {},
                }
            }
        }

        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("live-health-strip"), "{html}");
        assert!(html.contains("Recording"), "{html}");
        assert!(html.contains("Diagnostics"), "{html}");
    }

    #[test]
    fn diagnostics_sheet_renders_retry_recording_action() {
        fn app() -> Element {
            let server = server_health(HealthStatus::Ok, HealthStatus::Error);
            let browser = BrowserRoomHealth {
                devices_captured: true,
                publish_active: true,
                screen_publish_active: false,
                socket_status: SocketHealthStatus::Connected,
                quality: crate::live_room_stats::NetQuality::Good,
                video_error: None,
            };
            let model = merge_live_room_health(Some(&server), browser);
            rsx! {
                LiveRoomDiagnosticsSheet {
                    open: true,
                    model,
                    on_close: |_| {},
                    on_refresh: |_| {},
                    on_recheck_devices: |_| {},
                    on_retry_publish: |_| {},
                    on_restart_screen: |_| {},
                    on_retry_recording: |_| {},
                }
            }
        }

        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("live-health-sheet"), "{html}");
        assert!(html.contains("Retry recording"), "{html}");
    }

    #[test]
    fn stream_notice_renders_retry() {
        fn app() -> Element {
            rsx! {
                StreamStateNotice {
                    state: StreamNoticeState::Failed,
                    message: "Could not connect".to_string(),
                    on_retry: |_| {},
                }
            }
        }

        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("Could not connect"), "{html}");
        assert!(html.contains("Retry"), "{html}");
    }
}
```

- [ ] **Step 2: Run the frontend module tests to verify they fail**

Run:

```powershell
cargo test -p features-courses --lib live_room_health -- --nocapture
```

Expected: compile fails because the health types and components are not defined.

- [ ] **Step 3: Replace the module with the complete health model and components**

Replace `crates/features-courses/src/live_room_health.rs` with:

```rust
use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthStatus {
    Ok,
    Warning,
    Error,
    Unknown,
    NotApplicable,
}

impl HealthStatus {
    pub fn label(self) -> &'static str {
        match self {
            HealthStatus::Ok => "OK",
            HealthStatus::Warning => "Check",
            HealthStatus::Error => "Action needed",
            HealthStatus::Unknown => "Unknown",
            HealthStatus::NotApplicable => "Off",
        }
    }

    pub fn class(self) -> &'static str {
        match self {
            HealthStatus::Ok => "ok",
            HealthStatus::Warning => "warning",
            HealthStatus::Error => "error",
            HealthStatus::Unknown => "unknown",
            HealthStatus::NotApplicable => "muted",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct HealthCheckDto {
    pub status: HealthStatus,
    pub label: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct SessionLifecycleHealthDto {
    pub status: HealthStatus,
    pub lifecycle: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct RecordingHealthDto {
    pub enabled: bool,
    pub status: HealthStatus,
    pub processing_status: Option<String>,
    pub processing_error: Option<String>,
    pub retry_eligible: bool,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct LiveSessionHealthDto {
    pub session_id: String,
    pub checked_at: String,
    pub session: SessionLifecycleHealthDto,
    pub media_server: HealthCheckDto,
    pub main_stream: HealthCheckDto,
    pub screen_stream: HealthCheckDto,
    pub recording: RecordingHealthDto,
}

pub async fn fetch_session_health(
    cx: &crate::api::ApiContext,
    session_id: &str,
) -> Result<LiveSessionHealthDto, crate::api::ApiError> {
    crate::api::fetch_json(
        cx,
        "GET",
        &format!("/v1/sessions/{session_id}/health"),
        None::<&()>,
    )
    .await
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketHealthStatus {
    Connected,
    Reconnecting,
    Disconnected,
}

impl From<crate::live_room_socket::ConnStatus> for SocketHealthStatus {
    fn from(value: crate::live_room_socket::ConnStatus) -> Self {
        match value {
            crate::live_room_socket::ConnStatus::Connected => Self::Connected,
            crate::live_room_socket::ConnStatus::Reconnecting => Self::Reconnecting,
            crate::live_room_socket::ConnStatus::Disconnected => Self::Disconnected,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BrowserRoomHealth {
    pub devices_captured: bool,
    pub publish_active: bool,
    pub screen_publish_active: bool,
    pub socket_status: SocketHealthStatus,
    pub quality: crate::live_room_stats::NetQuality,
    pub video_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthItem {
    pub key: &'static str,
    pub label: String,
    pub status: HealthStatus,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LiveRoomHealthModel {
    pub overall_status: HealthStatus,
    pub devices: HealthItem,
    pub stream: HealthItem,
    pub screen: HealthItem,
    pub room: HealthItem,
    pub recording: HealthItem,
    pub quality: crate::live_room_stats::NetQuality,
    pub checked_at: Option<String>,
    pub retry_recording: bool,
}

fn item(
    key: &'static str,
    label: impl Into<String>,
    status: HealthStatus,
    detail: impl Into<String>,
) -> HealthItem {
    HealthItem {
        key,
        label: label.into(),
        status,
        detail: detail.into(),
    }
}

fn worst(a: HealthStatus, b: HealthStatus) -> HealthStatus {
    use HealthStatus::*;
    match (a, b) {
        (Error, _) | (_, Error) => Error,
        (Warning, _) | (_, Warning) => Warning,
        (Unknown, _) | (_, Unknown) => Unknown,
        (Ok, _) | (_, Ok) => Ok,
        _ => NotApplicable,
    }
}

pub fn merge_live_room_health(
    server: Option<&LiveSessionHealthDto>,
    browser: BrowserRoomHealth,
) -> LiveRoomHealthModel {
    let devices = if browser.devices_captured {
        item("devices", "Devices", HealthStatus::Ok, "Camera and microphone are captured")
    } else {
        item(
            "devices",
            "Devices",
            HealthStatus::Warning,
            "Camera and microphone are not captured",
        )
    };

    let server_stream = server
        .map(|h| h.main_stream.clone())
        .unwrap_or_else(|| HealthCheckDto {
            status: HealthStatus::Unknown,
            label: "Main stream".into(),
            detail: "Server stream health has not loaded yet".into(),
        });
    let local_stream_status = if browser.publish_active {
        HealthStatus::Ok
    } else {
        HealthStatus::Warning
    };
    let stream = item(
        "stream",
        "Stream",
        worst(server_stream.status, local_stream_status),
        server_stream.detail,
    );

    let server_screen = server
        .map(|h| h.screen_stream.clone())
        .unwrap_or_else(|| HealthCheckDto {
            status: HealthStatus::NotApplicable,
            label: "Screen share".into(),
            detail: "Screen share is not active".into(),
        });
    let screen_status = if browser.screen_publish_active
        && server_screen.status == HealthStatus::NotApplicable
    {
        HealthStatus::Warning
    } else {
        server_screen.status
    };
    let screen = item("screen", "Screen", screen_status, server_screen.detail);

    let room = match browser.socket_status {
        SocketHealthStatus::Connected => {
            item("room", "Room", HealthStatus::Ok, "Live room socket is connected")
        }
        SocketHealthStatus::Reconnecting => item(
            "room",
            "Room",
            HealthStatus::Warning,
            "Live room socket is reconnecting",
        ),
        SocketHealthStatus::Disconnected => item(
            "room",
            "Room",
            HealthStatus::Error,
            "Live room socket disconnected",
        ),
    };

    let recording = server
        .map(|h| {
            let detail = h
                .recording
                .processing_error
                .clone()
                .unwrap_or_else(|| h.recording.detail.clone());
            item("recording", "Recording", h.recording.status, detail)
        })
        .unwrap_or_else(|| {
            item(
                "recording",
                "Recording",
                HealthStatus::Unknown,
                "Recording health has not loaded yet",
            )
        });

    let mut overall = devices.status;
    for s in [stream.status, screen.status, room.status, recording.status] {
        overall = worst(overall, s);
    }

    LiveRoomHealthModel {
        overall_status: overall,
        devices,
        stream,
        screen,
        room,
        recording,
        quality: browser.quality,
        checked_at: server.map(|h| h.checked_at.clone()),
        retry_recording: server.map(|h| h.recording.retry_eligible).unwrap_or(false),
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct LiveRoomHealthStripProps {
    pub model: LiveRoomHealthModel,
    pub on_open_diagnostics: EventHandler<()>,
}

#[component]
pub fn LiveRoomHealthStrip(props: LiveRoomHealthStripProps) -> Element {
    let model = props.model;
    let items = vec![
        model.devices.clone(),
        model.stream.clone(),
        model.room.clone(),
        model.recording.clone(),
    ];

    rsx! {
        div { class: "live-health-strip",
            div { class: "live-health-strip__summary live-health-strip__summary--{model.overall_status.class()}",
                span { class: "live-health-dot", "aria-hidden": "true" }
                span { class: "live-health-summary-label", "{model.overall_status.label()}" }
            }
            div { class: "live-health-strip__items",
                for item in items {
                    span {
                        key: "{item.key}",
                        class: "live-health-chip live-health-chip--{item.status.class()}",
                        title: "{item.detail}",
                        "{item.label}: {item.status.label()}"
                    }
                }
                crate::live_room_stats::NetworkQualityBadge {
                    quality: model.quality,
                    show_label: false,
                }
            }
            button {
                r#type: "button",
                class: "live-health-diagnostics-button",
                onclick: move |_| props.on_open_diagnostics.call(()),
                "Diagnostics"
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct LiveRoomDiagnosticsSheetProps {
    pub open: bool,
    pub model: LiveRoomHealthModel,
    pub on_close: EventHandler<()>,
    pub on_refresh: EventHandler<()>,
    pub on_recheck_devices: EventHandler<()>,
    pub on_retry_publish: EventHandler<()>,
    pub on_restart_screen: EventHandler<()>,
    pub on_retry_recording: EventHandler<()>,
}

#[component]
pub fn LiveRoomDiagnosticsSheet(props: LiveRoomDiagnosticsSheetProps) -> Element {
    if !props.open {
        return rsx! {};
    }

    let rows = vec![
        props.model.devices.clone(),
        props.model.stream.clone(),
        props.model.screen.clone(),
        props.model.room.clone(),
        props.model.recording.clone(),
    ];
    let checked_at = props
        .model
        .checked_at
        .clone()
        .unwrap_or_else(|| "Not checked yet".into());
    let show_retry_publish = props.model.stream.status == HealthStatus::Error
        || props.model.stream.status == HealthStatus::Warning;
    let show_restart_screen = props.model.screen.status == HealthStatus::Warning
        || props.model.screen.status == HealthStatus::Error;

    rsx! {
        div { class: "live-health-sheet", role: "dialog", "aria-label": "Live room diagnostics",
            div { class: "live-health-sheet__header",
                div {
                    h3 { "Live room diagnostics" }
                    p { "Last checked: {checked_at}" }
                }
                button {
                    r#type: "button",
                    class: "live-health-sheet__close",
                    onclick: move |_| props.on_close.call(()),
                    "Close"
                }
            }
            div { class: "live-health-sheet__rows",
                for row in rows {
                    HealthCheckRow { key: "{row.key}", row }
                }
            }
            div { class: "live-health-actions",
                button {
                    r#type: "button",
                    class: "live-health-action",
                    onclick: move |_| props.on_refresh.call(()),
                    "Refresh room health"
                }
                button {
                    r#type: "button",
                    class: "live-health-action",
                    onclick: move |_| props.on_recheck_devices.call(()),
                    "Recheck devices"
                }
                if show_retry_publish {
                    button {
                        r#type: "button",
                        class: "live-health-action live-health-action--primary",
                        onclick: move |_| props.on_retry_publish.call(()),
                        "Retry publish"
                    }
                }
                if show_restart_screen {
                    button {
                        r#type: "button",
                        class: "live-health-action",
                        onclick: move |_| props.on_restart_screen.call(()),
                        "Restart screen share"
                    }
                }
                if props.model.retry_recording {
                    button {
                        r#type: "button",
                        class: "live-health-action live-health-action--danger",
                        onclick: move |_| props.on_retry_recording.call(()),
                        "Retry recording"
                    }
                }
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct HealthCheckRowProps {
    pub row: HealthItem,
}

#[component]
pub fn HealthCheckRow(props: HealthCheckRowProps) -> Element {
    let row = props.row;
    rsx! {
        div { class: "live-health-row live-health-row--{row.status.class()}",
            span { class: "live-health-row__status", "{row.status.label()}" }
            div { class: "live-health-row__body",
                strong { "{row.label}" }
                span { "{row.detail}" }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamNoticeState {
    Waiting,
    Connecting,
    Retrying,
    Failed,
}

#[derive(Props, Clone, PartialEq)]
pub struct StreamStateNoticeProps {
    pub state: StreamNoticeState,
    pub message: String,
    pub on_retry: EventHandler<()>,
}

#[component]
pub fn StreamStateNotice(props: StreamStateNoticeProps) -> Element {
    let title = match props.state {
        StreamNoticeState::Waiting => "Waiting for the teacher",
        StreamNoticeState::Connecting => "Connecting stream",
        StreamNoticeState::Retrying => "Retrying stream",
        StreamNoticeState::Failed => "Stream unavailable",
    };
    rsx! {
        div { class: "stream-state-notice stream-state-notice--{props.state as u8}",
            h3 { "{title}" }
            p { "{props.message}" }
            if props.state == StreamNoticeState::Failed {
                button {
                    r#type: "button",
                    class: "live-video-error-retry",
                    onclick: move |_| props.on_retry.call(()),
                    "Retry"
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(status: HealthStatus, label: &str) -> HealthCheckDto {
        HealthCheckDto {
            status,
            label: label.into(),
            detail: format!("{label} detail"),
        }
    }

    fn server_health(main: HealthStatus, recording: HealthStatus) -> LiveSessionHealthDto {
        LiveSessionHealthDto {
            session_id: "s1".into(),
            checked_at: "2026-06-29T00:00:00Z".into(),
            session: SessionLifecycleHealthDto {
                status: HealthStatus::Ok,
                lifecycle: "live".into(),
            },
            media_server: check(HealthStatus::Ok, "Media server"),
            main_stream: check(main, "Main stream"),
            screen_stream: check(HealthStatus::NotApplicable, "Screen share"),
            recording: RecordingHealthDto {
                enabled: true,
                status: recording,
                processing_status: Some("failed".into()),
                processing_error: Some("ffmpeg failed".into()),
                retry_eligible: recording == HealthStatus::Error,
                detail: "Recording detail".into(),
            },
        }
    }

    #[test]
    fn merge_promotes_main_stream_error_over_local_ok() {
        let server = server_health(HealthStatus::Error, HealthStatus::Ok);
        let browser = BrowserRoomHealth {
            devices_captured: true,
            publish_active: true,
            screen_publish_active: false,
            socket_status: SocketHealthStatus::Connected,
            quality: crate::live_room_stats::NetQuality::Good,
            video_error: None,
        };

        let model = merge_live_room_health(Some(&server), browser);
        assert_eq!(model.stream.status, HealthStatus::Error);
        assert_eq!(model.overall_status, HealthStatus::Error);
    }

    #[test]
    fn merge_marks_inactive_screen_warning_only_when_browser_is_sharing() {
        let server = server_health(HealthStatus::Ok, HealthStatus::Ok);
        let browser = BrowserRoomHealth {
            devices_captured: true,
            publish_active: true,
            screen_publish_active: true,
            socket_status: SocketHealthStatus::Connected,
            quality: crate::live_room_stats::NetQuality::Good,
            video_error: None,
        };

        let model = merge_live_room_health(Some(&server), browser);
        assert_eq!(model.screen.status, HealthStatus::Warning);
    }

    #[test]
    fn health_strip_renders_statuses() {
        fn app() -> Element {
            let server = server_health(HealthStatus::Ok, HealthStatus::Warning);
            let browser = BrowserRoomHealth {
                devices_captured: true,
                publish_active: true,
                screen_publish_active: false,
                socket_status: SocketHealthStatus::Connected,
                quality: crate::live_room_stats::NetQuality::Fair,
                video_error: None,
            };
            let model = merge_live_room_health(Some(&server), browser);
            rsx! {
                LiveRoomHealthStrip {
                    model,
                    on_open_diagnostics: |_| {},
                }
            }
        }

        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("live-health-strip"), "{html}");
        assert!(html.contains("Recording"), "{html}");
        assert!(html.contains("Diagnostics"), "{html}");
    }

    #[test]
    fn diagnostics_sheet_renders_retry_recording_action() {
        fn app() -> Element {
            let server = server_health(HealthStatus::Ok, HealthStatus::Error);
            let browser = BrowserRoomHealth {
                devices_captured: true,
                publish_active: true,
                screen_publish_active: false,
                socket_status: SocketHealthStatus::Connected,
                quality: crate::live_room_stats::NetQuality::Good,
                video_error: None,
            };
            let model = merge_live_room_health(Some(&server), browser);
            rsx! {
                LiveRoomDiagnosticsSheet {
                    open: true,
                    model,
                    on_close: |_| {},
                    on_refresh: |_| {},
                    on_recheck_devices: |_| {},
                    on_retry_publish: |_| {},
                    on_restart_screen: |_| {},
                    on_retry_recording: |_| {},
                }
            }
        }

        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("live-health-sheet"), "{html}");
        assert!(html.contains("Retry recording"), "{html}");
    }

    #[test]
    fn stream_notice_renders_retry() {
        fn app() -> Element {
            rsx! {
                StreamStateNotice {
                    state: StreamNoticeState::Failed,
                    message: "Could not connect".to_string(),
                    on_retry: |_| {},
                }
            }
        }

        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("Could not connect"), "{html}");
        assert!(html.contains("Retry"), "{html}");
    }
}
```

- [ ] **Step 4: Export the module**

In `crates/features-courses/src/lib.rs`, add this line near the other live-room modules:

```rust
pub mod live_room_health;
```

- [ ] **Step 5: Run frontend health tests**

Run:

```powershell
cargo test -p features-courses --lib live_room_health -- --nocapture
```

Expected: all `live_room_health` tests pass.

- [ ] **Step 6: Commit frontend health module**

Run:

```powershell
git add crates/features-courses/src/live_room_health.rs crates/features-courses/src/lib.rs
git commit -m "feat(live): add frontend health model"
```

Expected: commit succeeds with the new module and export.

## Task 4: Wire Teacher Health Into Broadcast Room

**Files:**
- Modify: `crates/features-courses/src/live_room_broadcast.rs`

- [ ] **Step 1: Add socket status to teacher sidebar state**

In `BroadcastSidebarState`, add:

```rust
    /// Coarse WebSocket health surfaced to the teacher diagnostics strip.
    socket_status: crate::live_room_socket::ConnStatus,
```

- [ ] **Step 2: Add health signals after `sidebar_state`**

In `LiveRoomBroadcast`, immediately after `let sidebar_state = use_signal(BroadcastSidebarState::default);`, add:

```rust
    let server_health = use_signal(|| None::<crate::live_room_health::LiveSessionHealthDto>);
    let health_error = use_signal(|| None::<String>);
    let diagnostics_open = use_signal(|| false);
```

- [ ] **Step 3: Add a reusable refresh closure**

After the `session_id` local is created, add this helper closure:

```rust
    let refresh_health = {
        let session_id = props.session_id.clone();
        let server_health = server_health;
        let health_error = health_error;
        move |_| {
            #[cfg(target_arch = "wasm32")]
            {
                let cx = crate::api::use_api();
                let session_id = session_id.clone();
                let mut server_health = server_health;
                let mut health_error = health_error;
                wasm_bindgen_futures::spawn_local(async move {
                    match crate::live_room_health::fetch_session_health(&cx, &session_id).await {
                        Ok(dto) => {
                            server_health.set(Some(dto));
                            health_error.set(None);
                        }
                        Err(err) => {
                            health_error.set(Some(err.to_string()));
                        }
                    }
                });
            }
        }
    };
```

- [ ] **Step 4: Poll server health while mounted**

Below the `refresh_health` closure, add:

```rust
    #[cfg(target_arch = "wasm32")]
    {
        let session_id = props.session_id.clone();
        let server_health = server_health;
        let health_error = health_error;
        let stopped: std::rc::Rc<std::cell::Cell<bool>> =
            use_hook(|| std::rc::Rc::new(std::cell::Cell::new(false)));
        {
            let stopped = stopped.clone();
            use_drop(move || stopped.set(true));
        }
        use_effect(move || {
            let cx = crate::api::use_api();
            let session_id = session_id.clone();
            let mut server_health = server_health;
            let mut health_error = health_error;
            let stopped = stopped.clone();
            wasm_bindgen_futures::spawn_local(async move {
                loop {
                    match crate::live_room_health::fetch_session_health(&cx, &session_id).await {
                        Ok(dto) => {
                            server_health.set(Some(dto));
                            health_error.set(None);
                        }
                        Err(err) => health_error.set(Some(err.to_string())),
                    }
                    gloo_timers::future::TimeoutFuture::new(12_000).await;
                    if stopped.get() {
                        break;
                    }
                }
            });
        });
    }
```

- [ ] **Step 5: Store socket status instead of only logging it**

In `use_persistent_socket_broadcast`, replace the current `on_status` closure:

```rust
        let on_status: Rc<dyn Fn(crate::live_room_socket::ConnStatus)> = Rc::new(|st| {
            web_sys::console::debug_2(
                &"[live_room_broadcast] conn status:".into(),
                &format!("{st:?}").into(),
            );
        });
```

with:

```rust
        let state_for_status = sidebar_state;
        let on_status: Rc<dyn Fn(crate::live_room_socket::ConnStatus)> = Rc::new(move |st| {
            let mut state_w = state_for_status;
            state_w.write().socket_status = st;
        });
```

- [ ] **Step 6: Build the health model in render**

Immediately before `rsx! {` in `LiveRoomBroadcast`, add:

```rust
    let browser_health = crate::live_room_health::BrowserRoomHealth {
        devices_captured: matches!(current, PublishState::Live { main_active: true, .. }),
        publish_active: matches!(current, PublishState::Live { main_active: true, .. }),
        screen_publish_active: matches!(current, PublishState::Live { screen_active: true, .. }),
        socket_status: sidebar_state.read().socket_status.into(),
        quality: *quality.read(),
        video_error: match &current {
            PublishState::Error(msg) => Some(msg.clone()),
            _ => None,
        },
    };
    let health_model =
        crate::live_room_health::merge_live_room_health(server_health.read().as_ref(), browser_health);
```

- [ ] **Step 7: Render the strip and diagnostics in the live state**

Inside the `PublishState::Live { main_active, screen_active } => rsx! { ... }` branch, after the existing `div { class: "broadcast-status", ... }`, insert:

```rust
                        crate::live_room_health::LiveRoomHealthStrip {
                            model: health_model.clone(),
                            on_open_diagnostics: move |_| {
                                let mut open = diagnostics_open;
                                open.set(true);
                            },
                        }
                        if let Some(err) = health_error.read().as_ref() {
                            div { class: "system-state system-state--error", "Health check failed: {err}" }
                        }
                        crate::live_room_health::LiveRoomDiagnosticsSheet {
                            open: *diagnostics_open.read(),
                            model: health_model.clone(),
                            on_close: move |_| {
                                let mut open = diagnostics_open;
                                open.set(false);
                            },
                            on_refresh: refresh_health,
                            on_recheck_devices: move |_| {
                                state.set(PublishState::Idle);
                                let mut open = diagnostics_open;
                                open.set(false);
                            },
                            on_retry_publish: move |_| {
                                state.set(PublishState::Idle);
                                let mut open = diagnostics_open;
                                open.set(false);
                            },
                            on_restart_screen: move |_| {
                                #[cfg(target_arch = "wasm32")]
                                {
                                    let session = session;
                                    let screen_stream = screen_stream;
                                    wasm_bindgen_futures::spawn_local(async move {
                                        stop_screen_share_flow(session, screen_stream).await;
                                    });
                                }
                            },
                            on_retry_recording: move |_| {
                                #[cfg(target_arch = "wasm32")]
                                {
                                    let cx = crate::api::use_api();
                                    let session_id = props.session_id.clone();
                                    let mut server_health = server_health;
                                    wasm_bindgen_futures::spawn_local(async move {
                                        let path = format!("/v1/sessions/{session_id}/recording/retry");
                                        let _ = crate::api::fetch_json::<serde_json::Value>(
                                            &cx,
                                            "POST",
                                            &path,
                                            None::<&()>,
                                        )
                                        .await;
                                        if let Ok(dto) =
                                            crate::live_room_health::fetch_session_health(&cx, &session_id).await
                                        {
                                            server_health.set(Some(dto));
                                        }
                                    });
                                }
                            },
                        }
```

- [ ] **Step 8: Run focused frontend tests**

Run:

```powershell
cargo test -p features-courses --lib live_room_broadcast -- --nocapture
cargo test -p features-courses --lib live_room_health -- --nocapture
```

Expected: tests compile and pass.

- [ ] **Step 9: Commit broadcast wiring**

Run:

```powershell
git add crates/features-courses/src/live_room_broadcast.rs
git commit -m "feat(live): show teacher room health"
```

Expected: commit succeeds with only the broadcast module staged.

## Task 5: Student Stream State Notices

**Files:**
- Modify: `crates/features-courses/src/live_room_view.rs`

- [ ] **Step 1: Add a stream notice test**

In the existing `#[cfg(test)] mod tests` in `crates/features-courses/src/live_room_view.rs`, add:

```rust
    #[test]
    fn stream_notice_component_renders_failed_retry_state() {
        fn app() -> Element {
            rsx! {
                crate::live_room_health::StreamStateNotice {
                    state: crate::live_room_health::StreamNoticeState::Failed,
                    message: "Could not connect to the video stream".to_string(),
                    on_retry: |_| {},
                }
            }
        }

        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("Stream unavailable"), "{html}");
        assert!(html.contains("Retry"), "{html}");
    }
```

- [ ] **Step 2: Run the new test**

Run:

```powershell
cargo test -p features-courses --lib live_room_view::tests::stream_notice_component_renders_failed_retry_state -- --nocapture
```

Expected: test passes because `StreamStateNotice` exists from Task 3.

- [ ] **Step 3: Use `StreamStateNotice` for WHEP error overlay**

In `render_webrtc`, replace the `Some(msg) => rsx! { div { class: "live-video-error", ... } }` branch with:

```rust
        Some(msg) => rsx! {
            div { class: "live-video-error",
                crate::live_room_health::StreamStateNotice {
                    state: crate::live_room_health::StreamNoticeState::Failed,
                    message: msg.clone(),
                    on_retry: move |_| {
                        #[cfg(target_arch = "wasm32")]
                        if let Some(win) = web_sys::window() {
                            let _ = win.location().reload();
                        }
                    },
                }
            }
        },
```

- [ ] **Step 4: Add waiting notices for missing stream URL**

In `render_webrtc`, before the existing `rsx! { div { class: "live-stage-surfaces", ... } }`, add:

```rust
    if props.main_url.clone().unwrap_or_default().is_empty() {
        return rsx! {
            div { class: "live-stage-surfaces",
                crate::live_room_health::StreamStateNotice {
                    state: crate::live_room_health::StreamNoticeState::Waiting,
                    message: "The teacher stream is not available yet.".to_string(),
                    on_retry: |_| {},
                }
            }
        };
    }
```

In `render_hls`, before the `use_effect`, add:

```rust
    if main_url.is_empty() {
        return rsx! {
            div { class: "live-stage-surfaces",
                crate::live_room_health::StreamStateNotice {
                    state: crate::live_room_health::StreamNoticeState::Waiting,
                    message: "The class stream is not available yet.".to_string(),
                    on_retry: |_| {},
                }
            }
        };
    }
```

- [ ] **Step 5: Run live-room view tests**

Run:

```powershell
cargo test -p features-courses --lib live_room_view -- --nocapture
cargo test -p features-courses --test live_room_smoke -- --nocapture
```

Expected: tests compile and pass.

- [ ] **Step 6: Commit student stream notices**

Run:

```powershell
git add crates/features-courses/src/live_room_view.rs
git commit -m "feat(live): clarify student stream states"
```

Expected: commit succeeds with only `live_room_view.rs` staged.

## Task 6: Health UI Styling

**Files:**
- Modify: `crates/design-system/assets/components.css`
- Modify: `crates/shell-web/public/assets/components.css`

- [ ] **Step 1: Add CSS to the design-system asset**

Append this CSS to `crates/design-system/assets/components.css`:

```css
.live-health-strip {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-2, 0.5rem);
  padding: 0.625rem 0.75rem;
  border: 1px solid var(--color-border, #ded6c8);
  border-radius: 8px;
  background: var(--color-surface, #fffaf1);
}

.live-health-strip__summary,
.live-health-strip__items {
  display: inline-flex;
  align-items: center;
  gap: 0.5rem;
  min-width: 0;
}

.live-health-dot {
  width: 0.625rem;
  height: 0.625rem;
  border-radius: 999px;
  background: currentColor;
}

.live-health-strip__summary--ok { color: var(--color-success, #2f6552); }
.live-health-strip__summary--warning { color: var(--color-warning, #b08842); }
.live-health-strip__summary--error { color: var(--color-danger, #8a3b4d); }
.live-health-strip__summary--unknown,
.live-health-strip__summary--muted { color: var(--color-text-muted, #6b6b6b); }

.live-health-summary-label {
  font-weight: 700;
  white-space: nowrap;
}

.live-health-chip {
  display: inline-flex;
  align-items: center;
  min-height: 1.75rem;
  padding: 0.25rem 0.5rem;
  border-radius: 999px;
  font-size: 0.8125rem;
  font-weight: 600;
  background: var(--color-surface-muted, #f4f1ea);
  color: var(--color-text, #1f2933);
  white-space: nowrap;
}

.live-health-chip--ok { color: var(--color-success, #2f6552); }
.live-health-chip--warning { color: var(--color-warning, #b08842); }
.live-health-chip--error { color: var(--color-danger, #8a3b4d); }
.live-health-chip--unknown,
.live-health-chip--muted { color: var(--color-text-muted, #6b6b6b); }

.live-health-diagnostics-button,
.live-health-action,
.live-health-sheet__close {
  border: 1px solid var(--color-border, #ded6c8);
  border-radius: 8px;
  background: var(--color-surface, #fffaf1);
  color: var(--color-text, #1f2933);
  font: inherit;
  font-weight: 700;
  min-height: 2rem;
  padding: 0.35rem 0.625rem;
  cursor: pointer;
}

.live-health-action--primary {
  background: var(--color-primary, #2f6552);
  border-color: var(--color-primary, #2f6552);
  color: #fff;
}

.live-health-action--danger {
  border-color: var(--color-danger, #8a3b4d);
  color: var(--color-danger, #8a3b4d);
}

.live-health-sheet {
  display: grid;
  gap: var(--space-3, 0.75rem);
  margin-top: var(--space-3, 0.75rem);
  padding: var(--space-4, 1rem);
  border: 1px solid var(--color-border, #ded6c8);
  border-radius: 8px;
  background: var(--color-surface, #fffaf1);
  box-shadow: var(--shadow-lg, 0 16px 40px rgba(31, 41, 51, 0.14));
}

.live-health-sheet__header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: var(--space-3, 0.75rem);
}

.live-health-sheet__header h3,
.live-health-sheet__header p {
  margin: 0;
}

.live-health-sheet__header p {
  color: var(--color-text-muted, #6b6b6b);
  font-size: 0.875rem;
}

.live-health-sheet__rows {
  display: grid;
  gap: 0.5rem;
}

.live-health-row {
  display: grid;
  grid-template-columns: minmax(6rem, max-content) 1fr;
  gap: 0.75rem;
  align-items: start;
  padding: 0.625rem 0;
  border-top: 1px solid var(--color-border-subtle, #ece4d7);
}

.live-health-row__status {
  font-weight: 800;
}

.live-health-row--ok .live-health-row__status { color: var(--color-success, #2f6552); }
.live-health-row--warning .live-health-row__status { color: var(--color-warning, #b08842); }
.live-health-row--error .live-health-row__status { color: var(--color-danger, #8a3b4d); }
.live-health-row--unknown .live-health-row__status,
.live-health-row--muted .live-health-row__status { color: var(--color-text-muted, #6b6b6b); }

.live-health-row__body {
  display: grid;
  gap: 0.125rem;
}

.live-health-row__body span {
  color: var(--color-text-muted, #6b6b6b);
  font-size: 0.875rem;
}

.live-health-actions {
  display: flex;
  flex-wrap: wrap;
  gap: 0.5rem;
}

.stream-state-notice {
  display: grid;
  place-items: center;
  align-content: center;
  gap: 0.5rem;
  min-height: 14rem;
  padding: var(--space-5, 1.25rem);
  text-align: center;
  color: var(--color-text, #1f2933);
  background: var(--color-surface-muted, #f4f1ea);
}

.stream-state-notice h3,
.stream-state-notice p {
  margin: 0;
}

.stream-state-notice p {
  max-width: 34rem;
  color: var(--color-text-muted, #6b6b6b);
}

@media (max-width: 720px) {
  .live-health-strip {
    align-items: stretch;
    flex-direction: column;
  }

  .live-health-strip__items {
    flex-wrap: wrap;
  }

  .live-health-row {
    grid-template-columns: 1fr;
  }
}
```

- [ ] **Step 2: Copy the same CSS to the shell-web public asset**

Append the exact same CSS block to `crates/shell-web/public/assets/components.css`.

- [ ] **Step 3: Run asset sync and frontend tests**

Run:

```powershell
cargo test -p shell-web --test editorial_assets -- --nocapture
cargo test -p features-courses --lib live_room_health -- --nocapture
```

Expected: asset sync tests and health component tests pass.

- [ ] **Step 4: Commit CSS**

Run:

```powershell
git add crates/design-system/assets/components.css crates/shell-web/public/assets/components.css
git commit -m "style(live): add room health surfaces"
```

Expected: commit succeeds with both CSS asset files staged.

## Task 7: Final Verification

**Files:**
- Verify all files touched by Tasks 1-6.

- [ ] **Step 1: Run formatting check**

Run:

```powershell
cargo fmt --all --check
```

Expected: exits 0.

- [ ] **Step 2: Run service-free workspace tests**

Run:

```powershell
cargo test --workspace
```

Expected: exits 0 without requiring Postgres.

- [ ] **Step 3: Run shell-web wasm check**

Run:

```powershell
cargo check -p shell-web --target wasm32-unknown-unknown
```

Expected: exits 0.

- [ ] **Step 4: Run focused live-room tests**

Run:

```powershell
cargo test -p backend --lib health_tests -- --nocapture
cargo test -p features-courses --lib live_room_health -- --nocapture
cargo test -p features-courses --lib live_room_broadcast -- --nocapture
cargo test -p features-courses --lib live_room_view -- --nocapture
cargo test -p shell-web --test editorial_assets -- --nocapture
```

Expected: all focused tests pass.

- [ ] **Step 5: Run DB-backed health endpoint tests when Postgres is available**

Run with a migrated database:

```powershell
cargo test -p backend --features db-tests --test live_room health_ -- --nocapture
```

Expected: the three health endpoint integration tests pass.

- [ ] **Step 6: Run clippy**

Run:

```powershell
cargo clippy --workspace --all-targets -- -D warnings
```

Expected: exits 0.

- [ ] **Step 7: Review final diff**

Run:

```powershell
git status --short
git log --oneline -6
```

Expected: the latest commits include:

```text
feat(live): add health status mapping
feat(live): expose session health endpoint
feat(live): add frontend health model
feat(live): show teacher room health
feat(live): clarify student stream states
style(live): add room health surfaces
```

Unrelated pre-existing working-tree changes may still exist, but no files from this feature should be left unstaged.
