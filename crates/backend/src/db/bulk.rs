// crates/backend/src/db/bulk.rs
//! Bulk-ops data layer: small read/write helpers the bulk roster + grade
//! importers need that are NOT already covered by `db::enrollments`,
//! `db::member_invitations`, `db::submissions`, or `db::assignments`.
//!
//! Everything here is TENANT-SCOPED under RLS: each fn runs inside a tx with
//! the `app.tenant_id` GUC set (mirroring `db::announcements` / `db::attendance`)
//! so the strict `tenant_isolation` policy applies under the non-bypass
//! `aulalite_app` role. Uses ONLY runtime sqlx (no compile-time macros — there
//! is no DATABASE_URL at build).
//!
//! The bulk handlers reuse the single-row write paths where possible
//! (`db::enrollments::insert_invitation`, `db::submissions::save_grade`, …);
//! these helpers cover the lookups the handlers need to decide WHICH path to
//! take per row (resolve an email to a user, find an assignment by title,
//! locate a student's submission, upsert a course membership at a given role).
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

/// Set the tenant GUC the RLS policies depend on, inside `tx`.
async fn set_tenant(tx: &mut Transaction<'_, Postgres>, tenant_id: Uuid) -> sqlx::Result<()> {
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// A user resolved by email, together with whether they currently hold an
/// ACTIVE tenant membership (a seat) in `tenant_id`.
#[derive(Debug, Clone)]
pub struct ResolvedUser {
    pub user_id: Uuid,
    pub is_active_tenant_member: bool,
}

#[derive(Debug, Clone)]
pub enum UserResolution {
    None,
    One(ResolvedUser),
    /// More than one active identity in this tenant uses the supplied email.
    /// The caller must require an explicit user id instead of guessing.
    Ambiguous(usize),
}

/// Resolve an email to a platform user and report whether they are an active
/// member of `tenant_id`. The `users` table is global (not tenant-scoped), so
/// the email lookup itself is not gated by RLS; the membership probe is scoped
/// to the tenant and runs under the tenant GUC. Returns `None` when no user
/// with that email exists yet (the caller then falls back to an invitation).
///
/// Case-insensitive on email to match the invitation flow
/// (`lazy_expire_stale_pending` lowercases both sides).
pub async fn resolve_user_by_email(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    email: &str,
) -> sqlx::Result<UserResolution> {
    set_tenant(tx, tenant_id).await?;
    let active_ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT u.id
           FROM tenant_memberships tm
           JOIN users u ON u.id = tm.user_id
          WHERE tm.tenant_id = $1
            AND tm.status = 'active'
            AND lower(u.email::text) = lower($2)
          ORDER BY u.id
          LIMIT 2",
    )
    .bind(tenant_id)
    .bind(email)
    .fetch_all(&mut **tx)
    .await?;
    match active_ids.as_slice() {
        [user_id] => {
            return Ok(UserResolution::One(ResolvedUser {
                user_id: *user_id,
                is_active_tenant_member: true,
            }));
        }
        ids if ids.len() > 1 => return Ok(UserResolution::Ambiguous(ids.len())),
        _ => {}
    }

    // Only the platform-verified/global identity may be selected before it has
    // a tenant membership. Tenant-controlled identities from another workspace
    // are never candidates, even when they assert the same address.
    let global_ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id
           FROM users
          WHERE identity_tenant_id IS NULL
            AND deleted_at IS NULL
            AND lower(email::text) = lower($1)
          ORDER BY id
          LIMIT 2",
    )
    .bind(email)
    .fetch_all(&mut **tx)
    .await?;
    match global_ids.as_slice() {
        [] => Ok(UserResolution::None),
        [user_id] => Ok(UserResolution::One(ResolvedUser {
            user_id: *user_id,
            is_active_tenant_member: false,
        })),
        ids => Ok(UserResolution::Ambiguous(ids.len())),
    }
}

/// The status of an existing course membership for `(course_id, user_id)`, or
/// `None` when the user is not (yet) a member of the course. Tenant-scoped.
pub async fn course_membership_status(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    course_id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<Option<String>> {
    set_tenant(tx, tenant_id).await?;
    sqlx::query_scalar(
        "SELECT status::text FROM course_memberships
          WHERE course_id = $1 AND user_id = $2",
    )
    .bind(course_id)
    .bind(user_id)
    .fetch_optional(&mut **tx)
    .await
}

/// Upsert a course membership at `role` for a user who already holds a tenant
/// seat. Activates a previously-removed membership and (re)sets the role.
/// Mirrors `db::enrollments::ensure_course_membership_student` but role-aware,
/// matching the role semantics of the single-invite accept path
/// (`handlers::enrollments::accept_invitation_inner`). Tenant-scoped.
pub async fn upsert_course_membership(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    course_id: Uuid,
    user_id: Uuid,
    role: &str,
) -> sqlx::Result<()> {
    set_tenant(tx, tenant_id).await?;
    sqlx::query(
        "INSERT INTO course_memberships (course_id, user_id, tenant_id, role, status)
         VALUES ($1, $2, $3, $4, 'active')
         ON CONFLICT (course_id, user_id) DO UPDATE
            SET role = EXCLUDED.role, status = 'active'",
    )
    .bind(course_id)
    .bind(user_id)
    .bind(tenant_id)
    .bind(role)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Resolve a single published assignment in `course_id` by an exact,
/// case-insensitive title match. Returns:
///   * `Ok(Some(id))` on a unique match,
///   * `Ok(None)`      when nothing matches,
///   * `Err(Ambiguous)`-style: the count is returned so the handler can report
///     "ambiguous title" distinctly. We surface ambiguity by returning the
///     match COUNT alongside the id of the first match.
/// Tenant-scoped under RLS.
pub async fn find_assignment_by_title(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    course_id: Uuid,
    title: &str,
) -> sqlx::Result<TitleMatch> {
    set_tenant(tx, tenant_id).await?;
    let rows: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM assignments
          WHERE course_id = $1 AND lower(title) = lower($2)
          ORDER BY created_at",
    )
    .bind(course_id)
    .bind(title)
    .fetch_all(&mut **tx)
    .await?;
    Ok(match rows.len() {
        0 => TitleMatch::None,
        1 => TitleMatch::One(rows[0]),
        n => TitleMatch::Ambiguous(n),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TitleMatch {
    None,
    One(Uuid),
    Ambiguous(usize),
}

/// Find a student's submission for an assignment, returning `(submission_id,
/// course_id, status)`. `None` when the student has no submission row for that
/// assignment yet (the bulk grader then creates one before grading). Tenant-scoped.
pub async fn find_submission(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    assignment_id: Uuid,
    student_user_id: Uuid,
) -> sqlx::Result<Option<(Uuid, Uuid, String)>> {
    set_tenant(tx, tenant_id).await?;
    sqlx::query_as(
        "SELECT id, course_id, status::text FROM submissions
          WHERE assignment_id = $1 AND student_user_id = $2",
    )
    .bind(assignment_id)
    .bind(student_user_id)
    .fetch_optional(&mut **tx)
    .await
}
