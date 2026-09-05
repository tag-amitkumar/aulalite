// crates/backend/src/db/seats.rs
//! Tenant-wide seat accounting for member-invite + seat-cap enforcement.
//!
//! SEAT MODEL = TENANT-WIDE: an institution (tenant) holds one subscription;
//! every member (teacher/student/ta/parent/org_admin) is a `tenant_memberships`
//! row (a seat). `plan.included_seats` is the total active-member cap and
//! `subscriptions.overage_behavior` decides what happens past it:
//!   * 'block'   — reject the seat-consuming action;
//! Metered overage is intentionally disabled until provider-side usage events
//! and reconciliation exist; every configured cap therefore fails closed.
//!
//! The seat check compares `active_seats + pending_invites` against
//! `included_seats`: a pending invite is a reserved seat, so two admins can't
//! each invite "the last seat" concurrently.
//!
//! Runs inside ONE tenant-scoped tx so the `tenant_isolation` policies on
//! `tenant_memberships`, `tenant_invitations`, and `subscriptions` apply under
//! the non-bypass `aulalite_app` role. `plans` is global (no RLS).

use serde::Serialize;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

/// Stable public/service error code for every blocked seat-consuming path.
pub const SEAT_LIMIT_REACHED: &str = "seat_limit_reached";
pub const MEMBERSHIP_SUSPENDED: &str = "membership_suspended";

/// Result of ensuring that a user occupies an active tenant seat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MembershipActivationOutcome {
    /// The membership was already active. This is always idempotently allowed,
    /// even if the tenant is currently at or above its plan limit.
    AlreadyActive,
    /// A missing or inactive membership was activated and consumed a seat.
    Activated,
    /// An administrator suspended this membership. Self-service activation
    /// paths must not override that decision.
    Suspended,
    /// A blocking plan had no unreserved seat available.
    SeatLimitReached,
}

/// A point-in-time seat picture for a tenant.
#[derive(Debug, Clone, Serialize)]
pub struct SeatUsage {
    /// Count of `active` tenant_memberships (occupied seats).
    pub active_seats: i64,
    /// Distinct invited emails with a reserved seat across tenant-member and
    /// parent invitations. A same-email active identity does not remove the
    /// reservation because enterprise and global identities are not implicitly
    /// linked merely by an email claim.
    pub pending_invites: i64,
    /// The tenant's plan seat cap, or `None` if it has no subscription / plan.
    pub included_seats: Option<i64>,
    /// 'block' | 'metered'. Defaults to 'block' when there is no subscription.
    pub overage_behavior: String,
}

impl SeatUsage {
    /// True when `overage_behavior == 'block'` AND a cap exists AND
    /// `active_seats + pending_invites >= cap`. This is the single source of
    /// truth for "issuing another seat would exceed the plan".
    pub fn would_block(&self) -> bool {
        match self.included_seats {
            Some(cap) => self.active_seats + self.pending_invites >= cap,
            None => false,
        }
    }
}

/// Compute the tenant's current seat usage. Active seats and pending invites are
/// read under the tenant GUC; the cap + overage come from the tenant's
/// subscription joined to its plan (both `None`/default if there is no
/// subscription row).
pub async fn seat_usage(pool: &PgPool, tenant_id: Uuid) -> sqlx::Result<SeatUsage> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut *tx)
        .await?;

    let usage = seat_usage_in_tx(&mut tx, tenant_id).await?;

    tx.commit().await?;
    Ok(usage)
}

/// Transaction-scoped variant used by membership mutations. The caller can
/// first lock the tenant row, then compute usage and update a membership in one
/// serializable critical section instead of racing between separate pools.
pub async fn seat_usage_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
) -> sqlx::Result<SeatUsage> {
    // Read every component in one SQL statement so Postgres gives the counts
    // one MVCC snapshot. Invitation acceptance changes one pending reservation
    // into one active membership; observing those two changes from different
    // statement snapshots could otherwise transiently under-count a seat.
    let (active_seats, pending_invites, included_seats, overage_behavior): (
        i64,
        i64,
        Option<i64>,
        String,
    ) = sqlx::query_as(
        "SELECT
            (SELECT COUNT(*)
               FROM tenant_memberships
              WHERE tenant_id = $1 AND status = 'active'),
            (SELECT COUNT(*)
               FROM (
                   SELECT lower(ti.email::text) AS email_key
                     FROM tenant_invitations ti
                    WHERE ti.tenant_id = $1 AND ti.status = 'pending'
                   UNION
                   SELECT lower(pi.parent_email::text) AS email_key
                     FROM parent_invitations pi
                    WHERE pi.tenant_id = $1
                      AND pi.status = 'pending'
                      AND pi.seat_reserved
               ) reserved),
            (SELECT p.included_seats::bigint
               FROM subscriptions s
               LEFT JOIN plans p ON p.id = s.plan_id
              WHERE s.tenant_id = $1),
            COALESCE(
                (SELECT s.overage_behavior
                   FROM subscriptions s
                  WHERE s.tenant_id = $1),
                'block'
            )",
    )
    .bind(tenant_id)
    .fetch_one(&mut **tx)
    .await?;

    Ok(SeatUsage {
        active_seats,
        pending_invites,
        included_seats,
        overage_behavior,
    })
}

