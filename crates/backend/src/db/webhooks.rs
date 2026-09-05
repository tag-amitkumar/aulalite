// crates/backend/src/db/webhooks.rs
//! Outbound webhook subscriptions + delivery queue.
//!
//! Two tenant-scoped tables (migration 054):
//!   * `webhook_subscriptions` — a tenant's registered endpoint URLs, the
//!     per-subscription signing `secret`, and the `events` it wants.
//!   * `webhook_deliveries`    — one row per (event × subscription) enqueued for
//!     delivery, carrying the JSON payload, attempt count, status, and the last
//!     response code.
//!
//! TENANT-SCOPED under RLS exactly like `db::announcements`: every request-path
//! read/write runs inside a tx with the `app.tenant_id` GUC set so the strict
//! `tenant_isolation` policy applies under the non-bypass `aulalite_app` role.
//!
//! The DELIVERY WORKER (`services::webhook_delivery::run_delivery_worker`) is a
//! trusted in-process background loop and reads/updates across ALL tenants. Its
//! functions here open a `db::begin_system_context` tx (sets `app.system='on'`),
//! which the `system_context` SELECT/UPDATE policies added in migration 054
//! permit. Request handlers NEVER set that GUC, so per-tenant isolation is
//! unchanged for normal traffic.
//!
//! Uses ONLY runtime sqlx (no compile-time macros).
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

/// A webhook subscription as surfaced to the admin UI (NEVER includes `secret`).
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct SubscriptionRow {
    pub id: Uuid,
    pub url: String,
    pub events: Vec<String>,
    pub active: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// A delivery attempt record as surfaced to the admin UI.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct DeliveryRow {
    pub id: Uuid,
    pub subscription_id: Uuid,
    pub event: String,
    pub payload_json: serde_json::Value,
    pub status: String,
    pub attempts: i32,
    pub last_attempt_at: Option<chrono::DateTime<chrono::Utc>>,
    pub response_code: Option<i32>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// A pending delivery the worker claims, joined with its subscription's URL +
/// secret so the worker can sign and POST without a second query.
#[derive(Debug, sqlx::FromRow)]
pub struct PendingDelivery {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub subscription_id: Uuid,
    pub event: String,
    pub payload_json: serde_json::Value,
    pub attempts: i32,
    pub url: String,
    pub secret: String,
}

/// Set the tenant GUC the RLS policy depends on, inside `tx`.
async fn set_tenant(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: Uuid,
) -> sqlx::Result<()> {
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut **tx)
        .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Subscriptions (request-path, tenant-scoped)
// ---------------------------------------------------------------------------

/// Insert a subscription with a freshly-generated `secret` and return the
/// public row (the secret is returned separately by the handler once). Tenant-scoped.
pub async fn insert_subscription(
    pool: &PgPool,
    tenant_id: Uuid,
    url: &str,
    secret: &str,
    events: &[String],
) -> sqlx::Result<SubscriptionRow> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    let row = sqlx::query_as::<_, SubscriptionRow>(
        "INSERT INTO webhook_subscriptions (tenant_id, url, secret, events)
         VALUES ($1, $2, $3, $4)
         RETURNING id, url, events, active, created_at",
    )
    .bind(tenant_id)
    .bind(url)
    .bind(secret)
    .bind(events)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(row)
}

/// List a tenant's subscriptions, newest first. Tenant-scoped.
pub async fn list_subscriptions(
    pool: &PgPool,
    tenant_id: Uuid,
) -> sqlx::Result<Vec<SubscriptionRow>> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    let rows = sqlx::query_as::<_, SubscriptionRow>(
        "SELECT id, url, events, active, created_at
           FROM webhook_subscriptions
          WHERE tenant_id = $1
          ORDER BY created_at DESC, id DESC",
    )
    .bind(tenant_id)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(rows)
}

/// Patch a subscription's url/events/active flags (each `None` leaves the column
/// as-is). Returns the updated public row, or None if not in this tenant.
/// Tenant-scoped.
pub async fn update_subscription(
    pool: &PgPool,
    tenant_id: Uuid,
    id: Uuid,
    url: Option<&str>,
    events: Option<&[String]>,
    active: Option<bool>,
) -> sqlx::Result<Option<SubscriptionRow>> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    let events_vec: Option<Vec<String>> = events.map(|e| e.to_vec());
    let row = sqlx::query_as::<_, SubscriptionRow>(
        "UPDATE webhook_subscriptions
            SET url    = COALESCE($2, url),
                events = COALESCE($3, events),
                active = COALESCE($4, active)
          WHERE id = $1
        RETURNING id, url, events, active, created_at",
    )
    .bind(id)
    .bind(url)
    .bind(events_vec)
    .bind(active)
    .fetch_optional(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(row)
}

/// Delete a subscription by id within `tenant_id`. The deliveries FK is
/// ON DELETE CASCADE, so its queued/sent rows go with it. Returns true if a row
/// was removed. Tenant-scoped.
pub async fn delete_subscription(pool: &PgPool, tenant_id: Uuid, id: Uuid) -> sqlx::Result<bool> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    let res = sqlx::query("DELETE FROM webhook_subscriptions WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(res.rows_affected() > 0)
}

