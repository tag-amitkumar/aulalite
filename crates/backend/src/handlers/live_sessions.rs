// crates/backend/src/handlers/live_sessions.rs
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{routing, Json, Router};
use chrono::Weekday;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::context::RequestContext;
use crate::db;
use crate::error::ApiError;
use crate::services::recurrence::{EndKind, ExpandError, Frequency, SeriesSpec};
use crate::AppState;

/// Internal trait so `start_now_inner` and `active_session_inner` can be
/// called from both the production and test routers without duplicating
/// the handler body.
trait AppStateOrTest {
    fn pool(&self) -> &PgPool;
    fn mediamtx(&self) -> &dyn crate::services::mediamtx::MediaMtxClient;
}

impl AppStateOrTest for AppState {
    fn pool(&self) -> &PgPool {
        &self.pool
    }
    fn mediamtx(&self) -> &dyn crate::services::mediamtx::MediaMtxClient {
        self.mediamtx.as_ref()
    }
}

impl AppStateOrTest for LiveRoomTestState {
    fn pool(&self) -> &PgPool {
        &self.pool
    }
    fn mediamtx(&self) -> &dyn crate::services::mediamtx::MediaMtxClient {
        self.mediamtx.as_ref()
    }
}

#[derive(Deserialize)]
pub struct CreateSeries {
    pub title: String,
    pub starts_at: chrono::DateTime<chrono::Utc>,
    pub duration_minutes: i32,
    pub frequency: String,
    pub byweekday: Option<Vec<String>>,
    pub end_kind: String,
    pub occurrence_count: Option<i32>,
    pub end_until: Option<chrono::DateTime<chrono::Utc>>,
    pub primary_teacher_id: Option<Uuid>,
    pub recording_enabled: Option<bool>,
    #[serde(default = "default_transport_mode")]
    pub transport_mode: String,
}

fn default_transport_mode() -> String {
    "webrtc".to_string()
}

fn ws_text(text: String) -> axum::extract::ws::Message {
    axum::extract::ws::Message::Text(text.into())
}

#[derive(Serialize)]
pub struct SeriesDto {
    pub id: Uuid,
    pub course_id: Uuid,
    pub title: String,
    pub frequency: String,
    pub end_kind: String,
}
#[derive(Serialize)]
pub struct OccurrenceDto {
    pub id: Uuid,
    pub series_id: Uuid,
    pub occurrence_index: i32,
    pub title: String,
    pub status: String,
    pub starts_at: chrono::DateTime<chrono::Utc>,
    pub duration_minutes: i32,
    pub diverged: bool,
}

#[derive(Deserialize, Default, Debug)]
pub struct CreateStartNow {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub duration_minutes: Option<i32>,
    #[serde(default)]
    pub recording_enabled: Option<bool>,
}

#[derive(Serialize, Debug)]
pub struct StartNowResponse {
    pub session_id: Uuid,
    pub series_id: Uuid,
    pub title: String,
    pub starts_at: chrono::DateTime<chrono::Utc>,
    pub duration_minutes: i32,
    pub recording_enabled: bool,
    pub transport_mode: String,
    pub status: String,
}

#[derive(Serialize, Debug)]
pub struct ActiveSessionInfo {
    pub session_id: Uuid,
    pub title: String,
    pub starts_at: chrono::DateTime<chrono::Utc>,
    pub transport_mode: String,
    /// Whether the teacher's stream is ACTUALLY flowing on the media server
    /// right now, as reported by MediaMTX for this session's main path.
    ///
    /// The presence of the session itself says nothing about this: `status`
    /// becomes `live` the instant the teacher presses Start, which is before
    /// any WHIP attempt is made and stays set when that attempt fails. The
    /// student-facing "Live now" badge is gated on this field, so a failed or
    /// still-connecting publish never advertises a class as live.
    ///
    /// False whenever readiness cannot be positively confirmed -- path
    /// inactive, no path recorded yet, or the media server unreachable -- so
    /// the badge fails closed.
    pub stream_ready: bool,
}

#[derive(Serialize, Debug)]
pub struct ActiveSessionResponse {
    pub active: Option<ActiveSessionInfo>,
}

#[derive(Serialize, Debug)]
pub struct StartNowConflictBody {
    pub error: String,
    pub active_session_id: Uuid,
    pub title: String,
    pub starts_at: chrono::DateTime<chrono::Utc>,
    /// Set on per-user conflicts so the UI can deep-link back to the
    /// course where the user already has a live session. Omitted on
    /// per-course conflicts because the caller already knows the course.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_course_id: Option<Uuid>,
}

#[derive(Serialize)]
pub struct SeriesCreatedDto {
    pub series: SeriesDto,
    pub occurrences: Vec<OccurrenceDto>,
}

#[derive(Deserialize, Default)]
pub struct PatchOccurrence {
    pub starts_at: Option<chrono::DateTime<chrono::Utc>>,
    pub duration_minutes: Option<i32>,
    pub title: Option<String>,
    pub primary_teacher_id: Option<Uuid>,
    pub status: Option<String>,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/courses/{cid}/sessions", routing::post(create_series))
        .route(
            "/v1/series/{sid}",
            routing::get(get_series).delete(delete_series),
        )
        .route("/v1/sessions/{id}", routing::patch(patch_occurrence_h))
}

#[doc(hidden)]
pub fn router_for_tests(pool: PgPool) -> Router {
    Router::new()
        .route("/v1/courses/{cid}/sessions", routing::post(create_series_t))
        .route(
            "/v1/series/{sid}",
            routing::get(get_series_t).delete(delete_series_t),
        )
        .route("/v1/sessions/{id}", routing::patch(patch_occurrence_t))
        .with_state(TestState { pool })
}

#[derive(Clone)]
struct TestState {
    pool: PgPool,
}

fn is_org_admin(ctx: &RequestContext) -> bool {
    ctx.can_manage_organization()
}

fn parse_freq(s: &str) -> Result<Frequency, ApiError> {
    Ok(match s {
        "none" => Frequency::None,
        "daily" => Frequency::Daily,
        "weekly" => Frequency::Weekly,
        "biweekly" => Frequency::Biweekly,
        "monthly" => Frequency::Monthly,
        other => return Err(ApiError::BadRequest(format!("invalid frequency: {other}"))),
    })
}

fn parse_weekday(s: &str) -> Result<Weekday, ApiError> {
    Ok(match s.to_lowercase().as_str() {
        "mon" => Weekday::Mon,
        "tue" => Weekday::Tue,
        "wed" => Weekday::Wed,
        "thu" => Weekday::Thu,
        "fri" => Weekday::Fri,
        "sat" => Weekday::Sat,
        "sun" => Weekday::Sun,
        other => return Err(ApiError::BadRequest(format!("invalid weekday: {other}"))),
    })
}

// Production handlers
async fn create_series(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(cid): Path<Uuid>,
    Json(b): Json<CreateSeries>,
) -> Result<Json<SeriesCreatedDto>, ApiError> {
    create_series_inner(&s.pool, &ctx, cid, b).await
}
async fn get_series(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(sid): Path<Uuid>,
) -> Result<Json<SeriesCreatedDto>, ApiError> {
    get_series_inner(&s.pool, &ctx, sid).await
}
async fn delete_series(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(sid): Path<Uuid>,
) -> Result<axum::http::StatusCode, ApiError> {
    delete_series_inner(&s.pool, &ctx, sid).await
}
async fn patch_occurrence_h(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    Json(b): Json<PatchOccurrence>,
) -> Result<Json<OccurrenceDto>, ApiError> {
    patch_occurrence_inner(&s.pool, &ctx, id, b).await
}

// Test mirrors
async fn create_series_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(cid): Path<Uuid>,
    Json(b): Json<CreateSeries>,
) -> Result<Json<SeriesCreatedDto>, ApiError> {
    create_series_inner(&s.pool, &ctx, cid, b).await
}
async fn get_series_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(sid): Path<Uuid>,
) -> Result<Json<SeriesCreatedDto>, ApiError> {
    get_series_inner(&s.pool, &ctx, sid).await
}
async fn delete_series_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(sid): Path<Uuid>,
) -> Result<axum::http::StatusCode, ApiError> {
    delete_series_inner(&s.pool, &ctx, sid).await
}
async fn patch_occurrence_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    Json(b): Json<PatchOccurrence>,
) -> Result<Json<OccurrenceDto>, ApiError> {
    patch_occurrence_inner(&s.pool, &ctx, id, b).await
}

