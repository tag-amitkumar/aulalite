// crates/backend/src/db/live_sessions.rs
use serde::Serialize;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct LiveSessionRow {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub course_id: Uuid,
    pub series_id: Uuid,
    pub occurrence_index: i32,
    pub title: String,
    pub status: String,
    pub starts_at: chrono::DateTime<chrono::Utc>,
    pub duration_minutes: i32,
    pub actual_started_at: Option<chrono::DateTime<chrono::Utc>>,
    pub actual_ended_at: Option<chrono::DateTime<chrono::Utc>>,
    pub primary_teacher_id: Uuid,
    pub mode: String,
    pub recording_enabled: bool,
    pub main_path: Option<String>,
    pub hls_fallback_enabled: bool,
    pub diverged: bool,
    pub screen_path: Option<String>,
    pub transport_mode: String,
    pub publish_nonce: Option<String>,
    pub publish_nonce_expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct SeriesRow {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub course_id: Uuid,
    pub title: String,
    pub starts_at: chrono::DateTime<chrono::Utc>,
    pub duration_minutes: i32,
    pub frequency: String,
    pub byweekday: Option<Vec<String>>,
    pub end_kind: String,
    pub occurrence_count: Option<i32>,
    pub end_until: Option<chrono::DateTime<chrono::Utc>>,
    pub primary_teacher_id: Uuid,
    pub recording_enabled: Option<bool>,
    pub transport_mode: String,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct OccurrenceRow {
    pub id: Uuid,
    pub series_id: Uuid,
    pub course_id: Uuid,
    pub occurrence_index: i32,
    pub title: String,
    pub status: String,
    pub starts_at: chrono::DateTime<chrono::Utc>,
    pub duration_minutes: i32,
    pub primary_teacher_id: Uuid,
    pub recording_enabled: bool,
    pub diverged: bool,
}

pub async fn insert_series(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    course_id: Uuid,
    title: &str,
    starts_at: chrono::DateTime<chrono::Utc>,
    duration_minutes: i32,
    frequency: &str,
    byweekday: Option<&[String]>,
    end_kind: &str,
    occurrence_count: Option<i32>,
    end_until: Option<chrono::DateTime<chrono::Utc>>,
    primary_teacher_id: Uuid,
    recording_enabled: Option<bool>,
    transport_mode: &str,
) -> sqlx::Result<Uuid> {
    sqlx::query_scalar(
        "INSERT INTO live_session_series
            (tenant_id, course_id, title, starts_at, duration_minutes,
             frequency, byweekday, end_kind, occurrence_count, end_until,
             primary_teacher_id, recording_enabled, transport_mode)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13) RETURNING id",
    )
    .bind(tenant_id)
    .bind(course_id)
    .bind(title)
    .bind(starts_at)
    .bind(duration_minutes)
    .bind(frequency)
    .bind(byweekday)
    .bind(end_kind)
    .bind(occurrence_count)
    .bind(end_until)
    .bind(primary_teacher_id)
    .bind(recording_enabled)
    .bind(transport_mode)
    .fetch_one(&mut **tx)
    .await
}

pub async fn insert_occurrence(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    course_id: Uuid,
    series_id: Uuid,
    occurrence_index: i32,
    title: &str,
    starts_at: chrono::DateTime<chrono::Utc>,
    duration_minutes: i32,
    primary_teacher_id: Uuid,
    recording_enabled: bool,
    transport_mode: &str,
) -> sqlx::Result<Uuid> {
    sqlx::query_scalar(
        "INSERT INTO live_sessions
            (tenant_id, course_id, series_id, occurrence_index, title,
             starts_at, duration_minutes, primary_teacher_id, recording_enabled,
             transport_mode)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10) RETURNING id",
    )
    .bind(tenant_id)
    .bind(course_id)
    .bind(series_id)
    .bind(occurrence_index)
    .bind(title)
    .bind(starts_at)
    .bind(duration_minutes)
    .bind(primary_teacher_id)
    .bind(recording_enabled)
    .bind(transport_mode)
    .fetch_one(&mut **tx)
    .await
}

