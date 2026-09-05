// crates/backend/src/db/notifications.rs
//! Notifications data layer: in-app notification feed, push device tokens, and
//! per-user notification preferences.
//!
//! Two RLS regimes, mirroring `db::member_invitations` + `db::billing`:
//!   * `notifications` + `device_tokens` are TENANT-SCOPED. Every read/write
//!     runs inside a tx with BOTH `app.tenant_id` and `app.user_id` GUCs set, so
//!     the strict `tenant_isolation` policy applies under the non-bypass
//!     `aulalite_app` role. Handlers additionally pass the caller's `user_id` so
//!     a caller only ever touches their OWN rows.
//!   * `notification_preferences` is GLOBAL per-user (PK = user_id, no RLS, like
//!     `users`). Access control is the handler scoping by `ctx.user_id`.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::fmt::Write as _;
use uuid::Uuid;

// ===========================================================================
// notifications
// ===========================================================================

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct NotificationRow {
    pub id: Uuid,
    pub kind: String,
    pub title: String,
    pub body: Option<String>,
    pub link: Option<String>,
    pub created_at: DateTime<Utc>,
    pub read_at: Option<DateTime<Utc>>,
}

const NOTIFICATION_COLS: &str = "id, kind, title, body, link, created_at, read_at";

/// Set both GUCs the tenant-scoped RLS policies depend on, inside `tx`.
async fn set_guc(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<()> {
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut **tx)
        .await?;
    sqlx::query("SELECT set_config('app.user_id', $1, true)")
        .bind(user_id.to_string())
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Persist a notification row for `user_id` in `tenant_id`. Tenant-scoped under
/// RLS.
pub async fn create_notification(
    pool: &PgPool,
    tenant_id: Uuid,
    user_id: Uuid,
    kind: &str,
    title: &str,
    body: Option<&str>,
    link: Option<&str>,
) -> sqlx::Result<NotificationRow> {
    let mut tx = pool.begin().await?;
    set_guc(&mut tx, tenant_id, user_id).await?;
    let row = sqlx::query_as::<_, NotificationRow>(sqlx::AssertSqlSafe(format!(
        "WITH ins AS (
             INSERT INTO notifications (tenant_id, user_id, kind, title, body, link)
             VALUES ($1, $2, $3, $4, $5, $6)
             RETURNING *
         )
         SELECT {NOTIFICATION_COLS} FROM ins"
    )))
    .bind(tenant_id)
    .bind(user_id)
    .bind(kind)
    .bind(title)
    .bind(body)
    .bind(link)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(row)
}

/// List `user_id`'s notifications in `tenant_id`, newest first, keyset-paginated
/// by `before` (created_at). Tenant-scoped under RLS; also filtered by user_id.
pub async fn list_for_user(
    pool: &PgPool,
    tenant_id: Uuid,
    user_id: Uuid,
    limit: i64,
    before: Option<DateTime<Utc>>,
) -> sqlx::Result<Vec<NotificationRow>> {
    let mut tx = pool.begin().await?;
    set_guc(&mut tx, tenant_id, user_id).await?;
    let rows = sqlx::query_as::<_, NotificationRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {NOTIFICATION_COLS} FROM notifications
          WHERE user_id = $1
            AND ($2::timestamptz IS NULL OR created_at < $2)
          ORDER BY created_at DESC
          LIMIT $3"
    )))
    .bind(user_id)
    .bind(before)
    .bind(limit)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(rows)
}

/// Count `user_id`'s unread notifications in `tenant_id`.
pub async fn unread_count(pool: &PgPool, tenant_id: Uuid, user_id: Uuid) -> sqlx::Result<i64> {
    let mut tx = pool.begin().await?;
    set_guc(&mut tx, tenant_id, user_id).await?;
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM notifications WHERE user_id = $1 AND read_at IS NULL",
    )
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(count)
}

/// Mark one notification read. Returns true if a row owned by `user_id` was
/// flipped (was previously unread), false otherwise.
pub async fn mark_read(
    pool: &PgPool,
    tenant_id: Uuid,
    user_id: Uuid,
    id: Uuid,
) -> sqlx::Result<bool> {
    let mut tx = pool.begin().await?;
    set_guc(&mut tx, tenant_id, user_id).await?;
    let res = sqlx::query(
        "UPDATE notifications SET read_at = now()
          WHERE id = $1 AND user_id = $2 AND read_at IS NULL",
    )
    .bind(id)
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(res.rows_affected() > 0)
}

