# AulaLite Phase 4 Push Notifications Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make push notifications production-grade with service-account-backed FCM sends, device self-service, persisted delivery outcomes, and admin diagnostics.

**Architecture:** Keep `services::notifications::notify` as the fan-out entry point, but make it load active device rows, send push one token at a time through the existing `PushSender` trait, and write sanitized `notification_deliveries` rows around each attempt. Add an isolated FCM config/token/payload module so OAuth and provider behavior can be tested without a database, while the existing notification handler gains self-service devices and admin delivery diagnostics.

**Tech Stack:** Rust 1.94 workspace, Axum, SQLx/PostgreSQL with RLS, Reqwest, `jsonwebtoken` RS256 signing, Dioxus 0.7, existing `features-courses` API helpers, `shell-web` routes, Firebase Cloud Messaging HTTP v1, and Google service-account OAuth.

---

## File Structure

- Create: `migrations/20260618000065_notification_deliveries.sql`
  - Adds delivery logs, device-token metadata, soft revocation, active-token index, and RLS.
- Modify: `crates/backend/src/db/notifications.rs`
  - Adds `DeviceTokenRow`, delivery DTOs, active-device queries, revoke-by-id, target hashing, delivery inserts, and admin delivery queries.
- Modify: `crates/backend/src/handlers/notifications.rs`
  - Adds `GET /v1/me/device-tokens`, `DELETE /v1/me/device-tokens/{id}`, and admin delivery diagnostics endpoints.
- Modify: `crates/backend/src/services/notifications.rs`
  - Replaces the FCM scaffolding with config parsing, OAuth token provider, safe link filtering, payload construction, cached token exchange, and one-token sends.
- Modify: `crates/backend/src/main.rs`
  - Wires the production push sender from service-account env vars while preserving disabled/mock behavior when push is not configured.
- Modify: `crates/backend/tests/notifications.rs`
  - Extends DB-backed tests for device list/revoke, delivery logs, and admin diagnostics authorization.
- Modify: `crates/features-courses/src/api.rs`
  - Adds device-token DTOs/helpers and admin delivery diagnostics DTOs/helpers.
- Modify: `crates/shell-web/src/routes/notification_settings.rs`
  - Adds device management, browser-push result states, and SSR tests.
- Modify: `crates/shell-web/Cargo.toml`
  - Adds the wasm `Navigator` web-sys feature used to capture a browser label/user agent.
- Create: `crates/shell-web/src/routes/admin_notification_deliveries.rs`
  - Adds admin delivery diagnostics table and detail sheet.
- Modify: `crates/shell-web/src/routes/mod.rs`
  - Exports the admin diagnostics route.
- Modify: `crates/shell-web/src/route_enum.rs`
  - Adds `/admin/notifications`.
- Modify: `crates/features-courses/src/app_shell.rs`
  - Adds one grouped admin navigation entry under the existing admin section.
- Modify: `crates/platform-bridge/src/web.rs`
  - Returns typed FCM browser-push outcomes instead of collapsing all graceful states to `None`.
- Modify: `crates/shell-web/public/assets/fcm-bridge.js`
  - Returns typed browser-push results for unsupported, missing VAPID, denied permission, and token acquired.
- Modify: `crates/design-system/assets/components.css`
  - Adds compact styles for device rows and delivery diagnostics.
- Modify: `crates/shell-web/public/assets/components.css`
  - Mirrors the same runtime CSS used by the web shell.
- Review: `docs/superpowers/specs/2026-06-18-aulalite-phase-4-push-notifications-design.md`
  - Update only if implementation discovers a concrete design correction.

## Task 1: Add Delivery And Device Schema

**Files:**
- Create: `migrations/20260618000065_notification_deliveries.sql`

- [ ] **Step 1: Add the migration**

Create `migrations/20260618000065_notification_deliveries.sql` with:

```sql
-- Phase 4 push notification hardening:
--   * delivery logs for email/push attempts, scoped by tenant under RLS
--   * device metadata for user self-service
--   * soft device revocation so old delivery rows remain understandable

ALTER TABLE device_tokens
    ADD COLUMN label TEXT,
    ADD COLUMN user_agent TEXT,
    ADD COLUMN revoked_at TIMESTAMPTZ;

CREATE INDEX device_tokens_user_active_idx
    ON device_tokens (tenant_id, user_id, last_seen_at DESC)
    WHERE revoked_at IS NULL;

CREATE TABLE notification_deliveries (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL REFERENCES tenants(id),
    user_id UUID NOT NULL REFERENCES users(id),
    notification_id UUID REFERENCES notifications(id) ON DELETE SET NULL,
    channel TEXT NOT NULL CHECK (channel IN ('email','push')),
    provider TEXT NOT NULL,
    target_hash TEXT NOT NULL,
    target_label TEXT,
    device_token_id UUID REFERENCES device_tokens(id) ON DELETE SET NULL,
    kind TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('queued','sent','failed','skipped')),
    provider_message_id TEXT,
    provider_status TEXT,
    error_code TEXT,
    error_message TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX notification_deliveries_tenant_created_idx
    ON notification_deliveries (tenant_id, created_at DESC);
CREATE INDEX notification_deliveries_user_created_idx
    ON notification_deliveries (tenant_id, user_id, created_at DESC);
CREATE INDEX notification_deliveries_status_idx
    ON notification_deliveries (tenant_id, status, created_at DESC);
CREATE INDEX notification_deliveries_device_idx
    ON notification_deliveries (device_token_id)
    WHERE device_token_id IS NOT NULL;

ALTER TABLE notification_deliveries ENABLE ROW LEVEL SECURITY;
ALTER TABLE notification_deliveries FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON notification_deliveries
    USING (tenant_id::text = current_setting('app.tenant_id', true))
    WITH CHECK (tenant_id::text = current_setting('app.tenant_id', true));

GRANT SELECT, INSERT, UPDATE, DELETE ON notification_deliveries TO aulalite_app;
```

- [ ] **Step 2: Verify migration ordering**

Run:

```powershell
Get-ChildItem migrations | Sort-Object Name | Select-Object -Last 5 -ExpandProperty Name
```

Expected: output includes `20260618000065_notification_deliveries.sql` after `20260616000064_mfa_trusted_devices.sql`.

- [ ] **Step 3: Commit schema**

Run:

```powershell
git add migrations/20260618000065_notification_deliveries.sql
git commit -m "feat(notifications): add delivery log schema"
```

Expected: commit succeeds with only the new migration.

## Task 2: Add Notification DB Helpers

**Files:**
- Modify: `crates/backend/src/db/notifications.rs`
- Modify: `crates/backend/tests/notifications.rs`

- [ ] **Step 1: Add failing backend tests**

Append these tests to `crates/backend/tests/notifications.rs`:

```rust
#[tokio::test]
async fn device_tokens_can_be_listed_and_revoked_by_id() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (user, _, email) = create_user(&pool).await;
    attach_membership(&pool, tenant, user, "student").await;
    let app = notif_app(&pool, user, email, tenant);
    let token = format!("tok-{}", Uuid::new_v4());

    let (status, _) = fire(
        &app,
        "POST",
        "/v1/me/device-tokens",
        Some(json!({
            "token": token,
            "platform": "web",
            "label": "Chrome on Windows",
            "user_agent": "Mozilla/5.0"
        })),
    )
    .await;
    assert_eq!(status, 204);

    let rows = backend::db::notifications::list_device_token_rows(&pool, tenant, user)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].platform, "web");
    assert_eq!(rows[0].label.as_deref(), Some("Chrome on Windows"));
    assert_eq!(rows[0].user_agent.as_deref(), Some("Mozilla/5.0"));

    backend::db::notifications::revoke_device_token_by_id(&pool, tenant, user, rows[0].id)
        .await
        .unwrap();
    let rows = backend::db::notifications::list_device_token_rows(&pool, tenant, user)
        .await
        .unwrap();
    assert!(rows.is_empty());
}

#[tokio::test]
async fn delivery_rows_are_tenant_scoped_and_sanitized() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (user, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, user, "student").await;
    let device_id = backend::db::notifications::register_device_token(
        &pool,
        tenant,
        user,
        "secret-push-token",
        "web",
        Some("Browser"),
        Some("Mozilla/5.0"),
    )
    .await
    .unwrap();

    let row = backend::db::notifications::record_delivery(
        &pool,
        backend::db::notifications::NewDelivery {
            tenant_id: tenant,
            user_id: user,
            notification_id: None,
            channel: "push",
            provider: "fcm",
            target_hash: &backend::db::notifications::target_hash("push", "secret-push-token"),
            target_label: Some("web · Browser"),
            device_token_id: Some(device_id),
            kind: "test",
            status: "sent",
            provider_message_id: Some("projects/p/messages/123"),
            provider_status: Some("200"),
            error_code: None,
            error_message: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(row.status, "sent");
    assert_ne!(row.target_hash, "secret-push-token");
    assert_eq!(row.target_label.as_deref(), Some("web · Browser"));

    let listed = backend::db::notifications::list_deliveries_for_admin(
        &pool,
        tenant,
        user,
        backend::db::notifications::DeliveryFilter {
            channel: Some("push".into()),
            status: Some("sent".into()),
            user_id: Some(user),
            kind: Some("test".into()),
            provider: Some("fcm".into()),
            before: None,
            limit: 20,
        },
    )
    .await
    .unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, row.id);
}
```

- [ ] **Step 2: Run the failing tests**

Run:

```powershell
cargo test -p backend --test notifications device_tokens_can_be_listed_and_revoked_by_id delivery_rows_are_tenant_scoped_and_sanitized -- --nocapture
```

Expected: compile fails because the new DB helpers and expanded `register_device_token` signature do not exist.

- [ ] **Step 3: Add imports and DTOs**

In `crates/backend/src/db/notifications.rs`, add `Deserialize`, `sha2`, and the new row types:

```rust
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

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
```

- [ ] **Step 4: Replace device-token helpers**

Replace `register_device_token`, `remove_device_token`, and `list_device_tokens` with:

```rust
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
          WHERE token = $1 AND user_id = $2 AND revoked_at IS NULL",
    )
    .bind(token)
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
          WHERE id = $1 AND user_id = $2 AND revoked_at IS NULL",
    )
    .bind(id)
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
          WHERE user_id = $1 AND revoked_at IS NULL
          ORDER BY last_seen_at DESC",
    )
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
          WHERE user_id = $1 AND revoked_at IS NULL
          ORDER BY last_seen_at DESC",
    )
    .bind(user_id)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(rows)
}

pub async fn list_device_tokens(
    pool: &PgPool,
    tenant_id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<Vec<String>> {
    let rows = list_active_device_tokens(pool, tenant_id, user_id).await?;
    Ok(rows.into_iter().map(|(_, token, _, _)| token).collect())
}
```

