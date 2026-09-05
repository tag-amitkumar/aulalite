// crates/backend/src/db/parent.rs
//! Parent role data layer: parent<->student links and the invitations that
//! create them.
//!
//! Two paths with different RLS handling:
//!   * Staff-facing reads/writes (`create_invitation`, `list_invitations`,
//!     `revoke_invitation`) and selected-workspace parent reads (`list_children`,
//!     `is_linked`) run inside a tx with
//!     the relevant tenant GUC set, so the strict `tenant_isolation` policies on
//!     `parent_links` / `parent_invitations` apply under the non-bypass
//!     `aulalite_app` role (see `migrations/20260517000020_app_role.sql`).
//!   * Acceptance (`accept_pending_for_email`) runs INSIDE the JIT-provisioning
//!     tx, where no per-request tenant context exists and several tenants may be
//!     touched at once. It delegates to the seat-enforcing
//!     `accept_parent_invitations_for_email_v2` SECURITY DEFINER function
//!     (migration `20260716000074_parent_invitation_seat_enforcement.sql`).
use serde::Serialize;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

/// A pending/accepted/revoked parent invitation row.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct InvitationRow {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub parent_email: String,
    pub student_user_id: Uuid,
    pub relationship: Option<String>,
    pub status: String,
    pub created_by: Option<Uuid>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub accepted_at: Option<chrono::DateTime<chrono::Utc>>,
    pub seat_reserved: bool,
}

#[derive(Debug)]
pub enum ParentInvitationReservationOutcome {
    /// A pending invitation exists and its email has a tenant seat reservation
    /// (or already belongs to an active tenant member).
    Reserved(InvitationRow),
    /// Creating the first reservation for this email would exceed a blocking
    /// plan's seat cap.
    SeatLimitReached,
}

/// A child (student) linked to a parent, with display fields joined from `users`.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct ChildRow {
    pub student_user_id: Uuid,
    pub display_name: Option<String>,
    pub email: Option<String>,
    pub tenant_id: Uuid,
}

const INVITATION_SELECT: &str = "SELECT id, tenant_id, parent_email::text AS parent_email, \
     student_user_id, relationship, status, created_by, created_at, accepted_at, seat_reserved \
     FROM parent_invitations";

