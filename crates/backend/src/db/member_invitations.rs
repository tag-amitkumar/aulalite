// crates/backend/src/db/member_invitations.rs
//! Tenant member-invite data layer: invitations that (re)create
//! `tenant_memberships` (seats) under a role.
//!
//! Two paths with different RLS handling, mirroring `db::parent`:
//!   * Staff-facing reads/writes (`create_invitation`, `list_invitations`,
//!     `revoke_invitation`) run inside a tx with the tenant GUC set, so the
//!     strict `tenant_isolation` policy on `tenant_invitations` applies under
//!     the non-bypass `aulalite_app` role.
//!   * Acceptance (`accept_pending_for_email`) runs INSIDE the JIT-provisioning
//!     tx, where no per-request tenant context exists and several tenants may be
//!     touched at once. It delegates to the `accept_tenant_invitations_for_email`
//!     SECURITY DEFINER function (migration `20260529000025_tenant_invitations.sql`).
//!
//! SEAT NOTE: normal member invitations use
//! [`create_invitation_with_seat_reservation`], which locks the tenant row and
//! performs the capacity check plus pending-invitation insert in one
//! transaction. Platform-created workspaces and their first administrator are
//! provisioned by one database function, so no invitation path bypasses plan
//! enforcement here. Acceptance does no seat math because it atomically converts an
//! already-reserved pending invitation into an active membership.
use serde::Serialize;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

/// A pending/accepted/revoked tenant member invitation row.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct InvitationRow {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub email: String,
    pub role: String,
    pub status: String,
    pub created_by: Option<Uuid>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub accepted_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Result of atomically reserving a seat with a tenant invitation.
#[derive(Debug)]
pub enum SeatReservationOutcome {
    /// A new invitation was inserted, or the existing pending invitation for
    /// this email was returned (idempotent retry).
    Reserved(InvitationRow),
    /// The tenant uses blocking overages and has no unreserved seat left.
    SeatLimitReached,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevokeInvitationOutcome {
    Revoked,
    NotFound,
    PrivilegedRole,
}

const INVITATION_SELECT: &str = "SELECT id, tenant_id, email::text AS email, role, status, \
     created_by, created_at, accepted_at FROM tenant_invitations";

const INVITATION_RETURNING: &str = "id, tenant_id, email::text AS email, role, status, \
     created_by, created_at, accepted_at";

/// Create a pending invitation only when the tenant has capacity for another
/// reserved seat.
///
/// The tenant row lock is the shared serialization primitive also used by
/// membership reactivation. Consequently, two admins inviting different
/// emails cannot both observe and reserve the final seat. An existing pending
/// invite is returned before the capacity check: retrying the same request is
/// idempotent and does not reserve another seat.
pub async fn create_invitation_with_seat_reservation(
    pool: &PgPool,
    tenant_id: Uuid,
    email: &str,
    role: &str,
    created_by: Uuid,
) -> sqlx::Result<SeatReservationOutcome> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.user_id', $1, true)")
        .bind(created_by.to_string())
        .execute(&mut *tx)
        .await?;
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut *tx)
        .await?;

    // Serialize all seat-consuming mutations for this tenant. This is a real
    // row lock (rather than an in-process mutex), so it covers every backend
    // replica and releases automatically with the transaction.
    crate::db::seats::lock_tenant_for_seat_mutation(&mut tx, tenant_id).await?;

    // A retry of an already-reserved email consumes no additional capacity.
    // Return it before evaluating the cap, including when the first request
    // filled the tenant's final seat.
    if let Some(existing) = sqlx::query_as::<_, InvitationRow>(sqlx::AssertSqlSafe(format!(
        "{INVITATION_SELECT}
          WHERE tenant_id = $1
            AND lower(email::text) = lower($2)
            AND status = 'pending'"
    )))
    .bind(tenant_id)
    .bind(email)
    .fetch_optional(&mut *tx)
    .await?
    {
        tx.commit().await?;
        return Ok(SeatReservationOutcome::Reserved(existing));
    }

    let usage = crate::db::seats::seat_usage_in_tx(&mut tx, tenant_id).await?;
    if usage.would_block() {
        // No state changed, but an explicit rollback releases the row lock
        // before returning rather than waiting for Transaction::drop.
        tx.rollback().await?;
        return Ok(SeatReservationOutcome::SeatLimitReached);
    }

    let invitation = sqlx::query_as::<_, InvitationRow>(sqlx::AssertSqlSafe(format!(
        "INSERT INTO tenant_invitations (tenant_id, email, role, created_by)
         VALUES ($1, $2, $3, $4)
         RETURNING {INVITATION_RETURNING}"
    )))
    .bind(tenant_id)
    .bind(email)
    .bind(role)
    .bind(created_by)
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(SeatReservationOutcome::Reserved(invitation))
}

