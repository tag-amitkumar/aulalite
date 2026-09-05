// crates/backend/src/db/billing.rs
//! Billing data layer: the global `plans` catalog, the per-tenant
//! `subscriptions` row, and the global `stripe_events` idempotency ledger.
//!
//! RLS handling mirrors the rest of the codebase:
//!   * `plans` is GLOBAL (no tenant_id, no RLS) — read with no GUC.
//!   * `subscriptions` is TENANT-SCOPED with a strict `tenant_isolation` policy
//!     (see `migrations/20260529000024_billing.sql`); reads/writes therefore run
//!     inside a tx with `app.tenant_id` set, so the policy applies under the
//!     non-bypass `aulalite_app` role.
//!   * `stripe_events` is GLOBAL (no RLS) — the webhook arrives server-to-server
//!     with no tenant context, so dedupe writes run on the pool directly.

use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

/// A row from the global `plans` catalog.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct PlanRow {
    pub id: String,
    pub name: String,
    pub monthly_price_cents: i32,
    pub included_seats: i32,
    pub included_class_minutes: i32,
    pub included_recording_gb: i32,
    pub stripe_price_id: Option<String>,
}

/// A tenant's billing subscription row (PK = tenant_id).
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct SubscriptionRow {
    pub tenant_id: Uuid,
    pub plan_id: String,
    pub status: String,
    pub current_period_start: Option<chrono::DateTime<chrono::Utc>>,
    pub current_period_end: Option<chrono::DateTime<chrono::Utc>>,
    pub trial_ends_at: Option<chrono::DateTime<chrono::Utc>>,
    pub stripe_subscription_id: Option<String>,
    pub overage_behavior: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// The tenant's current Stripe Checkout attempt. The row remains after expiry
/// so an atomic upsert can replace it without a delete/insert race.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct CheckoutIntentRow {
    pub tenant_id: Uuid,
    pub plan_id: String,
    pub customer_email: String,
    pub idempotency_key: String,
    pub stripe_session_id: Option<String>,
    pub checkout_url: Option<String>,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

const PLAN_SELECT: &str = "SELECT id, name, monthly_price_cents, included_seats, \
     included_class_minutes, included_recording_gb, stripe_price_id FROM plans";

const SUBSCRIPTION_COLS: &str = "tenant_id, plan_id, status, current_period_start, \
     current_period_end, trial_ends_at, stripe_subscription_id, overage_behavior, \
     created_at, updated_at";

const CHECKOUT_INTENT_COLS: &str = "tenant_id, plan_id, customer_email, idempotency_key, \
     stripe_session_id, checkout_url, expires_at";

/// List the global plan catalog, ordered by monthly price ascending. `plans`
/// has no RLS, so this reads directly off the pool with no tenant GUC.
pub async fn list_plans(pool: &PgPool) -> sqlx::Result<Vec<PlanRow>> {
    sqlx::query_as::<_, PlanRow>(sqlx::AssertSqlSafe(format!(
        "{PLAN_SELECT} ORDER BY monthly_price_cents ASC"
    )))
    .fetch_all(pool)
    .await
}

/// Fetch a single plan by id, or `None` if no such plan exists.
pub async fn get_plan(pool: &PgPool, id: &str) -> sqlx::Result<Option<PlanRow>> {
    sqlx::query_as::<_, PlanRow>(sqlx::AssertSqlSafe(format!("{PLAN_SELECT} WHERE id = $1")))
        .bind(id)
        .fetch_optional(pool)
        .await
}

/// Fetch the subscription row for `tenant_id`, or `None` if the tenant has no
/// subscription yet. Runs inside a tx with the tenant GUC set so the
/// `tenant_isolation` RLS policy applies.
pub async fn get_subscription(
    pool: &PgPool,
    tenant_id: Uuid,
) -> sqlx::Result<Option<SubscriptionRow>> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut *tx)
        .await?;
    let row = sqlx::query_as::<_, SubscriptionRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {SUBSCRIPTION_COLS} FROM subscriptions WHERE tenant_id = $1"
    )))
    .bind(tenant_id)
    .fetch_optional(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(row)
}