- [ ] **Step 5: Add delivery helpers**

Add these functions near the helpers section:

```rust
pub fn target_hash(channel: &str, target: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(channel.as_bytes());
    hasher.update(b":");
    hasher.update(target.as_bytes());
    format!("{:x}", hasher.finalize())
}

pub fn device_target_label(platform: &str, label: Option<&str>) -> String {
    match label.map(str::trim).filter(|value| !value.is_empty()) {
        Some(label) => format!("{platform} · {label}"),
        None => platform.to_string(),
    }
}

pub async fn record_delivery(pool: &PgPool, delivery: NewDelivery<'_>) -> sqlx::Result<DeliveryRow> {
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
          WHERE ($1::text IS NULL OR channel = $1)
            AND ($2::text IS NULL OR status = $2)
            AND ($3::uuid IS NULL OR user_id = $3)
            AND ($4::text IS NULL OR kind = $4)
            AND ($5::text IS NULL OR provider = $5)
            AND ($6::timestamptz IS NULL OR created_at < $6)
          ORDER BY created_at DESC
          LIMIT $7",
    )
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
          WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(row)
}
```

- [ ] **Step 6: Update existing tests for the new signature**

In `crates/backend/tests/notifications.rs`, change direct DB helper calls to include `None` metadata:

```rust
backend::db::notifications::register_device_token(
    &pool,
    tenant,
    user,
    &token,
    "web",
    None,
    None,
)
.await
.unwrap();
```

In `crates/backend/src/handlers/notifications.rs`, the handler update happens in Task 3.

- [ ] **Step 7: Run DB tests**

Run:

```powershell
cargo test -p backend --test notifications device_tokens_can_be_listed_and_revoked_by_id delivery_rows_are_tenant_scoped_and_sanitized -- --nocapture
```

Expected: both new DB-focused tests pass.

- [ ] **Step 8: Commit DB helpers**

Run:

```powershell
git add crates/backend/src/db/notifications.rs crates/backend/tests/notifications.rs
git commit -m "feat(notifications): add delivery and device db helpers"
```

Expected: commit succeeds with DB helper and test changes.

## Task 3: Add Self-Service Device And Admin Delivery Endpoints

**Files:**
- Modify: `crates/backend/src/handlers/notifications.rs`
- Modify: `crates/backend/tests/notifications.rs`

- [ ] **Step 1: Add failing route tests**

Append these tests to `crates/backend/tests/notifications.rs`:

```rust
fn admin_notif_app(
    pool: &sqlx::PgPool,
    user: Uuid,
    email: String,
    tenant: Uuid,
    role: core_types::TenantRole,
) -> axum::Router {
    build_test_app(
        backend::handlers::notifications::router_for_tests(pool.clone()),
        StubAuth {
            pool: pool.clone(),
            user_id: user,
            firebase_uid: format!("fb-{}", Uuid::new_v4()),
            email,
            tenant_id: Some(tenant),
            tenant_role: Some(role),
        },
    )
}

#[tokio::test]
async fn device_list_and_revoke_routes_are_owner_scoped() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (user, _, email) = create_user(&pool).await;
    attach_membership(&pool, tenant, user, "student").await;
    let app = notif_app(&pool, user, email, tenant);

    let token = format!("tok-{}", Uuid::new_v4());
    let (status, _) = fire(
        &app,
        "POST",
        "/v1/me/device-tokens",
        Some(json!({
            "token": token,
            "platform": "web",
            "label": "Edge",
            "user_agent": "Mozilla/5.0"
        })),
    )
    .await;
    assert_eq!(status, 204);

    let (status, body) = fire(&app, "GET", "/v1/me/device-tokens", None).await;
    assert_eq!(status, 200, "{body}");
    let devices = body["devices"].as_array().unwrap();
    assert_eq!(devices.len(), 1);
    let id = devices[0]["id"].as_str().unwrap();
    assert_eq!(devices[0]["label"], "Edge");
    assert_eq!(devices[0]["platform"], "web");
    assert!(body.to_string().contains("tok-").not());

    let (status, _) = fire(&app, "DELETE", &format!("/v1/me/device-tokens/{id}"), None).await;
    assert_eq!(status, 204);
    let (_, body) = fire(&app, "GET", "/v1/me/device-tokens", None).await;
    assert_eq!(body["devices"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn admin_delivery_routes_require_admin_role() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (student, _, student_email) = create_user(&pool).await;
    attach_membership(&pool, tenant, student, "student").await;
    let (admin, _, admin_email) = create_user(&pool).await;
    attach_membership(&pool, tenant, admin, "org_admin").await;

    let target_hash = backend::db::notifications::target_hash("push", "tok-secret");
    let row = backend::db::notifications::record_delivery(
        &pool,
        backend::db::notifications::NewDelivery {
            tenant_id: tenant,
            user_id: student,
            notification_id: None,
            channel: "push",
            provider: "fcm",
            target_hash: &target_hash,
            target_label: Some("web"),
            device_token_id: None,
            kind: "test",
            status: "failed",
            provider_message_id: None,
            provider_status: Some("404"),
            error_code: Some("UNREGISTERED"),
            error_message: Some("token is not registered"),
        },
    )
    .await
    .unwrap();

    let student_app = admin_notif_app(
        &pool,
        student,
        student_email,
        tenant,
        core_types::TenantRole::Student,
    );
    let (status, _) = fire(&student_app, "GET", "/v1/admin/notification-deliveries", None).await;
    assert_eq!(status, 403);

    let admin_app = admin_notif_app(
        &pool,
        admin,
        admin_email,
        tenant,
        core_types::TenantRole::OrgAdmin,
    );
    let (status, body) = fire(
        &admin_app,
        "GET",
        "/v1/admin/notification-deliveries?channel=push&status=failed",
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["deliveries"].as_array().unwrap().len(), 1);
    assert_eq!(body["deliveries"][0]["id"], row.id.to_string());
    assert!(!body.to_string().contains("tok-secret"));

    let (status, body) = fire(
        &admin_app,
        "GET",
        &format!("/v1/admin/notification-deliveries/{}", row.id),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["id"], row.id.to_string());
}
```

Add this import near the top:

```rust
use std::ops::Not;
```

- [ ] **Step 2: Run the failing route tests**

Run:

```powershell
cargo test -p backend --test notifications device_list_and_revoke_routes_are_owner_scoped admin_delivery_routes_require_admin_role -- --nocapture
```

Expected: compile fails because route DTOs and router paths do not exist.

- [ ] **Step 3: Add DTOs and request bodies**

In `crates/backend/src/handlers/notifications.rs`, extend the DTO section:

```rust
#[derive(Serialize)]
pub struct DeviceTokenDto {
    pub id: Uuid,
    pub platform: String,
    pub label: Option<String>,
    pub user_agent: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub last_seen_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Serialize)]
pub struct DeviceTokenListDto {
    pub devices: Vec<DeviceTokenDto>,
}

impl From<db::notifications::DeviceTokenRow> for DeviceTokenDto {
    fn from(row: db::notifications::DeviceTokenRow) -> Self {
        Self {
            id: row.id,
            platform: row.platform,
            label: row.label,
            user_agent: row.user_agent,
            created_at: row.created_at,
            last_seen_at: row.last_seen_at,
        }
    }
}

#[derive(Serialize)]
pub struct DeliveryDto {
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
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Serialize)]
pub struct DeliveryListDto {
    pub deliveries: Vec<DeliveryDto>,
}

impl From<db::notifications::DeliveryRow> for DeliveryDto {
    fn from(row: db::notifications::DeliveryRow) -> Self {
        Self {
            id: row.id,
            user_id: row.user_id,
            notification_id: row.notification_id,
            channel: row.channel,
            provider: row.provider,
            target_hash: row.target_hash,
            target_label: row.target_label,
            device_token_id: row.device_token_id,
            kind: row.kind,
            status: row.status,
            provider_message_id: row.provider_message_id,
            provider_status: row.provider_status,
            error_code: row.error_code,
            error_message: row.error_message,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

#[derive(Deserialize)]
pub struct DeliveryQuery {
    pub channel: Option<String>,
    pub status: Option<String>,
    pub user_id: Option<Uuid>,
    pub kind: Option<String>,
    pub provider: Option<String>,
    pub before: Option<chrono::DateTime<chrono::Utc>>,
    pub limit: Option<i64>,
}
```

Extend `DeviceTokenBody`:

```rust
#[derive(Deserialize)]
pub struct DeviceTokenBody {
    pub token: String,
    pub platform: String,
    pub label: Option<String>,
    pub user_agent: Option<String>,
}
```

- [ ] **Step 4: Add route wiring**

Update both `routes()` and `router_for_tests()` to include the new paths:

```rust
.route(
    "/v1/me/device-tokens",
    routing::get(list_device_tokens).post(register_device_token).delete(remove_device_token),
)
.route(
    "/v1/me/device-tokens/{id}",
    routing::delete(revoke_device_token),
)
.route(
    "/v1/admin/notification-deliveries",
    routing::get(list_admin_deliveries),
)
.route(
    "/v1/admin/notification-deliveries/{id}",
    routing::get(get_admin_delivery),
)
```

For the test router, use the `_t` wrappers with the same paths. Also change the
existing notification read route from `/v1/me/notifications/:id/read` to
`/v1/me/notifications/{id}/read` in both production and test routers; Axum 0.8
rejects legacy colon capture syntax at router construction.

- [ ] **Step 5: Add endpoint inner functions**

Add these helpers below `remove_device_token_inner`:

