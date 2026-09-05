// crates/backend/src/db/attendance.rs
//! Durable live-room attendance accounting. Live presence is ephemeral
//! (Redis); this module persists per-(session,user) join/leave records so
//! analytics and the parent dashboard have a source of truth.
//!
//! Each write sets the `app.tenant_id` GUC inside its own transaction so the
//! `attendance` RLS policy applies when the backend connects as the non-
//! superuser `aulalite_app` role (see `migrations/20260517000020_app_role.sql`).
//! Errors are surfaced to the caller; the WS handler logs and swallows them so
//! attendance can never break a live socket.
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

/// Record that `user_id` joined `session_id`. UPSERT semantics:
///   * first join  -> INSERT (first_joined_at = last_joined_at = now, open).
///   * re-join after a leave/drop (row closed) -> re-open, reset the accrual
///     window to now, and increment reconnect_count.
///   * duplicate join while already open (second tab / WS resume) -> no-op for
///     accounting: keep the original accrual window and do NOT count a
///     reconnect. Overwriting `last_joined_at` here would silently discard the
///     in-flight segment when whichever socket closes second hits the
///     open-guarded leave as a no-op, so an open join must never move it.
///
/// `total_seconds` is NOT touched here — it only accrues on leave/finalize,
/// when we know how long the most-recent segment lasted.
pub async fn record_join(
    pool: &PgPool,
    tenant_id: Uuid,
    session_id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO attendance
            (session_id, user_id, tenant_id,
             first_joined_at, last_joined_at, open, reconnect_count)
         VALUES ($1, $2, $3, now(), now(), true, 0)
         ON CONFLICT (session_id, user_id) DO UPDATE
            SET last_joined_at = CASE WHEN attendance.open
                                      THEN attendance.last_joined_at
                                      ELSE now() END,
                open = true,
                reconnect_count = attendance.reconnect_count
                                  + CASE WHEN attendance.open THEN 0 ELSE 1 END,
                updated_at = now()",
    )
    .bind(session_id)
    .bind(user_id)
    .bind(tenant_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Record that `user_id` left `session_id`. Only an OPEN row is affected:
/// accrue the most-recent segment into `total_seconds`, stamp `last_left_at`,
/// and close the row. A no-op if there is no open row (e.g. a duplicate leave).
pub async fn record_leave(
    pool: &PgPool,
    tenant_id: Uuid,
    session_id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "UPDATE attendance
            SET total_seconds = total_seconds
                    + GREATEST(0, EXTRACT(EPOCH FROM (now() - last_joined_at))::int),
                last_left_at = now(),
                open = false,
                updated_at = now()
          WHERE session_id = $1
            AND user_id = $2
            AND open = true",
    )
    .bind(session_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Close out every still-`open` row for `session_id` when the session ends.
/// Attendees whose socket dropped without a clean leave (closed tab, network
/// loss) never sent `record_leave`, so their final segment would otherwise be
/// uncounted. We accrue each open row up to `ended_at` (or now() if null).
///
/// Tenant scoping for this background path: there is no per-request tenant, so
/// we resolve the session's `tenant_id` first and set the `app.tenant_id` GUC
/// for the UPDATE — mirroring how the recording sweep scopes its tenant (it
/// sets `app.tenant_id` per session it processes). This keeps the write correct
/// even under the non-bypass `aulalite_app` role. The session lookup itself
/// runs without a GUC, matching `live_sessions::sweep_auto_end` /
/// `list_ended_sessions_needing_recording`, which read `live_sessions` on a
/// bare pool in their background tasks.
pub async fn finalize_open_for_session(
    pool: &PgPool,
    session_id: Uuid,
    ended_at: Option<chrono::DateTime<chrono::Utc>>,
) -> sqlx::Result<u64> {
    let mut system_tx = crate::db::begin_system_context(pool).await?;
    let tenant_id: Option<Uuid> =
        sqlx::query_scalar("SELECT tenant_id FROM live_sessions WHERE id = $1")
            .bind(session_id)
            .fetch_optional(&mut *system_tx)
            .await?;
    system_tx.commit().await?;
    let Some(tenant_id) = tenant_id else {
        return Ok(0);
    };

    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut *tx)
        .await?;
    let res = sqlx::query(
        "UPDATE attendance
            SET total_seconds = total_seconds
                    + GREATEST(0, EXTRACT(EPOCH FROM (COALESCE($2, now()) - last_joined_at))::int),
                last_left_at = COALESCE($2, now()),
                open = false,
                updated_at = now()
          WHERE session_id = $1
            AND open = true",
    )
    .bind(session_id)
    .bind(ended_at)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(res.rows_affected())
}

/// One attendee's record for a session, joined with `users` for display.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct AttendanceRow {
    pub user_id: Uuid,
    pub display_name: Option<String>,
    pub email: Option<String>,
    pub first_joined_at: chrono::DateTime<chrono::Utc>,
    pub last_left_at: Option<chrono::DateTime<chrono::Utc>>,
    pub total_seconds: i32,
    pub reconnect_count: i32,
}

/// One attendee row for a single user across a session, with the session and
/// course titles joined for display in the parent dashboard.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct UserAttendanceRow {
    pub session_id: Uuid,
    pub session_title: Option<String>,
    pub course_title: Option<String>,
    pub first_joined_at: chrono::DateTime<chrono::Utc>,
    pub last_left_at: Option<chrono::DateTime<chrono::Utc>>,
    pub total_seconds: i32,
    pub reconnect_count: i32,
    pub starts_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// List every attendance row for `user_id` across all of the tenant's sessions,
/// joined with `live_sessions` (session title + starts_at) and `courses` (title).
/// Runs inside a tx with the tenant GUC set so the RLS policy applies; the caller
/// has already authorized access (e.g. a parent linked to this child).
pub async fn list_for_user(
    pool: &PgPool,
    tenant_id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<Vec<UserAttendanceRow>> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut *tx)
        .await?;
    let rows = sqlx::query_as::<_, UserAttendanceRow>(
        "SELECT a.session_id,
                ls.title AS session_title,
                c.title AS course_title,
                a.first_joined_at,
                a.last_left_at,
                a.total_seconds,
                a.reconnect_count,
                ls.starts_at
           FROM attendance a
           LEFT JOIN live_sessions ls ON ls.id = a.session_id
           LEFT JOIN courses c ON c.id = ls.course_id
          WHERE a.user_id = $1
            AND a.tenant_id = $2
          ORDER BY a.first_joined_at DESC, a.session_id ASC",
    )
    .bind(user_id)
    .bind(tenant_id)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(rows)
}

/// List all attendance rows for `session_id`, newest joiners last (stable by
/// first_joined_at then user_id). Runs inside a tx with the tenant GUC set so
/// the RLS policy applies; the caller has already authorized access.
pub async fn list_for_session(
    pool: &PgPool,
    tenant_id: Uuid,
    session_id: Uuid,
) -> sqlx::Result<Vec<AttendanceRow>> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut *tx)
        .await?;
    let rows = sqlx::query_as::<_, AttendanceRow>(
        "SELECT a.user_id,
                u.display_name,
                u.email,
                a.first_joined_at,
                a.last_left_at,
                a.total_seconds,
                a.reconnect_count
           FROM attendance a
           LEFT JOIN users u ON u.id = a.user_id
          WHERE a.session_id = $1
          ORDER BY a.first_joined_at ASC, a.user_id ASC",
    )
    .bind(session_id)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(rows)
}