/// List a tenant's recent deliveries (optionally for one subscription), newest
/// first, capped at `limit`. Tenant-scoped.
pub async fn list_deliveries(
    pool: &PgPool,
    tenant_id: Uuid,
    subscription_id: Option<Uuid>,
    limit: i64,
) -> sqlx::Result<Vec<DeliveryRow>> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    // Separate query shapes keep the hot log reads sargable. The previous
    // `($1 IS NULL OR subscription_id = $1)` predicate prevented a reliable
    // index plan, and relying only on the RLS cast hid tenant_id from the index.
    let rows = if let Some(subscription_id) = subscription_id {
        sqlx::query_as::<_, DeliveryRow>(
            "SELECT id, subscription_id, event, payload_json, status, attempts,
                    last_attempt_at, response_code, created_at
               FROM webhook_deliveries
              WHERE tenant_id = $1
                AND subscription_id = $2
              ORDER BY created_at DESC, id DESC
              LIMIT $3",
        )
        .bind(tenant_id)
        .bind(subscription_id)
        .bind(limit)
        .fetch_all(&mut *tx)
        .await?
    } else {
        sqlx::query_as::<_, DeliveryRow>(
            "SELECT id, subscription_id, event, payload_json, status, attempts,
                    last_attempt_at, response_code, created_at
               FROM webhook_deliveries
              WHERE tenant_id = $1
              ORDER BY created_at DESC, id DESC
              LIMIT $2",
        )
        .bind(tenant_id)
        .bind(limit)
        .fetch_all(&mut *tx)
        .await?
    };
    tx.commit().await?;
    Ok(rows)
}

// ---------------------------------------------------------------------------
// Enqueue (request-path / emit_event, tenant-scoped)
// ---------------------------------------------------------------------------

/// Enqueue a `pending` delivery for every ACTIVE subscription in `tenant_id`
/// that is subscribed to `event` (a row in `events` array). Returns the number
/// of deliveries enqueued. Tenant-scoped — this runs on the request connection
/// inside `emit_event`, so the tenant GUC is set here.
pub async fn enqueue_for_event(
    pool: &PgPool,
    tenant_id: Uuid,
    event: &str,
    payload: &serde_json::Value,
) -> sqlx::Result<u64> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    let res = sqlx::query(
        "INSERT INTO webhook_deliveries
             (tenant_id, subscription_id, event, payload_json, status)
         SELECT s.tenant_id, s.id, $2, $3, 'pending'
           FROM webhook_subscriptions s
          WHERE s.tenant_id = $1
            AND s.active
            AND $2 = ANY(s.events)",
    )
    .bind(tenant_id)
    .bind(event)
    .bind(payload)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(res.rows_affected())
}

// ---------------------------------------------------------------------------
// Delivery worker (cross-tenant, system context)
// ---------------------------------------------------------------------------

/// Claim up to `limit` deliveries that are due for an attempt: `pending`, or
/// `retrying` whose backoff window has elapsed (`last_attempt_at + backoff`
/// where backoff grows with `attempts`). Marks them `sending` atomically (so a
/// second worker tick won't double-send) and returns them joined with the
/// subscription URL + secret. CROSS-TENANT via the system context.
///
/// Backoff schedule (capped): 1m, 5m, 15m, 1h, 6h. After `max_attempts` the row
/// is left as `failed` by `mark_result`, never re-claimed here.
pub async fn claim_due_deliveries(pool: &PgPool, limit: i64) -> sqlx::Result<Vec<PendingDelivery>> {
    let mut tx = crate::db::begin_system_context(pool).await?;
    let rows = sqlx::query_as::<_, PendingDelivery>(
        "WITH due AS (
             SELECT d.id
               FROM webhook_deliveries d
               JOIN webhook_subscriptions s ON s.id = d.subscription_id
              WHERE s.active
                AND (
                    d.status = 'pending'
                    OR (
                        d.status = 'sending'
                        AND d.last_attempt_at IS NOT NULL
                        AND d.last_attempt_at < now() - interval '5 minutes'
                    )
                    OR (
                        d.status = 'retrying'
                        AND d.last_attempt_at IS NOT NULL
                        AND now() >= d.last_attempt_at + (
                            CASE
                                WHEN d.attempts <= 1 THEN interval '1 minute'
                                WHEN d.attempts = 2 THEN interval '5 minutes'
                                WHEN d.attempts = 3 THEN interval '15 minutes'
                                WHEN d.attempts = 4 THEN interval '1 hour'
                                ELSE interval '6 hours'
                            END
                        )
                    )
                )
              ORDER BY d.created_at ASC
              LIMIT $1
              FOR UPDATE OF d SKIP LOCKED
         )
         UPDATE webhook_deliveries d
            SET status = 'sending',
                last_attempt_at = now()
           FROM due, webhook_subscriptions s
          WHERE d.id = due.id
            AND s.id = d.subscription_id
        RETURNING d.id, d.tenant_id, d.subscription_id, d.event,
                  d.payload_json, d.attempts, s.url, s.secret",
    )
    .bind(limit)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(rows)
}

/// Record the outcome of a delivery attempt. CROSS-TENANT via system context.
///
/// Increments `attempts`, stamps `last_attempt_at`, records `response_code`, and
/// sets the next `status`:
///   * success                                  → `delivered`
///   * failure AND attempts (now) < max_attempts → `retrying` (re-claimed later)
///   * failure AND attempts (now) >= max_attempts → `failed` (terminal)
pub async fn mark_result(
    pool: &PgPool,
    delivery_id: Uuid,
    success: bool,
    response_code: Option<i32>,
    max_attempts: i32,
) -> sqlx::Result<()> {
    let mut tx = crate::db::begin_system_context(pool).await?;
    sqlx::query(
        "UPDATE webhook_deliveries
            SET attempts        = attempts + 1,
                last_attempt_at = now(),
                response_code   = $3,
                status = CASE
                    WHEN $2 THEN 'delivered'
                    WHEN attempts + 1 >= $4 THEN 'failed'
                    ELSE 'retrying'
                END
          WHERE id = $1",
    )
    .bind(delivery_id)
    .bind(success)
    .bind(response_code)
    .bind(max_attempts)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}