/// Atomically claim the tenant's current Checkout intent. An unexpired row is
/// returned unchanged; an expired row is replaced with the supplied plan,
/// email, key, and expiry. The caller decides whether an active different-plan
/// intent is reusable or a conflict.
pub async fn claim_checkout_intent(
    pool: &PgPool,
    tenant_id: Uuid,
    plan_id: &str,
    customer_email: &str,
    candidate_idempotency_key: &str,
    candidate_expires_at: chrono::DateTime<chrono::Utc>,
) -> sqlx::Result<CheckoutIntentRow> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut *tx)
        .await?;

    let inserted = sqlx::query_as::<_, CheckoutIntentRow>(sqlx::AssertSqlSafe(format!(
        "INSERT INTO stripe_checkout_intents
            (tenant_id, plan_id, customer_email, idempotency_key, expires_at)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (tenant_id) DO UPDATE SET
            plan_id = EXCLUDED.plan_id,
            customer_email = EXCLUDED.customer_email,
            idempotency_key = EXCLUDED.idempotency_key,
            stripe_session_id = NULL,
            checkout_url = NULL,
            expires_at = EXCLUDED.expires_at,
            created_at = now(),
            updated_at = now()
         WHERE stripe_checkout_intents.expires_at <= now()
         RETURNING {CHECKOUT_INTENT_COLS}"
    )))
    .bind(tenant_id)
    .bind(plan_id)
    .bind(customer_email)
    .bind(candidate_idempotency_key)
    .bind(candidate_expires_at)
    .fetch_optional(&mut *tx)
    .await?;

    let row = match inserted {
        Some(row) => row,
        None => {
            sqlx::query_as::<_, CheckoutIntentRow>(sqlx::AssertSqlSafe(format!(
                "SELECT {CHECKOUT_INTENT_COLS}
                   FROM stripe_checkout_intents
                  WHERE tenant_id = $1"
            )))
            .bind(tenant_id)
            .fetch_one(&mut *tx)
            .await?
        }
    };
    tx.commit().await?;
    Ok(row)
}

/// Attach Stripe's result to the still-current intent. If another identical
/// request won the race, return its stored session. `None` means the intent
/// expired or was replaced while the provider request was in flight.
pub async fn complete_checkout_intent(
    pool: &PgPool,
    tenant_id: Uuid,
    idempotency_key: &str,
    stripe_session_id: &str,
    checkout_url: &str,
    provider_expires_at: chrono::DateTime<chrono::Utc>,
) -> sqlx::Result<Option<CheckoutIntentRow>> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut *tx)
        .await?;

    let updated = sqlx::query_as::<_, CheckoutIntentRow>(sqlx::AssertSqlSafe(format!(
        "UPDATE stripe_checkout_intents
            SET stripe_session_id = $3,
                checkout_url = $4,
                expires_at = LEAST(expires_at, $5),
                updated_at = now()
          WHERE tenant_id = $1
            AND idempotency_key = $2
            AND expires_at > now()
            AND checkout_url IS NULL
         RETURNING {CHECKOUT_INTENT_COLS}"
    )))
    .bind(tenant_id)
    .bind(idempotency_key)
    .bind(stripe_session_id)
    .bind(checkout_url)
    .bind(provider_expires_at)
    .fetch_optional(&mut *tx)
    .await?;

    let row = match updated {
        Some(row) => Some(row),
        None => {
            sqlx::query_as::<_, CheckoutIntentRow>(sqlx::AssertSqlSafe(format!(
                "SELECT {CHECKOUT_INTENT_COLS}
                   FROM stripe_checkout_intents
                  WHERE tenant_id = $1
                    AND idempotency_key = $2
                    AND expires_at > now()"
            )))
            .bind(tenant_id)
            .bind(idempotency_key)
            .fetch_optional(&mut *tx)
            .await?
        }
    };
    tx.commit().await?;
    Ok(row)
}

