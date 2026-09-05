// crates/backend/src/db/calendar.rs
//! Personal-calendar data layer + reminder-dedup bookkeeping.
//!
//! Two distinct access regimes, mirroring the rest of the codebase:
//!
//!   * The per-user calendar reads (`list_visible_sessions` /
//!     `list_visible_assignment_due`) run inside a tx with `app.user_id` +
//!     `app.tenant_id` set (via `begin_with_context`), so the `tenant_isolation`
//!     RLS policies apply under the non-bypass `aulalite_app` role. They join
//!     `course_memberships` so the caller only ever sees sessions / assignment
//!     due dates for courses they are an ACTIVE member of (owners read their
//!     own courses through the membership row inserted at create time).
//!
//!   * The reminder sweep (`due_sessions_for_reminders` /
//!     `due_assignments_for_reminders`) is a trusted in-process background
//!     worker that must see ACROSS tenants. It runs under `begin_system_context`
//!     (`app.system = 'on'`), which the `system_context_*` RLS policies honor —
//!     see `migrations/20260530000030_system_context_rls.sql` and the additional
//!     policies this feature's migration adds for `assignments`,
//!     `course_memberships`, and `reminders_sent`.
//!
//! Uses ONLY runtime sqlx (no compile-time macros — there is no DATABASE_URL at
//! build).
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

/// A live session visible to a user within a calendar window. Joined with
/// `courses` for the title + slug used to build the deep link.
#[derive(Debug, sqlx::FromRow)]
pub struct CalendarSessionRow {
    pub session_id: Uuid,
    pub course_id: Uuid,
    pub course_slug: String,
    pub course_title: String,
    pub title: String,
    pub starts_at: DateTime<Utc>,
    pub duration_minutes: i32,
    pub status: String,
}

/// A published assignment due date visible to a user within a calendar window.
#[derive(Debug, sqlx::FromRow)]
pub struct CalendarAssignmentRow {
    pub assignment_id: Uuid,
    pub course_id: Uuid,
    pub course_slug: String,
    pub course_title: String,
    pub title: String,
    pub due_at: DateTime<Utc>,
}

