// crates/backend/src/db/recordings.rs
use serde::Serialize;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

#[derive(Debug, Serialize, sqlx::FromRow, Clone)]
pub struct RecordingRow {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub session_id: Uuid,
    pub file_asset_id: Option<Uuid>,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub ended_at: chrono::DateTime<chrono::Utc>,
    pub duration_seconds: i32,
    pub processing_status: String,
    pub processing_error: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// `Some(false)` means the produced MP4 has no video stream (see the
    /// `recordings.has_video` migration). `None` means it has not been probed.
    pub has_video: Option<bool>,
}

/// Inserts a `recordings` row with `processing_status='pending'`.
/// Idempotent: `ON CONFLICT (session_id) DO NOTHING` prevents duplicates.
pub async fn insert_pending(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    session_id: Uuid,
    started_at: chrono::DateTime<chrono::Utc>,
    ended_at: chrono::DateTime<chrono::Utc>,
    duration_seconds: i32,
) -> sqlx::Result<Option<RecordingRow>> {
    sqlx::query_as::<_, RecordingRow>(
        "INSERT INTO recordings
            (tenant_id, session_id, started_at, ended_at, duration_seconds)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (session_id) DO NOTHING
         RETURNING id, tenant_id, session_id, file_asset_id, started_at, ended_at,
                   duration_seconds, processing_status, processing_error, created_at, has_video",
    )
    .bind(tenant_id)
    .bind(session_id)
    .bind(started_at)
    .bind(ended_at)
    .bind(duration_seconds)
    .fetch_optional(&mut **tx)
    .await
}

/// Atomically create-or-claim the pending recording for a session. Exactly one
/// worker can transition `pending` to `remuxing`; concurrent sweep ticks return
/// `None` after the winner changes the row state.
pub async fn claim_for_processing(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    session_id: Uuid,
    started_at: chrono::DateTime<chrono::Utc>,
    ended_at: chrono::DateTime<chrono::Utc>,
    duration_seconds: i32,
) -> sqlx::Result<Option<RecordingRow>> {
    sqlx::query(
        "INSERT INTO recordings
            (tenant_id, session_id, started_at, ended_at, duration_seconds)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (session_id) DO NOTHING",
    )
    .bind(tenant_id)
    .bind(session_id)
    .bind(started_at)
    .bind(ended_at)
    .bind(duration_seconds)
    .execute(&mut **tx)
    .await?;

    sqlx::query_as::<_, RecordingRow>(
        "UPDATE recordings
            SET processing_status = 'remuxing',
                processing_error = NULL,
                processing_updated_at = now()
          WHERE session_id = $1 AND processing_status = 'pending'
        RETURNING id, tenant_id, session_id, file_asset_id, started_at, ended_at,
                  duration_seconds, processing_status, processing_error, created_at, has_video",
    )
    .bind(session_id)
    .fetch_optional(&mut **tx)
    .await
}

pub async fn set_status(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    status: &str,
    error: Option<&str>,
) -> sqlx::Result<Option<RecordingRow>> {
    sqlx::query_as::<_, RecordingRow>(
        "UPDATE recordings
            SET processing_status = $2,
                processing_error = $3,
                processing_updated_at = now()
          WHERE id = $1
        RETURNING id, tenant_id, session_id, file_asset_id, started_at, ended_at,
                  duration_seconds, processing_status, processing_error, created_at, has_video",
    )
    .bind(id)
    .bind(status)
    .bind(error)
    .fetch_optional(&mut **tx)
    .await
}

pub async fn mark_available(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    file_asset_id: Uuid,
    duration_seconds: i32,
    has_video: Option<bool>,
) -> sqlx::Result<Option<RecordingRow>> {
    sqlx::query_as::<_, RecordingRow>(
        "UPDATE recordings
            SET processing_status = 'available',
                file_asset_id = $2,
                duration_seconds = $3,
                has_video = $4,
                processing_error = NULL,
                processing_updated_at = now()
          WHERE id = $1
        RETURNING id, tenant_id, session_id, file_asset_id, started_at, ended_at,
                  duration_seconds, processing_status, processing_error, created_at, has_video",
    )
    .bind(id)
    .bind(file_asset_id)
    .bind(duration_seconds)
    .bind(has_video)
    .fetch_optional(&mut **tx)
    .await
}