/// Insert-or-update the subscription row for `tenant_id` from a Stripe webhook.
/// `overage_behavior`, `created_at`, and `updated_at` are NOT touched here on
/// conflict beyond bumping `updated_at` — overage is set via
/// [`set_overage_behavior`]. Runs with the tenant GUC set so RLS applies.
#[allow(clippy::too_many_arguments)]
pub async fn upsert_subscription(
    pool: &PgPool,
    tenant_id: Uuid,
    plan_id: &str,
    status: &str,
    period_start: Option<chrono::DateTime<chrono::Utc>>,
    period_end: Option<chrono::DateTime<chrono::Utc>>,
    trial_ends_at: Option<chrono::DateTime<chrono::Utc>>,
    stripe_sub_id: Option<&str>,
) -> sqlx::Result<SubscriptionRow> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut *tx)
        .await?;
    let row = sqlx::query_as::<_, SubscriptionRow>(sqlx::AssertSqlSafe(format!(
        "INSERT INTO subscriptions
            (tenant_id, plan_id, status, current_period_start, current_period_end,
             trial_ends_at, stripe_subscription_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         ON CONFLICT (tenant_id) DO UPDATE SET
            plan_id = EXCLUDED.plan_id,
            status = EXCLUDED.status,
            current_period_start = EXCLUDED.current_period_start,
            current_period_end = EXCLUDED.current_period_end,
            trial_ends_at = EXCLUDED.trial_ends_at,
            stripe_subscription_id = EXCLUDED.stripe_subscription_id,
            updated_at = now()
         RETURNING {SUBSCRIPTION_COLS}"
    )))
    .bind(tenant_id)
    .bind(plan_id)
    .bind(status)
    .bind(period_start)
    .bind(period_end)
    .bind(trial_ends_at)
    .bind(stripe_sub_id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(row)
}

/// Apply a provider subscription event only when it is not older than the
/// event already reflected in the row. Stripe explicitly permits out-of-order
/// delivery, so event-id deduplication alone is insufficient.
#[allow(clippy::too_many_arguments)]
pub async fn apply_stripe_subscription_event(
    pool: &PgPool,
    tenant_id: Uuid,
    plan_id: &str,
    status: &str,
    period_start: Option<chrono::DateTime<chrono::Utc>>,
    period_end: Option<chrono::DateTime<chrono::Utc>>,
    trial_ends_at: Option<chrono::DateTime<chrono::Utc>>,
    stripe_sub_id: Option<&str>,
    event_created_at: chrono::DateTime<chrono::Utc>,
) -> sqlx::Result<bool> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut *tx)
        .await?;
    let result = sqlx::query(
        "INSERT INTO subscriptions
            (tenant_id, plan_id, status, current_period_start, current_period_end,
             trial_ends_at, stripe_subscription_id, stripe_event_created_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         ON CONFLICT (tenant_id) DO UPDATE SET
            plan_id = EXCLUDED.plan_id,
            status = EXCLUDED.status,
            current_period_start = EXCLUDED.current_period_start,
            current_period_end = EXCLUDED.current_period_end,
            trial_ends_at = EXCLUDED.trial_ends_at,
            stripe_subscription_id = EXCLUDED.stripe_subscription_id,
            stripe_event_created_at = EXCLUDED.stripe_event_created_at,
            updated_at = now()
         WHERE subscriptions.stripe_event_created_at IS NULL
            OR subscriptions.stripe_event_created_at <= EXCLUDED.stripe_event_created_at",
    )
    .bind(tenant_id)
    .bind(plan_id)
    .bind(status)
    .bind(period_start)
    .bind(period_end)
    .bind(trial_ends_at)
    .bind(stripe_sub_id)
    .bind(event_created_at)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(result.rows_affected() > 0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StripeStatusEventResult {
    Applied,
    Stale,
    SubscriptionNotFound,
    SubscriptionMismatch,
}

/// Apply invoice-derived status without replacing plan or period data. The
/// invoice's subscription id is checked while the row is locked, preventing an
/// invoice for another subscription on the same customer from changing local
/// entitlements.
pub async fn apply_stripe_status_event(
    pool: &PgPool,
    tenant_id: Uuid,
    invoice_subscription_id: &str,
    status: &str,
    event_created_at: chrono::DateTime<chrono::Utc>,
) -> sqlx::Result<StripeStatusEventResult> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut *tx)
        .await?;
    let existing: Option<(Option<chrono::DateTime<chrono::Utc>>, Option<String>)> = sqlx::query_as(
        "SELECT stripe_event_created_at, stripe_subscription_id
           FROM subscriptions
          WHERE tenant_id = $1
          FOR UPDATE",
    )
    .bind(tenant_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((previous, stored_subscription_id)) = existing else {
        tx.commit().await?;
        return Ok(StripeStatusEventResult::SubscriptionNotFound);
    };
    if stored_subscription_id.as_deref() != Some(invoice_subscription_id) {
        tx.commit().await?;
        return Ok(StripeStatusEventResult::SubscriptionMismatch);
    }
    if previous.is_some_and(|previous| previous > event_created_at) {
        tx.commit().await?;
        return Ok(StripeStatusEventResult::Stale);
    }
    sqlx::query(
        "UPDATE subscriptions
            SET status = $2, stripe_event_created_at = $3, updated_at = now()
          WHERE tenant_id = $1",
    )
    .bind(tenant_id)
    .bind(status)
    .bind(event_created_at)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(StripeStatusEventResult::Applied)
}

/// Set a tenant's overage behavior (`'block'` | `'metered'`). Returns true if a
/// subscription row existed and was updated. Runs with the tenant GUC set.
pub async fn set_overage_behavior(
    pool: &PgPool,
    tenant_id: Uuid,
    behavior: &str,
) -> sqlx::Result<bool> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut *tx)
        .await?;
    let res = sqlx::query(
        "UPDATE subscriptions SET overage_behavior = $2, updated_at = now()
          WHERE tenant_id = $1",
    )
    .bind(tenant_id)
    .bind(behavior)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(res.rows_affected() > 0)
}