pub async fn fetch_series<'e, E>(executor: E, id: Uuid) -> sqlx::Result<Option<SeriesRow>>
where
    E: sqlx::PgExecutor<'e>,
{
    sqlx::query_as::<_, SeriesRow>(
        "SELECT id, tenant_id, course_id, title, starts_at, duration_minutes,
                frequency, byweekday, end_kind, occurrence_count, end_until,
                primary_teacher_id, recording_enabled, transport_mode
           FROM live_session_series WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(executor)
    .await
}

pub async fn list_occurrences_for_series<'e, E>(
    executor: E,
    series_id: Uuid,
) -> sqlx::Result<Vec<OccurrenceRow>>
where
    E: sqlx::PgExecutor<'e>,
{
    sqlx::query_as::<_, OccurrenceRow>(
        "SELECT id, series_id, course_id, occurrence_index, title, status,
                starts_at, duration_minutes, primary_teacher_id, recording_enabled,
                diverged
           FROM live_sessions WHERE series_id = $1 ORDER BY occurrence_index",
    )
    .bind(series_id)
    .fetch_all(executor)
    .await
}

pub async fn delete_series(tx: &mut Transaction<'_, Postgres>, id: Uuid) -> sqlx::Result<bool> {
    Ok(sqlx::query("DELETE FROM live_session_series WHERE id = $1")
        .bind(id)
        .execute(&mut **tx)
        .await?
        .rows_affected()
        > 0)
}

pub async fn patch_occurrence(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    starts_at: Option<chrono::DateTime<chrono::Utc>>,
    duration_minutes: Option<i32>,
    title: Option<&str>,
    status: Option<&str>,
    diverged: bool,
) -> sqlx::Result<Option<OccurrenceRow>> {
    sqlx::query_as::<_, OccurrenceRow>(
        "UPDATE live_sessions SET
            starts_at = COALESCE($2, starts_at),
            duration_minutes = COALESCE($3, duration_minutes),
            title = COALESCE($4, title),
            status = COALESCE($5, status),
            diverged = diverged OR $6,
            updated_at = now()
          WHERE id = $1
        RETURNING id, series_id, course_id, occurrence_index, title, status,
                  starts_at, duration_minutes, primary_teacher_id,
                  recording_enabled, diverged",
    )
    .bind(id)
    .bind(starts_at)
    .bind(duration_minutes)
    .bind(title)
    .bind(status)
    .bind(diverged)
    .fetch_optional(&mut **tx)
    .await
}

use sha2::{Digest, Sha256};