pub async fn fetch_by_session<'e, E>(
    executor: E,
    session_id: Uuid,
) -> sqlx::Result<Option<RecordingRow>>
where
    E: sqlx::PgExecutor<'e>,
{
    sqlx::query_as::<_, RecordingRow>(
        "SELECT id, tenant_id, session_id, file_asset_id, started_at, ended_at,
                duration_seconds, processing_status, processing_error, created_at, has_video
           FROM recordings
          WHERE session_id = $1",
    )
    .bind(session_id)
    .fetch_optional(executor)
    .await
}

pub async fn list_ended_sessions_needing_recording(
    pool: &PgPool,
    limit: i64,
) -> sqlx::Result<
    Vec<(
        Uuid,
        Uuid,
        chrono::DateTime<chrono::Utc>,
        chrono::DateTime<chrono::Utc>,
    )>,
> {
    // Cross-tenant system sweep: elevate via app.system so the live_sessions /
    // recordings RLS policies don't filter every row under aulalite_app.
    let mut tx = super::begin_system_context(pool).await?;
    let rows = sqlx::query_as::<
        _,
        (
            Uuid,
            Uuid,
            chrono::DateTime<chrono::Utc>,
            chrono::DateTime<chrono::Utc>,
        ),
    >(
        "SELECT s.id, s.tenant_id, s.actual_started_at, s.actual_ended_at
           FROM live_sessions s
          WHERE s.status = 'ended'
            AND s.recording_enabled = true
            AND s.actual_started_at IS NOT NULL
            AND s.actual_ended_at IS NOT NULL
            AND s.actual_ended_at - s.actual_started_at >= interval '5 seconds'
            AND NOT EXISTS (
                SELECT 1 FROM recordings r
                 WHERE r.session_id = s.id
                   AND r.processing_status <> 'pending'
            )
          ORDER BY s.actual_ended_at ASC
          LIMIT $1",
    )
    .bind(limit)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(rows)
}

pub async fn list_orphaned_in_progress(
    pool: &PgPool,
    stuck_after: chrono::Duration,
    limit: i64,
) -> sqlx::Result<Vec<RecordingRow>> {
    let mut tx = super::begin_system_context(pool).await?;
    let rows = sqlx::query_as::<_, RecordingRow>(
        "SELECT id, tenant_id, session_id, file_asset_id, started_at, ended_at,
                duration_seconds, processing_status, processing_error, created_at, has_video
           FROM recordings
          WHERE processing_status IN ('remuxing','uploading')
            AND processing_updated_at < now() - $1::interval
          ORDER BY processing_updated_at ASC
          LIMIT $2",
    )
    .bind(format!("{} seconds", stuck_after.num_seconds()))
    .bind(limit)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(rows)
}

/// Lists recordings eligible for retention pruning, honoring each recording's
/// tenant-configured `recording_retention_days`. A recording is prunable when
/// it is older than its tenant's retention window. `default_retention_days` is
/// the global SAFETY-CAP fallback used only when the tenant's value is NULL.
pub async fn list_for_retention_prune(
    pool: &PgPool,
    default_retention_days: i64,
    limit: i64,
) -> sqlx::Result<Vec<(Uuid, Option<Uuid>, Uuid)>> {
    // Returns (recording_id, file_asset_id, tenant_id). The tenant_id is carried
    // out so the janitor doesn't need a second per-row lookup (and so the per-row
    // lookup wasn't itself RLS-filtered). Runs under system context so the
    // recordings/tenants RLS policies don't filter rows under aulalite_app.
    let mut tx = super::begin_system_context(pool).await?;
    let rows = sqlx::query_as::<_, (Uuid, Option<Uuid>, Uuid)>(
        "SELECT r.id, r.file_asset_id, r.tenant_id
           FROM recordings r
           JOIN tenants t ON t.id = r.tenant_id
          WHERE now() - r.created_at
                > make_interval(days => COALESCE(t.recording_retention_days, $1::int))
          ORDER BY r.created_at ASC
          LIMIT $2",
    )
    .bind(default_retention_days as i32)
    .bind(limit)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(rows)
}