/// Mark all of `user_id`'s unread notifications read. Returns the count updated.
pub async fn mark_all_read(pool: &PgPool, tenant_id: Uuid, user_id: Uuid) -> sqlx::Result<i64> {
    let mut tx = pool.begin().await?;
    set_guc(&mut tx, tenant_id, user_id).await?;
    let res = sqlx::query(
        "UPDATE notifications SET read_at = now()
          WHERE user_id = $1 AND read_at IS NULL",
    )
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(res.rows_affected() as i64)
}

// ===========================================================================
// device_tokens
// ===========================================================================

#[derive(Debug, Serialize, sqlx::FromRow, Clone)]
pub struct DeviceTokenRow {
    pub id: Uuid,
    pub platform: String,
    pub label: Option<String>,
    pub user_agent: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, sqlx::FromRow, Clone)]
pub struct DeliveryRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub notification_id: Option<Uuid>,
    pub channel: String,
    pub provider: String,
    pub target_hash: String,
    pub target_label: Option<String>,
    pub device_token_id: Option<Uuid>,
    pub kind: String,
    pub status: String,
    pub provider_message_id: Option<String>,
    pub provider_status: Option<String>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewDelivery<'a> {
    pub tenant_id: Uuid,
    pub user_id: Uuid,
    pub notification_id: Option<Uuid>,
    pub channel: &'a str,
    pub provider: &'a str,
    pub target_hash: &'a str,
    pub target_label: Option<&'a str>,
    pub device_token_id: Option<Uuid>,
    pub kind: &'a str,
    pub status: &'a str,
    pub provider_message_id: Option<&'a str>,
    pub provider_status: Option<&'a str>,
    pub error_code: Option<&'a str>,
    pub error_message: Option<&'a str>,
}

#[derive(Debug, Default, Deserialize)]
pub struct DeliveryFilter {
    pub channel: Option<String>,
    pub status: Option<String>,
    pub user_id: Option<Uuid>,
    pub kind: Option<String>,
    pub provider: Option<String>,
    pub before: Option<DateTime<Utc>>,
    pub limit: i64,
}

/// Register (or refresh) a push device token for `user_id` in `tenant_id`.
/// `token` is globally UNIQUE: a re-register upserts in place, refreshing
/// `last_seen_at` and re-homing the token to the current (user, tenant) so a
/// shared device that switches accounts is handled cleanly.
pub async fn register_device_token(
    pool: &PgPool,
    tenant_id: Uuid,
    user_id: Uuid,
    token: &str,
    platform: &str,
    label: Option<&str>,
    user_agent: Option<&str>,
) -> sqlx::Result<Uuid> {
    let mut tx = pool.begin().await?;
    set_guc(&mut tx, tenant_id, user_id).await?;
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO device_tokens (tenant_id, user_id, token, platform, label, user_agent, revoked_at)
         VALUES ($1, $2, $3, $4, $5, $6, NULL)
         ON CONFLICT (token) DO UPDATE
            SET tenant_id = EXCLUDED.tenant_id,
                user_id = EXCLUDED.user_id,
                platform = EXCLUDED.platform,
                label = EXCLUDED.label,
                user_agent = EXCLUDED.user_agent,
                revoked_at = NULL,
                last_seen_at = now()
         RETURNING id",
    )
    .bind(tenant_id)
    .bind(user_id)
    .bind(token)
    .bind(platform)
    .bind(label)
    .bind(user_agent)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(id)
}