/// List the live sessions the caller can see (by ACTIVE course membership)
/// whose `starts_at` falls within `[from, to)`. Tenant-scoped under RLS via
/// `begin_with_context`. Ordered by start time.
pub async fn list_visible_sessions(
    pool: &PgPool,
    user_id: Uuid,
    tenant_id: Option<Uuid>,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> sqlx::Result<Vec<CalendarSessionRow>> {
    let mut tx = crate::db::begin_with_context(pool, user_id, tenant_id).await?;
    let rows = sqlx::query_as::<_, CalendarSessionRow>(
        "SELECT ls.id AS session_id, ls.course_id, c.slug AS course_slug,
                c.title AS course_title, ls.title, ls.starts_at,
                ls.duration_minutes, ls.status
           FROM live_sessions ls
           JOIN course_memberships cm
             ON cm.course_id = ls.course_id
            AND cm.user_id = $1
            AND cm.status = 'active'
           JOIN courses c ON c.id = ls.course_id
          WHERE ls.starts_at >= $2
            AND ls.starts_at < $3
            AND ls.status IN ('scheduled', 'live', 'ended')
          ORDER BY ls.starts_at, ls.id",
    )
    .bind(user_id)
    .bind(from)
    .bind(to)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(rows)
}

/// List the published-assignment due dates the caller can see (by ACTIVE course
/// membership) whose `due_at` falls within `[from, to)`. Tenant-scoped under RLS
/// via `begin_with_context`. Ordered by due date.
pub async fn list_visible_assignment_due(
    pool: &PgPool,
    user_id: Uuid,
    tenant_id: Option<Uuid>,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> sqlx::Result<Vec<CalendarAssignmentRow>> {
    let mut tx = crate::db::begin_with_context(pool, user_id, tenant_id).await?;
    let rows = sqlx::query_as::<_, CalendarAssignmentRow>(
        "SELECT a.id AS assignment_id, a.course_id, c.slug AS course_slug,
                c.title AS course_title, a.title, a.due_at
           FROM assignments a
           JOIN course_memberships cm
             ON cm.course_id = a.course_id
            AND cm.user_id = $1
            AND cm.status = 'active'
           JOIN courses c ON c.id = a.course_id
          WHERE a.due_at IS NOT NULL
            AND a.due_at >= $2
            AND a.due_at < $3
            AND a.status = 'published'
          ORDER BY a.due_at, a.id",
    )
    .bind(user_id)
    .bind(from)
    .bind(to)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(rows)
}

// ===========================================================================
// reminder sweep (cross-tenant, system context)
// ===========================================================================

/// A live session that is starting soon and still needs a reminder fanned out.
#[derive(Debug, sqlx::FromRow, Clone)]
pub struct DueSessionRow {
    pub session_id: Uuid,
    pub tenant_id: Uuid,
    pub course_id: Uuid,
    pub course_slug: String,
    pub course_title: String,
    pub title: String,
    pub starts_at: DateTime<Utc>,
}

/// A published assignment whose due date is approaching and still needs a
/// reminder fanned out.
#[derive(Debug, sqlx::FromRow, Clone)]
pub struct DueAssignmentRow {
    pub assignment_id: Uuid,
    pub tenant_id: Uuid,
    pub course_id: Uuid,
    pub course_slug: String,
    pub course_title: String,
    pub title: String,
    pub due_at: DateTime<Utc>,
}

/// Cross-tenant: find scheduled live sessions starting within the next `window`
/// (and not already started) that have NOT yet had a `session_starting`
/// reminder recorded in `reminders_sent`. Runs under the system context so it
/// sees every tenant. The `reminder_kind` matches what `mark_reminder_sent`
/// writes so a session is only surfaced until its reminder is recorded.
pub async fn due_sessions_for_reminders(
    pool: &PgPool,
    window: chrono::Duration,
    limit: i64,
) -> sqlx::Result<Vec<DueSessionRow>> {
    let mut tx = crate::db::begin_system_context(pool).await?;
    let window_secs = window.num_seconds();
    let rows = sqlx::query_as::<_, DueSessionRow>(
        "SELECT ls.id AS session_id, ls.tenant_id, ls.course_id,
                c.slug AS course_slug, c.title AS course_title, ls.title,
                ls.starts_at
           FROM live_sessions ls
           JOIN courses c ON c.id = ls.course_id
          WHERE ls.status = 'scheduled'
            AND ls.starts_at > now()
            AND ls.starts_at <= now() + make_interval(secs => $1::double precision)
            AND NOT EXISTS (
                SELECT 1 FROM reminders_sent rs
                 WHERE rs.entity_type = 'session'
                   AND rs.entity_id = ls.id
                   AND rs.reminder_kind = 'session_starting'
            )
          ORDER BY ls.starts_at
          LIMIT $2",
    )
    .bind(window_secs)
    .bind(limit)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(rows)
}

/// Cross-tenant: find published assignments whose `due_at` is within the next
/// `window` (and not already past) that have NOT yet had an `assignment_due`
/// reminder recorded. Runs under the system context.
pub async fn due_assignments_for_reminders(
    pool: &PgPool,
    window: chrono::Duration,
    limit: i64,
) -> sqlx::Result<Vec<DueAssignmentRow>> {
    let mut tx = crate::db::begin_system_context(pool).await?;
    let window_secs = window.num_seconds();
    let rows = sqlx::query_as::<_, DueAssignmentRow>(
        "SELECT a.id AS assignment_id, a.tenant_id, a.course_id,
                c.slug AS course_slug, c.title AS course_title, a.title, a.due_at
           FROM assignments a
           JOIN courses c ON c.id = a.course_id
          WHERE a.status = 'published'
            AND a.due_at IS NOT NULL
            AND a.due_at > now()
            AND a.due_at <= now() + make_interval(secs => $1::double precision)
            AND NOT EXISTS (
                SELECT 1 FROM reminders_sent rs
                 WHERE rs.entity_type = 'assignment'
                   AND rs.entity_id = a.id
                   AND rs.reminder_kind = 'assignment_due'
            )
          ORDER BY a.due_at
          LIMIT $2",
    )
    .bind(window_secs)
    .bind(limit)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(rows)
}

/// Cross-tenant: the ACTIVE student member ids of `course_id`. Runs under the
/// system context so the reminder sweep can fan out without a tenant GUC.
pub async fn active_student_ids_system(pool: &PgPool, course_id: Uuid) -> sqlx::Result<Vec<Uuid>> {
    let mut tx = crate::db::begin_system_context(pool).await?;
    let ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT user_id FROM course_memberships
          WHERE course_id = $1
            AND status = 'active'
            AND role = 'student'",
    )
    .bind(course_id)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(ids)
}

/// Record that a reminder of `reminder_kind` for `(entity_type, entity_id)` has
/// been sent, so the next sweep skips it. Idempotent via the unique index
/// `(entity_type, entity_id, reminder_kind)` — a duplicate insert is swallowed.
/// Runs under the system context (cross-tenant); the row carries `tenant_id` for
/// auditability + retention pruning.
pub async fn mark_reminder_sent(
    pool: &PgPool,
    tenant_id: Uuid,
    entity_type: &str,
    entity_id: Uuid,
    reminder_kind: &str,
) -> sqlx::Result<()> {
    let mut tx = crate::db::begin_system_context(pool).await?;
    sqlx::query(
        "INSERT INTO reminders_sent (tenant_id, entity_type, entity_id, reminder_kind)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (entity_type, entity_id, reminder_kind) DO NOTHING",
    )
    .bind(tenant_id)
    .bind(entity_type)
    .bind(entity_id)
    .bind(reminder_kind)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}