```rust
fn is_admin(ctx: &RequestContext) -> bool {
    ctx.is_platform_admin || matches!(ctx.tenant_role, Some(core_types::TenantRole::OrgAdmin))
}

async fn list_device_tokens_inner(
    pool: &PgPool,
    ctx: &RequestContext,
) -> Result<Json<DeviceTokenListDto>, ApiError> {
    let tenant_id = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    let rows = db::notifications::list_device_token_rows(pool, tenant_id, ctx.user_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(DeviceTokenListDto {
        devices: rows.into_iter().map(DeviceTokenDto::from).collect(),
    }))
}

async fn revoke_device_token_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    id: Uuid,
) -> Result<StatusCode, ApiError> {
    let tenant_id = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    let revoked = db::notifications::revoke_device_token_by_id(pool, tenant_id, ctx.user_id, id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !revoked {
        return Err(ApiError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn list_admin_deliveries_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    q: DeliveryQuery,
) -> Result<Json<DeliveryListDto>, ApiError> {
    if !is_admin(ctx) {
        return Err(ApiError::Forbidden);
    }
    let tenant_id = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    let filter = db::notifications::DeliveryFilter {
        channel: q.channel,
        status: q.status,
        user_id: q.user_id,
        kind: q.kind,
        provider: q.provider,
        before: q.before,
        limit: q.limit.unwrap_or(50).clamp(1, 200),
    };
    let rows = db::notifications::list_deliveries_for_admin(pool, tenant_id, ctx.user_id, filter)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(DeliveryListDto {
        deliveries: rows.into_iter().map(DeliveryDto::from).collect(),
    }))
}

async fn get_admin_delivery_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    id: Uuid,
) -> Result<Json<DeliveryDto>, ApiError> {
    if !is_admin(ctx) {
        return Err(ApiError::Forbidden);
    }
    let tenant_id = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    let row = db::notifications::get_delivery_for_admin(pool, tenant_id, ctx.user_id, id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(DeliveryDto::from(row)))
}
```

Update `register_device_token_inner` to pass metadata:

```rust
db::notifications::register_device_token(
    pool,
    tenant_id,
    ctx.user_id,
    &b.token,
    &b.platform,
    b.label.as_deref(),
    b.user_agent.as_deref(),
)
.await
.map_err(|e| ApiError::Internal(e.to_string()))?;
```

- [ ] **Step 6: Add production and test wrappers**

Add wrappers matching the existing handler style:

```rust
async fn list_device_tokens(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<DeviceTokenListDto>, ApiError> {
    list_device_tokens_inner(&s.pool, &ctx).await
}

async fn revoke_device_token(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    revoke_device_token_inner(&s.pool, &ctx, id).await
}

async fn list_admin_deliveries(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Query(q): Query<DeliveryQuery>,
) -> Result<Json<DeliveryListDto>, ApiError> {
    list_admin_deliveries_inner(&s.pool, &ctx, q).await
}

async fn get_admin_delivery(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<DeliveryDto>, ApiError> {
    get_admin_delivery_inner(&s.pool, &ctx, id).await
}
```

Add equivalent `_t` wrappers using `State<TestState>`.

- [ ] **Step 7: Run route tests**

Run:

```powershell
cargo test -p backend --test notifications device_list_and_revoke_routes_are_owner_scoped admin_delivery_routes_require_admin_role -- --nocapture
```

Expected: route tests pass and response JSON does not include raw device tokens.

- [ ] **Step 8: Commit endpoints**

Run:

```powershell
git add crates/backend/src/handlers/notifications.rs crates/backend/tests/notifications.rs
git commit -m "feat(notifications): add device and delivery endpoints"
```

Expected: commit succeeds with handler and route-test changes.

## Task 4: Add FCM Config, Payload, And OAuth Token Provider

**Files:**
- Modify: `crates/backend/src/services/notifications.rs`

- [ ] **Step 1: Add failing service tests**

Inside `crates/backend/src/services/notifications.rs`, append these tests to the existing `#[cfg(test)] mod tests`:

```rust
#[test]
fn fcm_config_parses_inline_service_account_json() {
    let json = serde_json::json!({
        "project_id": "project-from-json",
        "private_key_id": "kid-123",
        "private_key": "-----BEGIN PRIVATE KEY-----\nabc\n-----END PRIVATE KEY-----\n",
        "client_email": "svc@example.iam.gserviceaccount.com",
        "token_uri": "https://oauth2.googleapis.com/token"
    })
    .to_string();

    let cfg = FcmProviderConfig::from_parts(
        Some("project-from-env".into()),
        Some(json),
        None,
        None,
        None,
    )
    .unwrap()
    .expect("configured");

    assert_eq!(cfg.project_id, "project-from-env");
    assert_eq!(cfg.client_email, "svc@example.iam.gserviceaccount.com");
    assert_eq!(cfg.private_key_id.as_deref(), Some("kid-123"));
    assert_eq!(cfg.token_uri, "https://oauth2.googleapis.com/token");
}

#[test]
fn fcm_config_disabled_without_project_id() {
    let cfg = FcmProviderConfig::from_parts(None, None, None, None, None).unwrap();
    assert!(cfg.is_none());
}

#[test]
fn safe_push_link_keeps_relative_paths_and_rejects_script_urls() {
    assert_eq!(safe_push_link(Some("/app/courses")), Some("/app/courses".into()));
    assert_eq!(safe_push_link(Some("https://app.example.com/x")), None);
    assert_eq!(safe_push_link(Some("javascript:alert(1)")), None);
    assert_eq!(safe_push_link(Some("   ")), None);
}

#[test]
fn fcm_message_payload_omits_unsafe_link() {
    let value = build_fcm_message("tok", "Title", "Body", Some("javascript:alert(1)"));
    assert_eq!(value["message"]["token"], "tok");
    assert_eq!(value["message"]["notification"]["title"], "Title");
    assert!(value["message"]["data"].as_object().unwrap().is_empty());
    assert!(value["message"].get("webpush").is_none());
}

#[test]
fn jwt_claims_use_fcm_scope_and_bounded_expiration() {
    let claims = FcmJwtClaims::new(
        "svc@example.iam.gserviceaccount.com",
        "https://oauth2.googleapis.com/token",
        1_800_000_000,
    );
    assert_eq!(claims.iss, "svc@example.iam.gserviceaccount.com");
    assert_eq!(claims.scope, FCM_SCOPE);
    assert_eq!(claims.aud, "https://oauth2.googleapis.com/token");
    assert_eq!(claims.iat, 1_800_000_000);
    assert_eq!(claims.exp, 1_800_003_600);
}

#[tokio::test]
async fn cached_token_provider_reuses_fresh_token() {
    let source = mock::MockAccessTokenSource::new(vec![Ok(FcmAccessToken {
        token: "access-1".into(),
        expires_at_unix: 1_800_003_600,
    })]);
    let provider = CachedFcmAccessTokenProvider::new(source.clone(), TestClock::new(1_800_000_000));

    assert_eq!(provider.access_token().await.unwrap(), "access-1");
    assert_eq!(provider.access_token().await.unwrap(), "access-1");
    assert_eq!(source.calls(), 1);
}
```

- [ ] **Step 2: Run the failing service tests**

Run:

```powershell
cargo test -p backend --lib services::notifications::tests::fcm -- --nocapture
```

Expected: compile fails because the FCM provider types do not exist.

- [ ] **Step 3: Add FCM config and payload types**

Add these imports near the top of `notifications.rs`:

```rust
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
```

Add these constants and types before the `PushSender` trait:

```rust
const FCM_SCOPE: &str = "https://www.googleapis.com/auth/firebase.messaging";
const DEFAULT_FCM_BASE_URL: &str = "https://fcm.googleapis.com";
const DEFAULT_FCM_TOKEN_URI: &str = "https://oauth2.googleapis.com/token";
const OAUTH_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:jwt-bearer";
const TOKEN_REFRESH_SKEW_SECONDS: i64 = 300;

#[derive(Debug, Clone)]
pub struct FcmProviderConfig {
    pub project_id: String,
    pub client_email: String,
    pub private_key: String,
    pub private_key_id: Option<String>,
    pub token_uri: String,
    pub fcm_base_url: String,
}

#[derive(Debug, Deserialize)]
struct ServiceAccountJson {
    project_id: Option<String>,
    private_key_id: Option<String>,
    private_key: Option<String>,
    client_email: Option<String>,
    token_uri: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum FcmConfigError {
    #[error("invalid service account json: {0}")]
    InvalidJson(String),
    #[error("service account is missing {0}")]
    MissingField(&'static str),
}

impl FcmProviderConfig {
    pub fn from_env() -> Result<Option<Self>, FcmConfigError> {
        let project_id = std::env::var("FCM_PROJECT_ID").ok();
        let inline_json = std::env::var("FCM_SERVICE_ACCOUNT_JSON").ok();
        let json_path = std::env::var("FCM_SERVICE_ACCOUNT_JSON_PATH").ok();
        let token_uri = std::env::var("FCM_TOKEN_URI").ok();
        let fcm_base_url = std::env::var("FCM_BASE_URL").ok();
        Self::from_parts(project_id, inline_json, json_path, token_uri, fcm_base_url)
    }

    pub fn from_parts(
        project_id: Option<String>,
        inline_json: Option<String>,
        json_path: Option<String>,
        token_uri: Option<String>,
        fcm_base_url: Option<String>,
    ) -> Result<Option<Self>, FcmConfigError> {
        let Some(project_id) = project_id.map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
        else {
            return Ok(None);
        };

        let raw_json = match inline_json.map(|v| v.trim().to_string()).filter(|v| !v.is_empty()) {
            Some(value) => value,
            None => match json_path.map(|v| v.trim().to_string()).filter(|v| !v.is_empty()) {
                Some(path) => std::fs::read_to_string(path)
                    .map_err(|e| FcmConfigError::InvalidJson(e.to_string()))?,
                None => return Ok(None),
            },
        };

        let parsed: ServiceAccountJson = serde_json::from_str(&raw_json)
            .map_err(|e| FcmConfigError::InvalidJson(e.to_string()))?;
        let client_email = required(parsed.client_email, "client_email")?;
        let private_key = required(parsed.private_key, "private_key")?;
        Ok(Some(Self {
            project_id,
            client_email,
            private_key,
            private_key_id: parsed.private_key_id,
            token_uri: token_uri
                .or(parsed.token_uri)
                .unwrap_or_else(|| DEFAULT_FCM_TOKEN_URI.to_string()),
            fcm_base_url: fcm_base_url.unwrap_or_else(|| DEFAULT_FCM_BASE_URL.to_string()),
        }))
    }
}

fn required(value: Option<String>, name: &'static str) -> Result<String, FcmConfigError> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .ok_or(FcmConfigError::MissingField(name))
}

fn safe_push_link(link: Option<&str>) -> Option<String> {
    let link = link.map(str::trim).filter(|value| !value.is_empty())?;
    if link.starts_with("/app/") || link == "/" {
        Some(link.to_string())
    } else {
        None
    }
}

fn build_fcm_message(token: &str, title: &str, body: &str, link: Option<&str>) -> serde_json::Value {
    let safe_link = safe_push_link(link);
    let mut data = serde_json::Map::new();
    if let Some(link) = safe_link.as_deref() {
        data.insert("link".into(), serde_json::Value::String(link.to_string()));
    }
    let mut message = serde_json::json!({
        "token": token,
        "notification": { "title": title, "body": body },
        "data": data,
    });
    if let Some(link) = safe_link {
        message["webpush"] = serde_json::json!({
            "fcm_options": { "link": link }
        });
    }
    serde_json::json!({ "message": message })
}
```