/// Create (or return the existing) pending invitation for
/// `(tenant_id, parent_email, student_user_id)`. Idempotent against the pending
/// partial-unique index: a second call with the same triple while a pending row
/// exists returns that row rather than erroring (mirrors course invitations).
pub async fn create_invitation(
    pool: &PgPool,
    tenant_id: Uuid,
    parent_email: &str,
    student_user_id: Uuid,
    relationship: Option<&str>,
    created_by: Uuid,
) -> sqlx::Result<ParentInvitationReservationOutcome> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.user_id', $1, true)")
        .bind(created_by.to_string())
        .execute(&mut *tx)
        .await?;
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut *tx)
        .await?;

    crate::db::seats::lock_tenant_for_seat_mutation(&mut tx, tenant_id).await?;

    let existing: Option<InvitationRow> = sqlx::query_as::<_, InvitationRow>(
        "SELECT id, tenant_id, parent_email::text AS parent_email,
                student_user_id, relationship, status, created_by,
                created_at, accepted_at, seat_reserved
           FROM parent_invitations
          WHERE tenant_id = $1
            AND lower(parent_email::text) = lower($2)
            AND student_user_id = $3
            AND status = 'pending'",
    )
    .bind(tenant_id)
    .bind(parent_email)
    .bind(student_user_id)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(row) = existing.as_ref().filter(|row| row.seat_reserved) {
        let row = row.clone();
        tx.commit().await?;
        return Ok(ParentInvitationReservationOutcome::Reserved(row));
    }

    // One parent may be linked to several children, so reuse a same-email
    // reservation from either invitation type. Do not infer that an active
    // same-email identity is the invitee: global and tenant-controlled
    // identities may share an address without being linked.
    let already_covered: bool = sqlx::query_scalar(
        "SELECT
            EXISTS (
                SELECT 1 FROM tenant_invitations ti
                 WHERE ti.tenant_id = $1
                   AND ti.status = 'pending'
                   AND lower(ti.email::text) = lower($2)
            ) OR EXISTS (
                SELECT 1
                  FROM parent_invitations pi
                 WHERE pi.tenant_id = $1
                   AND pi.status = 'pending'
                   AND pi.seat_reserved
                   AND lower(pi.parent_email::text) = lower($2)
            )",
    )
    .bind(tenant_id)
    .bind(parent_email)
    .fetch_one(&mut *tx)
    .await?;

    if !already_covered
        && crate::db::seats::seat_usage_in_tx(&mut tx, tenant_id)
            .await?
            .would_block()
    {
        tx.rollback().await?;
        return Ok(ParentInvitationReservationOutcome::SeatLimitReached);
    }

    if let Some(row) = existing {
        sqlx::query("UPDATE parent_invitations SET seat_reserved = true WHERE id = $1")
            .bind(row.id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        return Ok(ParentInvitationReservationOutcome::Reserved(row));
    }

    // Try to insert. The pending partial-unique can collide; DO NOTHING then
    // re-select so a repeated invite is a no-op that still returns the row.
    let inserted: Option<InvitationRow> = sqlx::query_as::<_, InvitationRow>(
        "INSERT INTO parent_invitations
            (tenant_id, parent_email, student_user_id, relationship, created_by, seat_reserved)
         VALUES ($1, $2, $3, $4, $5, true)
         ON CONFLICT DO NOTHING
         RETURNING id, tenant_id, parent_email::text AS parent_email,
                   student_user_id, relationship, status, created_by,
                   created_at, accepted_at, seat_reserved",
    )
    .bind(tenant_id)
    .bind(parent_email)
    .bind(student_user_id)
    .bind(relationship)
    .bind(created_by)
    .fetch_optional(&mut *tx)
    .await?;

    let row = match inserted {
        Some(row) => row,
        None => {
            // Collided with the existing pending row — re-select it.
            sqlx::query_as::<_, InvitationRow>(sqlx::AssertSqlSafe(format!(
                "{INVITATION_SELECT}
                  WHERE tenant_id = $1
                    AND lower(parent_email::text) = lower($2)
                    AND student_user_id = $3
                    AND status = 'pending'"
            )))
            .bind(tenant_id)
            .bind(parent_email)
            .bind(student_user_id)
            .fetch_one(&mut *tx)
            .await?
        }
    };

    tx.commit().await?;
    Ok(ParentInvitationReservationOutcome::Reserved(row))
}

/// List invitations for a tenant, optionally filtered to one student. Newest
/// first. Runs with the tenant GUC set so RLS applies.
pub async fn list_invitations(
    pool: &PgPool,
    tenant_id: Uuid,
    student_id: Option<Uuid>,
) -> sqlx::Result<Vec<InvitationRow>> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut *tx)
        .await?;

    let rows = sqlx::query_as::<_, InvitationRow>(sqlx::AssertSqlSafe(format!(
        "{INVITATION_SELECT}
          WHERE tenant_id = $1
            AND ($2::uuid IS NULL OR student_user_id = $2)
          ORDER BY created_at DESC"
    )))
    .bind(tenant_id)
    .bind(student_id)
    .fetch_all(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(rows)
}

/// Revoke a pending invitation (set status='revoked'). Returns true if a pending
/// row was flipped, false if it did not exist / was already non-pending. Scoped
/// to the tenant via both the WHERE clause and RLS.
pub async fn revoke_invitation(pool: &PgPool, tenant_id: Uuid, id: Uuid) -> sqlx::Result<bool> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut *tx)
        .await?;

    let res = sqlx::query(
        "UPDATE parent_invitations
            SET status = 'revoked'
          WHERE id = $1 AND tenant_id = $2 AND status = 'pending'",
    )
    .bind(id)
    .bind(tenant_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(res.rows_affected() > 0)
}

/// Accept every pending parent invitation matching `email`, ACROSS ALL TENANTS,
/// for the just-provisioned `user_id`. Runs inside the caller's JIT-provisioning
/// tx with `app.user_id` bound to `user_id` (there is intentionally no tenant
/// GUC yet). Delegates to the SECURITY DEFINER SQL function so the cross-tenant
/// writes bypass strict per-table RLS while locking each tenant and enforcing
/// its plan. Returns accepted invitations plus the number of tenants whose
/// blocking plan had no seat available. Blocked invitations remain pending.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParentInvitationAcceptance {
    pub accepted: usize,
    pub blocked_tenants: usize,
}

