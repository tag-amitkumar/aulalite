//! Transactional enforcement for monthly live-class minutes and total stored
//! recording bytes.
//!
//! Quota-consuming mutations serialize on the tenant row. Recording uploads
//! additionally reserve their exact byte size before the object-store write,
//! so concurrent workers cannot both consume the final available capacity.

use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

/// An absent legacy entitlement, or an explicit plan value of `-1`, preserves
/// the pre-quota unlimited behavior. Production-created workspaces always have
/// a subscription, while this fallback keeps old/self-hosted data operable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuotaDecision {
    Unlimited,
    Allowed {
        used: i64,
        requested: i64,
        limit: i64,
    },
    LimitReached {
        used: i64,
        requested: i64,
        limit: i64,
    },
}

impl QuotaDecision {
    pub fn is_limit_reached(self) -> bool {
        matches!(self, Self::LimitReached { .. })
    }
}

fn decide(limit: Option<i64>, used: i64, requested: i64) -> QuotaDecision {
    let Some(limit) = limit else {
        return QuotaDecision::Unlimited;
    };
    if limit < 0 {
        return QuotaDecision::Unlimited;
    }

    let used = used.max(0);
    let requested = requested.max(0);
    let exceeds = used > limit || requested > limit.saturating_sub(used);
    if exceeds {
        QuotaDecision::LimitReached {
            used,
            requested,
            limit,
        }
    } else {
        QuotaDecision::Allowed {
            used,
            requested,
            limit,
        }
    }
}

async fn lock_tenant(tx: &mut Transaction<'_, Postgres>, tenant_id: Uuid) -> sqlx::Result<()> {
    sqlx::query_scalar::<_, Uuid>("SELECT id FROM tenants WHERE id = $1 FOR UPDATE")
        .bind(tenant_id)
        .fetch_one(&mut **tx)
        .await?;
    Ok(())
}

async fn plan_limits(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
) -> sqlx::Result<Option<(i64, i64)>> {
    sqlx::query_as(
        "SELECT p.included_class_minutes::bigint,
                p.included_recording_gb::bigint
           FROM subscriptions s
           JOIN plans p ON p.id = s.plan_id
          WHERE s.tenant_id = $1",
    )
    .bind(tenant_id)
    .fetch_optional(&mut **tx)
    .await
}

/// Check and reserve (via the tenant lock held by the caller's transaction)
/// the scheduled duration of a session that is about to transition to live.
/// Ended sessions contribute their actual wall-clock time; other currently
/// live sessions contribute their full scheduled duration. That prevents
/// concurrent teachers from each starting a class against the same remaining
/// minutes while still releasing unused reserved minutes when a class ends.
pub async fn class_minutes_for_start(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    session_id: Uuid,
    requested_minutes: i64,
) -> sqlx::Result<QuotaDecision> {
    lock_tenant(tx, tenant_id).await?;
    let limit = plan_limits(tx, tenant_id).await?.map(|limits| limits.0);
    if limit.is_none_or(|value| value < 0) {
        return Ok(QuotaDecision::Unlimited);
    }

    // Keep the period boundary identical to db::billing::compute_usage.
    // CEIL avoids granting a free partial minute at the enforcement boundary.
    let used: i64 = sqlx::query_scalar(
        "SELECT COALESCE(
                    CEIL(SUM(
                        CASE
                            WHEN status = 'ended'
                                 AND actual_ended_at IS NOT NULL
                                THEN EXTRACT(EPOCH FROM (actual_ended_at - actual_started_at)) / 60.0
                            WHEN status = 'live'
                                THEN duration_minutes::numeric
                            ELSE 0
                        END
                    )),
                    0
                )::bigint
           FROM live_sessions
          WHERE tenant_id = $1
            AND id <> $2
            AND actual_started_at IS NOT NULL
            AND actual_started_at >= date_trunc('month', now())",
    )
    .bind(tenant_id)
    .bind(session_id)
    .fetch_one(&mut **tx)
    .await?;

    Ok(decide(limit, used, requested_minutes))
}

async fn recording_capacity_in_locked_tx(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    recording_id: Option<Uuid>,
    requested_bytes: i64,
) -> sqlx::Result<QuotaDecision> {
    // Expired reservations are ignored by the SUM and removed opportunistically
    // so a crashed worker cannot consume quota forever.
    sqlx::query(
        "DELETE FROM recording_storage_reservations
          WHERE tenant_id = $1 AND expires_at <= now()",
    )
    .bind(tenant_id)
    .execute(&mut **tx)
    .await?;

    let limit_gb = plan_limits(tx, tenant_id).await?.map(|limits| limits.1);
    let limit_bytes = limit_gb.map(|gb| {
        if gb < 0 {
            -1
        } else {
            gb.saturating_mul(1_000_000_000)
        }
    });
    if limit_bytes.is_none_or(|value| value < 0) {
        return Ok(QuotaDecision::Unlimited);
    }

    let used: i64 = sqlx::query_scalar(
        "SELECT
            COALESCE((
                SELECT SUM(fa.size_bytes)::bigint
                  FROM recordings r
                  JOIN file_assets fa ON fa.id = r.file_asset_id
                 WHERE r.tenant_id = $1
                   AND fa.status = 'available'
            ), 0)
            +
            COALESCE((
                SELECT SUM(rsr.size_bytes)::bigint
                  FROM recording_storage_reservations rsr
                 WHERE rsr.tenant_id = $1
                   AND rsr.expires_at > now()
                   AND ($2::uuid IS NULL OR rsr.recording_id <> $2)
            ), 0)",
    )
    .bind(tenant_id)
    .bind(recording_id)
    .fetch_one(&mut **tx)
    .await?;

    Ok(decide(limit_bytes, used, requested_bytes))
}