/// Usage computed on-read: seats and recording storage are point-in-time;
/// class minutes use the current calendar month.
///
/// All three figures are derived from existing tables — there is no
/// `usage_counters` table (see the migration header). Computed inside a single
/// tenant-scoped tx so the RLS policies on `tenant_memberships`, `live_sessions`
/// and `recordings` apply under the non-bypass `aulalite_app` role.
#[derive(Debug, Clone, Serialize)]
pub struct UsageTotals {
    /// Count of `active` tenant_memberships (the seat figure).
    pub active_seats: i64,
    /// Total live-class wall-clock minutes this calendar month, summed over the
    /// tenant's ended live_sessions (`actual_ended_at - actual_started_at`).
    pub class_minutes_used: i64,
    /// Total bytes in all currently available recordings (sum of linked
    /// file_assets' `size_bytes`); the handler divides by 1e9 to present GB.
    pub recording_bytes_used: i64,
}

/// Compute tenant usage. Seats and recording storage are point-in-time totals;
/// class minutes are bounded to rows whose actual start is in the current
/// calendar month.
///
/// `class_minutes_used` sums each session's actual wall-clock duration
/// (`actual_ended_at - actual_started_at`), which is the natural per-class
/// minute figure; attendee-seconds (`attendance.total_seconds`) are NOT summed
/// here because that would multiply by viewer count. Sessions still live (no
/// `actual_ended_at`) contribute nothing until they end. The aggregate rounds
/// up to the next minute, matching the enforcement boundary.
pub async fn compute_usage(pool: &PgPool, tenant_id: Uuid) -> sqlx::Result<UsageTotals> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut *tx)
        .await?;

    let active_seats: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM tenant_memberships
          WHERE tenant_id = $1 AND status = 'active'",
    )
    .bind(tenant_id)
    .fetch_one(&mut *tx)
    .await?;

    // Wall-clock minutes of ended sessions whose actual start falls in the
    // current month. COALESCE to 0 when there are no rows.
    let class_minutes_used: i64 = sqlx::query_scalar(
        "SELECT COALESCE(
                    CEIL(SUM(EXTRACT(EPOCH FROM (actual_ended_at - actual_started_at))) / 60.0),
                    0)::bigint
           FROM live_sessions
          WHERE tenant_id = $1
            AND actual_started_at IS NOT NULL
            AND actual_ended_at IS NOT NULL
            AND actual_started_at >= date_trunc('month', now())",
    )
    .bind(tenant_id)
    .fetch_one(&mut *tx)
    .await?;

    // Point-in-time bytes of every available recording-linked file_asset. We
    // join via recordings so arbitrary uploads never consume this plan quota.
    let recording_bytes_used: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(fa.size_bytes), 0)::bigint
           FROM recordings r
           JOIN file_assets fa ON fa.id = r.file_asset_id
          WHERE r.tenant_id = $1
            AND fa.status = 'available'",
    )
    .bind(tenant_id)
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(UsageTotals {
        active_seats,
        class_minutes_used,
        recording_bytes_used,
    })
}