- [ ] **Step 4: Add JWT claims, token source, and cache**

Add this code below the payload helpers:

```rust
#[derive(Debug, Serialize)]
struct FcmJwtClaims {
    iss: String,
    scope: String,
    aud: String,
    exp: i64,
    iat: i64,
}

impl FcmJwtClaims {
    fn new(client_email: &str, token_uri: &str, now_unix: i64) -> Self {
        Self {
            iss: client_email.to_string(),
            scope: FCM_SCOPE.to_string(),
            aud: token_uri.to_string(),
            iat: now_unix,
            exp: now_unix + 3600,
        }
    }
}

#[derive(Debug, Clone)]
pub struct FcmAccessToken {
    pub token: String,
    pub expires_at_unix: i64,
}

#[async_trait]
trait FcmAccessTokenSource: Send + Sync {
    async fn mint_access_token(&self) -> Result<FcmAccessToken, NotifyError>;
}

trait Clock: Send + Sync {
    fn now_unix(&self) -> i64;
}

#[derive(Clone)]
struct SystemClock;

impl Clock for SystemClock {
    fn now_unix(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64
    }
}

#[derive(Clone)]
struct GoogleServiceAccountTokenSource {
    cfg: FcmProviderConfig,
    http: reqwest::Client,
    clock: SystemClock,
}

#[derive(Debug, Deserialize)]
struct OAuthTokenResponse {
    access_token: String,
    expires_in: Option<i64>,
}

#[async_trait]
impl FcmAccessTokenSource for GoogleServiceAccountTokenSource {
    async fn mint_access_token(&self) -> Result<FcmAccessToken, NotifyError> {
        let now = self.clock.now_unix();
        let mut header = Header::new(Algorithm::RS256);
        header.typ = Some("JWT".into());
        header.kid = self.cfg.private_key_id.clone();
        let claims = FcmJwtClaims::new(&self.cfg.client_email, &self.cfg.token_uri, now);
        let key = EncodingKey::from_rsa_pem(self.cfg.private_key.as_bytes())
            .map_err(|e| NotifyError::Api(format!("fcm service account key: {e}")))?;
        let assertion = jsonwebtoken::encode(&header, &claims, &key)
            .map_err(|e| NotifyError::Api(format!("fcm jwt sign: {e}")))?;
        let resp = self
            .http
            .post(&self.cfg.token_uri)
            .form(&[
                ("grant_type", OAUTH_GRANT_TYPE),
                ("assertion", assertion.as_str()),
            ])
            .send()
            .await
            .map_err(|e| NotifyError::Transport(e.to_string()))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(NotifyError::Api(format!("fcm oauth {status}: {}", sanitize_provider_text(&text))));
        }
        let parsed: OAuthTokenResponse = serde_json::from_str(&text)
            .map_err(|e| NotifyError::Api(format!("fcm oauth decode: {e}")))?;
        Ok(FcmAccessToken {
            token: parsed.access_token,
            expires_at_unix: now + parsed.expires_in.unwrap_or(3600),
        })
    }
}

struct CachedFcmAccessTokenProvider<S, C> {
    source: S,
    clock: C,
    cached: Mutex<Option<FcmAccessToken>>,
}

impl<S, C> CachedFcmAccessTokenProvider<S, C> {
    fn new(source: S, clock: C) -> Self {
        Self {
            source,
            clock,
            cached: Mutex::new(None),
        }
    }
}

impl<S, C> CachedFcmAccessTokenProvider<S, C>
where
    S: FcmAccessTokenSource,
    C: Clock,
{
    async fn access_token(&self) -> Result<String, NotifyError> {
        if let Some(token) = self.cached.lock().unwrap().clone() {
            if token.expires_at_unix - TOKEN_REFRESH_SKEW_SECONDS > self.clock.now_unix() {
                return Ok(token.token);
            }
        }
        let token = self.source.mint_access_token().await?;
        let value = token.token.clone();
        *self.cached.lock().unwrap() = Some(token);
        Ok(value)
    }
}

fn sanitize_provider_text(text: &str) -> String {
    text.replace('\n', " ").chars().take(500).collect()
}
```

- [ ] **Step 5: Add test-only token source helpers**

Inside `pub mod mock`, add:

```rust
#[derive(Clone)]
pub struct MockAccessTokenSource {
    results: Arc<Mutex<Vec<Result<FcmAccessToken, String>>>>,
    calls: Arc<Mutex<usize>>,
}

impl MockAccessTokenSource {
    pub fn new(results: Vec<Result<FcmAccessToken, String>>) -> Self {
        Self {
            results: Arc::new(Mutex::new(results)),
            calls: Arc::new(Mutex::new(0)),
        }
    }

    pub fn calls(&self) -> usize {
        *self.calls.lock().unwrap()
    }
}

#[async_trait]
impl FcmAccessTokenSource for MockAccessTokenSource {
    async fn mint_access_token(&self) -> Result<FcmAccessToken, NotifyError> {
        *self.calls.lock().unwrap() += 1;
        let next = self.results.lock().unwrap().remove(0);
        next.map_err(NotifyError::Api)
    }
}

#[derive(Clone)]
pub struct TestClock {
    now: i64,
}

impl TestClock {
    pub fn new(now: i64) -> Self {
        Self { now }
    }
}

impl Clock for TestClock {
    fn now_unix(&self) -> i64 {
        self.now
    }
}
```

- [ ] **Step 6: Run service tests**

Run:

```powershell
cargo test -p backend --lib services::notifications::tests::fcm -- --nocapture
```

Expected: the FCM config, payload, claims, and token cache tests pass.

- [ ] **Step 7: Commit FCM foundations**

Run:

```powershell
git add crates/backend/src/services/notifications.rs
git commit -m "feat(notifications): add fcm oauth foundations"
```

Expected: commit succeeds with service-only changes.

## Task 5: Replace FCM Sender Stub And Record Delivery Outcomes

**Files:**
- Modify: `crates/backend/src/services/notifications.rs`
- Modify: `crates/backend/src/main.rs`
- Modify: `crates/backend/tests/notifications.rs`

- [ ] **Step 1: Add failing send and facade tests**

Append these tests to `crates/backend/src/services/notifications.rs`:

```rust
#[tokio::test]
async fn fcm_sender_uses_cached_bearer_and_http_v1_endpoint() {
    let source = mock::MockAccessTokenSource::new(vec![Ok(FcmAccessToken {
        token: "access-1".into(),
        expires_at_unix: 1_800_003_600,
    })]);
    let provider = CachedFcmAccessTokenProvider::new(source, mock::TestClock::new(1_800_000_000));
    let sender = FcmPushSender::with_token_provider(
        "project-123",
        "http://127.0.0.1:9",
        provider,
        reqwest::Client::new(),
    );
    let err = sender
        .send_push(&["tok".to_string()], "Title", "Body", Some("/app/x"))
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("transport") || err.contains("provider api error"), "{err}");
}
```

Append this test to `crates/backend/tests/notifications.rs`:

```rust
#[tokio::test]
async fn notify_facade_records_delivery_rows() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (user, _, _email) = create_user(&pool).await;
    attach_membership(&pool, tenant, user, "student").await;
    let device_id = backend::db::notifications::register_device_token(
        &pool,
        tenant,
        user,
        "tok-for-delivery-log",
        "web",
        Some("Chrome"),
        None,
    )
    .await
    .unwrap();

    let email = Arc::new(backend::services::notifications::mock::MockEmailNotifier::new());
    let push = Arc::new(backend::services::notifications::mock::MockPushSender::new());

    backend::services::notifications::notify(
        &pool,
        email.as_ref(),
        push.as_ref(),
        tenant,
        user,
        "grade_released",
        "Grade released",
        Some("body text"),
        Some("/app/courses/math"),
    )
    .await;

    let rows = backend::db::notifications::list_deliveries_for_admin(
        &pool,
        tenant,
        user,
        backend::db::notifications::DeliveryFilter {
            channel: Some("push".into()),
            status: Some("sent".into()),
            limit: 20,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].device_token_id, Some(device_id));
    assert_eq!(rows[0].target_label.as_deref(), Some("web · Chrome"));
    assert!(!rows[0].target_hash.contains("tok-for-delivery-log"));

    let email_rows = backend::db::notifications::list_deliveries_for_admin(
        &pool,
        tenant,
        user,
        backend::db::notifications::DeliveryFilter {
            channel: Some("email".into()),
            status: Some("sent".into()),
            limit: 20,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(email_rows.len(), 1);
    assert_eq!(email_rows[0].target_label.as_deref(), Some("email"));
    assert!(!email_rows[0].target_hash.contains("@example.test"));
}
```

- [ ] **Step 2: Run the failing tests**

Run:

```powershell
cargo test -p backend --lib services::notifications::tests::fcm_sender_uses_cached_bearer_and_http_v1_endpoint -- --nocapture
cargo test -p backend --test notifications notify_facade_records_delivery_rows -- --nocapture
```

Expected: compile fails because `with_token_provider` and channel delivery logging in `notify` do not exist.

- [ ] **Step 3: Replace the FCM sender internals**

Replace the old `fcm_without_token_is_not_configured` test with the config-disabled test from Task 4, then replace `FcmPushSender` with a generic cached-provider sender while keeping `PushSender` stable:

```rust
pub struct FcmPushSender<P = CachedFcmAccessTokenProvider<GoogleServiceAccountTokenSource, SystemClock>> {
    project_id: String,
    base_url: String,
    token_provider: P,
    http: reqwest::Client,
}

impl FcmPushSender {
    pub fn from_config(cfg: FcmProviderConfig) -> Self {
        let source = GoogleServiceAccountTokenSource {
            cfg: cfg.clone(),
            http: reqwest::Client::new(),
            clock: SystemClock,
        };
        let provider = CachedFcmAccessTokenProvider::new(source, SystemClock);
        Self {
            project_id: cfg.project_id,
            base_url: cfg.fcm_base_url,
            token_provider: provider,
            http: reqwest::Client::new(),
        }
    }
}

impl<P> FcmPushSender<P> {
    fn with_token_provider(
        project_id: impl Into<String>,
        base_url: impl Into<String>,
        token_provider: P,
        http: reqwest::Client,
    ) -> Self {
        Self {
            project_id: project_id.into(),
            base_url: base_url.into(),
            token_provider,
            http,
        }
    }
}

#[async_trait]
impl<P> PushSender for FcmPushSender<P>
where
    P: Send + Sync,
    P: FcmTokenProvider,
{
    async fn send_push(
        &self,
        tokens: &[String],
        title: &str,
        body: &str,
        link: Option<&str>,
    ) -> Result<(), NotifyError> {
        let access_token = self.token_provider.access_token().await?;
        let url = format!(
            "{}/v1/projects/{}/messages:send",
            self.base_url, self.project_id
        );
        let mut first_err: Option<NotifyError> = None;
        for token in tokens {
            let message = build_fcm_message(token, title, body, link);
            let resp = self
                .http
                .post(&url)
                .bearer_auth(&access_token)
                .json(&message)
                .send()
                .await;
            match resp {
                Ok(r) if r.status().is_success() => {}
                Ok(r) => {
                    let status = r.status();
                    let text = r.text().await.unwrap_or_default();
                    let e = NotifyError::Api(format!("fcm {status}: {}", sanitize_provider_text(&text)));
                    tracing::warn!(error = %e, "fcm send_push per-token failure");
                    first_err.get_or_insert(e);
                }
                Err(e) => {
                    let e = NotifyError::Transport(e.to_string());
                    tracing::warn!(error = %e, "fcm send_push transport failure");
                    first_err.get_or_insert(e);
                }
            }
        }
        match first_err {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}
```

Add this trait so the generic sender can call cached token providers:

```rust
#[async_trait]
trait FcmTokenProvider: Send + Sync {
    async fn access_token(&self) -> Result<String, NotifyError>;
}

#[async_trait]
impl<S, C> FcmTokenProvider for CachedFcmAccessTokenProvider<S, C>
where
    S: FcmAccessTokenSource,
    C: Clock,
{
    async fn access_token(&self) -> Result<String, NotifyError> {
        CachedFcmAccessTokenProvider::access_token(self).await
    }
}
```

Add an explicit disabled sender for configured-off environments:

```rust
#[derive(Clone, Default)]
pub struct DisabledPushSender;

#[async_trait]
impl PushSender for DisabledPushSender {
    async fn send_push(
        &self,
        tokens: &[String],
        title: &str,
        _body: &str,
        _link: Option<&str>,
    ) -> Result<(), NotifyError> {
        tracing::debug!(token_count = tokens.len(), %title, "push disabled; skipping send");
        Err(NotifyError::NotConfigured)
    }
}
```

- [ ] **Step 4: Update `notify` to record channel outcomes**

In `notify`, capture the in-app notification id before the push block:

```rust
let mut notification_id: Option<Uuid> = None;

if prefs.in_app_enabled {
    match ndb::create_notification(pool, tenant_id, user_id, kind, title, body, link).await {
        Ok(row) => notification_id = Some(row.id),
        Err(e) => {
            tracing::warn!(error = %e, %user_id, %kind, "notify: create_notification failed");
        }
    }
}
```

Then replace the push block with one-device-at-a-time delivery logging:

```rust
if prefs.push_enabled {
    match ndb::list_active_device_tokens(pool, tenant_id, user_id).await {
        Ok(devices) if !devices.is_empty() => {
            let push_body = body.unwrap_or(title);
            for (device_id, token, platform, label) in devices {
                let target_hash = ndb::target_hash("push", &token);
                let target_label = ndb::device_target_label(&platform, label.as_deref());
                let (status, error_code, error_message) = match push
                    .send_push(std::slice::from_ref(&token), title, push_body, link)
                    .await
                {
                    Ok(()) => ("sent", None, None),
                    Err(NotifyError::NotConfigured) => ("skipped", Some("push_not_configured"), None),
                    Err(e) => {
                        tracing::warn!(error = %e, %user_id, "notify: send_push failed");
                        ("failed", None, Some(e.to_string()))
                    }
                };
                if let Err(e) = ndb::record_delivery(
                    pool,
                    ndb::NewDelivery {
                        tenant_id,
                        user_id,
                        notification_id,
                        channel: "push",
                        provider: "fcm",
                        target_hash: &target_hash,
                        target_label: Some(&target_label),
                        device_token_id: Some(device_id),
                        kind,
                        status,
                        provider_message_id: None,
                        provider_status: None,
                        error_code,
                        error_message: error_message.as_deref(),
                    },
                )
                .await
                {
                    tracing::warn!(error = %e, %user_id, "notify: record push delivery failed");
                }
            }
        }
        Ok(_) => {
            tracing::trace!(%user_id, "notify: no device tokens; skipping push channel");
        }
        Err(e) => {
            tracing::warn!(error = %e, %user_id, "notify: list_device_tokens failed");
        }
    }
} else {
    let target_hash = ndb::target_hash("push", "preference-disabled");
    if let Err(e) = ndb::record_delivery(
        pool,
        ndb::NewDelivery {
            tenant_id,
            user_id,
            notification_id,
            channel: "push",
            provider: "fcm",
            target_hash: &target_hash,
            target_label: Some("push preference disabled"),
            device_token_id: None,
            kind,
            status: "skipped",
            provider_message_id: None,
            provider_status: None,
            error_code: Some("push_disabled"),
            error_message: None,
        },
    )
    .await
    {
        tracing::warn!(error = %e, %user_id, "notify: record push skipped delivery failed");
    }
}
```

Replace the email block with a version that records sent, failed, and missing-email outcomes:

```rust
if prefs.email_enabled {
    match ndb::lookup_user_email(pool, user_id).await {
        Ok(Some(to)) => {
            let text = body.unwrap_or("");
            let (esc_subject, html) = build_email_subject_html(title, body, link);
            let (status, error_message) = match email.send_email(&to, &esc_subject, &html, text).await {
                Ok(()) => ("sent", None),
                Err(e) => {
                    tracing::warn!(error = %e, %user_id, "notify: send_email failed");
                    ("failed", Some(e.to_string()))
                }
            };
            let target_hash = ndb::target_hash("email", &to);
            if let Err(e) = ndb::record_delivery(
                pool,
                ndb::NewDelivery {
                    tenant_id,
                    user_id,
                    notification_id,
                    channel: "email",
                    provider: "resend",
                    target_hash: &target_hash,
                    target_label: Some("email"),
                    device_token_id: None,
                    kind,
                    status,
                    provider_message_id: None,
                    provider_status: None,
                    error_code: None,
                    error_message: error_message.as_deref(),
                },
            )
            .await
            {
                tracing::warn!(error = %e, %user_id, "notify: record email delivery failed");
            }
        }
        Ok(None) => {
            let target_hash = ndb::target_hash("email", "missing-email");
            if let Err(e) = ndb::record_delivery(
                pool,
                ndb::NewDelivery {
                    tenant_id,
                    user_id,
                    notification_id,
                    channel: "email",
                    provider: "resend",
                    target_hash: &target_hash,
                    target_label: Some("email missing"),
                    device_token_id: None,
                    kind,
                    status: "skipped",
                    provider_message_id: None,
                    provider_status: None,
                    error_code: Some("email_missing"),
                    error_message: None,
                },
            )
            .await
            {
                tracing::warn!(error = %e, %user_id, "notify: record email skipped delivery failed");
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, %user_id, "notify: lookup_user_email failed");
        }
    }
}
```

- [ ] **Step 5: Wire startup config**

In `crates/backend/src/main.rs`, replace the FCM scaffold block with:

```rust
let push_sender: Arc<dyn backend::services::notifications::PushSender> =
    match backend::services::notifications::FcmProviderConfig::from_env() {
        Ok(Some(cfg)) => {
            tracing::info!(project_id = %cfg.project_id, "FCM push sender configured");
            Arc::new(backend::services::notifications::FcmPushSender::from_config(cfg))
        }
        Ok(None) => {
            tracing::warn!(
                "FCM push is not fully configured; using DisabledPushSender. Push notification attempts will be recorded as skipped."
            );
            Arc::new(backend::services::notifications::DisabledPushSender::default())
        }
        Err(err) => {
            tracing::warn!(
                error = %err,
                "FCM push configuration is invalid; using DisabledPushSender. Push notification attempts will be recorded as skipped."
            );
            Arc::new(backend::services::notifications::DisabledPushSender::default())
        }
    };
```

- [ ] **Step 6: Run backend tests**

Run:

```powershell
cargo test -p backend --lib services::notifications -- --nocapture
cargo test -p backend --test notifications notify_facade_records_delivery_rows -- --nocapture
```

Expected: service tests and the delivery-log facade test pass.

- [ ] **Step 7: Commit sender and delivery integration**

Run:

```powershell
git add crates/backend/src/services/notifications.rs crates/backend/src/main.rs crates/backend/tests/notifications.rs
git commit -m "feat(notifications): send fcm with service account tokens"
```

Expected: commit succeeds with service, startup, and test changes.

## Task 6: Add Frontend API Contracts

**Files:**
- Modify: `crates/features-courses/src/api.rs`

- [ ] **Step 1: Add failing API contract tests**

Append these tests to the existing `#[cfg(test)] mod tests` in `crates/features-courses/src/api.rs`:

```rust
#[test]
fn device_token_list_dto_decodes_without_raw_token() {
    let json = serde_json::json!({
        "devices": [{
            "id": "11111111-1111-1111-1111-111111111111",
            "platform": "web",
            "label": "Chrome",
            "user_agent": "Mozilla/5.0",
            "created_at": "2026-06-18T00:00:00Z",
            "last_seen_at": "2026-06-18T00:01:00Z"
        }]
    });
    let dto: DeviceTokenListDto = serde_json::from_value(json).unwrap();
    assert_eq!(dto.devices[0].platform, "web");
    assert_eq!(dto.devices[0].label.as_deref(), Some("Chrome"));
}

#[test]
fn delivery_list_dto_decodes_sanitized_provider_fields() {
    let json = serde_json::json!({
        "deliveries": [{
            "id": "11111111-1111-1111-1111-111111111111",
            "user_id": "22222222-2222-2222-2222-222222222222",
            "notification_id": null,
            "channel": "push",
            "provider": "fcm",
            "target_hash": "abc123",
            "target_label": "web · Chrome",
            "device_token_id": null,
            "kind": "grade_released",
            "status": "failed",
            "provider_message_id": null,
            "provider_status": "404",
            "error_code": "UNREGISTERED",
            "error_message": "token is not registered",
            "created_at": "2026-06-18T00:00:00Z",
            "updated_at": "2026-06-18T00:00:00Z"
        }]
    });
    let dto: DeliveryListDto = serde_json::from_value(json).unwrap();
    assert_eq!(dto.deliveries[0].target_hash, "abc123");
    assert_eq!(dto.deliveries[0].status, "failed");
}
```