/// Acquire the database-wide serialization lock for seat-consuming mutations
/// in one tenant. Every API replica contends on the same tenant row.
pub async fn lock_tenant_for_seat_mutation(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
) -> sqlx::Result<()> {
    sqlx::query_scalar::<_, Uuid>("SELECT id FROM tenants WHERE id = $1 FOR UPDATE")
        .bind(tenant_id)
        .fetch_one(&mut **tx)
        .await?;
    Ok(())
}

/// Ensure `user_id` has an active tenant membership without exceeding a
/// blocking plan's seat cap.
///
/// The tenant lock, current-membership read, capacity check, and insert/update
/// all happen in the caller's transaction. Existing active memberships return
/// immediately so sign-in and repeated redemption remain idempotent at cap.
/// An `invited` membership may activate and preserves its existing tenant role;
/// a `suspended` membership is never reactivated here. Only a brand-new
/// membership receives `default_role`.
pub async fn ensure_active_tenant_membership(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    user_id: Uuid,
    default_role: &str,
) -> sqlx::Result<MembershipActivationOutcome> {
    lock_tenant_for_seat_mutation(tx, tenant_id).await?;

    let current_status: Option<String> = sqlx::query_scalar(
        "SELECT status
           FROM tenant_memberships
          WHERE tenant_id = $1 AND user_id = $2
          FOR UPDATE",
    )
    .bind(tenant_id)
    .bind(user_id)
    .fetch_optional(&mut **tx)
    .await?;

    if current_status.as_deref() == Some("active") {
        return Ok(MembershipActivationOutcome::AlreadyActive);
    }
    if current_status.as_deref() == Some("suspended") {
        return Ok(MembershipActivationOutcome::Suspended);
    }

    if seat_usage_in_tx(tx, tenant_id).await?.would_block() {
        return Ok(MembershipActivationOutcome::SeatLimitReached);
    }

    match current_status {
        Some(_) => {
            sqlx::query(
                "UPDATE tenant_memberships
                    SET status = 'active', updated_at = now()
                  WHERE tenant_id = $1 AND user_id = $2",
            )
            .bind(tenant_id)
            .bind(user_id)
            .execute(&mut **tx)
            .await?;
        }
        None => {
            sqlx::query(
                "INSERT INTO tenant_memberships (tenant_id, user_id, role, status)
                 VALUES ($1, $2, $3, 'active')",
            )
            .bind(tenant_id)
            .bind(user_id)
            .bind(default_role)
            .execute(&mut **tx)
            .await?;
        }
    }

    Ok(MembershipActivationOutcome::Activated)
}

#[cfg(test)]
mod tests {
    use super::SeatUsage;

    fn usage(active: i64, pending: i64, cap: Option<i64>, behavior: &str) -> SeatUsage {
        SeatUsage {
            active_seats: active,
            pending_invites: pending,
            included_seats: cap,
            overage_behavior: behavior.to_string(),
        }
    }

    #[test]
    fn blocks_at_cap_when_block() {
        // 1 active + 0 pending, cap 1 -> at cap -> block.
        assert!(usage(1, 0, Some(1), "block").would_block());
        // active + pending counts toward the cap.
        assert!(usage(0, 1, Some(1), "block").would_block());
        // over cap also blocks.
        assert!(usage(2, 0, Some(1), "block").would_block());
    }

    #[test]
    fn allows_below_cap() {
        assert!(!usage(0, 0, Some(1), "block").would_block());
        assert!(!usage(1, 0, Some(2), "block").would_block());
    }

    #[test]
    fn legacy_metered_value_still_fails_closed() {
        assert!(usage(5, 5, Some(1), "metered").would_block());
    }

    #[test]
    fn no_cap_never_blocks() {
        assert!(!usage(100, 100, None, "block").would_block());
    }
}