/// Hash of a publish nonce, stored at-rest. Plaintext is never persisted.
pub fn hash_nonce(plaintext: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(plaintext.as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Atomically transitions a session from `scheduled` → `live`, populates
/// `actual_started_at`, paths, and the publish nonce hash. Returns `None` if
/// the session is not in a valid state for the transition.
pub async fn go_live(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    main_path: &str,
    screen_path: Option<&str>,
    publish_nonce_hash: &str,
    publish_nonce_expires_at: chrono::DateTime<chrono::Utc>,
) -> sqlx::Result<Option<LiveSessionRow>> {
    sqlx::query_as::<_, LiveSessionRow>(
        "UPDATE live_sessions
            SET status = 'live',
                actual_started_at = COALESCE(actual_started_at, now()),
                main_path = $2,
                screen_path = $3,
                publish_nonce = $4,
                publish_nonce_expires_at = $5
          WHERE id = $1
            AND status IN ('scheduled', 'live')
        RETURNING id, tenant_id, course_id, series_id, occurrence_index, title,
                  status, starts_at, duration_minutes, actual_started_at,
                  actual_ended_at, primary_teacher_id, mode, recording_enabled,
                  main_path, hls_fallback_enabled, diverged,
                  screen_path, transport_mode,
                  publish_nonce, publish_nonce_expires_at",
    )
    .bind(id)
    .bind(main_path)
    .bind(screen_path)
    .bind(publish_nonce_hash)
    .bind(publish_nonce_expires_at)
    .fetch_optional(&mut **tx)
    .await
}

/// Atomically transitions a session from `live` → `ended` (idempotent — if
/// already `ended`, returns the row without modification). Returns `None` if
/// the session doesn't exist or is in a terminal state other than `ended`.
pub async fn end_class(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
) -> sqlx::Result<Option<LiveSessionRow>> {
    sqlx::query_as::<_, LiveSessionRow>(
        "UPDATE live_sessions
            SET status = 'ended',
                actual_ended_at = COALESCE(actual_ended_at, now()),
                publish_nonce = NULL,
                publish_nonce_expires_at = NULL
          WHERE id = $1
            AND status IN ('live', 'ended')
        RETURNING id, tenant_id, course_id, series_id, occurrence_index, title,
                  status, starts_at, duration_minutes, actual_started_at,
                  actual_ended_at, primary_teacher_id, mode, recording_enabled,
                  main_path, hls_fallback_enabled, diverged,
                  screen_path, transport_mode,
                  publish_nonce, publish_nonce_expires_at",
    )
    .bind(id)
    .fetch_optional(&mut **tx)
    .await
}

/// Ends all `live` sessions whose `actual_started_at + duration_minutes + 30min`
/// is in the past. Returns `(id, tenant_id)` tuples for ended sessions, so
/// callers can emit audit events.
pub async fn sweep_auto_end(pool: &PgPool) -> sqlx::Result<Vec<(Uuid, Uuid)>> {
    // Cross-tenant system sweep: elevate via app.system so the live_sessions RLS
    // policies permit the UPDATE under the non-bypass aulalite_app role.
    let mut tx = crate::db::begin_system_context(pool).await?;
    let rows: Vec<(Uuid, Uuid)> = sqlx::query_as(
        "UPDATE live_sessions
            SET status = 'ended',
                actual_ended_at = now(),
                publish_nonce = NULL,
                publish_nonce_expires_at = NULL
          WHERE status = 'live'
            AND actual_started_at IS NOT NULL
            AND actual_started_at + (duration_minutes + 30) * interval '1 minute' < now()
        RETURNING id, tenant_id",
    )
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(rows)
}

/// Joins `live_sessions` with `courses` to provide everything the join
/// handler needs: session row + course slug + course title.
#[derive(Debug, sqlx::FromRow)]
pub struct LiveSessionForJoin {
    // session fields
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub course_id: Uuid,
    pub status: String,
    pub starts_at: chrono::DateTime<chrono::Utc>,
    pub duration_minutes: i32,
    pub actual_started_at: Option<chrono::DateTime<chrono::Utc>>,
    pub actual_ended_at: Option<chrono::DateTime<chrono::Utc>>,
    pub main_path: Option<String>,
    pub screen_path: Option<String>,
    pub transport_mode: String,
    pub recording_enabled: bool,
    pub primary_teacher_id: Option<Uuid>,
    pub title: String,
    // course fields (for the lobby UI)
    pub course_slug: String,
    pub course_title: String,
}

/// Convenience wrapper: load a live session for join under a fresh tx with
/// `app.user_id` + `app.tenant_id` set, then commit. Use this from handlers
/// that just need the session row for tenancy/permission validation.
pub async fn load_for_join_with_context(
    pool: &PgPool,
    user_id: Uuid,
    tenant_id: Option<Uuid>,
    id: Uuid,
) -> sqlx::Result<Option<LiveSessionForJoin>> {
    let mut tx = crate::db::begin_with_context(pool, user_id, tenant_id).await?;
    let row = load_for_join(&mut *tx, id).await?;
    tx.commit().await?;
    Ok(row)
}

pub async fn load_for_join<'e, E>(executor: E, id: Uuid) -> sqlx::Result<Option<LiveSessionForJoin>>
where
    E: sqlx::PgExecutor<'e>,
{
    sqlx::query_as::<_, LiveSessionForJoin>(
        "SELECT s.id, s.tenant_id, s.course_id, s.status, s.starts_at,
                s.duration_minutes, s.actual_started_at, s.actual_ended_at,
                s.main_path, s.screen_path, s.transport_mode,
                s.recording_enabled, s.primary_teacher_id, s.title,
                c.slug AS course_slug, c.title AS course_title
           FROM live_sessions s
           JOIN courses c ON c.id = s.course_id
          WHERE s.id = $1",
    )
    .bind(id)
    .fetch_optional(executor)
    .await
}

/// Returns the currently-live session for a course, if any. Used by the
/// start-now conflict check and the active-session read endpoint.
///
/// Returned fields are the minimum needed for the 409 / banner UI:
/// `id`, `title`, `starts_at` (the scheduled start), `actual_started_at`
/// (when it went live — may equal starts_at for ad-hoc sessions), and
/// `transport_mode`.
#[derive(Debug, sqlx::FromRow, Clone)]
pub struct ActiveSessionRow {
    pub id: Uuid,
    pub title: String,
    pub starts_at: chrono::DateTime<chrono::Utc>,
    pub actual_started_at: Option<chrono::DateTime<chrono::Utc>>,
    pub transport_mode: String,
}

/// Convenience wrapper of `find_live_for_course` with RLS GUCs preset.
pub async fn find_live_for_course_with_context(
    pool: &PgPool,
    user_id: Uuid,
    tenant_id: Option<Uuid>,
    course_id: Uuid,
) -> sqlx::Result<Option<ActiveSessionRow>> {
    let mut tx = crate::db::begin_with_context(pool, user_id, tenant_id).await?;
    let row = find_live_for_course(&mut *tx, course_id).await?;
    tx.commit().await?;
    Ok(row)
}

pub async fn find_live_for_course<'e, E>(
    executor: E,
    course_id: Uuid,
) -> sqlx::Result<Option<ActiveSessionRow>>
where
    E: sqlx::PgExecutor<'e>,
{
    sqlx::query_as::<_, ActiveSessionRow>(
        "SELECT id, title, starts_at, actual_started_at, transport_mode
           FROM live_sessions
          WHERE course_id = $1
            AND status = 'live'
          LIMIT 1",
    )
    .bind(course_id)
    .fetch_optional(executor)
    .await
}

/// Row shape for the "what is this user already running?" lookup. Carries
/// enough fields to build a self-service banner ("you have a live session
/// in course X — end it before starting another").
#[derive(Debug, sqlx::FromRow, Clone, Serialize)]
pub struct UserActiveSessionRow {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub course_id: Uuid,
    pub title: String,
    pub starts_at: chrono::DateTime<chrono::Utc>,
    pub actual_started_at: Option<chrono::DateTime<chrono::Utc>>,
    pub transport_mode: String,
}

/// Returns the user's currently-live session within the caller's tenant
/// context, if any. Used by the start-now preflight to surface a friendly
/// 409 ("you already have a session live in course X"). Cross-tenant
/// hits are caught by the `live_sessions_one_live_per_user` partial
/// unique index — see the constraint-violation branch in
/// `handlers/live_sessions.rs::start_now_inner`.
pub async fn find_live_for_user_with_context(
    pool: &PgPool,
    user_id: Uuid,
    tenant_id: Option<Uuid>,
) -> sqlx::Result<Option<UserActiveSessionRow>> {
    let mut tx = crate::db::begin_with_context(pool, user_id, tenant_id).await?;
    let row = sqlx::query_as::<_, UserActiveSessionRow>(
        "SELECT id, tenant_id, course_id, title, starts_at, actual_started_at, transport_mode
           FROM live_sessions
          WHERE primary_teacher_id = $1
            AND status = 'live'
          LIMIT 1",
    )
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(row)
}

/// Atomically validates and consumes a publish nonce. Returns `Some(row)`
/// if the candidate matches the stored hash AND has not expired AND the
/// session is in a publishable state. The nonce is NULL'd in the same
/// statement, so a second call with the same candidate fails.
pub async fn consume_publish_nonce(
    pool: &PgPool,
    session_id: Uuid,
    candidate_hash: &str,
) -> sqlx::Result<Option<LiveSessionRow>> {
    // MediaMTX calls this server-to-server, outside a user request, so there is
    // no tenant GUC to satisfy `live_sessions` RLS. Elevate only this
    // transaction through the tightly-scoped system UPDATE policy; possession
    // of the one-time nonce remains the authorization boundary.
    let mut tx = crate::db::begin_system_context(pool).await?;
    let row = sqlx::query_as::<_, LiveSessionRow>(
        "UPDATE live_sessions
            SET publish_nonce = NULL
          WHERE id = $1
            AND publish_nonce = $2
            AND publish_nonce_expires_at IS NOT NULL
            AND publish_nonce_expires_at > now()
            AND status IN ('scheduled', 'live')
        RETURNING id, tenant_id, course_id, series_id, occurrence_index, title,
                  status, starts_at, duration_minutes, actual_started_at,
                  actual_ended_at, primary_teacher_id, mode, recording_enabled,
                  main_path, hls_fallback_enabled, diverged,
                  screen_path, transport_mode,
                  publish_nonce, publish_nonce_expires_at",
    )
    .bind(session_id)
    .bind(candidate_hash)
    .fetch_optional(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(row)
}