- [ ] **Step 2: Run failing API tests**

Run:

```powershell
cargo test -p features-courses --lib device_token_list_dto_decodes_without_raw_token delivery_list_dto_decodes_sanitized_provider_fields
```

Expected: compile fails because DTOs do not exist.

- [ ] **Step 3: Add DTOs and helpers**

Add this block after the existing notification device-token API:

```rust
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct DeviceTokenDto {
    pub id: String,
    pub platform: String,
    pub label: Option<String>,
    pub user_agent: Option<String>,
    pub created_at: String,
    pub last_seen_at: String,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct DeviceTokenListDto {
    pub devices: Vec<DeviceTokenDto>,
}

#[derive(serde::Serialize)]
struct RegisterDeviceTokenBody<'a> {
    token: &'a str,
    platform: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    user_agent: Option<&'a str>,
}

pub async fn register_device_token_with_metadata(
    ctx: &ApiContext,
    token: &str,
    platform: &str,
    label: Option<&str>,
    user_agent: Option<&str>,
) -> Result<(), ApiError> {
    fetch_json(
        ctx,
        "POST",
        "/v1/me/device-tokens",
        Some(&RegisterDeviceTokenBody {
            token,
            platform,
            label,
            user_agent,
        }),
    )
    .await
}

pub async fn list_device_tokens(ctx: &ApiContext) -> Result<DeviceTokenListDto, ApiError> {
    fetch_json(ctx, "GET", "/v1/me/device-tokens", None::<&()>).await
}

pub async fn revoke_device_token(ctx: &ApiContext, id: &str) -> Result<(), ApiError> {
    fetch_json(
        ctx,
        "DELETE",
        &format!("/v1/me/device-tokens/{id}"),
        None::<&()>,
    )
    .await
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct DeliveryDto {
    pub id: String,
    pub user_id: String,
    pub notification_id: Option<String>,
    pub channel: String,
    pub provider: String,
    pub target_hash: String,
    pub target_label: Option<String>,
    pub device_token_id: Option<String>,
    pub kind: String,
    pub status: String,
    pub provider_message_id: Option<String>,
    pub provider_status: Option<String>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct DeliveryListDto {
    pub deliveries: Vec<DeliveryDto>,
}

pub async fn list_notification_deliveries(
    ctx: &ApiContext,
    channel: Option<&str>,
    status: Option<&str>,
    limit: Option<i64>,
) -> Result<DeliveryListDto, ApiError> {
    let mut path = String::from("/v1/admin/notification-deliveries");
    let mut parts = Vec::<String>::new();
    if let Some(value) = channel {
        parts.push(format!("channel={}", urlencode(value)));
    }
    if let Some(value) = status {
        parts.push(format!("status={}", urlencode(value)));
    }
    if let Some(value) = limit {
        parts.push(format!("limit={value}"));
    }
    if !parts.is_empty() {
        path.push('?');
        path.push_str(&parts.join("&"));
    }
    fetch_json(ctx, "GET", &path, None::<&()>).await
}

pub async fn get_notification_delivery(ctx: &ApiContext, id: &str) -> Result<DeliveryDto, ApiError> {
    fetch_json(
        ctx,
        "GET",
        &format!("/v1/admin/notification-deliveries/{id}"),
        None::<&()>,
    )
    .await
}
```

Update the old `register_device_token` helper to delegate:

```rust
pub async fn register_device_token(
    ctx: &ApiContext,
    token: &str,
    platform: &str,
) -> Result<(), ApiError> {
    register_device_token_with_metadata(ctx, token, platform, None, None).await
}
```

- [ ] **Step 4: Run API tests**

Run:

```powershell
cargo test -p features-courses --lib device_token_list_dto_decodes_without_raw_token delivery_list_dto_decodes_sanitized_provider_fields
```

Expected: API contract tests pass.

- [ ] **Step 5: Commit API contracts**

Run:

```powershell
git add crates/features-courses/src/api.rs
git commit -m "feat(notifications): add frontend delivery api contracts"
```

Expected: commit succeeds with API-only changes.

## Task 7: Improve Browser Push Bridge Result States

**Files:**
- Modify: `crates/platform-bridge/src/web.rs`
- Modify: `crates/shell-web/public/assets/fcm-bridge.js`

- [ ] **Step 1: Add typed Rust result**

In `crates/platform-bridge/src/web.rs`, add:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FcmRequestTokenOutcome {
    Token(String),
    MissingVapidKey,
    Unsupported,
    PermissionDenied,
    ServiceWorkerFailed,
    TokenFailed,
}
```

Replace `fcm_request_token` with:

```rust
pub async fn fcm_request_token() -> Result<Option<String>, BridgeError> {
    match fcm_request_token_outcome().await? {
        FcmRequestTokenOutcome::Token(token) => Ok(Some(token)),
        _ => Ok(None),
    }
}

pub async fn fcm_request_token_outcome() -> Result<FcmRequestTokenOutcome, BridgeError> {
    let value = js_fcm_request_token().await?;
    if value.is_null() || value.is_undefined() {
        return Ok(FcmRequestTokenOutcome::TokenFailed);
    }
    if let Some(token) = value.as_string() {
        return Ok(FcmRequestTokenOutcome::Token(token));
    }
    let status = js_sys::Reflect::get(&value, &JsValue::from_str("status"))
        .ok()
        .and_then(|v| v.as_string())
        .unwrap_or_else(|| "token_failed".into());
    let token = js_sys::Reflect::get(&value, &JsValue::from_str("token"))
        .ok()
        .and_then(|v| v.as_string());
    Ok(match (status.as_str(), token) {
        ("token", Some(token)) => FcmRequestTokenOutcome::Token(token),
        ("missing_vapid_key", _) => FcmRequestTokenOutcome::MissingVapidKey,
        ("unsupported", _) => FcmRequestTokenOutcome::Unsupported,
        ("permission_denied", _) => FcmRequestTokenOutcome::PermissionDenied,
        ("service_worker_failed", _) => FcmRequestTokenOutcome::ServiceWorkerFailed,
        _ => FcmRequestTokenOutcome::TokenFailed,
    })
}
```

- [ ] **Step 2: Update the JS bridge**

In `crates/shell-web/public/assets/fcm-bridge.js`, replace `return null` branches and token success with:

```javascript
      if (!vapidKey) {
        return { status: "missing_vapid_key" };
      }
      if (
        typeof Notification === "undefined" ||
        !("serviceWorker" in navigator)
      ) {
        return { status: "unsupported" };
      }
```

For service-worker failure:

```javascript
        return { status: "service_worker_failed" };
```

For permission denial:

```javascript
        return { status: "permission_denied" };
```

For token success/failure:

```javascript
        return token ? { status: "token", token } : { status: "token_failed" };
      } catch (error) {
        console.error("FCM getToken failed", error);
        return { status: "token_failed" };
      }
```

- [ ] **Step 3: Run platform bridge checks**

Run:

```powershell
cargo check -p platform-bridge --target wasm32-unknown-unknown
```

Expected: wasm platform bridge compiles.

- [ ] **Step 4: Commit bridge result states**

Run:

```powershell
git add crates/platform-bridge/src/web.rs crates/shell-web/public/assets/fcm-bridge.js
git commit -m "feat(notifications): classify browser push setup states"
```

Expected: commit succeeds with browser bridge changes.

## Task 8: Add Device Management To Notification Settings

**Files:**
- Modify: `crates/shell-web/src/routes/notification_settings.rs`
- Modify: `crates/shell-web/Cargo.toml`
- Modify: `crates/design-system/assets/components.css`
- Modify: `crates/shell-web/public/assets/components.css`

- [ ] **Step 1: Add failing SSR tests**

Append these tests to `notification_settings.rs`:

```rust
#[test]
fn device_table_renders_registered_devices_without_tokens() {
    fn app() -> Element {
        device_list_section(
            &[
                api::DeviceTokenDto {
                    id: "device-1".into(),
                    platform: "web".into(),
                    label: Some("Chrome".into()),
                    user_agent: Some("Mozilla/5.0".into()),
                    created_at: "2026-06-18T00:00:00Z".into(),
                    last_seen_at: "2026-06-18T00:01:00Z".into(),
                }
            ],
            None,
            false,
            EventHandler::new(|_| {}),
            EventHandler::new(|_| {}),
        )
    }
    let mut vdom = VirtualDom::new(app);
    vdom.rebuild_in_place();
    let html = dioxus_ssr::render(&vdom);
    assert!(html.contains("Registered devices"), "{html}");
    assert!(html.contains("Chrome"), "{html}");
    assert!(html.contains("Revoke"), "{html}");
    assert!(!html.contains("tok-"), "{html}");
}

#[test]
fn browser_push_status_text_distinguishes_denied_permission() {
    assert_eq!(
        push_status_copy(&BrowserPushStatus::PermissionDenied),
        "Notifications are blocked in this browser."
    );
    assert_eq!(
        push_status_copy(&BrowserPushStatus::MissingConfig),
        "Browser push is not configured for this workspace."
    );
}
```

- [ ] **Step 2: Run failing SSR tests**

Run:

```powershell
cargo test -p shell-web --lib notification_settings::ssr_tests::device_table_renders_registered_devices_without_tokens notification_settings::ssr_tests::browser_push_status_text_distinguishes_denied_permission
```

Expected: compile fails because helpers and state types do not exist.

- [ ] **Step 3: Add UI state helpers**

In `notification_settings.rs`, update imports:

```rust
use design_system::{
    use_toast_sender, Badge, BadgeTone, Button, ButtonVariant, Card, PageHeader, SkeletonCard,
    Switch, ToastLevel,
};
```

Add these helpers above `NotificationSettings`:

```rust
#[derive(Clone, Debug, PartialEq)]
enum BrowserPushStatus {
    Idle,
    Enabled,
    MissingConfig,
    Unsupported,
    PermissionDenied,
    ServiceWorkerFailed,
    TokenFailed,
}

fn push_status_copy(status: &BrowserPushStatus) -> &'static str {
    match status {
        BrowserPushStatus::Idle => "",
        BrowserPushStatus::Enabled => "Browser notifications are enabled on this device.",
        BrowserPushStatus::MissingConfig => "Browser push is not configured for this workspace.",
        BrowserPushStatus::Unsupported => "This browser does not support push notifications.",
        BrowserPushStatus::PermissionDenied => "Notifications are blocked in this browser.",
        BrowserPushStatus::ServiceWorkerFailed => "The notification service worker could not start.",
        BrowserPushStatus::TokenFailed => "The browser could not create a push token.",
    }
}