/// Lightweight start/retry gate. `requested_bytes` should be at least one so
/// an exactly-full plan is rejected even before the final MP4 size is known.
pub async fn recording_storage_capacity(
    pool: &PgPool,
    tenant_id: Uuid,
    requested_bytes: i64,
) -> sqlx::Result<QuotaDecision> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut *tx)
        .await?;
    lock_tenant(&mut tx, tenant_id).await?;
    let decision =
        recording_capacity_in_locked_tx(&mut tx, tenant_id, None, requested_bytes).await?;
    tx.commit().await?;
    Ok(decision)
}

/// Transaction-scoped form used by the go-live mutation. The tenant row stays
/// locked until the session transition commits.
pub async fn recording_storage_capacity_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    requested_bytes: i64,
) -> sqlx::Result<QuotaDecision> {
    lock_tenant(tx, tenant_id).await?;
    recording_capacity_in_locked_tx(tx, tenant_id, None, requested_bytes).await
}

/// Reserve exact output bytes before uploading the MP4. Finite plans get a
/// one-hour lease; unlimited plans need no reservation.
pub async fn reserve_recording_storage(
    pool: &PgPool,
    tenant_id: Uuid,
    recording_id: Uuid,
    size_bytes: i64,
) -> sqlx::Result<QuotaDecision> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut *tx)
        .await?;
    lock_tenant(&mut tx, tenant_id).await?;
    let decision =
        recording_capacity_in_locked_tx(&mut tx, tenant_id, Some(recording_id), size_bytes).await?;
    if matches!(decision, QuotaDecision::Allowed { .. }) {
        sqlx::query(
            "INSERT INTO recording_storage_reservations
                (recording_id, tenant_id, size_bytes, expires_at)
             VALUES ($1, $2, $3, now() + interval '1 hour')
             ON CONFLICT (recording_id) DO UPDATE SET
                size_bytes = EXCLUDED.size_bytes,
                expires_at = EXCLUDED.expires_at,
                updated_at = now()",
        )
        .bind(recording_id)
        .bind(tenant_id)
        .bind(size_bytes)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(decision)
}

/// Re-check a reservation at the final DB mutation boundary. This protects
/// against a plan downgrade while the upload was in flight.
pub async fn validate_recording_finalization(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    recording_id: Uuid,
    size_bytes: i64,
) -> sqlx::Result<QuotaDecision> {
    lock_tenant(tx, tenant_id).await?;
    recording_capacity_in_locked_tx(tx, tenant_id, Some(recording_id), size_bytes).await
}

pub async fn consume_recording_reservation(
    tx: &mut Transaction<'_, Postgres>,
    recording_id: Uuid,
) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM recording_storage_reservations WHERE recording_id = $1")
        .bind(recording_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

pub async fn release_recording_reservation(
    pool: &PgPool,
    tenant_id: Uuid,
    recording_id: Uuid,
) -> sqlx::Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut *tx)
        .await?;
    consume_recording_reservation(&mut tx, recording_id).await?;
    tx.commit().await
}

#[cfg(test)]
mod tests {
    use super::{decide, QuotaDecision};

    #[test]
    fn no_entitlement_and_negative_sentinel_remain_unlimited() {
        assert_eq!(decide(None, i64::MAX, i64::MAX), QuotaDecision::Unlimited);
        assert_eq!(
            decide(Some(-1), i64::MAX, i64::MAX),
            QuotaDecision::Unlimited
        );
    }

    #[test]
    fn exact_limit_is_allowed_but_one_more_unit_is_blocked() {
        assert_eq!(
            decide(Some(100), 40, 60),
            QuotaDecision::Allowed {
                used: 40,
                requested: 60,
                limit: 100
            }
        );
        assert_eq!(
            decide(Some(100), 40, 61),
            QuotaDecision::LimitReached {
                used: 40,
                requested: 61,
                limit: 100
            }
        );
    }

    #[test]
    fn zero_quota_disables_resource_consumption() {
        assert!(decide(Some(0), 0, 1).is_limit_reached());
    }

    #[test]
    fn overflow_cannot_wrap_into_an_allow() {
        assert!(decide(Some(i64::MAX), i64::MAX, 1).is_limit_reached());
    }
}