/// Revoke a device token owned by `user_id` in `tenant_id`.
pub async fn remove_device_token(
    pool: &PgPool,
    tenant_id: Uuid,
    user_id: Uuid,
    token: &str,
) -> sqlx::Result<()> {
    let mut tx = pool.begin().await?;
    set_guc(&mut tx, tenant_id, user_id).await?;
    sqlx::query(
        "UPDATE device_tokens
            SET revoked_at = now()
          WHERE token = $1 AND tenant_id = $2 AND user_id = $3 AND revoked_at IS NULL",
    )
    .bind(token)
    .bind(tenant_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn revoke_device_token_by_id(
    pool: &PgPool,
    tenant_id: Uuid,
    user_id: Uuid,
    id: Uuid,
) -> sqlx::Result<bool> {
    let mut tx = pool.begin().await?;
    set_guc(&mut tx, tenant_id, user_id).await?;
    let res = sqlx::query(
        "UPDATE device_tokens
            SET revoked_at = now()
          WHERE id = $1 AND tenant_id = $2 AND user_id = $3 AND revoked_at IS NULL",
    )
    .bind(id)
    .bind(tenant_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(res.rows_affected() > 0)
}

pub async fn list_device_token_rows(
    pool: &PgPool,
    tenant_id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<Vec<DeviceTokenRow>> {
    let mut tx = pool.begin().await?;
    set_guc(&mut tx, tenant_id, user_id).await?;
    let rows = sqlx::query_as::<_, DeviceTokenRow>(
        "SELECT id, platform, label, user_agent, created_at, last_seen_at
           FROM device_tokens
          WHERE tenant_id = $1 AND user_id = $2 AND revoked_at IS NULL
          ORDER BY last_seen_at DESC",
    )
    .bind(tenant_id)
    .bind(user_id)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(rows)
}

pub async fn list_active_device_tokens(
    pool: &PgPool,
    tenant_id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<Vec<(Uuid, String, String, Option<String>)>> {
    let mut tx = pool.begin().await?;
    set_guc(&mut tx, tenant_id, user_id).await?;
    let rows = sqlx::query_as::<_, (Uuid, String, String, Option<String>)>(
        "SELECT id, token, platform, label
           FROM device_tokens
          WHERE tenant_id = $1 AND user_id = $2 AND revoked_at IS NULL
          ORDER BY last_seen_at DESC",
    )
    .bind(tenant_id)
    .bind(user_id)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(rows)
}

/// Load `user_id`'s push device tokens in `tenant_id` (used by the `notify`
/// facade's push channel). Tenant-scoped under RLS.
pub async fn list_device_tokens(
    pool: &PgPool,
    tenant_id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<Vec<String>> {
    let rows = list_active_device_tokens(pool, tenant_id, user_id).await?;
    Ok(rows.into_iter().map(|(_, token, _, _)| token).collect())
}

pub fn target_hash(channel: &str, target: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(channel.as_bytes());
    hasher.update(b":");
    hasher.update(target.as_bytes());
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(&mut hex, "{byte:02x}");
    }
    hex
}

pub fn device_target_label(platform: &str, label: Option<&str>) -> String {
    match label.map(str::trim).filter(|value| !value.is_empty()) {
        Some(label) => format!("{platform} - {label}"),
        None => platform.to_string(),
    }
}

pub async fn record_delivery(
    pool: &PgPool,
    delivery: NewDelivery<'_>,
) -> sqlx::Result<DeliveryRow> {
    let mut tx = pool.begin().await?;
    set_guc(&mut tx, delivery.tenant_id, delivery.user_id).await?;
    let row = sqlx::query_as::<_, DeliveryRow>(
        "INSERT INTO notification_deliveries (
            tenant_id, user_id, notification_id, channel, provider, target_hash,
            target_label, device_token_id, kind, status, provider_message_id,
            provider_status, error_code, error_message
         )
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)
         RETURNING id, user_id, notification_id, channel, provider, target_hash,
                   target_label, device_token_id, kind, status, provider_message_id,
                   provider_status, error_code, error_message, created_at, updated_at",
    )
    .bind(delivery.tenant_id)
    .bind(delivery.user_id)
    .bind(delivery.notification_id)
    .bind(delivery.channel)
    .bind(delivery.provider)
    .bind(delivery.target_hash)
    .bind(delivery.target_label)
    .bind(delivery.device_token_id)
    .bind(delivery.kind)
    .bind(delivery.status)
    .bind(delivery.provider_message_id)
    .bind(delivery.provider_status)
    .bind(delivery.error_code)
    .bind(delivery.error_message)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(row)
}

pub async fn list_deliveries_for_admin(
    pool: &PgPool,
    tenant_id: Uuid,
    actor_user_id: Uuid,
    filter: DeliveryFilter,
) -> sqlx::Result<Vec<DeliveryRow>> {
    let mut tx = pool.begin().await?;
    set_guc(&mut tx, tenant_id, actor_user_id).await?;
    let limit = filter.limit.clamp(1, 200);
    let rows = sqlx::query_as::<_, DeliveryRow>(
        "SELECT id, user_id, notification_id, channel, provider, target_hash,
                target_label, device_token_id, kind, status, provider_message_id,
                provider_status, error_code, error_message, created_at, updated_at
           FROM notification_deliveries
          WHERE tenant_id = $1
            AND ($2::text IS NULL OR channel = $2)
            AND ($3::text IS NULL OR status = $3)
            AND ($4::uuid IS NULL OR user_id = $4)
            AND ($5::text IS NULL OR kind = $5)
            AND ($6::text IS NULL OR provider = $6)
            AND ($7::timestamptz IS NULL OR created_at < $7)
          ORDER BY created_at DESC
          LIMIT $8",
    )
    .bind(tenant_id)
    .bind(filter.channel.as_deref())
    .bind(filter.status.as_deref())
    .bind(filter.user_id)
    .bind(filter.kind.as_deref())
    .bind(filter.provider.as_deref())
    .bind(filter.before)
    .bind(limit)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(rows)
}

pub async fn get_delivery_for_admin(
    pool: &PgPool,
    tenant_id: Uuid,
    actor_user_id: Uuid,
    id: Uuid,
) -> sqlx::Result<Option<DeliveryRow>> {
    let mut tx = pool.begin().await?;
    set_guc(&mut tx, tenant_id, actor_user_id).await?;
    let row = sqlx::query_as::<_, DeliveryRow>(
        "SELECT id, user_id, notification_id, channel, provider, target_hash,
                target_label, device_token_id, kind, status, provider_message_id,
                provider_status, error_code, error_message, created_at, updated_at
           FROM notification_deliveries
          WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tenant_id)
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(row)
}

// ===========================================================================
// notification_preferences (global per-user, no RLS)
// ===========================================================================

#[derive(Debug, Serialize, sqlx::FromRow, Clone)]
pub struct PrefRow {
    pub user_id: Uuid,
    pub email_enabled: bool,
    pub push_enabled: bool,
    pub in_app_enabled: bool,
    pub updated_at: DateTime<Utc>,
}

impl PrefRow {
    /// The implicit default when no preferences row exists: all channels on.
    pub fn default_for(user_id: Uuid) -> Self {
        Self {
            user_id,
            email_enabled: true,
            push_enabled: true,
            in_app_enabled: true,
            updated_at: Utc::now(),
        }
    }
}

/// Read `user_id`'s notification preferences, defaulting all channels to true
/// when no row exists. Global table (no tenant GUC needed).
pub async fn get_preferences(pool: &PgPool, user_id: Uuid) -> sqlx::Result<PrefRow> {
    let row: Option<PrefRow> = sqlx::query_as::<_, PrefRow>(
        "SELECT user_id, email_enabled, push_enabled, in_app_enabled, updated_at
           FROM notification_preferences WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.unwrap_or_else(|| PrefRow::default_for(user_id)))
}

/// Upsert `user_id`'s notification preferences and return the persisted row.
/// Global table (no tenant GUC needed).
pub async fn set_preferences(
    pool: &PgPool,
    user_id: Uuid,
    email_enabled: bool,
    push_enabled: bool,
    in_app_enabled: bool,
) -> sqlx::Result<PrefRow> {
    let row = sqlx::query_as::<_, PrefRow>(
        "INSERT INTO notification_preferences
            (user_id, email_enabled, push_enabled, in_app_enabled, updated_at)
         VALUES ($1, $2, $3, $4, now())
         ON CONFLICT (user_id) DO UPDATE
            SET email_enabled = EXCLUDED.email_enabled,
                push_enabled = EXCLUDED.push_enabled,
                in_app_enabled = EXCLUDED.in_app_enabled,
                updated_at = now()
         RETURNING user_id, email_enabled, push_enabled, in_app_enabled, updated_at",
    )
    .bind(user_id)
    .bind(email_enabled)
    .bind(push_enabled)
    .bind(in_app_enabled)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

// ===========================================================================
// helpers used by the notify facade
// ===========================================================================

/// Look up a user's email (global `users` table; no RLS). Returns None if the
/// user does not exist.
pub async fn lookup_user_email(pool: &PgPool, user_id: Uuid) -> sqlx::Result<Option<String>> {
    let email: Option<String> = sqlx::query_scalar("SELECT email::text FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_optional(pool)
        .await?;
    Ok(email)
}