/// Row returned by the course-recordings list endpoint. Joins `recordings`
/// with `live_sessions` so the UI has title + scheduled time without a
/// second round-trip.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct CourseRecordingRow {
    pub recording_id: Uuid,
    pub session_id: Uuid,
    pub session_title: String,
    pub starts_at: chrono::DateTime<chrono::Utc>,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub ended_at: chrono::DateTime<chrono::Utc>,
    pub duration_seconds: i32,
    pub processing_status: String,
    pub file_asset_id: Option<Uuid>,
    pub has_video: Option<bool>,
}

pub async fn list_for_course<'e, E>(
    executor: E,
    course_id: Uuid,
) -> sqlx::Result<Vec<CourseRecordingRow>>
where
    E: sqlx::PgExecutor<'e>,
{
    sqlx::query_as::<_, CourseRecordingRow>(
        "SELECT r.id AS recording_id,
                r.session_id,
                s.title AS session_title,
                s.starts_at,
                r.started_at,
                r.ended_at,
                r.duration_seconds,
                r.processing_status,
                r.file_asset_id,
                r.has_video
           FROM recordings r
           JOIN live_sessions s ON s.id = r.session_id
          WHERE s.course_id = $1
          ORDER BY r.started_at DESC",
    )
    .bind(course_id)
    .fetch_all(executor)
    .await
}

pub async fn delete_by_id(tx: &mut Transaction<'_, Postgres>, id: Uuid) -> sqlx::Result<u64> {
    let r = sqlx::query("DELETE FROM recordings WHERE id = $1")
        .bind(id)
        .execute(&mut **tx)
        .await?;
    Ok(r.rows_affected())
}

// ============================================================================
// Recording chapters: named timestamp bookmarks into a recording's timeline.
//
// One `recording_chapters` row per (recording, label, position). Staff author
// and delete them; anyone who can read the course (the handler gate) reads them
// back on the replay view to seek the <video>. TENANT-SCOPED under RLS exactly
// like `recordings` (policy keys solely on app.tenant_id), so it's enforced
// under the non-bypass `aulalite_app` role. Runtime sqlx only.
// ============================================================================

#[derive(Debug, Serialize, sqlx::FromRow, Clone)]
pub struct RecordingChapterRow {
    pub id: Uuid,
    pub recording_id: Uuid,
    pub label: String,
    pub position_seconds: i32,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// List a recording's chapters in timeline order (earliest first). Tenant-scoped
/// under RLS; the caller has already been authorized to read the course.
pub async fn list_chapters<'e, E>(
    executor: E,
    recording_id: Uuid,
) -> sqlx::Result<Vec<RecordingChapterRow>>
where
    E: sqlx::PgExecutor<'e>,
{
    sqlx::query_as::<_, RecordingChapterRow>(
        "SELECT id, recording_id, label, position_seconds, created_at
           FROM recording_chapters
          WHERE recording_id = $1
          ORDER BY position_seconds ASC, created_at ASC",
    )
    .bind(recording_id)
    .fetch_all(executor)
    .await
}

/// Insert a chapter for `recording_id` and return the persisted row.
/// Tenant-scoped under RLS. `position_seconds` is clamped non-negative by the
/// handler; the table also CHECKs it.
pub async fn insert_chapter(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    recording_id: Uuid,
    label: &str,
    position_seconds: i32,
) -> sqlx::Result<RecordingChapterRow> {
    sqlx::query_as::<_, RecordingChapterRow>(
        "INSERT INTO recording_chapters
             (tenant_id, recording_id, label, position_seconds)
         VALUES ($1, $2, $3, $4)
         RETURNING id, recording_id, label, position_seconds, created_at",
    )
    .bind(tenant_id)
    .bind(recording_id)
    .bind(label)
    .bind(position_seconds)
    .fetch_one(&mut **tx)
    .await
}

/// Delete a chapter by id, constrained to its recording so a caller can't
/// delete another recording's chapter by guessing an id. Tenant-scoped under
/// RLS. Returns true when a row was removed.
pub async fn delete_chapter(
    tx: &mut Transaction<'_, Postgres>,
    recording_id: Uuid,
    chapter_id: Uuid,
) -> sqlx::Result<bool> {
    let r = sqlx::query("DELETE FROM recording_chapters WHERE id = $1 AND recording_id = $2")
        .bind(chapter_id)
        .bind(recording_id)
        .execute(&mut **tx)
        .await?;
    Ok(r.rows_affected() > 0)
}