/// Resolve a tenant by its `stripe_customer_id`, or `None` if no tenant carries
/// that customer id. Stripe webhooks are server-to-server and have no tenant
/// GUC, so the lookup uses the explicit system SELECT policy. A bare pool would
/// see zero rows under the non-bypass production role.
pub async fn tenant_for_stripe_customer(
    pool: &PgPool,
    customer_id: &str,
) -> sqlx::Result<Option<Uuid>> {
    let mut tx = crate::db::begin_system_context(pool).await?;
    let tenant_id = sqlx::query_scalar("SELECT id FROM tenants WHERE stripe_customer_id = $1")
        .bind(customer_id)
        .fetch_optional(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(tenant_id)
}

/// Remember the first non-empty Stripe customer assigned to a tenant and
/// return the canonical stored id. This compare-and-set closes the race where
/// concurrent Checkout requests both observe a missing customer.
pub async fn remember_tenant_stripe_customer(
    pool: &PgPool,
    tenant_id: Uuid,
    customer_id: &str,
) -> sqlx::Result<Option<String>> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut *tx)
        .await?;
    let stored: Option<Option<String>> = sqlx::query_scalar(
        "UPDATE tenants
            SET stripe_customer_id = COALESCE(NULLIF(btrim(stripe_customer_id), ''), $2),
                updated_at = CASE
                    WHEN NULLIF(btrim(stripe_customer_id), '') IS NULL THEN now()
                    ELSE updated_at
                END
          WHERE id = $1
         RETURNING stripe_customer_id",
    )
    .bind(tenant_id)
    .bind(customer_id)
    .fetch_optional(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(stored.flatten())
}

/// Persist `customer_id` when the tenant has none. Existing customer bindings
/// are deliberately never overwritten by a later webhook.
pub async fn set_tenant_stripe_customer(
    pool: &PgPool,
    tenant_id: Uuid,
    customer_id: &str,
) -> sqlx::Result<()> {
    remember_tenant_stripe_customer(pool, tenant_id, customer_id).await?;
    Ok(())
}

/// Read the existing `stripe_customer_id` for a tenant (set during a prior
/// checkout). Runs with the tenant GUC set so RLS applies.
pub async fn get_tenant_stripe_customer(
    pool: &PgPool,
    tenant_id: Uuid,
) -> sqlx::Result<Option<String>> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut *tx)
        .await?;
    let row: Option<Option<String>> =
        sqlx::query_scalar("SELECT stripe_customer_id FROM tenants WHERE id = $1")
            .bind(tenant_id)
            .fetch_optional(&mut *tx)
            .await?;
    tx.commit().await?;
    Ok(row.flatten())
}

/// Claim a Stripe event for processing. Completed events are permanent no-ops;
/// failed attempts can be released immediately, and an abandoned claim becomes
/// retryable after five minutes so a process crash cannot poison the event id.
pub async fn claim_stripe_event(
    pool: &PgPool,
    event_id: &str,
    event_type: &str,
) -> sqlx::Result<bool> {
    let claimed: Option<String> = sqlx::query_scalar(
        "INSERT INTO stripe_events
            (id, event_type, processing_started_at, attempt_count)
         VALUES ($1, $2, now(), 1)
         ON CONFLICT (id) DO UPDATE SET
            event_type = EXCLUDED.event_type,
            processing_started_at = now(),
            attempt_count = stripe_events.attempt_count + 1,
            last_error = NULL
         WHERE stripe_events.processed_at IS NULL
           AND (
                stripe_events.processing_started_at IS NULL
                OR stripe_events.processing_started_at < now() - interval '5 minutes'
           )
         RETURNING id",
    )
    .bind(event_id)
    .bind(event_type)
    .fetch_optional(pool)
    .await?;
    Ok(claimed.is_some())
}

pub async fn complete_stripe_event(pool: &PgPool, event_id: &str) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE stripe_events
            SET processed_at = now(), processing_started_at = NULL, last_error = NULL
          WHERE id = $1",
    )
    .bind(event_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn release_stripe_event(pool: &PgPool, event_id: &str, error: &str) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE stripe_events
            SET processing_started_at = NULL, last_error = left($2, 2000)
          WHERE id = $1 AND processed_at IS NULL",
    )
    .bind(event_id)
    .bind(error)
    .execute(pool)
    .await?;
    Ok(())
}