fn device_label(device: &api::DeviceTokenDto) -> String {
    device
        .label
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(device.platform.as_str())
        .to_string()
}

fn device_list_section(
    devices: &[api::DeviceTokenDto],
    error: Option<&str>,
    loading: bool,
    on_retry: EventHandler<()>,
    on_revoke: EventHandler<String>,
) -> Element {
    rsx! {
        div { class: "notif-device-section",
            div { class: "notif-device-section-head",
                h2 { class: "notif-settings-section-title", "Registered devices" }
                Button {
                    label: "Refresh".to_string(),
                    variant: ButtonVariant::Secondary,
                    loading,
                    on_click: move |_| on_retry.call(()),
                }
            }
            if let Some(err) = error {
                p { class: "error", "Could not load registered devices: {err}" }
            } else if loading {
                SkeletonCard { height: "96px".to_string() }
            } else if devices.is_empty() {
                p { class: "muted", "No registered devices." }
            } else {
                div { class: "notif-device-list",
                    for device in devices {
                        div { class: "notif-device-row", key: "{device.id}",
                            div { class: "notif-device-main",
                                span { class: "notif-device-label", "{device_label(device)}" }
                                span { class: "notif-device-meta", "{device.platform} · Last seen {device.last_seen_at}" }
                            }
                            Badge { label: device.platform.clone(), tone: BadgeTone::Info }
                            Button {
                                label: "Revoke".to_string(),
                                variant: ButtonVariant::Danger,
                                on_click: {
                                    let id = device.id.clone();
                                    move |_| on_revoke.call(id.clone())
                                },
                            }
                        }
                    }
                }
            }
        }
    }
}
```

- [ ] **Step 4: Add the wasm Navigator feature**

In `crates/shell-web/Cargo.toml`, extend the existing target-specific `web-sys` features:

```toml
web-sys = { version = "0.3", features = ["Window", "Storage", "Location", "Navigator"] }
```

- [ ] **Step 5: Load and refresh device rows**

Inside `NotificationSettings`, add signals:

```rust
let mut devices = use_signal(Vec::<api::DeviceTokenDto>::new);
let mut devices_loading = use_signal(|| true);
let mut devices_error = use_signal(|| None::<String>);
let mut push_status = use_signal(|| BrowserPushStatus::Idle);
```

Add a load closure:

```rust
let load_devices = {
    let api = api.clone();
    move || {
        let api = api.clone();
        spawn(async move {
            devices_loading.set(true);
            devices_error.set(None);
            match api::list_device_tokens(&api).await {
                Ok(resp) => devices.set(resp.devices),
                Err(e) => devices_error.set(Some(format!("{e}"))),
            }
            devices_loading.set(false);
        });
    }
};
```

Call it from `use_effect` once:

```rust
{
    let load_devices = load_devices.clone();
    use_effect(move || {
        load_devices();
    });
}
```

Add revoke handler:

```rust
let revoke_device = {
    let api = api.clone();
    move |id: String| {
        let api = api.clone();
        let mut toast = toast;
        spawn(async move {
            match api::revoke_device_token(&api, &id).await {
                Ok(()) => {
                    devices.write().retain(|device| device.id != id);
                    toast.push(ToastLevel::Success, "Device revoked", "This device will no longer receive push notifications.");
                }
                Err(err) => {
                    toast.push(ToastLevel::Danger, "Could not revoke device", format!("{err}"));
                }
            }
        });
    }
};
```

- [ ] **Step 6: Update browser push enablement**

In the wasm branch of `enable_browser_push`, replace the current `fcm_request_token` match with:

```rust
match platform_bridge::web::fcm_request_token_outcome().await {
    Ok(platform_bridge::web::FcmRequestTokenOutcome::Token(token)) => {
        let label = web_sys::window()
            .and_then(|window| window.navigator().user_agent().ok())
            .map(|ua| {
                if ua.contains("Chrome") { "Chrome" }
                else if ua.contains("Firefox") { "Firefox" }
                else if ua.contains("Safari") { "Safari" }
                else { "Browser" }
            })
            .unwrap_or("Browser")
            .to_string();
        let ua = web_sys::window().and_then(|window| window.navigator().user_agent().ok());
        match api::register_device_token_with_metadata(&_api, &token, "web", Some(&label), ua.as_deref()).await {
            Ok(()) => {
                push_status.set(BrowserPushStatus::Enabled);
                toast.push(
                    ToastLevel::Success,
                    "Browser notifications enabled",
                    "This browser will now receive push notifications.",
                );
                load_devices();
            }
            Err(err) => toast.push(ToastLevel::Danger, "Could not enable", format!("{err}")),
        }
    }
    Ok(platform_bridge::web::FcmRequestTokenOutcome::MissingVapidKey) => push_status.set(BrowserPushStatus::MissingConfig),
    Ok(platform_bridge::web::FcmRequestTokenOutcome::Unsupported) => push_status.set(BrowserPushStatus::Unsupported),
    Ok(platform_bridge::web::FcmRequestTokenOutcome::PermissionDenied) => push_status.set(BrowserPushStatus::PermissionDenied),
    Ok(platform_bridge::web::FcmRequestTokenOutcome::ServiceWorkerFailed) => push_status.set(BrowserPushStatus::ServiceWorkerFailed),
    Ok(platform_bridge::web::FcmRequestTokenOutcome::TokenFailed) => push_status.set(BrowserPushStatus::TokenFailed),
    Err(err) => {
        push_status.set(BrowserPushStatus::TokenFailed);
        toast.push(ToastLevel::Danger, "Could not enable", format!("{err}"));
    }
}
```

Add status copy under the browser push section:

```rust
let status_copy = push_status_copy(&push_status.read());
if !status_copy.is_empty() {
    p { class: "notif-push-status", "{status_copy}" }
}
```

Render the device section beneath the notification preference card:

```rust
Card {
    {device_list_section(
        &devices.read(),
        devices_error.read().as_deref(),
        *devices_loading.read(),
        EventHandler::new(move |_| load_devices()),
        EventHandler::new(move |id| revoke_device(id)),
    )}
}
```

- [ ] **Step 7: Add CSS**

Append this block to both `crates/design-system/assets/components.css` and `crates/shell-web/public/assets/components.css`:

```css
.notif-device-section {
  display: grid;
  gap: var(--space-3);
}

.notif-device-section-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-3);
}

.notif-device-list {
  display: grid;
  gap: var(--space-2);
}

.notif-device-row {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto auto;
  align-items: center;
  gap: var(--space-3);
  padding: var(--space-3);
  border: 1px solid var(--color-rule);
  border-radius: var(--radius-sm);
  background: var(--surface-card);
}

.notif-device-main {
  min-width: 0;
  display: grid;
  gap: 2px;
}

.notif-device-label {
  font-weight: 650;
  color: var(--color-text);
}

.notif-device-meta,
.notif-push-status {
  color: var(--color-muted);
  font-size: var(--font-size-sm);
}

@media (max-width: 720px) {
  .notif-device-row {
    grid-template-columns: minmax(0, 1fr);
    align-items: start;
  }
}
```

- [ ] **Step 8: Run shell-web tests**

Run:

```powershell
cargo test -p shell-web --lib notification_settings -- --nocapture
```

Expected: notification settings SSR tests pass.

- [ ] **Step 9: Commit notification settings UI**

Run:

```powershell
git add crates/shell-web/src/routes/notification_settings.rs crates/shell-web/Cargo.toml crates/design-system/assets/components.css crates/shell-web/public/assets/components.css
git commit -m "feat(notifications): add device management settings"
```

Expected: commit succeeds with settings UI and CSS changes.

## Task 9: Add Admin Delivery Diagnostics UI

**Files:**
- Create: `crates/shell-web/src/routes/admin_notification_deliveries.rs`
- Modify: `crates/shell-web/src/routes/mod.rs`
- Modify: `crates/shell-web/src/route_enum.rs`
- Modify: `crates/features-courses/src/app_shell.rs`
- Modify: `crates/shell-web/tests/shell_routes_smoke.rs`
- Modify: `crates/design-system/assets/components.css`
- Modify: `crates/shell-web/public/assets/components.css`

- [ ] **Step 1: Create route file with failing SSR tests**

Create `crates/shell-web/src/routes/admin_notification_deliveries.rs`:

```rust
use core_types::TenantRole;
use design_system::{Badge, BadgeTone, Sheet, SheetBody, SheetClose, SheetHeader, SheetTitle, Table};
use dioxus::prelude::*;
use dioxus_router::use_navigator;
use features_courses::api;
use features_courses::api::DeliveryDto;
use features_courses::app_shell::{AppShell, ShellUser};

use crate::route_enum::Route;
use crate::routes::{use_api, use_user_context};

const PAGE_SIZE: i64 = 50;

fn status_tone(status: &str) -> BadgeTone {
    match status {
        "sent" => BadgeTone::Success,
        "failed" => BadgeTone::Danger,
        "skipped" => BadgeTone::Warning,
        "queued" => BadgeTone::Info,
        _ => BadgeTone::Neutral,
    }
}

fn short_hash(hash: &str) -> String {
    let prefix: String = hash.chars().take(10).collect();
    if hash.chars().count() > 10 {
        format!("{prefix}…")
    } else {
        prefix
    }
}

fn deliveries_table(deliveries: &[DeliveryDto], on_select: EventHandler<String>) -> Element {
    rsx! {
        Table {
            compact: true,
            striped: true,
            head: rsx! {
                tr {
                    th { class: "ds-table-th", "When" }
                    th { class: "ds-table-th", "Status" }
                    th { class: "ds-table-th", "Channel" }
                    th { class: "ds-table-th", "Provider" }
                    th { class: "ds-table-th", "Kind" }
                    th { class: "ds-table-th", "Target" }
                    th { class: "ds-table-th", "Details" }
                }
            },
            body: rsx! {
                for row in deliveries {
                    tr { key: "{row.id}",
                        td { "{row.created_at}" }
                        td { Badge { label: row.status.clone(), tone: status_tone(&row.status) } }
                        td { "{row.channel}" }
                        td { "{row.provider}" }
                        td { "{row.kind}" }
                        td { "{row.target_label.clone().unwrap_or_else(|| short_hash(&row.target_hash))}" }
                        td {
                            button {
                                class: "linkish",
                                onclick: {
                                    let id = row.id.clone();
                                    move |_| on_select.call(id.clone())
                                },
                                "Open"
                            }
                        }
                    }
                }
            },
        }
    }
}