pub async fn accept_pending_for_email(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    email: &str,
) -> sqlx::Result<ParentInvitationAcceptance> {
    let (accepted, blocked): (i32, i32) = sqlx::query_as(
        "SELECT accepted_count, blocked_tenant_count
           FROM accept_parent_invitations_for_email_v2($1, $2)",
    )
    .bind(user_id)
    .bind(email)
    .fetch_one(&mut **tx)
    .await?;
    Ok(ParentInvitationAcceptance {
        accepted: clamp_accept_count(accepted),
        blocked_tenants: clamp_accept_count(blocked),
    })
}

/// Clamp the SECURITY DEFINER fn's signed count to a non-negative `usize`.
/// Defensive against an unexpected negative (the fn never returns one, but the
/// SQL type is INTEGER): a negative would otherwise wrap to a huge `usize`.
fn clamp_accept_count(count: i32) -> usize {
    count.max(0) as usize
}

/// List children linked to `parent_user_id` in the explicitly selected tenant.
/// The membership join is an entitlement check as well as a filter: a stale or
/// forged request context cannot read links after the parent membership is
/// suspended, and links from another workspace are never considered.
pub async fn list_children(
    pool: &PgPool,
    tenant_id: Uuid,
    parent_user_id: Uuid,
) -> sqlx::Result<Vec<ChildRow>> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.user_id', $1, true)")
        .bind(parent_user_id.to_string())
        .execute(&mut *tx)
        .await?;
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut *tx)
        .await?;

    let rows = sqlx::query_as::<_, ChildRow>(
        "SELECT pl.student_user_id,
                u.display_name,
                u.email::text AS email,
                pl.tenant_id
           FROM parent_links pl
           JOIN tenant_memberships tm
             ON tm.tenant_id = pl.tenant_id
            AND tm.user_id = pl.parent_user_id
            AND tm.role = 'parent'
            AND tm.status = 'active'
           LEFT JOIN users u ON u.id = pl.student_user_id
          WHERE pl.parent_user_id = $1
            AND pl.tenant_id = $2
          ORDER BY u.display_name ASC, pl.student_user_id ASC",
    )
    .bind(parent_user_id)
    .bind(tenant_id)
    .fetch_all(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(rows)
}

/// One RELEASED grade for a child, joined with the assignment + course titles.
/// Only `status = 'graded'` AND `released_at IS NOT NULL` rows are returned, and
/// `teacher_only_notes` is NEVER selected — a parent sees exactly the released
/// view a student sees, nothing more.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct ChildGradeRow {
    pub assignment_id: Uuid,
    pub assignment_title: String,
    pub course_title: Option<String>,
    pub status: String,
    pub numeric_grade: Option<sqlx::types::BigDecimal>,
    pub letter_grade: Option<String>,
    pub passed: Option<bool>,
    pub student_visible_feedback: Option<String>,
    pub released_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// List a child's RELEASED grades within `tenant_id`. Runs inside a tx with the
/// tenant GUC set so the `submissions` RLS policy applies under the non-bypass
/// `aulalite_app` role. The caller MUST have already verified that the parent is
/// linked to `student_user_id` (see `is_linked`).
pub async fn list_released_grades_for_student(
    pool: &PgPool,
    tenant_id: Uuid,
    student_user_id: Uuid,
) -> sqlx::Result<Vec<ChildGradeRow>> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut *tx)
        .await?;

    let rows = sqlx::query_as::<_, ChildGradeRow>(
        "SELECT s.assignment_id,
                a.title AS assignment_title,
                c.title AS course_title,
                s.status::text AS status,
                s.numeric_grade,
                s.letter_grade,
                s.passed,
                s.student_visible_feedback,
                s.released_at
           FROM submissions s
           JOIN assignments a ON a.id = s.assignment_id
           LEFT JOIN courses c ON c.id = s.course_id
          WHERE s.tenant_id = $1
            AND s.student_user_id = $2
            AND s.status = 'graded'
            AND s.released_at IS NOT NULL
          ORDER BY s.released_at DESC NULLS LAST, s.assignment_id ASC",
    )
    .bind(tenant_id)
    .bind(student_user_id)
    .fetch_all(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(rows)
}