async fn create_series_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
    b: CreateSeries,
) -> Result<Json<SeriesCreatedDto>, ApiError> {
    let allowed = db::courses::caller_can_admin_course(
        pool,
        course_id,
        ctx.user_id,
        ctx.tenant_id,
        is_org_admin(ctx),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !allowed {
        return Err(ApiError::Forbidden);
    }
    let tenant_id = ctx
        .tenant_id
        .ok_or_else(|| ApiError::BadRequest("no tenant".into()))?;
    let teacher = b.primary_teacher_id.unwrap_or(ctx.user_id);

    let frequency = parse_freq(&b.frequency)?;
    let byweekday: Vec<Weekday> = match &b.byweekday {
        Some(list) => list
            .iter()
            .map(|s| parse_weekday(s))
            .collect::<Result<_, _>>()?,
        None => vec![],
    };
    let end_kind = match b.end_kind.as_str() {
        "count" => EndKind::Count(
            b.occurrence_count
                .ok_or_else(|| ApiError::BadRequest("count requires occurrence_count".into()))?
                as u32,
        ),
        "until" => EndKind::Until(
            b.end_until
                .ok_or_else(|| ApiError::BadRequest("until requires end_until".into()))?,
        ),
        "open" => EndKind::Open,
        other => return Err(ApiError::BadRequest(format!("invalid end_kind: {other}"))),
    };

    let spec = SeriesSpec {
        starts_at: b.starts_at,
        duration_minutes: b.duration_minutes as u32,
        frequency,
        byweekday,
        end_kind,
    };
    let occurrences = crate::services::recurrence::expand(&spec).map_err(|e| match e {
        ExpandError::MissingByweekday => {
            ApiError::RecurrenceShapeInvalid("byweekday required".into())
        }
        ExpandError::ExtraByweekday => {
            ApiError::RecurrenceShapeInvalid("byweekday forbidden for this frequency".into())
        }
        ExpandError::InvalidCount => ApiError::RecurrenceShapeInvalid("count must be > 0".into()),
        ExpandError::InvalidUntil => {
            ApiError::RecurrenceShapeInvalid("until must be > starts_at".into())
        }
    })?;

    let recording_enabled_resolved = match b.recording_enabled {
        Some(v) => v,
        None => {
            let mut tenant_tx = db::begin_with_context(pool, ctx.user_id, Some(tenant_id))
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?;
            let v: bool = sqlx::query_scalar("SELECT recording_default FROM tenants WHERE id = $1")
                .bind(tenant_id)
                .fetch_one(&mut *tenant_tx)
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?;
            tenant_tx
                .commit()
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?;
            v
        }
    };

    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    db::set_request_guc(&mut tx, ctx.user_id, Some(tenant_id))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !matches!(b.transport_mode.as_str(), "webrtc" | "hls") {
        return Err(ApiError::BadRequest(format!(
            "transport_mode must be 'webrtc' or 'hls', got '{}'",
            b.transport_mode
        )));
    }

    let series_id = db::live_sessions::insert_series(
        &mut tx,
        tenant_id,
        course_id,
        &b.title,
        b.starts_at,
        b.duration_minutes,
        &b.frequency,
        b.byweekday.as_deref(),
        &b.end_kind,
        b.occurrence_count,
        b.end_until,
        teacher,
        b.recording_enabled,
        &b.transport_mode,
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;

    let mut occ_dtos = Vec::new();
    for occ in &occurrences {
        let id = db::live_sessions::insert_occurrence(
            &mut tx,
            tenant_id,
            course_id,
            series_id,
            occ.occurrence_index as i32,
            &b.title,
            occ.starts_at,
            occ.duration_minutes as i32,
            teacher,
            recording_enabled_resolved,
            &b.transport_mode,
        )
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
        occ_dtos.push(OccurrenceDto {
            id,
            series_id,
            occurrence_index: occ.occurrence_index as i32,
            title: b.title.clone(),
            status: "scheduled".into(),
            starts_at: occ.starts_at,
            duration_minutes: occ.duration_minutes as i32,
            diverged: false,
        });
    }

    db::audit::emit_audit_event(
        &mut tx,
        tenant_id,
        ctx.user_id,
        "live_session_series.create",
        "live_session_series",
        series_id,
        None,
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(SeriesCreatedDto {
        series: SeriesDto {
            id: series_id,
            course_id,
            title: b.title,
            frequency: b.frequency,
            end_kind: b.end_kind,
        },
        occurrences: occ_dtos,
    }))
}

async fn start_now_inner(
    state: &impl AppStateOrTest,
    ctx: &RequestContext,
    course_id: Uuid,
    body: CreateStartNow,
) -> Result<Response, ApiError> {
    let pool = state.pool();
    let mediamtx = state.mediamtx();

    // Admin check up front so a non-admin caller can't probe whether a
    // live session exists for the course (the fast-path conflict check
    // below would leak that). `create_series_inner` re-checks this
    // internally — one extra DB round-trip in the happy path, acceptable
    // because start-now is not a hot path.
    // 1. Auth: must be able to admin this course.
    let allowed = db::courses::caller_can_admin_course(
        pool,
        course_id,
        ctx.user_id,
        ctx.tenant_id,
        is_org_admin(ctx),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !allowed {
        return Err(ApiError::Forbidden);
    }

    // 2. Fast-path conflict check (clean error before any inserts).
    //    (a) per-course: someone else in the course is already live.
    if let Some(active) = db::live_sessions::find_live_for_course_with_context(
        pool,
        ctx.user_id,
        ctx.tenant_id,
        course_id,
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?
    {
        return Ok(conflict_response(&active));
    }
    //    (b) per-user: this caller already has a live session somewhere
    //        (different course in the same tenant). Cross-tenant hits
    //        are caught by the DB constraint below — the friendly
    //        409 here only covers the in-tenant case under RLS.
    if let Some(active) =
        db::live_sessions::find_live_for_user_with_context(pool, ctx.user_id, ctx.tenant_id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?
    {
        return Ok(user_conflict_response(&active));
    }

    // 3. Resolve defaults.
    let now = chrono::Utc::now();
    let title = body
        .title
        .unwrap_or_else(|| format!("Quick session — {}", now.format("%b %-d, %Y %-I:%M %p UTC")));
    let duration_minutes = body.duration_minutes.unwrap_or(60);
    if !(5..=480).contains(&duration_minutes) {
        return Err(ApiError::BadRequest(format!(
            "duration_minutes must be between 5 and 480, got {duration_minutes}"
        )));
    }
    let tenant_id = ctx
        .tenant_id
        .ok_or_else(|| ApiError::BadRequest("no tenant".into()))?;

    // Fast quota preflight prevents the routine over-limit case from creating
    // a scheduled ad-hoc series that immediately needs cleanup. The second
    // check at the transition remains authoritative for concurrent starts.
    let mut preflight = db::begin_with_context(pool, ctx.user_id, Some(tenant_id))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let recording_enabled = match body.recording_enabled {
        Some(enabled) => enabled,
        None => sqlx::query_scalar("SELECT recording_default FROM tenants WHERE id = $1")
            .bind(tenant_id)
            .fetch_one(&mut *preflight)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?,
    };
    enforce_live_start_limits(
        &mut preflight,
        tenant_id,
        Uuid::nil(),
        duration_minutes,
        recording_enabled,
    )
    .await?;
    preflight
        .commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    // 4. Reuse the existing series-create handler with an ad-hoc preset.
    //    This commits its own transaction; on return we have a `scheduled`
    //    occurrence and need to transition it to `live`.
    let series = CreateSeries {
        title: title.clone(),
        starts_at: now,
        duration_minutes,
        frequency: "none".to_string(),
        byweekday: None,
        end_kind: "count".to_string(),
        occurrence_count: Some(1),
        end_until: None,
        primary_teacher_id: None,
        recording_enabled: body.recording_enabled,
        transport_mode: "webrtc".to_string(),
    };
    let created = create_series_inner(pool, ctx, course_id, series).await?;
    let occurrence =
        created.0.occurrences.into_iter().next().ok_or_else(|| {
            ApiError::Internal("create_series_inner returned no occurrences".into())
        })?;
    let series_dto = created.0.series;

    let main_path =
        crate::services::mediamtx::path_for_session(tenant_id, course_id, occurrence.id);
    let screen_path =
        crate::services::mediamtx::screen_path_for_session(tenant_id, course_id, occurrence.id);
    // Mint a nonce now so the row is born with paths + hash. The plaintext
    // is intentionally discarded: the broadcast view re-POSTs /go-live on
    // mount, which re-mints a fresh nonce that's actually handed to the
    // client. The hash stored here gets overwritten on that call. Wasted
    // work is acceptable; the alternative (storing NULL nonce) would
    // require schema changes.
    let nonce_plain = mint_publish_nonce();
    let nonce_hash = db::live_sessions::hash_nonce(&nonce_plain);
    let nonce_expires_at = now + chrono::Duration::from_std(PUBLISH_NONCE_TTL).unwrap();

    // 5. Transition the new row to 'live'. Race safety: this is a SECOND
    //    transaction (create_series_inner already committed). Two
    //    concurrent start-now calls can both pass the fast-path check and
    //    both reach this point with their own `scheduled` rows; the partial
    //    unique index `live_sessions_one_live_per_course` (created in
    //    Task 1's migration) is what serialises them at the `go_live`
    //    UPDATE. The unique-violation branch below catches the loser.
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    db::set_request_guc(&mut tx, ctx.user_id, ctx.tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let recording_enabled: bool =
        sqlx::query_scalar("SELECT recording_enabled FROM live_sessions WHERE id = $1")
            .bind(occurrence.id)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
    if let Err(error) = enforce_live_start_limits(
        &mut tx,
        tenant_id,
        occurrence.id,
        duration_minutes,
        recording_enabled,
    )
    .await
    {
        // Series creation commits before this transition. If a concurrent
        // class consumed the final allowance, remove this ad-hoc scheduled
        // series so a blocked Start now does not leave a ghost class behind.
        drop(tx);
        let mut cleanup = db::begin_with_context(pool, ctx.user_id, Some(tenant_id))
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        db::live_sessions::delete_series(&mut cleanup, series_dto.id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        cleanup
            .commit()
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        return Err(error);
    }
    let live_row = match db::live_sessions::go_live(
        &mut tx,
        occurrence.id,
        &main_path,
        Some(&screen_path),
        &nonce_hash,
        nonce_expires_at,
    )
    .await
    {
        Ok(Some(row)) => row,
        Ok(None) => {
            return Err(ApiError::SessionStateInvalid(
                "session not in scheduled/live state immediately after creation".into(),
            ));
        }
        Err(e) => {
            if let sqlx::Error::Database(dbe) = &e {
                if dbe.is_unique_violation() {
                    match dbe.constraint() {
                        Some("live_sessions_one_live_per_course") => {
                            // Race lost to a concurrent start-now. Re-read the
                            // now-visible active session and return 409.
                            drop(tx);
                            if let Some(active) =
                                db::live_sessions::find_live_for_course_with_context(
                                    pool,
                                    ctx.user_id,
                                    ctx.tenant_id,
                                    course_id,
                                )
                                .await
                                .map_err(|e| ApiError::Internal(e.to_string()))?
                            {
                                return Ok(conflict_response(&active));
                            }
                        }
                        Some("live_sessions_one_live_per_user") => {
                            // Caller already has a live session — either
                            // a same-tenant race we missed at the preflight
                            // or a cross-tenant session invisible under
                            // RLS. Either way, surface a 409.
                            drop(tx);
                            if let Some(active) =
                                db::live_sessions::find_live_for_user_with_context(
                                    pool,
                                    ctx.user_id,
                                    ctx.tenant_id,
                                )
                                .await
                                .map_err(|e| ApiError::Internal(e.to_string()))?
                            {
                                return Ok(user_conflict_response(&active));
                            }
                            // Cross-tenant: details are intentionally not
                            // surfaced (different tenant). Return a generic
                            // 409 so the UI can prompt the user to end
                            // their other session.
                            return Ok((
                                StatusCode::CONFLICT,
                                axum::Json(StartNowConflictBody {
                                    error: "you already have a live session in another workspace; end it before starting a new one".into(),
                                    active_session_id: Uuid::nil(),
                                    title: String::new(),
                                    starts_at: chrono::Utc::now(),
                                    active_course_id: None,
                                }),
                            )
                                .into_response());
                        }
                        _ => {}
                    }
                }
            }
            return Err(ApiError::Internal(e.to_string()));
        }
    };
    db::audit::emit_audit_event(
        &mut tx,
        tenant_id,
        ctx.user_id,
        "live_session.start_now",
        "live_session",
        live_row.id,
        None,
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    // 6. Best-effort mediamtx notify.
    if let Err(e) = mediamtx.publish_started(&main_path).await {
        tracing::warn!(?e, %main_path, "publish_started best-effort failed");
    }

    let response = StartNowResponse {
        session_id: live_row.id,
        series_id: series_dto.id,
        title,
        starts_at: now,
        duration_minutes,
        recording_enabled: live_row.recording_enabled,
        transport_mode: live_row.transport_mode,
        status: live_row.status,
    };
    Ok((StatusCode::OK, axum::Json(response)).into_response())
}

fn conflict_response(active: &db::live_sessions::ActiveSessionRow) -> Response {
    let body = StartNowConflictBody {
        error: "a session is already live in this course".into(),
        active_session_id: active.id,
        title: active.title.clone(),
        starts_at: active.actual_started_at.unwrap_or(active.starts_at),
        active_course_id: None,
    };
    (StatusCode::CONFLICT, axum::Json(body)).into_response()
}

fn user_conflict_response(active: &db::live_sessions::UserActiveSessionRow) -> Response {
    let body = StartNowConflictBody {
        error: "you already have a live session running; end it before starting a new one".into(),
        active_session_id: active.id,
        title: active.title.clone(),
        starts_at: active.actual_started_at.unwrap_or(active.starts_at),
        active_course_id: Some(active.course_id),
    };
    (StatusCode::CONFLICT, axum::Json(body)).into_response()
}

async fn active_session_inner(
    state: &impl AppStateOrTest,
    ctx: &RequestContext,
    course_id: Uuid,
) -> Result<Json<ActiveSessionResponse>, ApiError> {
    let pool = state.pool();
    // Any course member (teacher, co-teacher, or enrolled student) may read.
    let allowed = db::courses::caller_can_read_course(
        pool,
        course_id,
        ctx.user_id,
        ctx.tenant_id,
        is_org_admin(ctx),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !allowed {
        return Err(ApiError::NotFound);
    }

    let row = db::live_sessions::find_live_for_course_with_context(
        pool,
        ctx.user_id,
        ctx.tenant_id,
        course_id,
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;

    let active = match row {
        None => None,
        Some(row) => {
            // Ask the media server whether anything is actually being
            // published, instead of trusting the row. Bounded by the same
            // short probe timeout the health endpoint uses, so a slow or dead
            // MediaMTX cannot hold this poll open.
            let stream_ready = match row.main_path.as_deref() {
                Some(path) => matches!(
                    path_status_with_timeout(state.mediamtx(), path).await,
                    Some(Ok(crate::services::mediamtx::PathStatus::Active))
                ),
                // Live row with no path recorded yet: the teacher has not got
                // as far as go-live, so nothing can be flowing.
                None => false,
            };
            Some(ActiveSessionInfo {
                session_id: row.id,
                title: row.title,
                starts_at: row.actual_started_at.unwrap_or(row.starts_at),
                transport_mode: row.transport_mode,
                stream_ready,
            })
        }
    };

    Ok(Json(ActiveSessionResponse { active }))
}

async fn active_session(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(cid): Path<Uuid>,
) -> Result<Json<ActiveSessionResponse>, ApiError> {
    active_session_inner(&s, &ctx, cid).await
}

async fn active_session_t(
    State(s): State<LiveRoomTestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(cid): Path<Uuid>,
) -> Result<Json<ActiveSessionResponse>, ApiError> {
    active_session_inner(&s, &ctx, cid).await
}

async fn get_series_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    sid: Uuid,
) -> Result<Json<SeriesCreatedDto>, ApiError> {
    let mut tx = db::begin_with_context(pool, ctx.user_id, ctx.tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let series = db::live_sessions::fetch_series(&mut *tx, sid)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::NotFound)?;
    let occs = db::live_sessions::list_occurrences_for_series(&mut *tx, sid)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let allowed = db::courses::caller_can_read_course(
        pool,
        series.course_id,
        ctx.user_id,
        ctx.tenant_id,
        is_org_admin(ctx),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !allowed {
        return Err(ApiError::NotFound);
    }
    Ok(Json(SeriesCreatedDto {
        series: SeriesDto {
            id: series.id,
            course_id: series.course_id,
            title: series.title.clone(),
            frequency: series.frequency.clone(),
            end_kind: series.end_kind.clone(),
        },
        occurrences: occs
            .into_iter()
            .map(|o| OccurrenceDto {
                id: o.id,
                series_id: o.series_id,
                occurrence_index: o.occurrence_index,
                title: o.title,
                status: o.status,
                starts_at: o.starts_at,
                duration_minutes: o.duration_minutes,
                diverged: o.diverged,
            })
            .collect(),
    }))
}

async fn delete_series_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    sid: Uuid,
) -> Result<axum::http::StatusCode, ApiError> {
    let mut prefetch_tx = db::begin_with_context(pool, ctx.user_id, ctx.tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let series = db::live_sessions::fetch_series(&mut *prefetch_tx, sid)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::NotFound)?;
    prefetch_tx
        .commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let allowed = db::courses::caller_can_admin_course(
        pool,
        series.course_id,
        ctx.user_id,
        ctx.tenant_id,
        is_org_admin(ctx),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !allowed {
        return Err(ApiError::Forbidden);
    }
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    db::set_request_guc(&mut tx, ctx.user_id, ctx.tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    db::live_sessions::delete_series(&mut tx, sid)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

async fn patch_occurrence_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    id: Uuid,
    b: PatchOccurrence,
) -> Result<Json<OccurrenceDto>, ApiError> {
    let course_id: Uuid = {
        let mut tx = db::begin_with_context(pool, ctx.user_id, ctx.tenant_id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        let row: Option<Uuid> =
            sqlx::query_scalar("SELECT course_id FROM live_sessions WHERE id = $1")
                .bind(id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?;
        tx.commit()
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        row.ok_or(ApiError::NotFound)?
    };
    let allowed = db::courses::caller_can_admin_course(
        pool,
        course_id,
        ctx.user_id,
        ctx.tenant_id,
        is_org_admin(ctx),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !allowed {
        return Err(ApiError::Forbidden);
    }

    if let Some(ref new_status) = b.status {
        if !matches!(new_status.as_str(), "scheduled" | "cancelled") {
            return Err(ApiError::BadRequest(format!(
                "status '{new_status}' not allowed at 1a"
            )));
        }
    }

    let diverged = b.starts_at.is_some() || b.duration_minutes.is_some() || b.title.is_some();

    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    db::set_request_guc(&mut tx, ctx.user_id, ctx.tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let row = db::live_sessions::patch_occurrence(
        &mut tx,
        id,
        b.starts_at,
        b.duration_minutes,
        b.title.as_deref(),
        b.status.as_deref(),
        diverged,
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?
    .ok_or(ApiError::NotFound)?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(OccurrenceDto {
        id: row.id,
        series_id: row.series_id,
        occurrence_index: row.occurrence_index,
        title: row.title,
        status: row.status,
        starts_at: row.starts_at,
        duration_minutes: row.duration_minutes,
        diverged: row.diverged,
    }))
}

// ============================================================================
// Phase 1b-beta: Live Room handlers
// ============================================================================

use crate::services::mediamtx::{JwtSigner, MediaMtxClient, MediaMtxError, PathStatus};
use rand::RngCore;
use std::sync::Arc;
use std::time::Duration as StdDuration;

const PUBLISH_NONCE_TTL: StdDuration = StdDuration::from_secs(4 * 3600);
const GO_LIVE_WINDOW_BEFORE: chrono::Duration = chrono::Duration::minutes(30);
const GO_LIVE_WINDOW_AFTER: chrono::Duration = chrono::Duration::hours(4);

#[derive(serde::Serialize)]
pub struct GoLiveResponse {
    pub session_id: Uuid,
    pub main_publish_url: String,
    pub screen_publish_url: String,
    pub publish_password: String,
    pub transport_mode: String,
    /// ICE servers (STUN default + optional TURN) for the publisher's
    /// RTCPeerConnection. Additive: older clients ignore it.
    pub ice_servers: Vec<crate::services::ice::IceServer>,
}

fn mint_publish_nonce() -> String {
    let mut buf = [0u8; 24];
    rand::rng().fill_bytes(&mut buf);
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf)
}

async fn require_admin_for_session_course(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
) -> Result<(), ApiError> {
    let allowed = db::courses::caller_can_admin_course(
        pool,
        course_id,
        ctx.user_id,
        ctx.tenant_id,
        is_org_admin(ctx),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !allowed {
        return Err(ApiError::Forbidden);
    }
    Ok(())
}

/// Enforce media plan limits in the same transaction that transitions a
/// scheduled session to live. The quota layer holds the tenant-row lock until
/// this transaction commits, serializing concurrent teachers across replicas.
async fn enforce_live_start_limits(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: Uuid,
    session_id: Uuid,
    duration_minutes: i32,
    recording_enabled: bool,
) -> Result<(), ApiError> {
    if db::usage_limits::class_minutes_for_start(
        tx,
        tenant_id,
        session_id,
        i64::from(duration_minutes),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?
    .is_limit_reached()
    {
        return Err(ApiError::ClassMinutesLimitReached);
    }

    // Exact bytes are reserved by the recording worker after remuxing. A
    // one-byte sentinel prevents starting capture when storage is already full.
    if recording_enabled
        && db::usage_limits::recording_storage_capacity_in_tx(tx, tenant_id, 1)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?
            .is_limit_reached()
    {
        return Err(ApiError::RecordingStorageLimitReached);
    }
    Ok(())
}

async fn go_live_inner(
    pool: &PgPool,
    mediamtx_client: &dyn MediaMtxClient,
    public_webrtc_url: &str,
    ctx: &RequestContext,
    session_id: Uuid,
) -> Result<Json<GoLiveResponse>, ApiError> {
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
    require_admin_for_session_course(pool, ctx, session.course_id).await?;

    if !matches!(session.status.as_str(), "scheduled" | "live") {
        return Err(ApiError::SessionStateInvalid(format!(
            "session is {}; cannot go live",
            session.status
        )));
    }

    // The go-live window governs STARTING a class, not continuing one.
    //
    // A session that is already `live` reaches here when the teacher
    // re-publishes: switching camera, recovering from a dropped connection, or
    // reloading the room all re-run go-live to mint a fresh publish
    // credential. Applying the window to those turned GO_LIVE_WINDOW_AFTER
    // into a hard 4h cap on class length -- past it, a teacher who lost their
    // connection could not get back on air. There is no maximum class
    // duration, so an already-started class is exempt.
    let now = chrono::Utc::now();
    if session.status != "live" {
        let earliest = session.starts_at - GO_LIVE_WINDOW_BEFORE;
        let latest = session.starts_at + GO_LIVE_WINDOW_AFTER;
        if now < earliest || now > latest {
            return Err(ApiError::SessionWindowClosed(format!(
                "go-live allowed between {earliest} and {latest}; now is {now}"
            )));
        }
    }

    let main_path =
        crate::services::mediamtx::path_for_session(tenant_id, session.course_id, session_id);
    let screen_path = crate::services::mediamtx::screen_path_for_session(
        tenant_id,
        session.course_id,
        session_id,
    );
    let nonce_plain = mint_publish_nonce();
    let nonce_hash = db::live_sessions::hash_nonce(&nonce_plain);
    let nonce_expires_at = now + chrono::Duration::from_std(PUBLISH_NONCE_TTL).unwrap();

    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    db::set_request_guc(&mut tx, ctx.user_id, Some(tenant_id))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    // Re-POSTing go-live for an already-live room only rotates its publisher
    // credential; do not consume its scheduled duration twice.
    if session.status == "scheduled" {
        enforce_live_start_limits(
            &mut tx,
            tenant_id,
            session_id,
            session.duration_minutes,
            session.recording_enabled,
        )
        .await?;
    }
    let _row = match db::live_sessions::go_live(
        &mut tx,
        session_id,
        &main_path,
        Some(&screen_path),
        &nonce_hash,
        nonce_expires_at,
    )
    .await
    {
        Ok(Some(row)) => row,
        Ok(None) => {
            return Err(ApiError::SessionStateInvalid(
                "session no longer in valid state".into(),
            ));
        }
        Err(e) => {
            // A unique-constraint violation here is a client/business conflict
            // (the caller already has a live session, or the course already
            // has one), not a server fault. Return a clean 409 instead of
            // letting the raw Postgres error surface as a 500 with the
            // constraint name leaked to the client — mirrors the start-now
            // handler above.
            if let sqlx::Error::Database(dbe) = &e {
                if dbe.is_unique_violation() {
                    match dbe.constraint() {
                        Some("live_sessions_one_live_per_user") => {
                            return Err(ApiError::Conflict(
                                "you already have a live session running; end it before starting a new one".into(),
                            ));
                        }
                        Some("live_sessions_one_live_per_course") => {
                            return Err(ApiError::Conflict(
                                "this course already has a live session running".into(),
                            ));
                        }
                        _ => {}
                    }
                }
            }
            return Err(ApiError::Internal(e.to_string()));
        }
    };
    db::audit::emit_audit_event(
        &mut tx,
        tenant_id,
        ctx.user_id,
        "live_session.go_live",
        "live_session",
        session_id,
        None,
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    if let Err(e) = mediamtx_client.publish_started(&main_path).await {
        tracing::warn!(?e, %main_path, "publish_started best-effort failed");
    }

    Ok(Json(GoLiveResponse {
        session_id,
        main_publish_url: format!("{public_webrtc_url}/{main_path}/whip"),
        screen_publish_url: format!("{public_webrtc_url}/{screen_path}/whip"),
        publish_password: nonce_plain,
        transport_mode: session.transport_mode,
        ice_servers: crate::services::ice::ice_servers(),
    }))
}

#[derive(Clone)]
pub struct LiveRoomTestState {
    pub pool: PgPool,
    pub mediamtx: Arc<dyn MediaMtxClient>,
    pub signer: Arc<JwtSigner>,
    pub public_webrtc_url: String,
    pub public_hls_url: String,
    pub broker: Arc<dyn crate::services::live_room::LiveRoomBroker>,
    pub s3_client: Arc<dyn crate::storage::S3Client>,
}

async fn go_live_t(
    State(s): State<LiveRoomTestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(session_id): Path<Uuid>,
) -> Result<Json<GoLiveResponse>, ApiError> {
    go_live_inner(
        &s.pool,
        s.mediamtx.as_ref(),
        &s.public_webrtc_url,
        &ctx,
        session_id,
    )
    .await
}

async fn start_now(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(cid): Path<Uuid>,
    Json(body): Json<CreateStartNow>,
) -> Result<Response, ApiError> {
    start_now_inner(&s, &ctx, cid, body).await
}

async fn start_now_t(
    State(s): State<LiveRoomTestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(cid): Path<Uuid>,
    Json(body): Json<CreateStartNow>,
) -> Result<Response, ApiError> {
    start_now_inner(&s, &ctx, cid, body).await
}

async fn go_live(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(session_id): Path<Uuid>,
) -> Result<Json<GoLiveResponse>, ApiError> {
    go_live_inner(
        &s.pool,
        s.mediamtx.as_ref(),
        &s.mediamtx_public_webrtc_url,
        &ctx,
        session_id,
    )
    .await
}

#[derive(serde::Serialize)]
pub struct EndClassResponse {
    pub session_id: Uuid,
    pub status: String,
}

async fn end_class_inner(
    pool: &PgPool,
    mediamtx_client: &dyn MediaMtxClient,
    broker: &dyn crate::services::live_room::LiveRoomBroker,
    ctx: &RequestContext,
    session_id: Uuid,
) -> Result<Json<EndClassResponse>, ApiError> {
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
    require_admin_for_session_course(pool, ctx, session.course_id).await?;

    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    db::set_request_guc(&mut tx, ctx.user_id, Some(tenant_id))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let row = db::live_sessions::end_class(&mut tx, session_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::SessionStateInvalid(
            "session not live or ended".into(),
        ))?;
    db::audit::emit_audit_event(
        &mut tx,
        tenant_id,
        ctx.user_id,
        "live_session.end_class",
        "live_session",
        session_id,
        None,
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    // Reconcile attendance: close any rows still open (sockets that never sent
    // a clean leave) up to the session's end time. Best-effort — a failure here
    // must not fail the end-class request.
    if let Err(e) =
        db::attendance::finalize_open_for_session(pool, session_id, row.actual_ended_at).await
    {
        tracing::warn!(?e, %session_id, "attendance finalize on end_class failed");
    }

    if let Some(p) = &row.main_path {
        if let Err(e) = mediamtx_client.publish_ended(p).await {
            tracing::warn!(?e, path = %p, "publish_ended best-effort failed");
        }
    }

    // Tell every connected socket the class is over (they render the
    // "session ended" state and close), then drop the per-room broker state
    // (whiteboard history, presence, hand queue) so nothing lingers in
    // Redis. Both best-effort: the class is already ended in the DB.
    if let Err(e) = broker
        .publish(
            session_id,
            crate::services::live_room::BrokerEvent::SessionEnded,
        )
        .await
    {
        tracing::warn!(?e, %session_id, "SessionEnded broadcast failed");
    }
    if let Err(e) = broker.clear_room_state(session_id).await {
        tracing::warn!(?e, %session_id, "room state cleanup failed");
    }

    Ok(Json(EndClassResponse {
        session_id,
        status: row.status,
    }))
}

async fn end_class(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(session_id): Path<Uuid>,
) -> Result<Json<EndClassResponse>, ApiError> {
    end_class_inner(
        &s.pool,
        s.mediamtx.as_ref(),
        s.live_room.as_ref(),
        &ctx,
        session_id,
    )
    .await
}

async fn end_class_t(
    State(s): State<LiveRoomTestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(session_id): Path<Uuid>,
) -> Result<Json<EndClassResponse>, ApiError> {
    end_class_inner(
        &s.pool,
        s.mediamtx.as_ref(),
        s.broker.as_ref(),
        &ctx,
        session_id,
    )
    .await
}

/// Viewer (read) JWT lifetime. Must comfortably cover a full class: MediaMTX
/// re-validates the token on every HLS segment request and on each WHEP
/// (re)connect, so a short TTL silently kills playback mid-lecture — students
/// on HLS lost video after 15 minutes and WHEP reconnects 403'd. Aligned with
/// the 4-hour publish-nonce TTL; access is still gated by enrollment at join
/// time and the token only carries read permissions for this session's paths.
const VIEWER_JWT_TTL: StdDuration = StdDuration::from_secs(4 * 60 * 60);
const JOIN_WINDOW_BEFORE: i64 = 5; // minutes
const JOIN_WINDOW_AFTER_END: i64 = 15; // minutes

/// Whether `/join` may be served for a session in `status` at `now`.
///
/// The window gates entry to a **live room**: it stops people walking into a
/// lobby days early, and stops them re-entering a room whose media plane is
/// gone. Everything it protects is live-only — the viewer JWT with MediaMTX
/// read permissions is minted solely on the `"live"` branch of `join_inner`.
///
/// A session that has **ended** is not an entry, it is a replay, and replay is
/// already authorised by `caller_can_read_course` a few lines above. Applying
/// the live window to it made every recording unwatchable 15 minutes after
/// class: `/join` answered `400 session window closed`, and because the client
/// asks `/join` for the branch to render (it carries `has_recording`), the
/// replay page never mounted at all. The recording itself was fine and the
/// Ended branch already knows how to show it.
///
/// Deliberately unbounded afterwards: how long a recording stays watchable is a
/// retention decision, enforced by the retention janitor deleting the object,
/// not something to re-litigate with a clock here. Once the row is gone
/// `has_recording` is false and the Ended branch says so.
///
/// `"cancelled"` is intentionally NOT exempt: that class never happened and has
/// nothing to replay, so the ordinary window still applies.
fn join_window_open(
    status: &str,
    now: chrono::DateTime<chrono::Utc>,
    starts_at: chrono::DateTime<chrono::Utc>,
    duration_minutes: i64,
    actual_ended_at: Option<chrono::DateTime<chrono::Utc>>,
) -> bool {
    if status == "ended" {
        return true;
    }
    let open_from = starts_at - chrono::Duration::minutes(JOIN_WINDOW_BEFORE);
    // `duration_minutes` is a scheduling hint and is deliberately NOT a bound
    // on entry.
    //
    // It used to be: `open_until` was the booked duration (60 minutes for a
    // start-now class), so a student who dropped out at minute 70 of a class
    // that was still running could not get back in, and a late joiner was
    // refused outright -- while the teacher was still publishing. Raising it to
    // a longer floor only moved the same wall further out.
    //
    // A running class has no expiry: it is over when the teacher ends it (which
    // stamps `actual_ended_at`, taken below) or when the sweep observes that
    // the publisher is genuinely gone and flips the status. Both are real
    // events, so the clock has no say here.
    let _ = duration_minutes;
    match actual_ended_at {
        // Class is over: the ordinary short rejoin grace applies.
        Some(ended) => {
            now >= open_from && now <= ended + chrono::Duration::minutes(JOIN_WINDOW_AFTER_END)
        }
        // Still running: open from the lobby onwards, with no upper bound.
        None => now >= open_from,
    }
}
const LIVE_HEALTH_PROBE_TIMEOUT: StdDuration = StdDuration::from_secs(2);

#[derive(serde::Serialize)]
pub struct JoinResponse {
    pub state: String,
    pub session_id: Uuid,
    pub transport_mode: String,
    pub viewer_jwt: Option<String>,
    pub main_url: Option<String>,
    pub screen_url: Option<String>,
    pub instructor_user_id: Option<Uuid>,
    pub course_title: String,
    pub scheduled_starts_at: chrono::DateTime<chrono::Utc>,
    #[serde(default)]
    pub has_recording: bool,
    /// ICE servers (STUN default + optional TURN) for viewer / promoted-student
    /// RTCPeerConnections. Additive: older clients ignore it.
    #[serde(default)]
    pub ice_servers: Vec<crate::services::ice::IceServer>,
}

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
        Err(_) => health_check(
            LiveHealthStatus::Error,
            "Media server",
            "Media server API check failed",
        ),
    }
}

fn media_server_timeout_health() -> LiveHealthCheckDto {
    health_check(
        LiveHealthStatus::Error,
        "Media server",
        "Media server API check timed out",
    )
}

async fn media_server_health_with_timeout(
    mediamtx_client: &dyn MediaMtxClient,
) -> LiveHealthCheckDto {
    match tokio::time::timeout(LIVE_HEALTH_PROBE_TIMEOUT, mediamtx_client.healthz()).await {
        Ok(result) => media_server_health(result),
        Err(_) => media_server_timeout_health(),
    }
}

fn main_stream_health(
    session_status: &str,
    path_status: Option<
        Result<crate::services::mediamtx::PathStatus, crate::services::mediamtx::MediaMtxError>,
    >,
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
        Some(Err(_)) => health_check(
            LiveHealthStatus::Unknown,
            "Main stream",
            "Could not check teacher stream path",
        ),
        None => health_check(
            LiveHealthStatus::Unknown,
            "Main stream",
            "Session is live but no main media path is recorded yet",
        ),
    }
}

fn path_status_timeout() -> Option<Result<PathStatus, MediaMtxError>> {
    Some(Err(MediaMtxError::Transport(
        "media server path status timed out".into(),
    )))
}

async fn path_status_with_timeout(
    mediamtx_client: &dyn MediaMtxClient,
    path: &str,
) -> Option<Result<PathStatus, MediaMtxError>> {
    match tokio::time::timeout(LIVE_HEALTH_PROBE_TIMEOUT, mediamtx_client.path_status(path)).await {
        Ok(result) => Some(result),
        Err(_) => path_status_timeout(),
    }
}

fn screen_stream_health(
    path_status: Option<
        Result<crate::services::mediamtx::PathStatus, crate::services::mediamtx::MediaMtxError>,
    >,
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
        Some(Err(_)) => health_check(
            LiveHealthStatus::Unknown,
            "Screen share",
            "Could not check screen share path",
        ),
    }
}

fn summarize_processing_error(error: Option<&str>) -> Option<String> {
    error
        .filter(|raw| !raw.trim().is_empty())
        .map(|_| "Recording processor reported a failure. Retry after class has ended.".into())
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
        None if matches!(session_status, "scheduled" | "cancelled") => LiveRecordingHealthDto {
            enabled,
            status: LiveHealthStatus::NotApplicable,
            processing_status: None,
            processing_error: None,
            retry_eligible: false,
            detail: "Recording processing is checked after class starts".into(),
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

async fn join_inner(
    pool: &PgPool,
    signer: &JwtSigner,
    public_webrtc_url: &str,
    public_hls_url: &str,
    ctx: &RequestContext,
    session_id: Uuid,
) -> Result<Json<JoinResponse>, ApiError> {
    use crate::services::mediamtx::{MediaMtxPermission, ViewerClaims};

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

    let allowed = db::courses::caller_can_read_course(
        pool,
        session.course_id,
        ctx.user_id,
        ctx.tenant_id,
        is_org_admin(ctx),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !allowed {
        return Err(ApiError::Forbidden);
    }

    if !join_window_open(
        &session.status,
        chrono::Utc::now(),
        session.starts_at,
        session.duration_minutes as i64,
        session.actual_ended_at,
    ) {
        return Err(ApiError::SessionWindowClosed("outside join window".into()));
    }

    let state_str = match session.status.as_str() {
        "scheduled" => "lobby",
        "live" => "live",
        "ended" => "ended",
        "cancelled" => "cancelled",
        other => other,
    };

    let (viewer_jwt, main_url, screen_url) = if state_str == "live" {
        let main_path = session
            .main_path
            .clone()
            .ok_or_else(|| ApiError::Internal("session live but main_path null".into()))?;
        let screen_path = session.screen_path.clone();

        let mut perms = vec![MediaMtxPermission {
            action: "read".into(),
            path: main_path.clone(),
        }];
        if let Some(sp) = &screen_path {
            perms.push(MediaMtxPermission {
                action: "read".into(),
                path: sp.clone(),
            });
        }
        // Promoted students publish to `<main_path>/student/<uuid>`. Grant a
        // single-segment wildcard read so every participant can open a WHEP
        // subscription onto a promoted student's feed and actually hear them.
        // (See `matches_wildcard_path` — the `/*` matches exactly one segment.)
        perms.push(MediaMtxPermission {
            action: "read".into(),
            path: format!("{main_path}/student/*"),
        });
        // Breakout sub-rooms publish to `<main_path>/breakout/<id_simple>`.
        // Grant a single-segment wildcard read so an assigned student can open
        // a WHEP subscription onto their breakout room's feed. (See
        // `matches_wildcard_path` — the `/*` matches exactly one segment.)
        perms.push(MediaMtxPermission {
            action: "read".into(),
            path: format!("{main_path}/breakout/*"),
        });
        let claims = ViewerClaims {
            iss: "aulalite".into(),
            sub: ctx.user_id.to_string(),
            tnt: tenant_id.to_string(),
            mediamtx_permissions: perms,
            exp: 0,
        };
        let jwt = signer.mint_viewer_jwt(claims, VIEWER_JWT_TTL);

        // WebRTC viewers now send `Authorization: Bearer <jwt>` instead of
        // `?jwt=<jwt>` in the URL (Q1-C). HLS still uses the query because
        // third-party players have no way to attach a header.
        let main_url = match session.transport_mode.as_str() {
            "webrtc" => format!("{public_webrtc_url}/{main_path}/whep"),
            "hls" => format!("{public_hls_url}/{main_path}/index.m3u8?jwt={jwt}"),
            other => {
                return Err(ApiError::Internal(format!(
                    "unknown transport_mode '{other}'"
                )));
            }
        };
        // Same transport_mode contract as `main_url`. The previous fallback
        // arm `_ => format!("{public_webrtc_url}/{sp}/whep")` was dead code:
        // `main_url` above already errors on any non-webrtc/non-hls mode, so
        // by the time we reach here `transport_mode` is one of those two.
        let screen_url = screen_path.map(|sp| match session.transport_mode.as_str() {
            "hls" => format!("{public_hls_url}/{sp}/index.m3u8?jwt={jwt}"),
            // "webrtc" and any future-additive WebRTC-like modes default to
            // WHEP. We treat this arm as the WebRTC branch because the upstream
            // `main_url` match has already filtered to `"webrtc" | "hls"`.
            _ => format!("{public_webrtc_url}/{sp}/whep"),
        });
        (Some(jwt), Some(main_url), screen_url)
    } else {
        (None, None, None)
    };

    let has_recording: bool = {
        let mut rec_tx = db::begin_with_context(pool, ctx.user_id, ctx.tenant_id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        let v = sqlx::query_scalar(
            "SELECT EXISTS (
                SELECT 1 FROM recordings
                 WHERE session_id = $1
                   AND processing_status IN ('pending','remuxing','uploading','available')
            )",
        )
        .bind(session_id)
        .fetch_one(&mut *rec_tx)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
        rec_tx
            .commit()
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        v
    };

    Ok(Json(JoinResponse {
        state: state_str.to_string(),
        session_id,
        transport_mode: session.transport_mode,
        viewer_jwt,
        main_url,
        screen_url,
        instructor_user_id: session.primary_teacher_id,
        course_title: session.course_title,
        scheduled_starts_at: session.starts_at,
        has_recording,
        ice_servers: crate::services::ice::ice_servers(),
    }))
}

async fn join(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(session_id): Path<Uuid>,
) -> Result<Json<JoinResponse>, ApiError> {
    join_inner(
        &s.pool,
        s.jwt_signer.as_ref(),
        &s.mediamtx_public_webrtc_url,
        &s.mediamtx_public_hls_url,
        &ctx,
        session_id,
    )
    .await
}

async fn join_t(
    State(s): State<LiveRoomTestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(session_id): Path<Uuid>,
) -> Result<Json<JoinResponse>, ApiError> {
    join_inner(
        &s.pool,
        s.signer.as_ref(),
        &s.public_webrtc_url,
        &s.public_hls_url,
        &ctx,
        session_id,
    )
    .await
}

async fn health_inner(
    pool: &PgPool,
    mediamtx_client: &dyn MediaMtxClient,
    ctx: &RequestContext,
    session_id: Uuid,
) -> Result<Json<LiveSessionHealthDto>, ApiError> {
    let (session, tenant_id) = if ctx.is_platform_admin {
        // Platform diagnostics may start without tenant context. Use a bounded
        // system-context lookup for this session id, then return to the
        // resolved tenant context for all tenant-scoped follow-up reads.
        let mut tx = db::begin_system_context(pool)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        let session = db::live_sessions::load_for_join(&mut *tx, session_id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?
            .ok_or(ApiError::NotFound)?;
        tx.commit()
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        let tenant_id = session.tenant_id;
        (session, tenant_id)
    } else {
        let session = db::live_sessions::load_for_join_with_context(
            pool,
            ctx.user_id,
            ctx.tenant_id,
            session_id,
        )
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::NotFound)?;
        let tenant_id = ctx
            .tenant_id
            .ok_or_else(|| ApiError::BadRequest("no tenant".into()))?;
        if session.tenant_id != tenant_id {
            return Err(ApiError::NotFound);
        }

        let allowed = db::courses::caller_can_staff_course(
            pool,
            session.course_id,
            ctx.user_id,
            ctx.tenant_id,
            is_org_admin(ctx),
        )
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
        if !allowed {
            return Err(ApiError::Forbidden);
        }
        (session, tenant_id)
    };

    let media_server_fut = media_server_health_with_timeout(mediamtx_client);
    let main_status_fut = async {
        match session.main_path.as_deref() {
            Some(path) => path_status_with_timeout(mediamtx_client, path).await,
            None => None,
        }
    };
    let screen_status_fut = async {
        match session.screen_path.as_deref() {
            Some(path) => path_status_with_timeout(mediamtx_client, path).await,
            None => None,
        }
    };
    let (media_server, main_status, screen_status) =
        tokio::join!(media_server_fut, main_status_fut, screen_status_fut);
    let main_stream = main_stream_health(&session.status, main_status);
    let screen_stream = screen_stream_health(screen_status);

    let (recording_enabled, recording_row) = {
        let mut tx = db::begin_with_context(pool, ctx.user_id, Some(tenant_id))
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        let recording_enabled = sqlx::query_scalar::<_, bool>(
            "SELECT recording_enabled FROM live_sessions WHERE id = $1",
        )
        .bind(session_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::NotFound)?;
        let row = db::recordings::fetch_by_session(&mut *tx, session_id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        tx.commit()
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        (recording_enabled, row)
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
        recording: recording_health(recording_enabled, &session.status, recording_row.as_ref()),
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

#[derive(serde::Serialize)]
pub struct RefreshTokenResponse {
    pub viewer_jwt: String,
}

async fn refresh_token_inner(
    pool: &PgPool,
    signer: &JwtSigner,
    ctx: &RequestContext,
    session_id: Uuid,
) -> Result<Json<RefreshTokenResponse>, ApiError> {
    use crate::services::mediamtx::{MediaMtxPermission, ViewerClaims};

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
    if session.status != "live" {
        return Err(ApiError::SessionStateInvalid("session not live".into()));
    }

    let allowed = db::courses::caller_can_read_course(
        pool,
        session.course_id,
        ctx.user_id,
        ctx.tenant_id,
        is_org_admin(ctx),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !allowed {
        return Err(ApiError::Forbidden);
    }

    let main_path = session
        .main_path
        .clone()
        .ok_or_else(|| ApiError::Internal("live session has no main_path".into()))?;
    let mut perms = vec![MediaMtxPermission {
        action: "read".into(),
        path: main_path.clone(),
    }];
    if let Some(sp) = &session.screen_path {
        perms.push(MediaMtxPermission {
            action: "read".into(),
            path: sp.clone(),
        });
    }
    // Wildcard read for promoted-student feeds (`<main_path>/student/<uuid>`),
    // matching the join-response grant so a refreshed token keeps the ability
    // to subscribe to promoted students.
    perms.push(MediaMtxPermission {
        action: "read".into(),
        path: format!("{main_path}/student/*"),
    });
    // Breakout sub-room feeds (`<main_path>/breakout/<id>`), matching the
    // join-response grant so a refreshed token keeps the ability to subscribe
    // to a breakout room the student is assigned to.
    perms.push(MediaMtxPermission {
        action: "read".into(),
        path: format!("{main_path}/breakout/*"),
    });
    let claims = ViewerClaims {
        iss: "aulalite".into(),
        sub: ctx.user_id.to_string(),
        tnt: tenant_id.to_string(),
        mediamtx_permissions: perms,
        exp: 0,
    };
    Ok(Json(RefreshTokenResponse {
        viewer_jwt: signer.mint_viewer_jwt(claims, VIEWER_JWT_TTL),
    }))
}

async fn refresh_token(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(session_id): Path<Uuid>,
) -> Result<Json<RefreshTokenResponse>, ApiError> {
    refresh_token_inner(&s.pool, s.jwt_signer.as_ref(), &ctx, session_id).await
}

async fn refresh_token_t(
    State(s): State<LiveRoomTestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(session_id): Path<Uuid>,
) -> Result<Json<RefreshTokenResponse>, ApiError> {
    refresh_token_inner(&s.pool, s.signer.as_ref(), &ctx, session_id).await
}

pub fn live_room_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/v1/courses/{cid}/sessions/start-now",
            routing::post(start_now),
        )
        .route(
            "/v1/courses/{cid}/active-session",
            routing::get(active_session),
        )
        .route(
            "/v1/courses/{cid}/recordings",
            routing::get(list_course_recordings),
        )
        .route("/v1/sessions/{id}/go-live", routing::post(go_live))
        .route("/v1/sessions/{id}/end-class", routing::post(end_class))
        .route("/v1/sessions/{id}/join", routing::post(join))
        .route("/v1/sessions/{id}/health", routing::get(health))
        .route(
            "/v1/sessions/{id}/refresh-token",
            routing::post(refresh_token),
        )
        .route("/v1/sessions/{id}/messages", routing::get(messages))
        .route("/v1/sessions/{id}/socket", routing::get(socket))
        .route("/v1/mediamtx/healthz", routing::get(mediamtx_healthz))
        .route("/v1/sessions/{id}/recording", routing::get(recording))
        .route("/v1/sessions/{id}/attendance", routing::get(attendance))
        .route(
            "/v1/sessions/{id}/recording/chat",
            routing::get(recording_chat),
        )
        .route(
            "/v1/sessions/{id}/recording/retry",
            routing::post(recording_retry),
        )
}

#[derive(serde::Serialize)]
pub struct CourseRecordingListItem {
    pub recording_id: Uuid,
    pub session_id: Uuid,
    pub session_title: String,
    pub starts_at: chrono::DateTime<chrono::Utc>,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub ended_at: chrono::DateTime<chrono::Utc>,
    pub duration_seconds: i32,
    pub processing_status: String,
    pub has_playback: bool,
    /// `Some(false)` when the stored MP4 has no video stream, so the list can
    /// mark it audio-only before anyone opens it. `None` = not probed.
    pub has_video: Option<bool>,
}

async fn list_course_recordings_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
) -> Result<Json<Vec<CourseRecordingListItem>>, ApiError> {
    // Enforce read-access. Same gate as recording_inner.
    let allowed = db::courses::caller_can_read_course(
        pool,
        course_id,
        ctx.user_id,
        ctx.tenant_id,
        is_org_admin(ctx),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !allowed {
        return Err(ApiError::Forbidden);
    }
    let mut tx = db::begin_with_context(pool, ctx.user_id, ctx.tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let rows = db::recordings::list_for_course(&mut *tx, course_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let items = rows
        .into_iter()
        .map(|r| CourseRecordingListItem {
            recording_id: r.recording_id,
            session_id: r.session_id,
            session_title: r.session_title,
            starts_at: r.starts_at,
            started_at: r.started_at,
            ended_at: r.ended_at,
            duration_seconds: r.duration_seconds,
            processing_status: r.processing_status,
            has_playback: r.file_asset_id.is_some(),
            has_video: r.has_video,
        })
        .collect();
    Ok(Json(items))
}

async fn list_course_recordings(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(course_id): Path<Uuid>,
) -> Result<Json<Vec<CourseRecordingListItem>>, ApiError> {
    list_course_recordings_inner(&s.pool, &ctx, course_id).await
}

#[doc(hidden)]
pub fn live_room_router_for_tests(
    pool: PgPool,
    mediamtx: Arc<dyn MediaMtxClient>,
    signer: Arc<JwtSigner>,
    public_webrtc_url: String,
    public_hls_url: String,
) -> Router {
    let broker: Arc<dyn crate::services::live_room::LiveRoomBroker> =
        Arc::new(crate::services::live_room::MockLiveRoomBroker::new());
    let s3_client: Arc<dyn crate::storage::S3Client> =
        Arc::new(crate::storage::mock::MockS3Client::new());
    Router::new()
        .route(
            "/v1/courses/{cid}/sessions/start-now",
            routing::post(start_now_t),
        )
        .route(
            "/v1/courses/{cid}/active-session",
            routing::get(active_session_t),
        )
        .route("/v1/sessions/{id}/go-live", routing::post(go_live_t))
        .route("/v1/sessions/{id}/end-class", routing::post(end_class_t))
        .route("/v1/sessions/{id}/join", routing::post(join_t))
        .route("/v1/sessions/{id}/health", routing::get(health_t))
        .route(
            "/v1/sessions/{id}/refresh-token",
            routing::post(refresh_token_t),
        )
        .route("/v1/sessions/{id}/messages", routing::get(messages_t))
        .route("/v1/sessions/{id}/socket", routing::get(socket_t))
        .route(
            "/v1/mediamtx/auth/publish",
            routing::post(mediamtx_auth_publish_t),
        )
        .route("/v1/sessions/{id}/recording", routing::get(recording_t))
        .route("/v1/sessions/{id}/attendance", routing::get(attendance_t))
        .route(
            "/v1/sessions/{id}/recording/chat",
            routing::get(recording_chat_t),
        )
        .route(
            "/v1/sessions/{id}/recording/retry",
            routing::post(recording_retry_t),
        )
        .with_state(LiveRoomTestState {
            pool,
            mediamtx,
            signer,
            public_webrtc_url,
            public_hls_url,
            broker,
            s3_client,
        })
}

#[doc(hidden)]
pub fn live_room_router_for_tests_with_broker(
    pool: PgPool,
    mediamtx: Arc<dyn MediaMtxClient>,
    signer: Arc<JwtSigner>,
    public_webrtc_url: String,
    public_hls_url: String,
    broker: Arc<dyn crate::services::live_room::LiveRoomBroker>,
) -> Router {
    let s3_client: Arc<dyn crate::storage::S3Client> =
        Arc::new(crate::storage::mock::MockS3Client::new());
    Router::new()
        .route(
            "/v1/courses/{cid}/sessions/start-now",
            routing::post(start_now_t),
        )
        .route(
            "/v1/courses/{cid}/active-session",
            routing::get(active_session_t),
        )
        .route("/v1/sessions/{id}/go-live", routing::post(go_live_t))
        .route("/v1/sessions/{id}/end-class", routing::post(end_class_t))
        .route("/v1/sessions/{id}/join", routing::post(join_t))
        .route("/v1/sessions/{id}/health", routing::get(health_t))
        .route(
            "/v1/sessions/{id}/refresh-token",
            routing::post(refresh_token_t),
        )
        .route("/v1/sessions/{id}/messages", routing::get(messages_t))
        .route("/v1/sessions/{id}/socket", routing::get(socket_t))
        .route(
            "/v1/mediamtx/auth/publish",
            routing::post(mediamtx_auth_publish_t),
        )
        .route("/v1/sessions/{id}/recording", routing::get(recording_t))
        .route("/v1/sessions/{id}/attendance", routing::get(attendance_t))
        .route(
            "/v1/sessions/{id}/recording/chat",
            routing::get(recording_chat_t),
        )
        .route(
            "/v1/sessions/{id}/recording/retry",
            routing::post(recording_retry_t),
        )
        .with_state(LiveRoomTestState {
            pool,
            mediamtx,
            signer,
            public_webrtc_url,
            public_hls_url,
            broker,
            s3_client,
        })
}

// ============================================================================
// Phase 1b-beta: MediaMTX auth-publish callback
// ============================================================================

#[derive(serde::Deserialize)]
pub struct MediaMtxAuthRequest {
    pub action: String,
    pub path: String,
    pub ip: Option<String>,
    pub user: Option<String>,
    pub password: Option<String>,
    pub protocol: Option<String>,
    pub query: Option<String>,
}

/// Extract a viewer JWT from a query string body, e.g. `jwt=eyJh...&foo=1`.
///
/// Used as a fallback when MediaMTX delivers the token in the URL query
/// instead of the password field (third-party HLS / WHEP players that can't
/// attach a header).
fn extract_jwt_from_query(query: &Option<String>) -> Option<String> {
    let q = query.as_deref()?;
    for pair in q.split('&') {
        if let Some(rest) = pair.strip_prefix("jwt=") {
            if !rest.is_empty() {
                return Some(rest.to_string());
            }
        }
    }
    None
}

/// Extract the session UUID from an aula path. Accepts both the standard
/// 4-segment form `aula/<t>/<c>/<s>` and the extended student form
/// `aula/<t>/<c>/<s>/student/<sid_simple>`, as well as the screen suffix.
fn session_id_from_path(path: &str) -> Option<uuid::Uuid> {
    // Use the strict parser for known shapes first.
    if let Ok(p) = crate::services::mediamtx::parse_path(path) {
        return Some(p.session_id);
    }
    // Fallback: extract segment index 3 (0-based after "aula").
    let segs: Vec<&str> = path.split('/').collect();
    if segs.len() >= 4 && segs[0] == "aula" {
        return uuid::Uuid::parse_str(segs[3]).ok();
    }
    None
}

async fn mediamtx_auth_publish_inner(
    pool: &PgPool,
    signer: &JwtSigner,
    broker: &dyn crate::services::live_room::LiveRoomBroker,
    public_webrtc_url: &str,
    body: MediaMtxAuthRequest,
) -> Result<axum::http::StatusCode, ApiError> {
    match body.action.as_str() {
        "publish" => {
            let session_id = session_id_from_path(&body.path).ok_or(ApiError::Forbidden)?;
            let candidate = body.password.clone().unwrap_or_default();
            if candidate.is_empty() {
                return Err(ApiError::PublishNonceInvalid);
            }
            let candidate_hash = db::live_sessions::hash_nonce(&candidate);

            // Try the teacher's main nonce first (Phase 1b-β behavior).
            let row = db::live_sessions::consume_publish_nonce(pool, session_id, &candidate_hash)
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?;
            if row.is_some() {
                return Ok(axum::http::StatusCode::OK);
            }

            // Fall through: maybe this is a student publishing via a
            // hand-raise-promoted path. Path shape:
            //   aula/<t>/<c>/<s>/student/<student_uuid_simple>
            let last = body.path.rsplit('/').next().unwrap_or("");
            let middle = body.path.rsplit('/').nth(1).unwrap_or("");
            if middle == "student" && last.len() == 32 {
                if let Ok(student_id) = uuid::Uuid::parse_str(last) {
                    let consumed = db::live_room::consume_student_publish_nonce(
                        pool,
                        session_id,
                        student_id,
                        &candidate_hash,
                    )
                    .await
                    .map_err(|e| ApiError::Internal(e.to_string()))?;
                    if consumed {
                        // The student's WHIP publish just authorized — announce
                        // it to the room so every participant opens a WHEP
                        // viewer onto the student's feed. Without this the
                        // promoted student publishes but is never heard: the
                        // `StudentPublishing` event was declared on both sides
                        // but emitted nowhere. The full WHEP URL is built from
                        // the same public WebRTC base the teacher's feeds use,
                        // mirroring `Promoted.publish_url`.
                        let display_name = db::live_room::lookup_display_name(pool, student_id)
                            .await
                            .unwrap_or_default();
                        let whep_url = format!(
                            "{}/{}/whep",
                            public_webrtc_url.trim_end_matches('/'),
                            body.path
                        );
                        let _ = broker
                            .publish(
                                session_id,
                                crate::services::live_room::BrokerEvent::StudentPublishing {
                                    user_id: student_id,
                                    path: body.path.clone(),
                                    whep_url,
                                    display_name,
                                },
                            )
                            .await;
                        return Ok(axum::http::StatusCode::OK);
                    }
                }
            }
            Err(ApiError::PublishNonceInvalid)
        }
        "read" => {
            // Accept the token in any of three forms:
            //   1. Password = `Bearer <jwt>` (case-insensitive, whitespace-tolerant).
            //      Standards-compliant clients send this.
            //   2. Password = `<jwt>` directly.
            //      MediaMTX coalesces some HTTP auth headers into the password.
            //   3. `?jwt=<jwt>` in the query body.
            //      Third-party HLS / WHEP players that can't attach a header.
            let candidate_jwt = body
                .password
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| {
                    let has_bearer_prefix = s
                        .get(..7)
                        .map(|p| p.eq_ignore_ascii_case("bearer "))
                        .unwrap_or(false);
                    if has_bearer_prefix {
                        s[7..].trim().to_string()
                    } else {
                        s.to_string()
                    }
                })
                .or_else(|| extract_jwt_from_query(&body.query));
            let token = match candidate_jwt {
                Some(t) => t,
                None => {
                    tracing::warn!(
                        path = %body.path,
                        has_password = body.password.is_some(),
                        "mediamtx read auth denied: no viewer JWT supplied (password/query empty)"
                    );
                    return Err(ApiError::Forbidden);
                }
            };
            let claims = match signer.verify_viewer_jwt(&token) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!(
                        path = %body.path,
                        error = %e,
                        "mediamtx read auth denied: viewer JWT verification failed \
                         (stale token after a backend restart, wrong signer, or expired?)"
                    );
                    return Err(ApiError::Forbidden);
                }
            };
            // Wildcard-aware match (Phase 1b-γ).
            let allowed = claims.mediamtx_permissions.iter().any(|p| {
                p.action == "read"
                    && crate::services::live_room::matches_wildcard_path(&p.path, &body.path)
            });
            if allowed {
                tracing::debug!(path = %body.path, "mediamtx read auth allowed");
                Ok(axum::http::StatusCode::OK)
            } else {
                tracing::warn!(
                    path = %body.path,
                    "mediamtx read auth denied: viewer JWT valid but carries no read \
                     permission matching the requested path"
                );
                Err(ApiError::Forbidden)
            }
        }
        _ => Err(ApiError::Forbidden),
    }
}

pub async fn mediamtx_auth_publish_pub(
    State(s): State<AppState>,
    headers: axum::http::HeaderMap,
    axum::extract::RawQuery(raw_query): axum::extract::RawQuery,
    Json(body): Json<MediaMtxAuthRequest>,
) -> Result<axum::http::StatusCode, ApiError> {
    // Require MediaMTX to present the shared secret via either the
    // `X-MediaMTX-Auth-Shared` header or the `shared` query parameter on
    // the request line. Reject otherwise. `main.rs` guarantees the
    // secret is `Some` for production and auto-generates an ephemeral
    // one outside production, so the `None` branch only triggers when
    // AppState is constructed without a secret (tests).
    if let Some(expected) = s.mediamtx_auth_shared_secret.as_deref() {
        fn shared_from_query(q: &str) -> Option<&str> {
            q.split('&').find_map(|pair| {
                let mut it = pair.splitn(2, '=');
                match (it.next(), it.next()) {
                    (Some("shared"), Some(v)) if !v.is_empty() => Some(v.trim()),
                    _ => None,
                }
            })
        }
        let header_secret = headers
            .get("x-mediamtx-auth-shared")
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        // The shared secret is injected into the auth-callback URL
        // (`MTX_AUTHHTTPADDRESS=...?shared=...`), so it arrives on THIS
        // request's own URI query. MediaMTX's `body.query` is the *publisher's*
        // query string (empty for a WHIP POST), so reading only that always
        // failed — the secret must be read from the request URI here.
        let uri_query_secret = raw_query.as_deref().and_then(shared_from_query);
        let body_query_secret = body.query.as_deref().and_then(shared_from_query);
        let provided = header_secret
            .or(uri_query_secret)
            .or(body_query_secret)
            .unwrap_or("");
        // Constant-time compare so the response timing can't be used to recover
        // the shared secret byte-by-byte (length still leaks, which is fine).
        if provided.is_empty()
            || !crate::auth::local_login::constant_time_eq(provided.as_bytes(), expected.as_bytes())
        {
            tracing::warn!(
                action = %body.action,
                "mediamtx auth callback rejected: shared-secret mismatch"
            );
            return Err(ApiError::Forbidden);
        }
    }
    mediamtx_auth_publish_inner(
        &s.pool,
        s.jwt_signer.as_ref(),
        s.live_room.as_ref(),
        &s.mediamtx_public_webrtc_url,
        body,
    )
    .await
}

async fn mediamtx_auth_publish_t(
    State(s): State<LiveRoomTestState>,
    Json(body): Json<MediaMtxAuthRequest>,
) -> Result<axum::http::StatusCode, ApiError> {
    mediamtx_auth_publish_inner(
        &s.pool,
        s.signer.as_ref(),
        s.broker.as_ref(),
        &s.public_webrtc_url,
        body,
    )
    .await
}

// ============================================================================
// Phase 1b-beta: JWKS endpoint
// ============================================================================

pub async fn mediamtx_jwks(State(s): State<AppState>) -> impl axum::response::IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        s.jwt_signer.public_jwks().to_string(),
    )
}

// ============================================================================
// Phase 1b-beta: MediaMTX healthz proxy
// ============================================================================

#[derive(serde::Serialize)]
pub struct MediaMtxHealthDto {
    pub healthy: bool,
}

async fn mediamtx_healthz(State(s): State<AppState>) -> Json<MediaMtxHealthDto> {
    Json(MediaMtxHealthDto {
        healthy: s.mediamtx.healthz().await.is_ok(),
    })
}

// ============================================================================
// Phase 1b-γ: GET /v1/sessions/{id}/messages — paginated chat history
// ============================================================================

#[derive(serde::Deserialize, Default)]
pub struct MessagesQuery {
    #[serde(default)]
    pub before: Option<Uuid>,
    #[serde(default)]
    pub limit: Option<i64>,
}

#[derive(serde::Serialize)]
pub struct ChatMessageDto {
    pub id: Uuid,
    pub sender_user_id: Uuid,
    pub body: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub deleted: bool,
}

#[derive(serde::Serialize)]
pub struct MessagesResponse {
    pub messages: Vec<ChatMessageDto>,
    pub next_cursor: Option<Uuid>,
}

async fn messages_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    session_id: Uuid,
    q: MessagesQuery,
) -> Result<Json<MessagesResponse>, ApiError> {
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
    let allowed = db::courses::caller_can_read_course(
        pool,
        session.course_id,
        ctx.user_id,
        ctx.tenant_id,
        is_org_admin(ctx),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !allowed {
        return Err(ApiError::Forbidden);
    }

    // Clamp the per-request page size; defends against attackers passing
    // `limit=1000000` and against accidental zero/negative values.
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let mut tx = db::begin_with_context(pool, ctx.user_id, ctx.tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let rows = db::live_room::fetch_paginated(&mut *tx, session_id, q.before, limit)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let is_admin = is_org_admin(ctx);
    let is_teacher = db::courses::caller_can_admin_course(
        pool,
        session.course_id,
        ctx.user_id,
        ctx.tenant_id,
        is_admin,
    )
    .await
    .unwrap_or(false);

    let messages: Vec<ChatMessageDto> = rows
        .iter()
        .map(|r| ChatMessageDto {
            id: r.id,
            sender_user_id: r.sender_user_id,
            body: if r.deleted_at.is_some() && !is_teacher {
                "[deleted]".into()
            } else {
                r.body.clone()
            },
            created_at: r.created_at,
            deleted: r.deleted_at.is_some(),
        })
        .collect();
    let next_cursor = rows.last().map(|r| r.id);
    Ok(Json(MessagesResponse {
        messages,
        next_cursor,
    }))
}

async fn messages(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(session_id): Path<Uuid>,
    axum::extract::Query(q): axum::extract::Query<MessagesQuery>,
) -> Result<Json<MessagesResponse>, ApiError> {
    messages_inner(&s.pool, &ctx, session_id, q).await
}

async fn messages_t(
    State(s): State<LiveRoomTestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(session_id): Path<Uuid>,
    axum::extract::Query(q): axum::extract::Query<MessagesQuery>,
) -> Result<Json<MessagesResponse>, ApiError> {
    messages_inner(&s.pool, &ctx, session_id, q).await
}

// ============================================================================
// Phase 1b-γ: GET /v1/sessions/{id}/socket — WebSocket upgrade + auth + kick gate
// ============================================================================

use axum::extract::ws::{WebSocket, WebSocketUpgrade};

/// Run all auth and kick gate checks. Returns Ok(()) if the caller is allowed
/// to open the socket, or the appropriate ApiError otherwise.
/// This must complete *before* WebSocketUpgrade extraction so that kick/auth
/// errors are returned as HTTP errors even on plain GET requests.
async fn socket_gate(
    pool: &PgPool,
    ctx: &RequestContext,
    session_id: Uuid,
) -> Result<(), ApiError> {
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

    // Course membership gate.
    let allowed = db::courses::caller_can_read_course(
        pool,
        session.course_id,
        ctx.user_id,
        ctx.tenant_id,
        is_org_admin(ctx),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !allowed {
        return Err(ApiError::Forbidden);
    }

    // Kick gate (DB is durable source of truth).
    {
        let mut tx = db::begin_with_context(pool, ctx.user_id, ctx.tenant_id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        let kicked = db::live_room::kick_exists(&mut *tx, session_id, ctx.user_id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        tx.commit()
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        if kicked {
            return Err(ApiError::Forbidden);
        }
    }

    // Session must be scheduled or live.
    if !matches!(session.status.as_str(), "scheduled" | "live") {
        return Err(ApiError::SessionStateInvalid(format!(
            "session is {}; cannot open socket",
            session.status
        )));
    }

    Ok(())
}

// ============================================================================
// Phase 1b-γ: Client envelope + WebSocket message dispatch loop
// ============================================================================

#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientEnvelope {
    Heartbeat,
    Chat {
        body: String,
    },
    HandRaise {
        raise: bool,
    },
    DeleteMessage {
        message_id: Uuid,
    },
    WhiteboardStroke {
        stroke: crate::services::live_room::WhiteboardStroke,
    },
    /// Undo a stroke. When `author` is present (per-author undo) the server
    /// removes that author's most-recent stroke; when absent the legacy global
    /// undo removes the board's most-recent stroke. The client may only request
    /// an undo for its OWN strokes — the server stamps strokes with the sender's
    /// id and ignores a mismatched `author` (see the handler).
    WhiteboardUndo {
        #[serde(default)]
        author: Option<Uuid>,
    },
    /// Geometric eraser: remove a single element by id (the client hit-tests
    /// the eraser path against existing elements). Broadcasts the removal.
    WhiteboardErase {
        stroke_id: String,
    },
    WhiteboardClear,
    /// Ephemeral per-user cursor over the whiteboard. Permitted senders
    /// (teacher, or any participant when the draw-permission flag is open)
    /// broadcast their pointer position; never persisted. `x`/`y` are
    /// normalized board coordinates in `0.0..=1.0`.
    WhiteboardCursor {
        x: f32,
        y: f32,
    },
    /// Teacher toggles whether non-teachers may draw on the board.
    SetDrawPermission {
        open: bool,
    },
    Reaction {
        emoji: String,
    },
    /// Teacher starts an in-class poll. `options` must be 2..=6 entries; the
    /// server mints the `poll_id` and broadcasts `PollStarted`.
    StartPoll {
        question: String,
        options: Vec<String>,
    },
    /// Any participant casts a single vote for `option_index` in `poll_id`.
    /// The server dedupes voters and broadcasts `PollResults` on the first vote.
    PollVote {
        poll_id: Uuid,
        option_index: u32,
    },
    /// Teacher ends `poll_id`; the server broadcasts `PollEnded` with the final
    /// tally and clears the poll.
    EndPoll {
        poll_id: Uuid,
    },
    /// Teacher creates a fresh breakout layout from explicit room names (the
    /// rooms start empty). Replaces any existing layout; does NOT open them.
    CreateBreakouts {
        names: Vec<String>,
    },
    /// Teacher auto-splits everyone currently present into `room_count`
    /// round-robin rooms and (re)builds the layout. Does NOT open them.
    AutoSplitBreakouts {
        room_count: u32,
    },
    /// Teacher moves `user_id` into `room_id` (or back to the main room when
    /// `room_id` is `None`). Re-broadcasts the updated layout + targeted
    /// assignments.
    AssignBreakout {
        user_id: Uuid,
        #[serde(default)]
        room_id: Option<Uuid>,
    },
    /// Teacher opens the created breakout rooms (transitions the layout to
    /// `open` and fans assignments out so assigned students re-subscribe).
    OpenBreakouts,
    /// Teacher closes all breakout rooms; everyone returns to the main room.
    CloseBreakouts,
    Kick {
        user_id: Uuid,
    },
    AcceptHand {
        user_id: Uuid,
    },
    DemoteHand {
        user_id: Uuid,
    },
}

#[allow(clippy::too_many_arguments)]
async fn run_socket(
    socket: WebSocket,
    pool: PgPool,
    broker: Arc<dyn crate::services::live_room::LiveRoomBroker>,
    user_id: Uuid,
    tenant_id: Uuid,
    session_id: Uuid,
    course_id: Uuid,
    is_teacher: bool,
    display_name: String,
    public_webrtc_url: String,
) {
    use crate::services::live_room::{BrokerEvent, PresenceEntry, TokenBucket};
    use futures_util::{SinkExt, StreamExt};
    use std::time::Duration;

    let (mut sender, mut receiver) = socket.split();

    let mut sub = match broker.subscribe(session_id).await {
        Ok(s) => s,
        Err(_) => return,
    };

    // Register presence.
    let _ = broker
        .presence_join(
            session_id,
            PresenceEntry {
                user_id,
                display_name: display_name.clone(),
                role: if is_teacher {
                    "teacher".into()
                } else {
                    "student".into()
                },
                last_seen_ms: chrono::Utc::now().timestamp_millis(),
            },
        )
        .await;
    // Persist the join for durable attendance. Best-effort: log+swallow so a
    // DB hiccup can never break the live socket (same contract as presence).
    if let Err(e) = db::attendance::record_join(&pool, tenant_id, session_id, user_id).await {
        tracing::warn!(?e, %session_id, %user_id, "attendance record_join failed");
    }
    // Best-effort attendance XP for students (once per session via dedup).
    if !is_teacher {
        let award_result = async {
            let mut tx = db::begin_with_context(&pool, user_id, Some(tenant_id)).await?;
            db::gamification::award(
                &mut tx,
                tenant_id,
                user_id,
                Some(course_id),
                "session_attended",
                db::gamification::XP_SESSION_ATTENDED,
                &format!("session:{session_id}:{user_id}"),
            )
            .await?;
            tx.commit().await
        }
        .await;
        if let Err(e) = award_result {
            tracing::warn!(?e, %session_id, %user_id, "attendance XP award failed");
        }
    }
    let count = broker.presence_count(session_id).await.unwrap_or(0);
    let _ = broker
        .publish(session_id, BrokerEvent::PresenceCount { count })
        .await;

    // Hydrate this socket with the current whiteboard so late joiners and
    // reconnects see existing strokes. Point-to-point (not broadcast): only
    // this socket needs it. Sent even when empty so a reconnecting client
    // can drop strokes that were cleared while it was away.
    match broker.whiteboard_strokes(session_id).await {
        Ok(strokes) => {
            let _ = sender
                .send(ws_text(
                    serde_json::to_string(&BrokerEvent::WhiteboardSnapshot { strokes })
                        .expect("BrokerEvent::WhiteboardSnapshot must serialize"),
                ))
                .await;
        }
        Err(e) => {
            tracing::warn!(error = %e, %session_id, "whiteboard snapshot hydrate failed");
        }
    }

    // Hydrate the current draw-permission flag so a late joiner knows whether
    // students may draw. Point-to-point. Best-effort: a flag read failure just
    // leaves the client at its default (teacher-only).
    match broker.draw_open(session_id).await {
        Ok(open) => {
            let _ = sender
                .send(ws_text(
                    serde_json::to_string(&BrokerEvent::DrawPermissionChanged { open })
                        .expect("BrokerEvent::DrawPermissionChanged must serialize"),
                ))
                .await;
        }
        Err(e) => {
            tracing::warn!(error = %e, %session_id, "draw permission hydrate failed");
        }
    }

    let mut chat_bucket = TokenBucket::new(1, Duration::from_secs(2));
    let mut hand_bucket = TokenBucket::new(5, Duration::from_secs(60));
    // Whiteboard strokes are emitted once per completed pen stroke (on
    // pointer-up), so a teacher sketching quickly produces a handful per
    // second. Allow a generous burst of 20 with a sustained ~10/sec refill so
    // legitimate drawing is never throttled, while a buggy or hostile client
    // can't flood the broker (each stroke is a Redis RPUSH + LTRIM + fan-out).
    let mut whiteboard_bucket = TokenBucket::new(20, Duration::from_millis(100));
    // Reactions are bursty (a few quick taps) but must not flood the room; a
    // small burst with a steady ~2/sec refill is plenty.
    let mut reaction_bucket = TokenBucket::new(8, Duration::from_millis(500));
    // Cursor moves are throttled client-side (~60ms) but the server also caps
    // them so a misbehaving client can't flood the room. A generous burst with
    // a sustained ~20/sec refill matches the client throttle headroom.
    let mut cursor_bucket = TokenBucket::new(30, Duration::from_millis(50));
    // Poll actions (start/vote/end). A student votes at most once per poll, but
    // a small burst tolerates rapid taps / accidental double-clicks while still
    // capping a hostile client. Shared across start/vote/end (all low-rate).
    let mut poll_bucket = TokenBucket::new(5, Duration::from_secs(2));
    // Breakout actions (create/auto-split/assign/open/close) — teacher-only and
    // low-rate, but assignment fine-tuning can come in quick bursts as the
    // teacher drags participants between rooms. A generous burst with a steady
    // ~2/sec refill keeps the UI responsive without letting a buggy client
    // hammer the broker.
    let mut breakout_bucket = TokenBucket::new(10, Duration::from_millis(500));

    // Hydrate the current breakout layout point-to-point so a (re)joining client
    // picks up open breakouts (and a student re-subscribes to their assigned
    // sub-room). Best-effort; sent even when closed so a reconnecting client can
    // drop a stale local layout. Filtered server-side is unnecessary — the
    // snapshot carries the full layout and each client picks its own assignment
    // from the targeted `BreakoutAssignment` that follows for assigned users.
    match broker.breakout_get(session_id).await {
        Ok(bk) => {
            let _ = sender
                .send(ws_text(
                    serde_json::to_string(&BrokerEvent::BreakoutSnapshot {
                        open: bk.open,
                        rooms: bk.rooms.clone(),
                    })
                    .expect("BrokerEvent::BreakoutSnapshot must serialize"),
                ))
                .await;
            // If this socket's user is assigned to a room, send the targeted
            // assignment too so they immediately re-subscribe their WHEP viewer.
            if bk.room_for(user_id).is_some() {
                let main_path =
                    crate::services::mediamtx::path_for_session(tenant_id, course_id, session_id);
                let evt = crate::handlers::breakout::assignment_event(
                    &bk,
                    user_id,
                    &main_path,
                    &public_webrtc_url,
                );
                let _ = sender
                    .send(ws_text(
                        serde_json::to_string(&evt)
                            .expect("BrokerEvent::BreakoutAssignment must serialize"),
                    ))
                    .await;
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, %session_id, "breakout snapshot hydrate failed");
        }
    }

    loop {
        tokio::select! {
            msg = receiver.next() => {
                match msg {
                    Some(Ok(axum::extract::ws::Message::Text(t))) => {
                        if let Ok(env) = serde_json::from_str::<ClientEnvelope>(&t) {
                            handle_client_envelope(
                                &pool, broker.as_ref(),
                                user_id, tenant_id, session_id, course_id,
                                is_teacher, &display_name, &public_webrtc_url,
                                &mut chat_bucket, &mut hand_bucket, &mut whiteboard_bucket,
                                &mut reaction_bucket, &mut cursor_bucket, &mut poll_bucket,
                                &mut breakout_bucket,
                                env, &mut sender,
                            ).await;
                        }
                    }
                    Some(Ok(axum::extract::ws::Message::Close(_))) | None => break,
                    _ => {}
                }
            }
            evt = sub.recv() => {
                match evt {
                    Some(BrokerEvent::SessionEnded) => {
                        let _ = sender.send(ws_text(
                            serde_json::to_string(&BrokerEvent::SessionEnded)
                                .expect("BrokerEvent::SessionEnded must serialize")
                        )).await;
                        break;
                    }
                    Some(BrokerEvent::PresenceList { .. }) if !is_teacher => continue,
                    Some(BrokerEvent::Promoted { user_id: target, .. }) if target != user_id => continue,
                    // Targeted assignment: only the recipient (re)subscribes
                    // their WHEP viewer. Everyone else ignores it (the shared
                    // layout already reached them via BreakoutOpened/Updated).
                    Some(BrokerEvent::BreakoutAssignment { user_id: target, .. }) if target != user_id => continue,
                    Some(BrokerEvent::Kicked { user_id: target }) if target == user_id => {
                        let _ = sender.send(ws_text(
                            serde_json::to_string(&BrokerEvent::Kicked { user_id: target })
                                .expect("BrokerEvent::Kicked must serialize")
                        )).await;
                        break;
                    }
                    Some(evt) => {
                        let _ = sender.send(ws_text(
                            serde_json::to_string(&evt)
                                .expect("BrokerEvent must serialize")
                        )).await;
                    }
                    None => break,
                }
            }
        }
    }

    let _ = broker.presence_leave(session_id, user_id).await;
    // Persist the leave for durable attendance (accrues total_seconds for the
    // most-recent segment). Best-effort, same as the join above.
    if let Err(e) = db::attendance::record_leave(&pool, tenant_id, session_id, user_id).await {
        tracing::warn!(?e, %session_id, %user_id, "attendance record_leave failed");
    }
    let count = broker.presence_count(session_id).await.unwrap_or(0);
    let _ = broker
        .publish(session_id, BrokerEvent::PresenceCount { count })
        .await;
}

/// Serialize a `CommandFailed` event and push it down the caller's socket
/// sink. The error is best-effort: if the send itself fails, the socket is
/// already gone and there is nothing we can do.
async fn send_command_failed(
    sender: &mut futures_util::stream::SplitSink<WebSocket, axum::extract::ws::Message>,
    command: &str,
    reason: &str,
) {
    use crate::services::live_room::BrokerEvent;
    use futures_util::SinkExt;
    let evt = BrokerEvent::CommandFailed {
        command: command.to_string(),
        reason: reason.to_string(),
    };
    if let Ok(json) = serde_json::to_string(&evt) {
        let _ = sender.send(ws_text(json)).await;
    }
}

/// Persist a newly-started poll for history. Tenant-scoped under RLS (sets the
/// request GUC inside its own transaction). The live poll itself is
/// broker-authoritative; this row is only for after-class review, so every
/// failure here is logged-and-swallowed by the caller.
async fn persist_poll_started(
    pool: &PgPool,
    tenant_id: Uuid,
    session_id: Uuid,
    created_by: Uuid,
    poll_id: Uuid,
    question: &str,
    options: &[String],
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    db::set_request_guc(&mut tx, created_by, Some(tenant_id)).await?;
    let options_json =
        serde_json::to_value(options).unwrap_or(serde_json::Value::Array(Vec::new()));
    let zero_counts: Vec<i64> = vec![0; options.len()];
    let counts_json =
        serde_json::to_value(&zero_counts).unwrap_or(serde_json::Value::Array(Vec::new()));
    sqlx::query(
        "INSERT INTO live_session_polls
            (id, tenant_id, session_id, created_by, question, options, counts)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         ON CONFLICT (id) DO NOTHING",
    )
    .bind(poll_id)
    .bind(tenant_id)
    .bind(session_id)
    .bind(created_by)
    .bind(question)
    .bind(options_json)
    .bind(counts_json)
    .execute(&mut *tx)
    .await?;
    tx.commit().await
}

/// Update the persisted tally for an in-flight poll. Best-effort history;
/// tenant-scoped under RLS (sets the request GUC). `actor` is the voting user
/// (only used to satisfy the GUC; the WHERE clause pins poll + session).
async fn persist_poll_counts(
    pool: &PgPool,
    tenant_id: Uuid,
    actor: Uuid,
    session_id: Uuid,
    poll_id: Uuid,
    counts: &[u32],
) -> Result<(), sqlx::Error> {
    let counts_i64: Vec<i64> = counts.iter().map(|c| *c as i64).collect();
    let counts_json =
        serde_json::to_value(&counts_i64).unwrap_or(serde_json::Value::Array(Vec::new()));
    let mut tx = pool.begin().await?;
    db::set_request_guc(&mut tx, actor, Some(tenant_id)).await?;
    sqlx::query(
        "UPDATE live_session_polls
            SET counts = $3, updated_at = now()
          WHERE id = $1 AND session_id = $2",
    )
    .bind(poll_id)
    .bind(session_id)
    .bind(counts_json)
    .execute(&mut *tx)
    .await?;
    tx.commit().await
}

/// Stamp a poll closed (final counts + ended_at) for history. Best-effort;
/// tenant-scoped under RLS.
async fn persist_poll_ended(
    pool: &PgPool,
    tenant_id: Uuid,
    actor: Uuid,
    session_id: Uuid,
    poll_id: Uuid,
    counts: &[u32],
) -> Result<(), sqlx::Error> {
    let counts_i64: Vec<i64> = counts.iter().map(|c| *c as i64).collect();
    let counts_json =
        serde_json::to_value(&counts_i64).unwrap_or(serde_json::Value::Array(Vec::new()));
    let mut tx = pool.begin().await?;
    db::set_request_guc(&mut tx, actor, Some(tenant_id)).await?;
    sqlx::query(
        "UPDATE live_session_polls
            SET counts = $3, ended_at = now(), updated_at = now()
          WHERE id = $1 AND session_id = $2 AND ended_at IS NULL",
    )
    .bind(poll_id)
    .bind(session_id)
    .bind(counts_json)
    .execute(&mut *tx)
    .await?;
    tx.commit().await
}

#[allow(clippy::too_many_arguments)]
async fn handle_client_envelope(
    pool: &PgPool,
    broker: &dyn crate::services::live_room::LiveRoomBroker,
    user_id: Uuid,
    tenant_id: Uuid,
    session_id: Uuid,
    course_id: Uuid,
    is_teacher: bool,
    display_name: &str,
    public_webrtc_url: &str,
    chat_bucket: &mut crate::services::live_room::TokenBucket,
    hand_bucket: &mut crate::services::live_room::TokenBucket,
    whiteboard_bucket: &mut crate::services::live_room::TokenBucket,
    reaction_bucket: &mut crate::services::live_room::TokenBucket,
    cursor_bucket: &mut crate::services::live_room::TokenBucket,
    poll_bucket: &mut crate::services::live_room::TokenBucket,
    breakout_bucket: &mut crate::services::live_room::TokenBucket,
    env: ClientEnvelope,
    sender: &mut futures_util::stream::SplitSink<WebSocket, axum::extract::ws::Message>,
) {
    use crate::services::live_room::BrokerEvent;
    use futures_util::SinkExt;

    match env {
        ClientEnvelope::Heartbeat => {
            let _ = broker.presence_heartbeat(session_id, user_id).await;
        }
        ClientEnvelope::Chat { body } => {
            if chat_bucket.try_consume().is_err() {
                let _ = sender
                    .send(ws_text(
                        serde_json::to_string(&BrokerEvent::RateLimited {
                            retry_after_ms: 2000,
                        })
                        .expect("BrokerEvent::RateLimited must serialize"),
                    ))
                    .await;
                return;
            }
            // Validate (trim + non-empty + length cap) then sanitize (strip
            // control chars) the plain-text chat line before persistence.
            let body = match crate::services::validate::chat(&body) {
                Ok(c) => crate::services::sanitize::clean_text(
                    c,
                    crate::services::validate::MAX_CHAT_LEN,
                ),
                Err(_) => {
                    send_command_failed(sender, "chat", "validation error").await;
                    return;
                }
            };
            let mut tx = match pool.begin().await {
                Ok(t) => t,
                Err(e) => {
                    tracing::warn!(error = %e, command = "chat", "command failed");
                    send_command_failed(
                        sender,
                        "chat",
                        &ApiError::Internal(e.to_string()).user_facing(),
                    )
                    .await;
                    return;
                }
            };
            if let Err(e) = db::set_request_guc(&mut tx, user_id, Some(tenant_id)).await {
                tracing::warn!(error = %e, command = "chat", "set_request_guc failed");
                send_command_failed(
                    sender,
                    "chat",
                    &ApiError::Internal(e.to_string()).user_facing(),
                )
                .await;
                return;
            }
            let row =
                match db::live_room::insert_message(&mut tx, tenant_id, session_id, user_id, &body)
                    .await
                {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::warn!(error = %e, command = "chat", "command failed");
                        send_command_failed(
                            sender,
                            "chat",
                            &ApiError::Internal(e.to_string()).user_facing(),
                        )
                        .await;
                        return;
                    }
                };
            if let Err(e) = tx.commit().await {
                tracing::warn!(error = %e, command = "chat", "command failed");
                send_command_failed(
                    sender,
                    "chat",
                    &ApiError::Internal(e.to_string()).user_facing(),
                )
                .await;
                return;
            }
            let _ = broker
                .publish(
                    session_id,
                    BrokerEvent::Chat {
                        id: row.id,
                        sender_user_id: user_id,
                        sender_display_name: display_name.to_string(),
                        body: row.body,
                        created_at: row.created_at,
                    },
                )
                .await;
        }
        ClientEnvelope::DeleteMessage { message_id } => {
            if !is_teacher {
                send_command_failed(sender, "delete_message", &ApiError::Forbidden.user_facing())
                    .await;
                return;
            }
            let mut tx = match pool.begin().await {
                Ok(t) => t,
                Err(e) => {
                    tracing::warn!(error = %e, command = "delete_message", "command failed");
                    send_command_failed(
                        sender,
                        "delete_message",
                        &ApiError::Internal(e.to_string()).user_facing(),
                    )
                    .await;
                    return;
                }
            };
            if let Err(e) = db::set_request_guc(&mut tx, user_id, Some(tenant_id)).await {
                tracing::warn!(error = %e, command = "delete_message", "set_request_guc failed");
                send_command_failed(
                    sender,
                    "delete_message",
                    &ApiError::Internal(e.to_string()).user_facing(),
                )
                .await;
                return;
            }
            let row = match db::live_room::soft_delete(&mut tx, message_id, user_id).await {
                Ok(Some(r)) => r,
                Ok(None) => {
                    tracing::warn!(
                        command = "delete_message",
                        "message not found or already deleted"
                    );
                    send_command_failed(
                        sender,
                        "delete_message",
                        &ApiError::NotFound.user_facing(),
                    )
                    .await;
                    return;
                }
                Err(e) => {
                    tracing::warn!(error = %e, command = "delete_message", "command failed");
                    send_command_failed(
                        sender,
                        "delete_message",
                        &ApiError::Internal(e.to_string()).user_facing(),
                    )
                    .await;
                    return;
                }
            };
            if let Err(e) = tx.commit().await {
                tracing::warn!(error = %e, command = "delete_message", "command failed");
                send_command_failed(
                    sender,
                    "delete_message",
                    &ApiError::Internal(e.to_string()).user_facing(),
                )
                .await;
                return;
            }
            let _ = broker
                .publish(session_id, BrokerEvent::ChatDeleted { id: row.id })
                .await;
        }
        ClientEnvelope::HandRaise { raise } => {
            if hand_bucket.try_consume().is_err() {
                let _ = sender
                    .send(ws_text(
                        serde_json::to_string(&BrokerEvent::RateLimited {
                            retry_after_ms: 12000,
                        })
                        .expect("BrokerEvent::RateLimited must serialize"),
                    ))
                    .await;
                return;
            }
            let pos = if raise {
                broker.hand_raise(session_id, user_id).await.ok()
            } else {
                let _ = broker.hand_lower(session_id, user_id).await;
                None
            };
            let _ = broker
                .publish(
                    session_id,
                    BrokerEvent::HandRaiseChanged {
                        user_id,
                        raised: raise,
                        display_name: display_name.to_string(),
                        queue_position: pos,
                    },
                )
                .await;
        }
        ClientEnvelope::AcceptHand {
            user_id: target_user_id,
        } => {
            if !is_teacher {
                send_command_failed(sender, "accept_hand", &ApiError::Forbidden.user_facing())
                    .await;
                return;
            }
            let mut buf = [0u8; 24];
            rand::rng().fill_bytes(&mut buf);
            use base64::Engine;
            let nonce_plain = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf);
            let nonce_hash = db::live_sessions::hash_nonce(&nonce_plain);
            let mut tx = match pool.begin().await {
                Ok(t) => t,
                Err(e) => {
                    tracing::warn!(error = %e, command = "accept_hand", "command failed");
                    send_command_failed(
                        sender,
                        "accept_hand",
                        &ApiError::Internal(e.to_string()).user_facing(),
                    )
                    .await;
                    return;
                }
            };
            if let Err(e) = db::set_request_guc(&mut tx, user_id, Some(tenant_id)).await {
                tracing::warn!(error = %e, command = "accept_hand", "set_request_guc failed");
                send_command_failed(
                    sender,
                    "accept_hand",
                    &ApiError::Internal(e.to_string()).user_facing(),
                )
                .await;
                return;
            }
            if let Err(e) = db::live_room::set_student_publish_nonce(
                &mut tx,
                session_id,
                target_user_id,
                &nonce_hash,
                chrono::Utc::now() + chrono::Duration::hours(4),
            )
            .await
            {
                tracing::warn!(error = %e, command = "accept_hand", "command failed");
                send_command_failed(
                    sender,
                    "accept_hand",
                    &ApiError::Internal(e.to_string()).user_facing(),
                )
                .await;
                return;
            }
            if let Err(e) = tx.commit().await {
                tracing::warn!(error = %e, command = "accept_hand", "command failed");
                send_command_failed(
                    sender,
                    "accept_hand",
                    &ApiError::Internal(e.to_string()).user_facing(),
                )
                .await;
                return;
            }
            let _ = broker.hand_lower(session_id, target_user_id).await;
            let target_display_name = db::live_room::lookup_display_name(pool, target_user_id)
                .await
                .unwrap_or_default();
            let path = format!(
                "aula/{}/{}/{}/student/{}",
                tenant_id.simple(),
                course_id.simple(),
                session_id.simple(),
                target_user_id.simple()
            );
            let publish_url = format!("{}/{}/whip", public_webrtc_url, path);
            let _ = broker
                .publish(
                    session_id,
                    BrokerEvent::Promoted {
                        user_id: target_user_id,
                        publish_url,
                        publish_password: nonce_plain,
                    },
                )
                .await;
            let _ = broker
                .publish(
                    session_id,
                    BrokerEvent::HandRaiseChanged {
                        user_id: target_user_id,
                        raised: false,
                        display_name: target_display_name,
                        queue_position: None,
                    },
                )
                .await;
        }
        ClientEnvelope::DemoteHand {
            user_id: target_user_id,
        } => {
            if !is_teacher {
                send_command_failed(sender, "demote_hand", &ApiError::Forbidden.user_facing())
                    .await;
                return;
            }
            // The clear MUST succeed BEFORE we emit `Demoted`: if a client
            // sees a Demoted event for a target whose publish nonce is still
            // valid, the target could keep publishing for the rest of the
            // nonce TTL.
            let mut tx = match pool.begin().await {
                Ok(t) => t,
                Err(e) => {
                    tracing::warn!(error = %e, command = "demote_hand", "command failed");
                    send_command_failed(
                        sender,
                        "demote_hand",
                        &ApiError::Internal(e.to_string()).user_facing(),
                    )
                    .await;
                    return;
                }
            };
            if let Err(e) = db::set_request_guc(&mut tx, user_id, Some(tenant_id)).await {
                tracing::warn!(error = %e, command = "demote_hand", "set_request_guc failed");
                send_command_failed(
                    sender,
                    "demote_hand",
                    &ApiError::Internal(e.to_string()).user_facing(),
                )
                .await;
                return;
            }
            if let Err(e) =
                db::live_room::clear_student_publish_nonce(&mut tx, session_id, target_user_id)
                    .await
            {
                tracing::warn!(error = %e, command = "demote_hand", "command failed");
                send_command_failed(
                    sender,
                    "demote_hand",
                    &ApiError::Internal(e.to_string()).user_facing(),
                )
                .await;
                return;
            }
            if let Err(e) = tx.commit().await {
                tracing::warn!(error = %e, command = "demote_hand", "command failed");
                send_command_failed(
                    sender,
                    "demote_hand",
                    &ApiError::Internal(e.to_string()).user_facing(),
                )
                .await;
                return;
            }
            let target_display_name = db::live_room::lookup_display_name(pool, target_user_id)
                .await
                .unwrap_or_default();
            let _ = broker
                .publish(
                    session_id,
                    BrokerEvent::Demoted {
                        user_id: target_user_id,
                        display_name: target_display_name,
                    },
                )
                .await;
        }
        ClientEnvelope::WhiteboardStroke { mut stroke } => {
            // Non-teachers may draw only while the room's draw-permission flag
            // is open. Teachers always may.
            if !is_teacher && !broker.draw_open(session_id).await.unwrap_or(false) {
                send_command_failed(
                    sender,
                    "whiteboard_stroke",
                    &ApiError::Forbidden.user_facing(),
                )
                .await;
                return;
            }
            if whiteboard_bucket.try_consume().is_err() {
                let _ = sender
                    .send(ws_text(
                        serde_json::to_string(&BrokerEvent::RateLimited {
                            retry_after_ms: 100,
                        })
                        .expect("BrokerEvent::RateLimited must serialize"),
                    ))
                    .await;
                return;
            }
            // Stamp the author server-side so a client cannot spoof another
            // user's id (the per-author undo trusts this field). This also
            // backfills `author` for clients that don't set it.
            stroke.author = Some(user_id);
            if let Err(reason) = crate::services::live_room::validate_whiteboard_stroke(&stroke) {
                send_command_failed(sender, "whiteboard_stroke", reason).await;
                return;
            }
            // Record in the board history first so late joiners hydrate a
            // board consistent with what subscribers saw broadcast.
            if let Err(e) = broker.whiteboard_append(session_id, stroke.clone()).await {
                tracing::warn!(error = %e, command = "whiteboard_stroke", "history append failed");
            }
            let _ = broker
                .publish(session_id, BrokerEvent::WhiteboardStroke { stroke })
                .await;
        }
        ClientEnvelope::WhiteboardUndo { author } => {
            // A teacher may undo globally (no author) or undo their own most
            // recent stroke (author == self). A non-teacher may only undo their
            // OWN strokes, and only while drawing is open. Any author value
            // other than the sender's own id is rejected for non-teachers.
            let draw_open = broker.draw_open(session_id).await.unwrap_or(false);
            if !is_teacher {
                let permitted = draw_open && author.is_some_and(|a| a == user_id);
                if !permitted {
                    send_command_failed(
                        sender,
                        "whiteboard_undo",
                        &ApiError::Forbidden.user_facing(),
                    )
                    .await;
                    return;
                }
            }
            // Per-author undo when an author is supplied; otherwise the legacy
            // global undo (teacher-only path).
            let removed_id = match author {
                Some(a) => broker
                    .whiteboard_remove_last_by_author(session_id, a)
                    .await
                    .map(|opt| opt.map(|s| s.id)),
                None => broker.whiteboard_remove_last(session_id).await,
            };
            match removed_id {
                Ok(Some(stroke_id)) => {
                    let _ = broker
                        .publish(
                            session_id,
                            BrokerEvent::WhiteboardStrokeRemoved { stroke_id },
                        )
                        .await;
                }
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!(error = %e, command = "whiteboard_undo", "command failed");
                    send_command_failed(
                        sender,
                        "whiteboard_undo",
                        &ApiError::Internal(e.to_string()).user_facing(),
                    )
                    .await;
                }
            }
        }
        ClientEnvelope::WhiteboardErase { stroke_id } => {
            // Non-teachers may erase only while the draw-permission flag is
            // open (mirrors the stroke gate). Teachers always may.
            if !is_teacher && !broker.draw_open(session_id).await.unwrap_or(false) {
                send_command_failed(
                    sender,
                    "whiteboard_erase",
                    &ApiError::Forbidden.user_facing(),
                )
                .await;
                return;
            }
            // Validate the id shape before touching the broker (mirrors the
            // stroke-id rule in `validate_whiteboard_stroke`).
            if stroke_id.trim().is_empty() || stroke_id.len() > 96 {
                send_command_failed(sender, "whiteboard_erase", "stroke id is invalid").await;
                return;
            }
            match broker.whiteboard_remove_by_id(session_id, &stroke_id).await {
                // Only broadcast a removal when an element actually existed, so
                // a stale/duplicate erase from one client doesn't echo a no-op
                // to everyone.
                Ok(true) => {
                    let _ = broker
                        .publish(
                            session_id,
                            BrokerEvent::WhiteboardStrokeRemoved { stroke_id },
                        )
                        .await;
                }
                Ok(false) => {}
                Err(e) => {
                    tracing::warn!(error = %e, command = "whiteboard_erase", "command failed");
                    send_command_failed(
                        sender,
                        "whiteboard_erase",
                        &ApiError::Internal(e.to_string()).user_facing(),
                    )
                    .await;
                }
            }
        }
        ClientEnvelope::WhiteboardClear => {
            if !is_teacher {
                send_command_failed(
                    sender,
                    "whiteboard_clear",
                    &ApiError::Forbidden.user_facing(),
                )
                .await;
                return;
            }
            if let Err(e) = broker.whiteboard_clear(session_id).await {
                tracing::warn!(error = %e, command = "whiteboard_clear", "history clear failed");
            }
            let _ = broker
                .publish(session_id, BrokerEvent::WhiteboardClear)
                .await;
        }
        ClientEnvelope::WhiteboardCursor { x, y } => {
            // Permitted senders: the teacher always, or any participant while
            // drawing is open. Silently drop otherwise (no CommandFailed — a
            // cursor is best-effort and high-frequency).
            if !is_teacher && !broker.draw_open(session_id).await.unwrap_or(false) {
                return;
            }
            // Coordinates must be finite and on-board; clamp into 0..=1 so a
            // jittery client can't broadcast off-canvas dots.
            if !x.is_finite() || !y.is_finite() {
                return;
            }
            // Server-side throttle backstop (client throttles to ~60ms).
            if cursor_bucket.try_consume().is_err() {
                return;
            }
            let _ = broker
                .publish(
                    session_id,
                    BrokerEvent::WhiteboardCursor {
                        user_id,
                        display_name: display_name.to_string(),
                        x: x.clamp(0.0, 1.0),
                        y: y.clamp(0.0, 1.0),
                    },
                )
                .await;
        }
        ClientEnvelope::SetDrawPermission { open } => {
            if !is_teacher {
                send_command_failed(
                    sender,
                    "set_draw_permission",
                    &ApiError::Forbidden.user_facing(),
                )
                .await;
                return;
            }
            if let Err(e) = broker.set_draw_open(session_id, open).await {
                tracing::warn!(error = %e, command = "set_draw_permission", "flag set failed");
                send_command_failed(
                    sender,
                    "set_draw_permission",
                    &ApiError::Internal(e.to_string()).user_facing(),
                )
                .await;
                return;
            }
            let _ = broker
                .publish(session_id, BrokerEvent::DrawPermissionChanged { open })
                .await;
        }
        ClientEnvelope::Reaction { emoji } => {
            // Any participant may react. Only a fixed whitelist is allowed (so a
            // crafted client can't broadcast arbitrary text as a "reaction").
            const ALLOWED: [&str; 6] = [
                "\u{1f44d}",
                "\u{2764}\u{fe0f}",
                "\u{1f389}",
                "\u{1f44f}",
                "\u{1f602}",
                "\u{1f64c}",
            ];
            if !ALLOWED.contains(&emoji.as_str()) {
                return;
            }
            if reaction_bucket.try_consume().is_err() {
                let _ = sender
                    .send(ws_text(
                        serde_json::to_string(&BrokerEvent::RateLimited {
                            retry_after_ms: 500,
                        })
                        .expect("BrokerEvent::RateLimited must serialize"),
                    ))
                    .await;
                return;
            }
            let _ = broker
                .publish(
                    session_id,
                    BrokerEvent::Reaction {
                        user_id,
                        emoji,
                        display_name: display_name.to_string(),
                    },
                )
                .await;
        }
        ClientEnvelope::StartPoll { question, options } => {
            use crate::services::live_room::{
                POLL_MAX_OPTIONS, POLL_MIN_OPTIONS, POLL_OPTION_MAX_LEN, POLL_QUESTION_MAX_LEN,
            };
            if !is_teacher {
                send_command_failed(sender, "start_poll", &ApiError::Forbidden.user_facing()).await;
                return;
            }
            if poll_bucket.try_consume().is_err() {
                let _ = sender
                    .send(ws_text(
                        serde_json::to_string(&BrokerEvent::RateLimited {
                            retry_after_ms: 2000,
                        })
                        .expect("BrokerEvent::RateLimited must serialize"),
                    ))
                    .await;
                return;
            }
            // Validate the prompt: non-empty question, 2..=6 trimmed non-empty
            // options within the length caps. Reject (CommandFailed) rather than
            // silently drop so the teacher sees why nothing happened.
            let question = question.trim().to_string();
            if question.is_empty() || question.chars().count() > POLL_QUESTION_MAX_LEN {
                send_command_failed(
                    sender,
                    "start_poll",
                    &ApiError::BadRequest("poll question is empty or too long".into())
                        .user_facing(),
                )
                .await;
                return;
            }
            let options: Vec<String> = options.into_iter().map(|o| o.trim().to_string()).collect();
            if !(POLL_MIN_OPTIONS..=POLL_MAX_OPTIONS).contains(&options.len())
                || options
                    .iter()
                    .any(|o| o.is_empty() || o.chars().count() > POLL_OPTION_MAX_LEN)
            {
                send_command_failed(
                    sender,
                    "start_poll",
                    &ApiError::BadRequest(format!(
                        "a poll needs {POLL_MIN_OPTIONS}–{POLL_MAX_OPTIONS} non-empty options"
                    ))
                    .user_facing(),
                )
                .await;
                return;
            }
            let poll_id = Uuid::new_v4();
            if let Err(e) = broker.poll_start(session_id, poll_id, options.len()).await {
                tracing::warn!(error = %e, command = "start_poll", "poll start failed");
                send_command_failed(
                    sender,
                    "start_poll",
                    &ApiError::Internal(e.to_string()).user_facing(),
                )
                .await;
                return;
            }
            // Optional history persistence (tenant-scoped). Best-effort: a DB
            // hiccup must not break the live poll, which is broker-authoritative.
            if let Err(e) = persist_poll_started(
                pool, tenant_id, session_id, user_id, poll_id, &question, &options,
            )
            .await
            {
                tracing::warn!(error = %e, %session_id, %poll_id, "poll history insert failed");
            }
            let _ = broker
                .publish(
                    session_id,
                    BrokerEvent::PollStarted {
                        poll_id,
                        question,
                        options,
                    },
                )
                .await;
        }
        ClientEnvelope::PollVote {
            poll_id,
            option_index,
        } => {
            // Any participant may vote. Rate-limit guards rapid double-taps.
            if poll_bucket.try_consume().is_err() {
                let _ = sender
                    .send(ws_text(
                        serde_json::to_string(&BrokerEvent::RateLimited {
                            retry_after_ms: 2000,
                        })
                        .expect("BrokerEvent::RateLimited must serialize"),
                    ))
                    .await;
                return;
            }
            match broker
                .poll_vote(session_id, poll_id, user_id, option_index as usize)
                .await
            {
                Ok(Some(outcome)) => {
                    // Only broadcast a fresh tally when this vote actually
                    // counted (first vote by this user). A duplicate vote is a
                    // silent no-op — the voter already sees the latest results.
                    if outcome.counted {
                        if let Err(e) = persist_poll_counts(
                            pool,
                            tenant_id,
                            user_id,
                            session_id,
                            poll_id,
                            &outcome.counts,
                        )
                        .await
                        {
                            tracing::warn!(error = %e, %session_id, %poll_id, "poll count persist failed");
                        }
                        let _ = broker
                            .publish(
                                session_id,
                                BrokerEvent::PollResults {
                                    poll_id,
                                    counts: outcome.counts,
                                },
                            )
                            .await;
                    }
                }
                // No active poll / out-of-range option: silently ignore (the
                // poll likely ended between render and click).
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!(error = %e, command = "poll_vote", "poll vote failed");
                }
            }
        }
        ClientEnvelope::EndPoll { poll_id } => {
            if !is_teacher {
                send_command_failed(sender, "end_poll", &ApiError::Forbidden.user_facing()).await;
                return;
            }
            if poll_bucket.try_consume().is_err() {
                let _ = sender
                    .send(ws_text(
                        serde_json::to_string(&BrokerEvent::RateLimited {
                            retry_after_ms: 2000,
                        })
                        .expect("BrokerEvent::RateLimited must serialize"),
                    ))
                    .await;
                return;
            }
            match broker.poll_end(session_id, poll_id).await {
                Ok(Some(counts)) => {
                    if let Err(e) =
                        persist_poll_ended(pool, tenant_id, user_id, session_id, poll_id, &counts)
                            .await
                    {
                        tracing::warn!(error = %e, %session_id, %poll_id, "poll history end failed");
                    }
                    let _ = broker
                        .publish(session_id, BrokerEvent::PollEnded { poll_id, counts })
                        .await;
                }
                // No matching active poll — already ended; nothing to broadcast.
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!(error = %e, command = "end_poll", "poll end failed");
                    send_command_failed(
                        sender,
                        "end_poll",
                        &ApiError::Internal(e.to_string()).user_facing(),
                    )
                    .await;
                }
            }
        }
        ClientEnvelope::CreateBreakouts { names } => {
            if !is_teacher {
                send_command_failed(
                    sender,
                    "create_breakouts",
                    &ApiError::Forbidden.user_facing(),
                )
                .await;
                return;
            }
            if breakout_bucket.try_consume().is_err() {
                let _ = sender
                    .send(ws_text(
                        serde_json::to_string(&BrokerEvent::RateLimited {
                            retry_after_ms: 500,
                        })
                        .expect("BrokerEvent::RateLimited must serialize"),
                    ))
                    .await;
                return;
            }
            let rooms = match crate::handlers::breakout::build_rooms(&names) {
                Ok(r) => r,
                Err(e) => {
                    send_command_failed(sender, "create_breakouts", &e.message()).await;
                    return;
                }
            };
            // Fresh layout with empty rooms — the teacher assigns participants
            // next (manually or via auto-split). Replaces any prior layout.
            let new_state = crate::services::live_room::BreakoutState { open: false, rooms };
            // Not yet open: persist + broadcast the layout (BreakoutUpdated) but
            // don't fan assignments — students re-subscribe only on open.
            if let Err(e) = crate::handlers::breakout::commit_and_broadcast(
                broker,
                session_id,
                new_state,
                false,
                &crate::services::mediamtx::path_for_session(tenant_id, course_id, session_id),
                public_webrtc_url,
                &[],
            )
            .await
            {
                tracing::warn!(error = %e, command = "create_breakouts", "command failed");
                send_command_failed(sender, "create_breakouts", &e).await;
            }
        }
        ClientEnvelope::AutoSplitBreakouts { room_count } => {
            if !is_teacher {
                send_command_failed(
                    sender,
                    "auto_split_breakouts",
                    &ApiError::Forbidden.user_facing(),
                )
                .await;
                return;
            }
            if breakout_bucket.try_consume().is_err() {
                let _ = sender
                    .send(ws_text(
                        serde_json::to_string(&BrokerEvent::RateLimited {
                            retry_after_ms: 500,
                        })
                        .expect("BrokerEvent::RateLimited must serialize"),
                    ))
                    .await;
                return;
            }
            // Split the students currently present (exclude the teacher) into
            // round-robin rooms.
            let participants: Vec<Uuid> = match broker.presence_list(session_id).await {
                Ok(list) => list
                    .into_iter()
                    .filter(|p| p.role != "teacher")
                    .map(|p| p.user_id)
                    .collect(),
                Err(e) => {
                    tracing::warn!(error = %e, command = "auto_split_breakouts", "presence read failed");
                    send_command_failed(
                        sender,
                        "auto_split_breakouts",
                        &ApiError::Internal(e.to_string()).user_facing(),
                    )
                    .await;
                    return;
                }
            };
            let rooms =
                match crate::handlers::breakout::auto_split(&participants, room_count as usize) {
                    Ok(r) => r,
                    Err(e) => {
                        send_command_failed(sender, "auto_split_breakouts", &e.message()).await;
                        return;
                    }
                };
            let new_state = crate::services::live_room::BreakoutState { open: false, rooms };
            if let Err(e) = crate::handlers::breakout::commit_and_broadcast(
                broker,
                session_id,
                new_state,
                false,
                &crate::services::mediamtx::path_for_session(tenant_id, course_id, session_id),
                public_webrtc_url,
                &[],
            )
            .await
            {
                tracing::warn!(error = %e, command = "auto_split_breakouts", "command failed");
                send_command_failed(sender, "auto_split_breakouts", &e).await;
            }
        }
        ClientEnvelope::AssignBreakout {
            user_id: target_user_id,
            room_id,
        } => {
            if !is_teacher {
                send_command_failed(
                    sender,
                    "assign_breakout",
                    &ApiError::Forbidden.user_facing(),
                )
                .await;
                return;
            }
            if breakout_bucket.try_consume().is_err() {
                let _ = sender
                    .send(ws_text(
                        serde_json::to_string(&BrokerEvent::RateLimited {
                            retry_after_ms: 500,
                        })
                        .expect("BrokerEvent::RateLimited must serialize"),
                    ))
                    .await;
                return;
            }
            let mut state = match broker.breakout_get(session_id).await {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(error = %e, command = "assign_breakout", "breakout read failed");
                    send_command_failed(
                        sender,
                        "assign_breakout",
                        &ApiError::Internal(e.to_string()).user_facing(),
                    )
                    .await;
                    return;
                }
            };
            if state.rooms.is_empty() {
                send_command_failed(
                    sender,
                    "assign_breakout",
                    &crate::handlers::breakout::BreakoutError::NotInitialized.message(),
                )
                .await;
                return;
            }
            if let Err(e) = crate::handlers::breakout::assign(&mut state, target_user_id, room_id) {
                send_command_failed(sender, "assign_breakout", &e.message()).await;
                return;
            }
            // Re-broadcast the layout + fan assignments. Include the moved user
            // in `also_notify` so a move back to the main room still tells them
            // to re-subscribe to the main feed. `just_opened = false`: this is a
            // layout edit, not an open transition (BreakoutUpdated, not Opened).
            if let Err(e) = crate::handlers::breakout::commit_and_broadcast(
                broker,
                session_id,
                state,
                false,
                &crate::services::mediamtx::path_for_session(tenant_id, course_id, session_id),
                public_webrtc_url,
                &[target_user_id],
            )
            .await
            {
                tracing::warn!(error = %e, command = "assign_breakout", "command failed");
                send_command_failed(sender, "assign_breakout", &e).await;
            }
        }
        ClientEnvelope::OpenBreakouts => {
            if !is_teacher {
                send_command_failed(sender, "open_breakouts", &ApiError::Forbidden.user_facing())
                    .await;
                return;
            }
            if breakout_bucket.try_consume().is_err() {
                let _ = sender
                    .send(ws_text(
                        serde_json::to_string(&BrokerEvent::RateLimited {
                            retry_after_ms: 500,
                        })
                        .expect("BrokerEvent::RateLimited must serialize"),
                    ))
                    .await;
                return;
            }
            let mut state = match broker.breakout_get(session_id).await {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(error = %e, command = "open_breakouts", "breakout read failed");
                    send_command_failed(
                        sender,
                        "open_breakouts",
                        &ApiError::Internal(e.to_string()).user_facing(),
                    )
                    .await;
                    return;
                }
            };
            if state.rooms.is_empty() {
                send_command_failed(
                    sender,
                    "open_breakouts",
                    &crate::handlers::breakout::BreakoutError::NotInitialized.message(),
                )
                .await;
                return;
            }
            state.open = true;
            if let Err(e) = crate::handlers::breakout::commit_and_broadcast(
                broker,
                session_id,
                state,
                true,
                &crate::services::mediamtx::path_for_session(tenant_id, course_id, session_id),
                public_webrtc_url,
                &[],
            )
            .await
            {
                tracing::warn!(error = %e, command = "open_breakouts", "command failed");
                send_command_failed(sender, "open_breakouts", &e).await;
            }
        }
        ClientEnvelope::CloseBreakouts => {
            if !is_teacher {
                send_command_failed(
                    sender,
                    "close_breakouts",
                    &ApiError::Forbidden.user_facing(),
                )
                .await;
                return;
            }
            if breakout_bucket.try_consume().is_err() {
                let _ = sender
                    .send(ws_text(
                        serde_json::to_string(&BrokerEvent::RateLimited {
                            retry_after_ms: 500,
                        })
                        .expect("BrokerEvent::RateLimited must serialize"),
                    ))
                    .await;
                return;
            }
            let prev = match broker.breakout_get(session_id).await {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(error = %e, command = "close_breakouts", "breakout read failed");
                    send_command_failed(
                        sender,
                        "close_breakouts",
                        &ApiError::Internal(e.to_string()).user_facing(),
                    )
                    .await;
                    return;
                }
            };
            if let Err(e) =
                crate::handlers::breakout::close_and_broadcast(broker, session_id, &prev).await
            {
                tracing::warn!(error = %e, command = "close_breakouts", "command failed");
                send_command_failed(sender, "close_breakouts", &e).await;
            }
        }
        ClientEnvelope::Kick {
            user_id: target_user_id,
        } => {
            if !is_teacher {
                send_command_failed(sender, "kick", &ApiError::Forbidden.user_facing()).await;
                return;
            }
            if target_user_id == user_id {
                send_command_failed(
                    sender,
                    "kick",
                    &ApiError::BadRequest("cannot kick self".into()).user_facing(),
                )
                .await;
                return;
            }
            let mut tx = match pool.begin().await {
                Ok(t) => t,
                Err(e) => {
                    tracing::warn!(error = %e, command = "kick", "command failed");
                    send_command_failed(
                        sender,
                        "kick",
                        &ApiError::Internal(e.to_string()).user_facing(),
                    )
                    .await;
                    return;
                }
            };
            if let Err(e) = db::set_request_guc(&mut tx, user_id, Some(tenant_id)).await {
                tracing::warn!(error = %e, command = "kick", "set_request_guc failed");
                send_command_failed(
                    sender,
                    "kick",
                    &ApiError::Internal(e.to_string()).user_facing(),
                )
                .await;
                return;
            }
            if let Err(e) =
                db::live_room::insert_kick(&mut tx, tenant_id, session_id, target_user_id, user_id)
                    .await
            {
                tracing::warn!(error = %e, command = "kick", "command failed");
                send_command_failed(
                    sender,
                    "kick",
                    &ApiError::Internal(e.to_string()).user_facing(),
                )
                .await;
                return;
            }
            if let Err(e) = tx.commit().await {
                tracing::warn!(error = %e, command = "kick", "command failed");
                send_command_failed(
                    sender,
                    "kick",
                    &ApiError::Internal(e.to_string()).user_facing(),
                )
                .await;
                return;
            }
            let _ = broker
                .kick_set(
                    session_id,
                    target_user_id,
                    std::time::Duration::from_secs(86400),
                )
                .await;
            let _ = broker
                .publish(
                    session_id,
                    BrokerEvent::Kicked {
                        user_id: target_user_id,
                    },
                )
                .await;
        }
    }
}

/// Derive display name from email (part before '@').
fn display_name_from_email(email: &str) -> String {
    email.split('@').next().unwrap_or(email).to_string()
}

async fn socket(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(session_id): Path<Uuid>,
    ws: WebSocketUpgrade,
) -> Result<axum::response::Response, ApiError> {
    socket_gate(&s.pool, &ctx, session_id).await?;

    let session = db::live_sessions::load_for_join(&s.pool, session_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::NotFound)?;
    let tenant_id = ctx
        .tenant_id
        .ok_or_else(|| ApiError::BadRequest("no tenant".into()))?;
    let is_teacher = db::courses::caller_can_admin_course(
        &s.pool,
        session.course_id,
        ctx.user_id,
        ctx.tenant_id,
        is_org_admin(&ctx),
    )
    .await
    .unwrap_or(false);
    let display_name = display_name_from_email(&ctx.email);

    let pool = s.pool.clone();
    let broker = Arc::clone(&s.live_room);
    let user_id = ctx.user_id;
    let course_id = session.course_id;
    let public_webrtc_url = s.mediamtx_public_webrtc_url.clone();

    let upgrade = ws.on_upgrade(move |socket: WebSocket| async move {
        run_socket(
            socket,
            pool,
            broker,
            user_id,
            tenant_id,
            session_id,
            course_id,
            is_teacher,
            display_name,
            public_webrtc_url,
        )
        .await;
    });
    Ok(upgrade.into_response())
}

async fn socket_t(
    State(s): State<LiveRoomTestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(session_id): Path<Uuid>,
    req: axum::extract::Request,
) -> Result<axum::response::Response, ApiError> {
    socket_gate(&s.pool, &ctx, session_id).await?;

    let session = db::live_sessions::load_for_join(&s.pool, session_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::NotFound)?;
    let tenant_id = ctx
        .tenant_id
        .ok_or_else(|| ApiError::BadRequest("no tenant".into()))?;
    let is_teacher = db::courses::caller_can_admin_course(
        &s.pool,
        session.course_id,
        ctx.user_id,
        ctx.tenant_id,
        is_org_admin(&ctx),
    )
    .await
    .unwrap_or(false);
    let display_name = display_name_from_email(&ctx.email);

    let pool = s.pool.clone();
    let broker = Arc::clone(&s.broker);
    let user_id = ctx.user_id;
    let course_id = session.course_id;
    let public_webrtc_url = s.public_webrtc_url.clone();

    // Gate passed — now attempt WS upgrade. Extract WebSocketUpgrade from the
    // raw request parts. If missing upgrade headers the extractor returns a
    // rejection response (400), which we propagate as-is.
    use axum::extract::FromRequestParts;
    let (mut parts, _body) = req.into_parts();
    match WebSocketUpgrade::from_request_parts(&mut parts, &s).await {
        Ok(ws) => {
            let upgrade = ws.on_upgrade(move |socket: WebSocket| async move {
                run_socket(
                    socket,
                    pool,
                    broker,
                    user_id,
                    tenant_id,
                    session_id,
                    course_id,
                    is_teacher,
                    display_name,
                    public_webrtc_url,
                )
                .await;
            });
            Ok(upgrade.into_response())
        }
        Err(e) => {
            // Not a WS upgrade request — return 400 as axum normally would.
            Ok(e.into_response())
        }
    }
}

// ============================================================================
// Phase 1b-delta: Recording handlers
// ============================================================================

const RECORDING_PLAYBACK_TTL: std::time::Duration = std::time::Duration::from_secs(15 * 60);

#[derive(serde::Serialize)]
pub struct RecordingDto {
    pub session_id: Uuid,
    pub processing_status: String,
    pub processing_error: Option<String>,
    pub duration_seconds: Option<i32>,
    pub started_at: Option<chrono::DateTime<chrono::Utc>>,
    pub playback_url: Option<String>,
    pub course_title: String,
    pub instructor_user_id: Option<Uuid>,
    /// `Some(false)` when the recording is audio-only. See the
    /// `recordings.has_video` migration; `None` means it was never probed, and
    /// the player then behaves exactly as it did before this field existed.
    pub has_video: Option<bool>,
}

async fn recording_inner(
    pool: &PgPool,
    storage: &dyn crate::storage::S3Client,
    ctx: &RequestContext,
    session_id: Uuid,
) -> Result<Json<RecordingDto>, ApiError> {
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
    let allowed = db::courses::caller_can_read_course(
        pool,
        session.course_id,
        ctx.user_id,
        ctx.tenant_id,
        is_org_admin(ctx),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !allowed {
        return Err(ApiError::Forbidden);
    }

    let mut tx = db::begin_with_context(pool, ctx.user_id, ctx.tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let row = db::recordings::fetch_by_session(&mut *tx, session_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::NotFound)?;

    let playback_url = if row.processing_status == "available" {
        if let Some(file_asset_id) = row.file_asset_id {
            let asset = db::file_assets::fetch(&mut *tx, file_asset_id)
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?
                .ok_or_else(|| ApiError::Internal("recording file_asset missing".into()))?;
            let url = storage
                .presigned_get_url(&asset.object_key, RECORDING_PLAYBACK_TTL)
                .await
                .map_err(|e| ApiError::Internal(format!("presign failed: {e}")))?;
            Some(url)
        } else {
            None
        }
    } else {
        None
    };

    let duration = if row.processing_status == "available" {
        Some(row.duration_seconds)
    } else {
        None
    };
    let started = if row.processing_status == "available" {
        Some(row.started_at)
    } else {
        None
    };
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(RecordingDto {
        session_id,
        processing_status: row.processing_status,
        processing_error: row.processing_error,
        duration_seconds: duration,
        started_at: started,
        playback_url,
        course_title: session.course_title,
        instructor_user_id: session.primary_teacher_id,
        has_video: row.has_video,
    }))
}

async fn recording(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(session_id): Path<Uuid>,
) -> Result<Json<RecordingDto>, ApiError> {
    recording_inner(&s.pool, s.storage.as_ref(), &ctx, session_id).await
}

async fn recording_t(
    State(s): State<LiveRoomTestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(session_id): Path<Uuid>,
) -> Result<Json<RecordingDto>, ApiError> {
    recording_inner(&s.pool, s.s3_client.as_ref(), &ctx, session_id).await
}

// ============================================================================
// Phase 2: GET /v1/sessions/{id}/attendance — durable attendance report.
// Staff-only (platform admin, org admin, course owner, or assigned teacher/TA).
// ============================================================================

#[derive(serde::Serialize)]
pub struct AttendanceDto {
    pub user_id: Uuid,
    pub display_name: Option<String>,
    pub email: Option<String>,
    pub first_joined_at: chrono::DateTime<chrono::Utc>,
    pub last_left_at: Option<chrono::DateTime<chrono::Utc>>,
    pub total_seconds: i32,
    pub reconnect_count: i32,
}

async fn attendance_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    session_id: Uuid,
) -> Result<Json<Vec<AttendanceDto>>, ApiError> {
    // Resolve the session (and its course) under the caller's tenant context.
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

    // Staff-only: platform admins bypass; otherwise the caller must be able to
    // staff the course (org_admin / owner / assigned teacher|ta).
    if !ctx.can_manage_organization() {
        let allowed = db::courses::caller_can_staff_course(
            pool,
            session.course_id,
            ctx.user_id,
            ctx.tenant_id,
            is_org_admin(ctx),
        )
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
        if !allowed {
            return Err(ApiError::Forbidden);
        }
    }

    let rows = db::attendance::list_for_session(pool, tenant_id, session_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let items = rows
        .into_iter()
        .map(|r| AttendanceDto {
            user_id: r.user_id,
            display_name: r.display_name,
            email: r.email,
            first_joined_at: r.first_joined_at,
            last_left_at: r.last_left_at,
            total_seconds: r.total_seconds,
            reconnect_count: r.reconnect_count,
        })
        .collect();
    Ok(Json(items))
}

async fn attendance(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(session_id): Path<Uuid>,
) -> Result<Json<Vec<AttendanceDto>>, ApiError> {
    attendance_inner(&s.pool, &ctx, session_id).await
}

async fn attendance_t(
    State(s): State<LiveRoomTestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(session_id): Path<Uuid>,
) -> Result<Json<Vec<AttendanceDto>>, ApiError> {
    attendance_inner(&s.pool, &ctx, session_id).await
}

// ============================================================================
// Phase 1b-δ Task 12: GET /v1/sessions/{id}/recording/chat — windowed chat replay
// ============================================================================

#[derive(serde::Deserialize, Default)]
pub struct RecordingChatQuery {
    pub from_seconds: Option<f64>,
    pub to_seconds: Option<f64>,
}

#[derive(serde::Serialize)]
pub struct RecordingChatMessageDto {
    pub id: Uuid,
    pub sender_user_id: Uuid,
    pub sender_display_name: String,
    pub body: String,
    pub video_offset_seconds: f64,
    pub deleted: bool,
}

#[derive(serde::Serialize)]
pub struct RecordingChatWindowResponse {
    pub messages: Vec<RecordingChatMessageDto>,
}

async fn recording_chat_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    session_id: Uuid,
    q: RecordingChatQuery,
) -> Result<Json<RecordingChatWindowResponse>, ApiError> {
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
    let allowed = db::courses::caller_can_read_course(
        pool,
        session.course_id,
        ctx.user_id,
        ctx.tenant_id,
        is_org_admin(ctx),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !allowed {
        return Err(ApiError::Forbidden);
    }

    let mut prefetch_tx = db::begin_with_context(pool, ctx.user_id, ctx.tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let recording = db::recordings::fetch_by_session(&mut *prefetch_tx, session_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::NotFound)?;
    prefetch_tx
        .commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let from_secs = q.from_seconds.unwrap_or(0.0).max(0.0);
    let to_secs = q.to_seconds.unwrap_or(f64::MAX);
    // 4_102_444_800.0 = 2100-01-01 UTC in seconds; chosen well below i64::MAX
    // so chrono::Duration::seconds(MAX_SAFE_SECS as i64) can't overflow when
    // we later add to a started_at timestamp.
    const MAX_SAFE_SECS: f64 = 4_102_444_800.0;
    let from_ts =
        recording.started_at + chrono::Duration::milliseconds((from_secs * 1000.0) as i64);
    let to_ts = if !to_secs.is_finite() || to_secs > MAX_SAFE_SECS {
        chrono::DateTime::<chrono::Utc>::MAX_UTC
    } else {
        recording.started_at + chrono::Duration::milliseconds((to_secs * 1000.0) as i64)
    };

    let is_admin = is_org_admin(ctx);
    let is_teacher = db::courses::caller_can_admin_course(
        pool,
        session.course_id,
        ctx.user_id,
        ctx.tenant_id,
        is_admin,
    )
    .await
    .unwrap_or(false);

    let rows = {
        let mut tx = db::begin_with_context(pool, ctx.user_id, ctx.tenant_id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        let r = sqlx::query_as::<_, db::live_room::ChatMessageRow>(
            "SELECT id, tenant_id, session_id, sender_user_id, body, created_at,
                    deleted_at, deleted_by_user_id
               FROM live_room_messages
              WHERE session_id = $1
                AND created_at >= $2
                AND created_at <= $3
              ORDER BY created_at ASC",
        )
        .bind(session_id)
        .bind(from_ts)
        .bind(to_ts)
        .fetch_all(&mut *tx)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
        tx.commit()
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        r
    };

    use std::collections::HashMap;
    let unique_ids: Vec<Uuid> = {
        let mut s: Vec<Uuid> = rows.iter().map(|r| r.sender_user_id).collect();
        s.sort_unstable();
        s.dedup();
        s
    };
    let names: HashMap<Uuid, String> = if unique_ids.is_empty() {
        HashMap::new()
    } else {
        let pairs: Vec<(Uuid, String)> = sqlx::query_as(
            "SELECT id, COALESCE(SPLIT_PART(email, '@', 1), '') FROM users WHERE id = ANY($1)",
        )
        .bind(&unique_ids)
        .fetch_all(pool)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
        pairs.into_iter().collect()
    };

    let messages: Vec<RecordingChatMessageDto> = rows
        .iter()
        .map(|r| {
            let display_name = names.get(&r.sender_user_id).cloned().unwrap_or_default();
            let body = if r.deleted_at.is_some() && !is_teacher {
                "[deleted]".into()
            } else {
                r.body.clone()
            };
            RecordingChatMessageDto {
                id: r.id,
                sender_user_id: r.sender_user_id,
                sender_display_name: display_name,
                body,
                video_offset_seconds: crate::services::recording::video_offset_seconds(
                    r.created_at,
                    recording.started_at,
                ),
                deleted: r.deleted_at.is_some(),
            }
        })
        .collect();

    Ok(Json(RecordingChatWindowResponse { messages }))
}

async fn recording_chat(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(session_id): Path<Uuid>,
    axum::extract::Query(q): axum::extract::Query<RecordingChatQuery>,
) -> Result<Json<RecordingChatWindowResponse>, ApiError> {
    recording_chat_inner(&s.pool, &ctx, session_id, q).await
}

async fn recording_chat_t(
    State(s): State<LiveRoomTestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(session_id): Path<Uuid>,
    axum::extract::Query(q): axum::extract::Query<RecordingChatQuery>,
) -> Result<Json<RecordingChatWindowResponse>, ApiError> {
    recording_chat_inner(&s.pool, &ctx, session_id, q).await
}

// ============================================================================
// Phase 1b-δ Task 13: POST /v1/sessions/{id}/recording/retry — teacher resets failed
// ============================================================================

async fn recording_retry_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    session_id: Uuid,
) -> Result<Json<RecordingDto>, ApiError> {
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
    require_admin_for_session_course(pool, ctx, session.course_id).await?;

    let mut prefetch_tx = db::begin_with_context(pool, ctx.user_id, ctx.tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let row = db::recordings::fetch_by_session(&mut *prefetch_tx, session_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::NotFound)?;
    prefetch_tx
        .commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let updated = if row.processing_status == "failed" {
        let mut tx = pool
            .begin()
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        db::set_request_guc(&mut tx, ctx.user_id, Some(tenant_id))
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        if db::usage_limits::recording_storage_capacity_in_tx(&mut tx, tenant_id, 1)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?
            .is_limit_reached()
        {
            return Err(ApiError::RecordingStorageLimitReached);
        }
        let r = db::recordings::set_status(&mut tx, row.id, "pending", None)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        tx.commit()
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        r.unwrap_or(row)
    } else {
        row
    };

    Ok(Json(RecordingDto {
        session_id,
        processing_status: updated.processing_status,
        processing_error: updated.processing_error,
        duration_seconds: None,
        started_at: None,
        playback_url: None,
        course_title: session.course_title,
        instructor_user_id: session.primary_teacher_id,
        // Reprocessing: the old answer no longer describes the file being
        // rebuilt, and the new one is not known until the remux finishes.
        has_video: None,
    }))
}

async fn recording_retry(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(session_id): Path<Uuid>,
) -> Result<Json<RecordingDto>, ApiError> {
    recording_retry_inner(&s.pool, &ctx, session_id).await
}

async fn recording_retry_t(
    State(s): State<LiveRoomTestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(session_id): Path<Uuid>,
) -> Result<Json<RecordingDto>, ApiError> {
    recording_retry_inner(&s.pool, &ctx, session_id).await
}

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
        assert!(
            err.detail.contains("Media server API check failed"),
            "{err:?}"
        );
    }

    #[test]
    fn stream_error_details_do_not_expose_provider_internals() {
        let raw = "SECRET_INTERNAL_PATH /v3/paths/get rtsp://mediamtx.local/live";

        let media = media_server_health(Err(MediaMtxError::Api(raw.into())));
        assert_eq!(media.status, LiveHealthStatus::Error);
        assert!(media.detail.contains("Media server API check failed"));
        assert!(!media.detail.contains("SECRET_INTERNAL_PATH"));
        assert!(!media.detail.contains("/v3/paths/get"));
        assert!(!media.detail.contains("rtsp://"));

        let main = main_stream_health("live", Some(Err(MediaMtxError::Api(raw.into()))));
        assert_eq!(main.status, LiveHealthStatus::Unknown);
        assert!(!main.detail.contains("SECRET_INTERNAL_PATH"));
        assert!(!main.detail.contains("/v3/paths/get"));
        assert!(!main.detail.contains("rtsp://"));

        let screen = screen_stream_health(Some(Err(MediaMtxError::Api(raw.into()))));
        assert_eq!(screen.status, LiveHealthStatus::Unknown);
        assert!(!screen.detail.contains("SECRET_INTERNAL_PATH"));
        assert!(!screen.detail.contains("/v3/paths/get"));
        assert!(!screen.detail.contains("rtsp://"));
    }

    #[test]
    fn media_server_timeout_health_is_sanitized() {
        let media = media_server_timeout_health();
        assert_eq!(media.status, LiveHealthStatus::Error);
        assert_eq!(media.label, "Media server");
        assert_eq!(media.detail, "Media server API check timed out");
        assert!(!media.detail.contains("SECRET_INTERNAL_PATH"));
        assert!(!media.detail.contains("s3://"));
        assert!(!media.detail.contains("/v3/paths/get"));
    }

    #[test]
    fn path_status_timeout_health_is_sanitized() {
        let main = main_stream_health("live", path_status_timeout());
        assert_eq!(main.status, LiveHealthStatus::Unknown);
        assert_eq!(main.detail, "Could not check teacher stream path");

        let screen = screen_stream_health(path_status_timeout());
        assert_eq!(screen.status, LiveHealthStatus::Unknown);
        assert_eq!(screen.detail, "Could not check screen share path");
    }

    /// Mirrors the `end_if_publisher_gone` predicate in SQL:
    ///   publisher_last_seen_at + PUBLISHER_GONE_GRACE_MINUTES < now()
    ///
    /// Note what is absent: `actual_started_at`, `duration_minutes` and any
    /// notion of elapsed class time. The sweep cannot see how long a class has
    /// been running, which is what makes an unlimited duration structural
    /// rather than a large number.
    fn sweep_would_end(minutes_publisher_absent: i64) -> bool {
        minutes_publisher_absent > crate::db::live_sessions::PUBLISHER_GONE_GRACE_MINUTES
    }

    #[test]
    fn a_healthy_class_is_never_ended_no_matter_how_long_it_runs() {
        // The original failure: `start-now` books 60 minutes, so the wall-clock
        // sweep ended a running class at 60 + 30 = 90 while the teacher was
        // still teaching. Raising that deadline to 180 only moved the wall to
        // 210. Both are gone: while the publisher is present the absence clock
        // is reset on every tick, so the sweep has nothing to act on.
        for running in [60, 90, 210, 8 * 60, 24 * 60, 7 * 24 * 60] {
            // A publisher seen on this tick has been absent for 0 minutes.
            assert!(
                !sweep_would_end(0),
                "a publishing class must survive {running} minutes"
            );
            assert!(
                join_window_open(
                    "live",
                    chrono::Utc::now(),
                    chrono::Utc::now() - chrono::Duration::minutes(running),
                    60,
                    None,
                ),
                "a student must still be able to join at {running} minutes"
            );
        }
    }

    #[test]
    fn a_running_class_has_no_upper_join_bound() {
        // Regression: measured at 209 minutes the room was still `live` and
        // publishing, but /join returned 400 because the join window carried
        // its own deadline. A running class now has no upper bound at all, so
        // the two can no longer disagree.
        let starts = chrono::Utc::now();
        for running in [30, 210, 12 * 60, 30 * 24 * 60] {
            assert!(
                join_window_open(
                    "live",
                    starts + chrono::Duration::minutes(running),
                    starts,
                    60,
                    None,
                ),
                "a live class must stay joinable at {running} minutes"
            );
        }
    }

    #[test]
    fn the_booked_duration_never_bounds_a_running_class() {
        // `duration_minutes` is a scheduling hint. It must not shorten entry,
        // whatever it is set to -- including the 60 that `start-now` books.
        let starts = chrono::Utc::now();
        for booked in [1, 15, 60, 300] {
            assert!(
                join_window_open(
                    "live",
                    starts + chrono::Duration::minutes(booked * 10 + 600),
                    starts,
                    booked,
                    None,
                ),
                "a class booked for {booked} minutes must not be cut off"
            );
        }
    }

    #[test]
    fn an_abandoned_room_is_still_reclaimed_once_the_publisher_is_gone() {
        // Removing the duration cap must not mean rooms leak forever. The
        // bound is now real disconnection, not elapsed time.
        let grace = crate::db::live_sessions::PUBLISHER_GONE_GRACE_MINUTES;
        assert!(
            !sweep_would_end(grace),
            "must survive to the end of the confirmation window"
        );
        assert!(
            sweep_would_end(grace + 1),
            "an abandoned room must be reclaimed once the publisher is confirmed gone"
        );
    }

    #[test]
    fn a_brief_publisher_blip_does_not_end_the_class() {
        // A dropped Wi-Fi packet, a laptop sleeping briefly, or a re-publish
        // when the teacher switches camera must all survive. Anything shorter
        // than the confirmation window is not a disconnection.
        for blip in [0, 1, 5, crate::db::live_sessions::PUBLISHER_GONE_GRACE_MINUTES - 1] {
            assert!(
                !sweep_would_end(blip),
                "a {blip}-minute interruption must not end the class"
            );
        }
    }

    #[test]
    fn end_class_still_terminates_immediately() {
        // Explicit End Class sets actual_ended_at; the join window then keys off
        // that instant, not the floor, so the room closes right away (plus the
        // ordinary 15-minute rejoin grace) rather than lingering for 180.
        let started = chrono::Utc::now() - chrono::Duration::minutes(20);
        let ended_now = chrono::Utc::now();
        assert!(
            !join_window_open(
                "live",
                ended_now + chrono::Duration::minutes(JOIN_WINDOW_AFTER_END + 1),
                started,
                60,
                Some(ended_now)
            ),
            "after End Class the window must close on actual_ended_at"
        );
    }

    #[test]
    fn media_credentials_are_reissued_rather_than_sized_to_a_max_session() {
        // This test used to assert `VIEWER_JWT_TTL >= the maximum live
        // session`. There is no maximum live session any more, so that
        // comparison has no right-hand side -- and the invariant it encoded is
        // not merely unavailable, it is unachievable: NO fixed token lifetime
        // can cover an unbounded class.
        //
        // What must hold instead is that these are credentials for an
        // OPERATION, not a lease on the class:
        //
        //   * the publish nonce is re-minted by `/go-live`, which an
        //     already-`live` session may call at any age (see `go_live_inner`),
        //     so a teacher who reconnects at hour nine gets a fresh one;
        //   * the viewer JWT is re-minted by `/join`, which a running class
        //     answers with no upper bound (see `join_window_open`).
        //
        // Both are therefore bounded by the time between reconnects, not by
        // class length, and are kept comfortably longer than any single
        // connection so a healthy stream is never churned to refresh them.
        assert!(
            VIEWER_JWT_TTL.as_secs() >= 60 * 60,
            "viewer JWT must comfortably outlast a single connection"
        );
        assert!(
            PUBLISH_NONCE_TTL.as_secs() >= 60 * 60,
            "publish nonce must comfortably outlast a single connection"
        );
    }

    #[test]
    fn ended_sessions_stay_joinable_so_recordings_stay_viewable() {
        // The bug: a class that ended long ago answered 400 on /join, and since
        // /join is what tells the client which branch to render, the replay
        // page never mounted even though the recording was intact.
        let starts = chrono::Utc::now() - chrono::Duration::days(30);
        let ended = starts + chrono::Duration::minutes(60);
        assert!(join_window_open(
            "ended",
            chrono::Utc::now(),
            starts,
            60,
            Some(ended)
        ));
        // Still true a year later -- retention decides when a recording stops
        // being watchable, not the join clock.
        assert!(join_window_open(
            "ended",
            ended + chrono::Duration::days(365),
            starts,
            60,
            Some(ended)
        ));
    }

    #[test]
    fn live_room_entry_is_still_time_boxed() {
        // The exemption must not turn into a blanket bypass: everything that is
        // not a replay keeps the original window.
        let starts = chrono::Utc::now();
        let ended = starts + chrono::Duration::minutes(60);

        // Too early for the lobby.
        assert!(!join_window_open(
            "scheduled",
            starts - chrono::Duration::minutes(JOIN_WINDOW_BEFORE + 1),
            starts,
            60,
            None
        ));
        // Just inside the pre-roll.
        assert!(join_window_open(
            "scheduled",
            starts - chrono::Duration::minutes(JOIN_WINDOW_BEFORE - 1),
            starts,
            60,
            None
        ));
        // Live, mid-class.
        assert!(join_window_open(
            "live",
            starts + chrono::Duration::minutes(30),
            starts,
            60,
            None
        ));
        // A `live` row is joinable for as long as it says `live`, however old
        // it is. This is the deliberate consequence of removing the duration
        // cap: entry is gated by SESSION STATE, never by a clock, so there is
        // no age at which a still-running class starts refusing students.
        //
        // What bounds a stale row is therefore the sweep, not this function.
        // A room whose publisher vanished flips to `ended` within
        // PUBLISHER_GONE_GRACE_MINUTES plus one sweep tick, and the `ended`
        // branch above then applies. That is why the sweep tick must stay
        // short relative to the absence window -- it is the only thing
        // closing this door now.
        for age in [
            crate::db::live_sessions::PUBLISHER_GONE_GRACE_MINUTES + 1,
            210,
            24 * 60,
        ] {
            assert!(
                join_window_open("live", starts + chrono::Duration::minutes(age), starts, 60, None),
                "a row still marked live must stay joinable at {age} minutes"
            );
        }
        // A cancelled class never happened and has nothing to replay.
        assert!(!join_window_open(
            "cancelled",
            ended + chrono::Duration::days(1),
            starts,
            60,
            None
        ));
    }

    #[test]
    fn join_window_uses_actual_end_when_a_class_runs_over() {
        // A class that ran past its scheduled duration must stay joinable for
        // the grace period after it ACTUALLY ended, not after it was booked to.
        let booked = 300;
        let starts = chrono::Utc::now() - chrono::Duration::minutes(booked + 60);
        let actual_end = starts + chrono::Duration::minutes(booked - 10);
        let just_after = actual_end + chrono::Duration::minutes(JOIN_WINDOW_AFTER_END - 1);
        assert!(join_window_open(
            "live",
            just_after,
            starts,
            booked,
            Some(actual_end)
        ));
        // ...and the grace does expire, keyed off the REAL end instant.
        let well_past = actual_end + chrono::Duration::minutes(JOIN_WINDOW_AFTER_END + 1);
        assert!(!join_window_open(
            "live",
            well_past,
            starts,
            booked,
            Some(actual_end)
        ));
        // Ending the class is what closes entry. Running long does not: the
        // same instant, with no actual end recorded, is still open.
        assert!(join_window_open("live", well_past, starts, booked, None));
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
    fn summarize_processing_error_returns_sanitized_generic_detail() {
        let error =
            summarize_processing_error(Some("ffmpeg failed writing s3://SECRET_INTERNAL_PATH/log"));
        let error = error.expect("non-empty raw errors should produce a generic summary");

        assert!(!error.contains("ffmpeg"));
        assert!(!error.contains("s3://"));
        assert!(!error.contains("SECRET_INTERNAL_PATH"));
        assert!(error.contains("Recording processor reported"));
        assert_eq!(summarize_processing_error(Some("   ")), None);
        assert_eq!(summarize_processing_error(None), None);
    }

    #[test]
    fn recording_health_maps_processing_states_and_retry() {
        let disabled = recording_health_from_parts(false, "live", None, None);
        assert_eq!(disabled.status, LiveHealthStatus::NotApplicable);
        assert!(!disabled.retry_eligible);

        let no_row_live = recording_health_from_parts(true, "live", None, None);
        assert_eq!(no_row_live.status, LiveHealthStatus::Warning);
        assert!(!no_row_live.retry_eligible);

        let no_row_scheduled = recording_health_from_parts(true, "scheduled", None, None);
        assert_eq!(no_row_scheduled.status, LiveHealthStatus::NotApplicable);
        assert!(!no_row_scheduled.retry_eligible);

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
            Some("ffmpeg failed writing s3://SECRET_INTERNAL_PATH/provider-log.txt"),
        );
        assert_eq!(failed.status, LiveHealthStatus::Error);
        assert!(failed.retry_eligible);
        assert_eq!(failed.processing_status.as_deref(), Some("failed"));
        let error = failed.processing_error.unwrap();
        assert!(!error.contains("ffmpeg"));
        assert!(!error.contains("s3://"));
        assert!(!error.contains("SECRET_INTERNAL_PATH"));
        assert!(error.contains("Recording processor reported a failure"));
    }
}