fn delivery_detail(open: Signal<bool>, delivery: Option<DeliveryDto>) -> Element {
    rsx! {
        Sheet { open, width: 520,
            SheetHeader {
                SheetTitle { "Delivery details" }
                SheetClose { open }
            }
            SheetBody {
                if let Some(row) = delivery {
                    dl { class: "delivery-detail-list",
                        dt { "Status" } dd { "{row.status}" }
                        dt { "Provider" } dd { "{row.provider}" }
                        dt { "Provider status" } dd { "{row.provider_status.unwrap_or_else(|| \"None\".into())}" }
                        dt { "Error code" } dd { "{row.error_code.unwrap_or_else(|| \"None\".into())}" }
                        dt { "Error message" } dd { "{row.error_message.unwrap_or_else(|| \"None\".into())}" }
                        dt { "Target" } dd { "{row.target_label.unwrap_or_else(|| short_hash(&row.target_hash))}" }
                    }
                }
            }
        }
    }
}

#[component]
pub fn AdminNotificationDeliveries() -> Element {
    let nav = use_navigator();
    let api = use_api();
    let user_ctx = use_user_context();

    let user = match user_ctx.read().clone() {
        Some(u) => u,
        None => {
            nav.push(Route::Login {});
            return rsx! { p { "Redirecting…" } };
        }
    };
    let is_admin = user.is_platform_admin || matches!(user.tenant_role, Some(TenantRole::OrgAdmin));
    if !is_admin {
        return rsx! {
            div { class: "container",
                h1 { "Forbidden" }
                p { "You don't have permission to view notification deliveries." }
            }
        };
    }

    let mut deliveries = use_signal(Vec::<DeliveryDto>::new);
    let mut loading = use_signal(|| true);
    let mut load_error = use_signal(|| None::<String>);
    let mut selected = use_signal(|| None::<DeliveryDto>);
    let sheet_open = use_signal(|| false);

    {
        let api = api.clone();
        use_effect(move || {
            let api = api.clone();
            spawn(async move {
                loading.set(true);
                match api::list_notification_deliveries(&api, None, None, Some(PAGE_SIZE)).await {
                    Ok(resp) => {
                        load_error.set(None);
                        deliveries.set(resp.deliveries);
                    }
                    Err(err) => load_error.set(Some(format!("{err}"))),
                }
                loading.set(false);
            });
        });
    }

    let open_delivery = {
        let api = api.clone();
        move |id: String| {
            let api = api.clone();
            let mut selected = selected;
            let mut sheet_open = sheet_open;
            spawn(async move {
                if let Ok(row) = api::get_notification_delivery(&api, &id).await {
                    selected.set(Some(row));
                    sheet_open.set(true);
                }
            });
        }
    };

    let body = match (load_error.read().clone(), *loading.read(), deliveries.read().is_empty()) {
        (Some(err), _, true) => rsx! {
            div { class: "admin-deliveries-page",
                h1 { "Notification deliveries" }
                p { class: "error", "Could not load delivery diagnostics: {err}" }
            }
        },
        (None, true, true) => rsx! {
            div { class: "admin-deliveries-page",
                h1 { "Notification deliveries" }
                p { "Loading delivery diagnostics…" }
            }
        },
        (None, false, true) => rsx! {
            div { class: "admin-deliveries-page",
                h1 { "Notification deliveries" }
                p { class: "muted", "No delivery attempts recorded yet." }
            }
        },
        _ => {
            let rows = deliveries.read().clone();
            rsx! {
                div { class: "admin-deliveries-page",
                    h1 { "Notification deliveries" }
                    p { class: "muted", "{rows.len()} delivery attempts loaded." }
                    {deliveries_table(&rows, EventHandler::new(move |id| open_delivery(id)))}
                    {delivery_detail(sheet_open, selected.read().clone())}
                }
            }
        }
    };

    let shell_user = ShellUser {
        display_name: user.display_name.clone(),
        email: user.email.clone(),
        tenant_role: user.tenant_role.clone(),
        is_platform_admin: user.is_platform_admin,
    };

    rsx! {
        AppShell {
            user: shell_user,
            on_signout: move |_| {
                nav.push(Route::Login {});
            },
            {body}
        }
    }
}

#[cfg(test)]
mod ssr_tests {
    use super::*;

    fn sample() -> Vec<DeliveryDto> {
        vec![DeliveryDto {
            id: "delivery-1".into(),
            user_id: "user-1".into(),
            notification_id: None,
            channel: "push".into(),
            provider: "fcm".into(),
            target_hash: "abcdef123456".into(),
            target_label: Some("web · Chrome".into()),
            device_token_id: None,
            kind: "grade_released".into(),
            status: "failed".into(),
            provider_message_id: None,
            provider_status: Some("404".into()),
            error_code: Some("UNREGISTERED".into()),
            error_message: Some("token is not registered".into()),
            created_at: "2026-06-18T00:00:00Z".into(),
            updated_at: "2026-06-18T00:00:00Z".into(),
        }]
    }

    #[test]
    fn deliveries_table_renders_status_and_sanitized_target() {
        fn app() -> Element {
            deliveries_table(&sample(), EventHandler::new(|_| {}))
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("failed"), "{html}");
        assert!(html.contains("web · Chrome"), "{html}");
        assert!(html.contains("grade_released"), "{html}");
        assert!(!html.contains("tok-"), "{html}");
    }

    #[test]
    fn short_hash_truncates_long_hashes() {
        assert_eq!(short_hash("abcdef123456"), "abcdef1234…");
        assert_eq!(short_hash("abc"), "abc");
    }
}
```

- [ ] **Step 2: Wire module and route**

In `crates/shell-web/src/routes/mod.rs`, add:

```rust
pub mod admin_notification_deliveries;
pub use admin_notification_deliveries::AdminNotificationDeliveries;
```

In `crates/shell-web/src/route_enum.rs`, import `AdminNotificationDeliveries` and add:

```rust
#[route("/admin/notifications")]
AdminNotificationDeliveries {},
```

In `crates/features-courses/src/app_shell.rs`, add one grouped admin link inside the existing admin menu extension:

```rust
("Delivery log", "/admin/notifications", UiIcon::File),
```

- [ ] **Step 3: Add route smoke test**

In `crates/shell-web/tests/shell_routes_smoke.rs`, add:

```rust
#[test]
fn admin_notification_deliveries_route_renders_for_non_admin_as_forbidden() {
    let mut dom = dom_for_path("/admin/notifications", true);
    let _ = dom.rebuild_in_place();
    let html = render(&dom);
    assert!(
        html.contains("Forbidden") || html.contains("permission"),
        "non-admin should see Forbidden, got: {html}"
    );
}
```

- [ ] **Step 4: Add CSS**

Append this to both CSS files:

```css
.admin-deliveries-page {
  display: grid;
  gap: var(--space-4);
}

.delivery-detail-list {
  display: grid;
  grid-template-columns: max-content minmax(0, 1fr);
  gap: var(--space-2) var(--space-4);
  margin: 0;
}

.delivery-detail-list dt {
  color: var(--color-muted);
  font-size: var(--font-size-sm);
}

.delivery-detail-list dd {
  margin: 0;
  min-width: 0;
  overflow-wrap: anywhere;
}
```

- [ ] **Step 5: Run shell-web route tests**

Run:

```powershell
cargo test -p shell-web --lib admin_notification_deliveries -- --nocapture
cargo test -p shell-web --test shell_routes_smoke admin_notification_deliveries_route_renders_for_non_admin_as_forbidden -- --nocapture
```

Expected: admin delivery route SSR tests pass.

- [ ] **Step 6: Commit admin diagnostics UI**

Run:

```powershell
git add crates/shell-web/src/routes/admin_notification_deliveries.rs crates/shell-web/src/routes/mod.rs crates/shell-web/src/route_enum.rs crates/features-courses/src/app_shell.rs crates/shell-web/tests/shell_routes_smoke.rs crates/design-system/assets/components.css crates/shell-web/public/assets/components.css
git commit -m "feat(notifications): add admin delivery diagnostics"
```

Expected: commit succeeds with admin UI and route changes.

## Task 10: Final Verification

**Files:**
- Review: all files changed in Tasks 1-9.

- [ ] **Step 1: Run formatting**

Run:

```powershell
cargo fmt --all --check
```

Expected: formatting check passes. If it fails, run `cargo fmt --all`, inspect the diff, and commit formatting with the task's files.

- [ ] **Step 2: Run backend unit tests**

Run:

```powershell
cargo test -p backend --lib services::notifications -- --nocapture
```

Expected: backend notification service tests pass.

- [ ] **Step 3: Run backend integration tests**

Run:

```powershell
cargo test -p backend --test notifications -- --nocapture
```

Expected: notification integration tests pass against the configured local Postgres.

- [ ] **Step 4: Run frontend API tests**

Run:

```powershell
cargo test -p features-courses --lib -- --nocapture
```

Expected: API DTO contract tests pass.

- [ ] **Step 5: Run shell-web SSR tests**

Run:

```powershell
cargo test -p shell-web --lib -- --nocapture
cargo test -p shell-web --test shell_routes_smoke -- --nocapture
```

Expected: shell-web route and SSR tests pass.

- [ ] **Step 6: Run native compile checks**

Run:

```powershell
cargo check -p backend --all-targets
cargo check -p shell-web --all-targets
```

Expected: backend and host shell-web targets compile.

- [ ] **Step 7: Run wasm compile checks**

Run:

```powershell
cargo check -p platform-bridge --target wasm32-unknown-unknown
cargo check -p shell-web --target wasm32-unknown-unknown
```

Expected: wasm targets compile and exclude native-only code.

- [ ] **Step 8: Inspect changed files**

Run:

```powershell
git status --short
git diff --stat HEAD
git diff --check
```

Expected: no whitespace errors. Changed files match this plan's scope.

- [ ] **Step 9: Commit final verification fixes**

If verification required formatting or compile-fix edits, run:

```powershell
git add <changed-files>
git commit -m "fix(notifications): stabilize phase 4 verification"
```

Expected: either no commit is needed, or one focused verification commit is created.

- [ ] **Step 10: Prepare branch completion**

Run:

```powershell
git log --oneline --decorate -10
git status --short --branch
```

Expected: implementation branch is clean and contains the Phase 4 commits on top of the approved spec and plan commits.