/// One upcoming scheduled/live session for a course the child is actively
/// enrolled in. Mirrors `handlers::me::ScheduleEntry` but scoped to the CHILD's
/// active `course_memberships`.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct ChildScheduleRow {
    pub session_id: Uuid,
    pub course_id: Uuid,
    pub course_title: String,
    pub title: String,
    pub starts_at: chrono::DateTime<chrono::Utc>,
    pub duration_minutes: i32,
    pub status: String,
}

/// Upcoming (scheduled/live, not yet ended) sessions for the courses the child
/// is actively enrolled in, within `days` from now. Runs with the tenant GUC set
/// so RLS applies. The caller MUST have verified the parent<->child link first.
pub async fn list_upcoming_schedule_for_student(
    pool: &PgPool,
    tenant_id: Uuid,
    student_user_id: Uuid,
    days: i32,
) -> sqlx::Result<Vec<ChildScheduleRow>> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut *tx)
        .await?;

    let rows = sqlx::query_as::<_, ChildScheduleRow>(
        "SELECT ls.id AS session_id, ls.course_id, c.title AS course_title,
                ls.title, ls.starts_at, ls.duration_minutes, ls.status::text AS status
           FROM live_sessions ls
           JOIN course_memberships cm
             ON cm.course_id = ls.course_id
            AND cm.user_id = $2
            AND cm.status = 'active'
           JOIN courses c ON c.id = ls.course_id
          WHERE ls.tenant_id = $1
            AND (ls.starts_at + make_interval(mins => ls.duration_minutes)) >= now()
            AND ls.starts_at <= now() + ($3::int || ' days')::interval
            AND ls.status IN ('scheduled','live')
          ORDER BY ls.starts_at",
    )
    .bind(tenant_id)
    .bind(student_user_id)
    .bind(days)
    .fetch_all(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(rows)
}

/// The student's active tenant id, if they have exactly the tenant we expect.
/// Used by admin parent-invitation creation to verify the named student is an
/// ACTIVE member of the caller's tenant before issuing an invite. Runs with the
/// tenant GUC set so `tenant_memberships` RLS applies.
pub async fn student_is_active_member(
    pool: &PgPool,
    tenant_id: Uuid,
    student_user_id: Uuid,
) -> sqlx::Result<bool> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut *tx)
        .await?;
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM tenant_memberships
          WHERE tenant_id = $1
            AND user_id = $2
            AND status = 'active'",
    )
    .bind(tenant_id)
    .bind(student_user_id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(count > 0)
}

/// Whether the parent is actively entitled and linked to the child in the
/// explicitly selected tenant. Links in other tenants (and links whose parent
/// membership is suspended) cannot authorize a read.
pub async fn is_linked(
    pool: &PgPool,
    tenant_id: Uuid,
    parent_user_id: Uuid,
    student_user_id: Uuid,
) -> sqlx::Result<bool> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.user_id', $1, true)")
        .bind(parent_user_id.to_string())
        .execute(&mut *tx)
        .await?;
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut *tx)
        .await?;

    let linked: bool = sqlx::query_scalar(
        "SELECT EXISTS (
             SELECT 1
               FROM parent_links pl
               JOIN tenant_memberships tm
                 ON tm.tenant_id = pl.tenant_id
                AND tm.user_id = pl.parent_user_id
                AND tm.role = 'parent'
                AND tm.status = 'active'
              WHERE pl.parent_user_id = $1
                AND pl.student_user_id = $2
                AND pl.tenant_id = $3
         )",
    )
    .bind(parent_user_id)
    .bind(student_user_id)
    .bind(tenant_id)
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(linked)
}

#[cfg(test)]
mod tests {
    use super::clamp_accept_count;

    #[test]
    fn clamp_accept_count_passes_through_non_negative() {
        assert_eq!(clamp_accept_count(0), 0);
        assert_eq!(clamp_accept_count(3), 3);
        assert_eq!(clamp_accept_count(i32::MAX), i32::MAX as usize);
    }

    #[test]
    fn clamp_accept_count_floors_negative_at_zero() {
        assert_eq!(clamp_accept_count(-1), 0);
        assert_eq!(clamp_accept_count(i32::MIN), 0);
    }
}