/// List a tenant's member invitations, newest first. Runs with the tenant GUC
/// set so RLS applies.
pub async fn list_invitations(pool: &PgPool, tenant_id: Uuid) -> sqlx::Result<Vec<InvitationRow>> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut *tx)
        .await?;

    let rows = sqlx::query_as::<_, InvitationRow>(sqlx::AssertSqlSafe(format!(
        "{INVITATION_SELECT} WHERE tenant_id = $1 ORDER BY created_at DESC"
    )))
    .bind(tenant_id)
    .fetch_all(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(rows)
}

/// Revoke a pending invitation with the actor identity present in transaction
/// context for the database authorization trigger. Organization-owner invites
/// cannot be revoked through this path, and organization-admin invites require
/// an active organization owner. The row is scoped by both the WHERE clause and
/// tenant RLS.
pub async fn revoke_invitation(
    pool: &PgPool,
    tenant_id: Uuid,
    id: Uuid,
    actor_user_id: Uuid,
) -> sqlx::Result<RevokeInvitationOutcome> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.user_id', $1, true)")
        .bind(actor_user_id.to_string())
        .execute(&mut *tx)
        .await?;
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut *tx)
        .await?;

    let invitation: Option<(String,)> = sqlx::query_as(
        "SELECT role FROM tenant_invitations
          WHERE id = $1 AND tenant_id = $2 AND status = 'pending'
          FOR UPDATE",
    )
    .bind(id)
    .bind(tenant_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((role,)) = invitation else {
        tx.rollback().await?;
        return Ok(RevokeInvitationOutcome::NotFound);
    };
    if role == "org_owner" {
        tx.rollback().await?;
        return Ok(RevokeInvitationOutcome::PrivilegedRole);
    }
    if role == "org_admin" {
        let actor_is_owner: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                SELECT 1 FROM tenant_memberships
                 WHERE tenant_id = $1 AND user_id = $2
                   AND role = 'org_owner' AND status = 'active'
            )",
        )
        .bind(tenant_id)
        .bind(actor_user_id)
        .fetch_one(&mut *tx)
        .await?;
        if !actor_is_owner {
            tx.rollback().await?;
            return Ok(RevokeInvitationOutcome::PrivilegedRole);
        }
    }

    sqlx::query(
        "UPDATE tenant_invitations
            SET status = 'revoked'
          WHERE id = $1 AND tenant_id = $2 AND status = 'pending'",
    )
    .bind(id)
    .bind(tenant_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(RevokeInvitationOutcome::Revoked)
}

/// Accept every pending tenant invitation matching `email`, ACROSS ALL TENANTS,
/// for the just-provisioned `user_id`. Runs inside the caller's JIT-provisioning
/// tx with `app.user_id` bound to `user_id` (there is intentionally no tenant
/// GUC yet). Delegates to the SECURITY DEFINER SQL function so the cross-tenant
/// writes bypass the strict per-table RLS policy. Returns the number accepted.
pub async fn accept_pending_for_email(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    email: &str,
) -> sqlx::Result<usize> {
    let count: i32 = sqlx::query_scalar("SELECT accept_tenant_invitations_for_email($1, $2)")
        .bind(user_id)
        .bind(email)
        .fetch_one(&mut **tx)
        .await?;
    Ok(clamp_accept_count(count))
}

/// Clamp the SECURITY DEFINER fn's signed count to a non-negative `usize`.
/// Defensive against an unexpected negative (the fn never returns one, but the
/// SQL type is INTEGER): a negative would otherwise wrap to a huge `usize`.
fn clamp_accept_count(count: i32) -> usize {
    count.max(0) as usize
}

#[cfg(test)]
mod tests {
    use super::clamp_accept_count;

    #[test]
    fn clamp_accept_count_passes_through_non_negative() {
        assert_eq!(clamp_accept_count(0), 0);
        assert_eq!(clamp_accept_count(2), 2);
        assert_eq!(clamp_accept_count(i32::MAX), i32::MAX as usize);
    }

    #[test]
    fn clamp_accept_count_floors_negative_at_zero() {
        assert_eq!(clamp_accept_count(-1), 0);
        assert_eq!(clamp_accept_count(i32::MIN), 0);
    }
}
