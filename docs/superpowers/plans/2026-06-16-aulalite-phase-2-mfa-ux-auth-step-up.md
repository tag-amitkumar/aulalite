# AulaLite Phase 2 MFA UX And Auth Step-Up Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Complete MFA sign-in, recovery-code handling, remembered-device support, and admin recovery-code reset while preserving the existing primary auth flow.

**Architecture:** Add backend-owned trusted-device persistence and extend the existing MFA challenge endpoint so a user can step up with TOTP, recovery code, or a valid remembered-device token. Keep login as a two-step state machine and add focused self-service/admin UI around recovery codes and trusted devices.

**Tech Stack:** Rust 1.94 workspace, Axum, SQLx/PostgreSQL with RLS, Dioxus 0.7, existing `features-auth`, `features-courses`, `platform-bridge`, and design-system primitives.

---

## File Structure

- Create: `migrations/20260616000064_mfa_trusted_devices.sql`
  - Stores hashed remembered-device tokens with owner-scoped RLS.
- Modify: `crates/backend/src/db/mfa.rs`
  - Adds trusted-device DTOs/helpers, token generation/hash helpers, trusted-device validation, revoke/list, and recovery-code replacement.
- Modify: `crates/backend/src/handlers/mfa.rs`
  - Extends challenge request/response, adds trusted-device list/revoke routes, exposes recovery-code minting helper, and adds a test router.
- Modify: `crates/backend/src/handlers/admin.rs`
  - Adds tenant-admin recovery-code reset route, shared inner handler, and a test router for the new admin MFA endpoint.
- Create: `crates/backend/tests/mfa.rs`
  - DB-backed route tests for TOTP/recovery/trusted-device challenge, trusted-device list/revoke, and admin reset permissions.
- Modify: `crates/features-courses/src/api.rs`
  - Adds DTOs/API helpers for MFA challenge, trusted devices, and admin recovery reset.
- Modify: `crates/platform-bridge/src/web.rs`
  - Adds web localStorage helpers for trusted-device tokens and clears them on sign-out.
- Modify: `crates/platform-bridge/src/native.rs`
  - Adds native JSON-file helpers for trusted-device tokens and clears them on sign-out.
- Modify: `crates/features-auth/src/login.rs`
  - Converts login to a two-step MFA-aware state machine and adds pure helper tests.
- Modify: `crates/features-auth/tests/login.rs`
  - Covers MFA-required classifier and MFA panel rendering.
- Modify: `crates/features-courses/src/security_settings.rs`
  - Adds trusted-device list/revoke UI plus recovery-code acknowledgement, copy, and download controls.
- Modify: `crates/shell-web/src/routes/admin_tenant.rs`
  - Adds admin recovery-code reset action and one-time replacement-code panel.

## Task 1: Add Trusted-Device Schema

**Files:**
- Create: `migrations/20260616000064_mfa_trusted_devices.sql`

- [ ] **Step 1: Add the migration**

Create `migrations/20260616000064_mfa_trusted_devices.sql` with:

```sql
-- Remembered MFA devices. MFA is a global user property, not tenant-scoped.
-- The token itself is shown to the client once; the server stores only a hash.

CREATE TABLE user_mfa_trusted_devices (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v7(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash TEXT NOT NULL UNIQUE,
    label TEXT NOT NULL,
    user_agent TEXT,
    last_used_at TIMESTAMPTZ,
    expires_at TIMESTAMPTZ NOT NULL,
    revoked_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX user_mfa_trusted_devices_user_active_idx
    ON user_mfa_trusted_devices (user_id, expires_at DESC)
    WHERE revoked_at IS NULL;

ALTER TABLE user_mfa_trusted_devices ENABLE ROW LEVEL SECURITY;
ALTER TABLE user_mfa_trusted_devices FORCE ROW LEVEL SECURITY;

CREATE POLICY trusted_device_owner_access ON user_mfa_trusted_devices
    USING (user_id::text = current_setting('app.user_id', true))
    WITH CHECK (user_id::text = current_setting('app.user_id', true));

GRANT SELECT, INSERT, UPDATE, DELETE ON user_mfa_trusted_devices TO aulalite_app;
```

- [ ] **Step 2: Verify migration ordering**

Run:

```powershell
Get-ChildItem migrations | Sort-Object Name | Select-Object -Last 5 -ExpandProperty Name
```

Expected: output includes `20260616000064_mfa_trusted_devices.sql` after the existing `20260614000063_sso_login_states_pkce_verifier.sql`.

- [ ] **Step 3: Commit schema**

Run:

```powershell
git add migrations/20260616000064_mfa_trusted_devices.sql
git commit -m "feat(auth): add mfa trusted devices schema"
```

Expected: commit succeeds with only the new migration.

## Task 2: Add MFA DB Helpers

**Files:**
- Modify: `crates/backend/src/db/mfa.rs`

- [ ] **Step 1: Add failing helper tests**

Append this new test module at the end of `crates/backend/src/db/mfa.rs`.

```rust
#[cfg(test)]
mod tests {
    use super::{
        normalize_trusted_device_label, trusted_device_hash, trusted_device_token,
        TrustedDeviceCheck,
    };

    #[test]
    fn trusted_device_token_is_random_and_url_safe() {
        let a = trusted_device_token();
        let b = trusted_device_token();

        assert_ne!(a, b);
        assert!(a.len() >= 43, "{a}");
        assert!(a.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    }

    #[test]
    fn trusted_device_hash_is_stable_and_not_plaintext() {
        let token = "sample-device-token";
        let a = trusted_device_hash(token);
        let b = trusted_device_hash(token);

        assert_eq!(a, b);
        assert_ne!(a, token);
        assert_eq!(a.len(), 64);
    }

    #[test]
    fn trusted_device_label_is_bounded_and_defaulted() {
        assert_eq!(normalize_trusted_device_label(None), "This device");
        assert_eq!(
            normalize_trusted_device_label(Some("  Chiranjib laptop  ".to_string())),
            "Chiranjib laptop"
        );
        let long = "a".repeat(200);
        assert_eq!(normalize_trusted_device_label(Some(long)).len(), 80);
    }

    #[test]
    fn trusted_device_check_variants_are_stable() {
        assert_eq!(TrustedDeviceCheck::Invalid.error_code(), "trusted_device_invalid");
        assert_eq!(TrustedDeviceCheck::Expired.error_code(), "trusted_device_expired");
        assert_eq!(TrustedDeviceCheck::Revoked.error_code(), "trusted_device_revoked");
    }
}
```

- [ ] **Step 2: Run the failing helper tests**

Run:

```powershell
cargo test -p backend --lib db::mfa::tests::trusted_device
```

Expected: compile fails because `trusted_device_token`, `trusted_device_hash`, `normalize_trusted_device_label`, and `TrustedDeviceCheck` do not exist.

- [ ] **Step 3: Add imports and constants**

Add these imports near the top of `crates/backend/src/db/mfa.rs`:

```rust
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::RngCore;
```

Add these constants below the imports:

```rust
const TRUSTED_DEVICE_TOKEN_BYTES: usize = 32;
const TRUSTED_DEVICE_LIFETIME_DAYS: i64 = 30;
const TRUSTED_DEVICE_LABEL_MAX: usize = 80;
```

- [ ] **Step 4: Add trusted-device types and pure helpers**

Add this block after `pub struct UserMfa`:

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct TrustedDevice {
    pub id: Uuid,
    pub user_id: Uuid,
    pub label: String,
    pub user_agent: Option<String>,
    pub last_used_at: Option<chrono::DateTime<chrono::Utc>>,
    pub expires_at: chrono::DateTime<chrono::Utc>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrustedDeviceCheck {
    Valid(TrustedDevice),
    Invalid,
    Expired,
    Revoked,
}

impl TrustedDeviceCheck {
    pub fn error_code(&self) -> &'static str {
        match self {
            TrustedDeviceCheck::Valid(_) => "trusted_device_valid",
            TrustedDeviceCheck::Invalid => "trusted_device_invalid",
            TrustedDeviceCheck::Expired => "trusted_device_expired",
            TrustedDeviceCheck::Revoked => "trusted_device_revoked",
        }
    }
}

pub fn trusted_device_token() -> String {
    let mut bytes = [0u8; TRUSTED_DEVICE_TOKEN_BYTES];
    rand::thread_rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

pub fn trusted_device_hash(token: &str) -> String {
    crate::db::api_keys::hash_secret(token)
}

pub fn normalize_trusted_device_label(label: Option<String>) -> String {
    let trimmed = label.unwrap_or_default().trim().to_string();
    let base = if trimmed.is_empty() {
        "This device".to_string()
    } else {
        trimmed
    };
    base.chars().take(TRUSTED_DEVICE_LABEL_MAX).collect()
}
```

- [ ] **Step 5: Add DB helpers**

Add these functions after `consume_recovery_code`:

```rust
pub async fn create_trusted_device(
    pool: &PgPool,
    user_id: Uuid,
    label: Option<String>,
    user_agent: Option<String>,
) -> sqlx::Result<(TrustedDevice, String)> {
    let plaintext = trusted_device_token();
    let hash = trusted_device_hash(&plaintext);
    let label = normalize_trusted_device_label(label);
    let expires_at = chrono::Utc::now() + chrono::Duration::days(TRUSTED_DEVICE_LIFETIME_DAYS);

    let mut tx = pool.begin().await?;
    set_user(&mut tx, user_id).await?;
    let row: (
        Uuid,
        Uuid,
        String,
        Option<String>,
        Option<chrono::DateTime<chrono::Utc>>,
        chrono::DateTime<chrono::Utc>,
        chrono::DateTime<chrono::Utc>,
    ) = sqlx::query_as(
        "INSERT INTO user_mfa_trusted_devices
             (user_id, token_hash, label, user_agent, expires_at)
         VALUES ($1, $2, $3, $4, $5)
         RETURNING id, user_id, label, user_agent, last_used_at, expires_at, created_at",
    )
    .bind(user_id)
    .bind(hash)
    .bind(label)
    .bind(user_agent)
    .bind(expires_at)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;

    Ok((
        TrustedDevice {
            id: row.0,
            user_id: row.1,
            label: row.2,
            user_agent: row.3,
            last_used_at: row.4,
            expires_at: row.5,
            created_at: row.6,
        },
        plaintext,
    ))
}

pub async fn validate_trusted_device(
    pool: &PgPool,
    user_id: Uuid,
    plaintext_token: &str,
) -> sqlx::Result<TrustedDeviceCheck> {
    let hash = trusted_device_hash(plaintext_token);
    let mut tx = pool.begin().await?;
    set_user(&mut tx, user_id).await?;
    let row: Option<(
        Uuid,
        Uuid,
        String,
        Option<String>,
        Option<chrono::DateTime<chrono::Utc>>,
        chrono::DateTime<chrono::Utc>,
        Option<chrono::DateTime<chrono::Utc>>,
        chrono::DateTime<chrono::Utc>,
    )> = sqlx::query_as(
        "SELECT id, user_id, label, user_agent, last_used_at, expires_at, revoked_at, created_at
           FROM user_mfa_trusted_devices
          WHERE user_id = $1 AND token_hash = $2",
    )
    .bind(user_id)
    .bind(hash)
    .fetch_optional(&mut *tx)
    .await?;

    let Some(row) = row else {
        tx.commit().await?;
        return Ok(TrustedDeviceCheck::Invalid);
    };
    if row.6.is_some() {
        tx.commit().await?;
        return Ok(TrustedDeviceCheck::Revoked);
    }
    if row.5 <= chrono::Utc::now() {
        tx.commit().await?;
        return Ok(TrustedDeviceCheck::Expired);
    }

    sqlx::query(
        "UPDATE user_mfa_trusted_devices
            SET last_used_at = now(), updated_at = now()
          WHERE id = $1",
    )
    .bind(row.0)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    Ok(TrustedDeviceCheck::Valid(TrustedDevice {
        id: row.0,
        user_id: row.1,
        label: row.2,
        user_agent: row.3,
        last_used_at: Some(chrono::Utc::now()),
        expires_at: row.5,
        created_at: row.7,
    }))
}

pub async fn list_trusted_devices(
    pool: &PgPool,
    user_id: Uuid,
) -> sqlx::Result<Vec<TrustedDevice>> {
    let mut tx = pool.begin().await?;
    set_user(&mut tx, user_id).await?;
    let rows: Vec<(
        Uuid,
        Uuid,
        String,
        Option<String>,
        Option<chrono::DateTime<chrono::Utc>>,
        chrono::DateTime<chrono::Utc>,
        chrono::DateTime<chrono::Utc>,
    )> = sqlx::query_as(
        "SELECT id, user_id, label, user_agent, last_used_at, expires_at, created_at
           FROM user_mfa_trusted_devices
          WHERE user_id = $1
            AND revoked_at IS NULL
          ORDER BY created_at DESC",
    )
    .bind(user_id)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;

    Ok(rows
        .into_iter()
        .map(|row| TrustedDevice {
            id: row.0,
            user_id: row.1,
            label: row.2,
            user_agent: row.3,
            last_used_at: row.4,
            expires_at: row.5,
            created_at: row.6,
        })
        .collect())
}

pub async fn revoke_trusted_device(
    pool: &PgPool,
    user_id: Uuid,
    device_id: Uuid,
) -> sqlx::Result<bool> {
    let mut tx = pool.begin().await?;
    set_user(&mut tx, user_id).await?;
    let res = sqlx::query(
        "UPDATE user_mfa_trusted_devices
            SET revoked_at = now(), updated_at = now()
          WHERE user_id = $1
            AND id = $2
            AND revoked_at IS NULL",
    )
    .bind(user_id)
    .bind(device_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(res.rows_affected() > 0)
}

pub async fn replace_recovery_codes(
    pool: &PgPool,
    user_id: Uuid,
    recovery_code_hashes: &[String],
) -> sqlx::Result<bool> {
    let mut tx = pool.begin().await?;
    set_user(&mut tx, user_id).await?;
    let res = sqlx::query(
        "UPDATE user_mfa
            SET recovery_codes = $2,
                updated_at = now()
          WHERE user_id = $1 AND enabled = true",
    )
    .bind(user_id)
    .bind(recovery_code_hashes)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(res.rows_affected() > 0)
}
```

- [ ] **Step 6: Run helper tests**

Run:

```powershell
cargo test -p backend --lib db::mfa::tests::trusted_device
```

Expected: trusted-device helper tests pass.

- [ ] **Step 7: Commit DB helpers**

Run:

```powershell
git add crates/backend/src/db/mfa.rs
git commit -m "feat(auth): add mfa trusted device helpers"
```

Expected: commit succeeds with only `crates/backend/src/db/mfa.rs` changed.

## Task 3: Extend MFA Routes

**Files:**
- Modify: `crates/backend/src/handlers/mfa.rs`

- [ ] **Step 1: Add route test support and shared recovery-code minting**

In `crates/backend/src/handlers/mfa.rs`, add this test router below `routes()`:

```rust
#[doc(hidden)]
pub fn router_for_tests(pool: sqlx::PgPool) -> Router {
    Router::new()
        .route("/v1/me/mfa", routing::get(status_t))
        .route("/v1/me/mfa/enroll", routing::post(enroll_t))
        .route("/v1/me/mfa/verify", routing::post(verify_t))
        .route("/v1/me/mfa/disable", routing::post(disable_t))
        .route("/v1/auth/mfa/challenge", routing::post(challenge_t))
        .route("/v1/me/mfa/trusted-devices", routing::get(list_trusted_devices_t))
        .route(
            "/v1/me/mfa/trusted-devices/:id",
            routing::delete(revoke_trusted_device_t),
        )
        .with_state(MfaTestState { pool })
}

#[derive(Clone)]
struct MfaTestState {
    pool: sqlx::PgPool,
}
```

Replace the plaintext recovery-code generation block in `verify` with a helper. Add this helper above `verify`:

```rust
pub(crate) fn mint_recovery_codes() -> (Vec<String>, Vec<String>) {
    let plaintext: Vec<String> = (0..RECOVERY_CODE_COUNT).map(|_| recovery_code()).collect();
    let hashes = plaintext
        .iter()
        .map(|c| db::api_keys::hash_secret(c))
        .collect();
    (plaintext, hashes)
}
```

Then in `verify`, replace:

```rust
    let plaintext: Vec<String> = (0..RECOVERY_CODE_COUNT).map(|_| recovery_code()).collect();
    let hashes: Vec<String> = plaintext
        .iter()
        .map(|c| db::api_keys::hash_secret(c))
        .collect();
```

with:

```rust
    let (plaintext, hashes) = mint_recovery_codes();
```

- [ ] **Step 2: Update challenge DTOs**

Replace the existing `ChallengeRequest` and `ChallengeResponse` with:

```rust
#[derive(Deserialize)]
pub struct ChallengeRequest {
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub trusted_device_token: Option<String>,
    #[serde(default)]
    pub remember_device: bool,
    #[serde(default)]
    pub device_label: Option<String>,
}

#[derive(Serialize)]
pub struct ChallengeResponse {
    pub stepped_up: bool,
    pub stepup_token: String,
    pub used_recovery_code: bool,
    pub trusted_device_token: Option<String>,
    pub trusted_device_expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

fn exactly_one_challenge_method(body: &ChallengeRequest) -> bool {
    let has_code = body.code.as_deref().is_some_and(|v| !v.trim().is_empty());
    let has_device = body
        .trusted_device_token
        .as_deref()
        .is_some_and(|v| !v.trim().is_empty());
    has_code ^ has_device
}
```

- [ ] **Step 3: Add trusted-device DTOs and handlers**

Add these DTOs and handlers below `ChallengeResponse`:

```rust
#[derive(Serialize)]
pub struct TrustedDeviceDto {
    pub id: uuid::Uuid,
    pub label: String,
    pub user_agent: Option<String>,
    pub last_used_at: Option<chrono::DateTime<chrono::Utc>>,
    pub expires_at: chrono::DateTime<chrono::Utc>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Serialize)]
pub struct TrustedDeviceListResponse {
    pub devices: Vec<TrustedDeviceDto>,
}

impl From<db::mfa::TrustedDevice> for TrustedDeviceDto {
    fn from(value: db::mfa::TrustedDevice) -> Self {
        Self {
            id: value.id,
            label: value.label,
            user_agent: value.user_agent,
            last_used_at: value.last_used_at,
            expires_at: value.expires_at,
            created_at: value.created_at,
        }
    }
}

async fn list_trusted_devices_inner(
    pool: &sqlx::PgPool,
    ctx: &RequestContext,
) -> Result<Json<TrustedDeviceListResponse>, ApiError> {
    let devices = db::mfa::list_trusted_devices(pool, ctx.user_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .into_iter()
        .map(TrustedDeviceDto::from)
        .collect();
    Ok(Json(TrustedDeviceListResponse { devices }))
}

async fn revoke_trusted_device_inner(
    pool: &sqlx::PgPool,
    ctx: &RequestContext,
    device_id: uuid::Uuid,
) -> Result<Json<serde_json::Value>, ApiError> {
    let revoked = db::mfa::revoke_trusted_device(pool, ctx.user_id, device_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !revoked {
        return Err(ApiError::NotFound);
    }
    Ok(Json(serde_json::json!({ "revoked": true })))
}
```

Add production and test handler wrappers:

```rust
async fn list_trusted_devices(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<TrustedDeviceListResponse>, ApiError> {
    list_trusted_devices_inner(&s.pool, &ctx).await
}

async fn revoke_trusted_device(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    axum::extract::Path(id): axum::extract::Path<uuid::Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    revoke_trusted_device_inner(&s.pool, &ctx, id).await
}

async fn list_trusted_devices_t(
    State(s): State<MfaTestState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<TrustedDeviceListResponse>, ApiError> {
    list_trusted_devices_inner(&s.pool, &ctx).await
}

async fn revoke_trusted_device_t(
    State(s): State<MfaTestState>,
    Extension(ctx): Extension<RequestContext>,
    axum::extract::Path(id): axum::extract::Path<uuid::Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    revoke_trusted_device_inner(&s.pool, &ctx, id).await
}
```

Add these routes to production `routes()`:

```rust
.route("/v1/me/mfa/trusted-devices", routing::get(list_trusted_devices))
.route(
    "/v1/me/mfa/trusted-devices/:id",
    routing::delete(revoke_trusted_device),
)
```

- [ ] **Step 4: Refactor challenge into an inner handler**

Replace the existing `challenge` function body with wrappers plus an inner handler:

```rust
async fn challenge_inner(
    pool: &sqlx::PgPool,
    ctx: &RequestContext,
    body: ChallengeRequest,
) -> Result<Json<ChallengeResponse>, ApiError> {
    if !exactly_one_challenge_method(&body) {
        return Err(ApiError::BadRequest("one_challenge_method_required".into()));
    }

    let mut used_recovery_code = false;
    let mut trusted_device_token = None;
    let mut trusted_device_expires_at = None;

    if let Some(device_token) = body
        .trusted_device_token
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        match db::mfa::validate_trusted_device(pool, ctx.user_id, device_token)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?
        {
            db::mfa::TrustedDeviceCheck::Valid(_) => {}
            other => return Err(ApiError::BadRequest(other.error_code().into())),
        }
    } else {
        let presented = body
            .code
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| ApiError::BadRequest("code_required".into()))?;

        let row = db::mfa::get(pool, ctx.user_id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?
            .ok_or_else(|| ApiError::BadRequest("mfa_not_enabled".into()))?;
        if !row.enabled {
            return Err(ApiError::BadRequest("mfa_not_enabled".into()));
        }

        used_recovery_code = if totp::verify(&row.secret, presented) {
            false
        } else {
            let hash = db::api_keys::hash_secret(presented);
            let consumed = db::mfa::consume_recovery_code(pool, ctx.user_id, &hash)
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?;
            if !consumed {
                return Err(ApiError::BadRequest("invalid_code".into()));
            }
            true
        };

        if body.remember_device {
            let (device, token) = db::mfa::create_trusted_device(
                pool,
                ctx.user_id,
                body.device_label.clone(),
                None,
            )
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
            trusted_device_token = Some(token);
            trusted_device_expires_at = Some(device.expires_at);
        }
    }

    let secret = crate::services::oidc::session_secret_from_env()
        .ok_or_else(|| ApiError::Internal("sso_session_secret_unset".into()))?;
    let stepup_token = crate::services::oidc::mint_session_token(
        &secret,
        &ctx.firebase_uid,
        &ctx.email,
        ctx.display_name.as_deref(),
    )
    .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(ChallengeResponse {
        stepped_up: true,
        stepup_token,
        used_recovery_code,
        trusted_device_token,
        trusted_device_expires_at,
    }))
}

async fn challenge(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Json(body): Json<ChallengeRequest>,
) -> Result<Json<ChallengeResponse>, ApiError> {
    challenge_inner(&s.pool, &ctx, body).await
}

async fn challenge_t(
    State(s): State<MfaTestState>,
    Extension(ctx): Extension<RequestContext>,
    Json(body): Json<ChallengeRequest>,
) -> Result<Json<ChallengeResponse>, ApiError> {
    challenge_inner(&s.pool, &ctx, body).await
}
```

- [ ] **Step 5: Add test wrappers for existing handlers**

Add test wrappers for status/enroll/verify/disable:

```rust
async fn status_t(
    State(s): State<MfaTestState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<MfaStatus>, ApiError> {
    status_inner(&s.pool, &ctx).await
}

async fn enroll_t(
    State(s): State<MfaTestState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<EnrollResponse>, ApiError> {
    enroll_inner(&s.pool, &ctx).await
}

async fn verify_t(
    State(s): State<MfaTestState>,
    Extension(ctx): Extension<RequestContext>,
    Json(body): Json<VerifyRequest>,
) -> Result<Json<VerifyResponse>, ApiError> {
    verify_inner(&s.pool, &ctx, body).await
}

async fn disable_t(
    State(s): State<MfaTestState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<DisableResponse>, ApiError> {
    disable_inner(&s.pool, &ctx).await
}
```

Replace the production `status`, `enroll`, `verify`, and `disable` functions with these wrappers and inner helpers:

```rust
async fn status_inner(
    pool: &sqlx::PgPool,
    ctx: &RequestContext,
) -> Result<Json<MfaStatus>, ApiError> {
    let row = db::mfa::get(pool, ctx.user_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let (enabled, pending) = match row {
        Some(m) => (m.enabled, !m.enabled),
        None => (false, false),
    };
    Ok(Json(MfaStatus { enabled, pending }))
}

async fn status(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<MfaStatus>, ApiError> {
    status_inner(&s.pool, &ctx).await
}

async fn enroll_inner(
    pool: &sqlx::PgPool,
    ctx: &RequestContext,
) -> Result<Json<EnrollResponse>, ApiError> {
    if let Some(existing) = db::mfa::get(pool, ctx.user_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
    {
        if existing.enabled {
            return Err(ApiError::Conflict("mfa_already_enabled".into()));
        }
    }

    let secret = totp::generate_secret();
    db::mfa::start_enrollment(pool, ctx.user_id, &secret)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let secret_base32 = totp::base32_encode(&secret);
    let otpauth_uri = totp::otpauth_uri(&secret_base32, ISSUER, &ctx.email);

    Ok(Json(EnrollResponse {
        secret_base32,
        otpauth_uri,
    }))
}

async fn enroll(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<EnrollResponse>, ApiError> {
    enroll_inner(&s.pool, &ctx).await
}

async fn verify_inner(
    pool: &sqlx::PgPool,
    ctx: &RequestContext,
    body: VerifyRequest,
) -> Result<Json<VerifyResponse>, ApiError> {
    let row = db::mfa::get(pool, ctx.user_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::BadRequest("not_enrolled".into()))?;

    if row.enabled {
        return Err(ApiError::Conflict("mfa_already_enabled".into()));
    }

    if !totp::verify(&row.secret, body.code.trim()) {
        return Err(ApiError::BadRequest("invalid_code".into()));
    }

    let (plaintext, hashes) = mint_recovery_codes();
    let activated = db::mfa::confirm_enrollment(pool, ctx.user_id, &hashes)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !activated {
        return Err(ApiError::Conflict("mfa_state_changed".into()));
    }

    Ok(Json(VerifyResponse {
        enabled: true,
        recovery_codes: plaintext,
    }))
}

async fn verify(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Json(body): Json<VerifyRequest>,
) -> Result<Json<VerifyResponse>, ApiError> {
    verify_inner(&s.pool, &ctx, body).await
}

async fn disable_inner(
    pool: &sqlx::PgPool,
    ctx: &RequestContext,
) -> Result<Json<DisableResponse>, ApiError> {
    let disabled = db::mfa::disable(pool, ctx.user_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(DisableResponse { disabled }))
}

async fn disable(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<DisableResponse>, ApiError> {
    disable_inner(&s.pool, &ctx).await
}
```

- [ ] **Step 6: Run route compile check**

Run:

```powershell
cargo test -p backend --lib handlers::mfa --no-run
```

Expected: backend library test binary builds.

- [ ] **Step 7: Commit MFA route extension**

Run:

```powershell
git add crates/backend/src/handlers/mfa.rs
git commit -m "feat(auth): extend mfa challenge routes"
```

Expected: commit succeeds with only `crates/backend/src/handlers/mfa.rs` changed.

## Task 4: Add Backend MFA Integration Tests

**Files:**
- Create: `crates/backend/tests/mfa.rs`

- [ ] **Step 1: Write DB-backed integration tests**

Create `crates/backend/tests/mfa.rs`:

```rust
mod fixtures;

use axum::http::StatusCode;
use backend::services::totp;
use fixtures::{attach_membership, build_test_app, create_tenant, create_user, fire, pool, StubAuth};
use serde_json::json;

fn app(pool: sqlx::PgPool, user_id: uuid::Uuid, firebase_uid: String, email: String, tenant_id: uuid::Uuid) -> axum::Router {
    build_test_app(
        backend::handlers::mfa::router_for_tests(pool.clone()),
        StubAuth {
            pool,
            user_id,
            firebase_uid,
            email,
            tenant_id: Some(tenant_id),
            tenant_role: Some(core_types::TenantRole::Student),
        },
    )
}

async fn enrolled_user() -> (sqlx::PgPool, uuid::Uuid, uuid::Uuid, String, String, Vec<u8>) {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (user, fb, email) = create_user(&pool).await;
    attach_membership(&pool, tenant, user, "student").await;
    let secret = totp::generate_secret();
    let hashes = vec![backend::db::api_keys::hash_secret("abcde-23456")];
    backend::db::mfa::start_enrollment(&pool, user, &secret).await.unwrap();
    assert!(backend::db::mfa::confirm_enrollment(&pool, user, &hashes).await.unwrap());
    (pool, tenant, user, fb, email, secret)
}

#[tokio::test]
async fn totp_challenge_can_create_trusted_device() {
    let (pool, tenant, user, fb, email, secret) = enrolled_user().await;
    let app = app(pool.clone(), user, fb, email, tenant);
    let code = totp::code_at(&secret, chrono::Utc::now().timestamp());

    let (status, body) = fire(
        &app,
        "POST",
        "/v1/auth/mfa/challenge",
        Some(json!({
            "code": code,
            "remember_device": true,
            "device_label": "Work laptop"
        })),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["stepped_up"], true);
    assert_eq!(body["used_recovery_code"], false);
    assert!(body["stepup_token"].as_str().unwrap().len() > 20);
    let token = body["trusted_device_token"].as_str().unwrap();
    assert!(token.len() >= 43);

    let rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM user_mfa_trusted_devices
          WHERE user_id = $1 AND token_hash = $2",
    )
    .bind(user)
    .bind(backend::db::mfa::trusted_device_hash(token))
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(rows, 1);
}

#[tokio::test]
async fn recovery_code_challenge_consumes_code_once() {
    let (pool, tenant, user, fb, email, _secret) = enrolled_user().await;
    let app = app(pool, user, fb, email, tenant);

    let (status, body) = fire(
        &app,
        "POST",
        "/v1/auth/mfa/challenge",
        Some(json!({ "code": "abcde-23456" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["used_recovery_code"], true);

    let (status, body) = fire(
        &app,
        "POST",
        "/v1/auth/mfa/challenge",
        Some(json!({ "code": "abcde-23456" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"], "invalid_code");
}

#[tokio::test]
async fn trusted_device_challenge_succeeds_then_revoke_blocks_it() {
    let (pool, tenant, user, fb, email, secret) = enrolled_user().await;
    let app = app(pool.clone(), user, fb, email, tenant);
    let code = totp::code_at(&secret, chrono::Utc::now().timestamp());
    let (_, body) = fire(
        &app,
        "POST",
        "/v1/auth/mfa/challenge",
        Some(json!({ "code": code, "remember_device": true })),
    )
    .await;
    let token = body["trusted_device_token"].as_str().unwrap().to_string();

    let (status, body) = fire(
        &app,
        "POST",
        "/v1/auth/mfa/challenge",
        Some(json!({ "trusted_device_token": token })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, body) = fire(&app, "GET", "/v1/me/mfa/trusted-devices", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let id = body["devices"][0]["id"].as_str().unwrap().to_string();

    let (status, body) = fire(
        &app,
        "DELETE",
        &format!("/v1/me/mfa/trusted-devices/{id}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, body) = fire(
        &app,
        "POST",
        "/v1/auth/mfa/challenge",
        Some(json!({ "trusted_device_token": token })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"], "trusted_device_revoked");
}
```

- [ ] **Step 2: Add `code_at` helper**

Add this pure helper to `crates/backend/src/services/totp.rs` below `current_code`:

```rust
pub fn code_at(secret: &[u8], unix_seconds: i64) -> String {
    let unix_seconds = u64::try_from(unix_seconds).unwrap_or(0);
    totp_at(secret, unix_seconds, DEFAULT_PERIOD, DEFAULT_DIGITS)
}
```

- [ ] **Step 3: Run the focused integration tests**

Run with a migrated PostgreSQL database:

```powershell
cargo test -p backend --test mfa -- --nocapture
```

Expected: all three MFA integration tests pass.

- [ ] **Step 4: Commit backend MFA tests**

Run:

```powershell
git add crates/backend/tests/mfa.rs crates/backend/src/services/totp.rs
git commit -m "test(auth): cover mfa trusted device challenge"
```

Expected: commit succeeds with the new MFA integration test and the `code_at` helper.

## Task 5: Add Admin Recovery-Code Reset

**Files:**
- Modify: `crates/backend/src/handlers/admin.rs`
- Modify: `crates/backend/tests/mfa.rs`

- [ ] **Step 1: Add admin test router**

In `crates/backend/src/handlers/admin.rs`, add this router near `branding_router_for_tests`:

```rust
#[doc(hidden)]
pub fn mfa_admin_router_for_tests(pool: PgPool) -> Router {
    Router::new()
        .route(
            "/v1/admin/tenant/memberships/:user_id/mfa/recovery-codes",
            routing::post(reset_member_mfa_recovery_codes_t),
        )
        .with_state(AdminMfaTestState { pool })
}

#[derive(Clone)]
struct AdminMfaTestState {
    pool: PgPool,
}
```

- [ ] **Step 2: Add DTO and inner handler**

Add this block after `PatchMembershipBody`:

```rust
#[derive(Serialize)]
pub struct ResetMfaRecoveryCodesResponse {
    pub recovery_codes: Vec<String>,
}

async fn reset_member_mfa_recovery_codes_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    target_user_id: Uuid,
) -> Result<Json<ResetMfaRecoveryCodesResponse>, ApiError> {
    let is_admin =
        ctx.is_platform_admin || matches!(ctx.tenant_role, Some(core_types::TenantRole::OrgAdmin));
    if !is_admin {
        return Err(ApiError::Forbidden);
    }
    let tenant_id = ctx
        .tenant_id
        .ok_or_else(|| ApiError::BadRequest("no tenant".into()))?;

    let mut tx = db::begin_with_context(pool, ctx.user_id, Some(tenant_id))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let target_in_tenant: Option<i64> = sqlx::query_scalar(
        "SELECT 1
           FROM tenant_memberships
          WHERE tenant_id = $1 AND user_id = $2",
    )
    .bind(tenant_id)
    .bind(target_user_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    if target_in_tenant.is_none() {
        return Err(ApiError::NotFound);
    }
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let (plaintext, hashes) = crate::handlers::mfa::mint_recovery_codes();
    let replaced = db::mfa::replace_recovery_codes(pool, target_user_id, &hashes)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !replaced {
        return Err(ApiError::BadRequest("mfa_not_enabled".into()));
    }

    let mut tx = db::begin_with_context(pool, ctx.user_id, Some(tenant_id))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    db::audit::emit_audit_event(
        &mut tx,
        tenant_id,
        ctx.user_id,
        "user_mfa.recovery_codes.reset",
        "user",
        target_user_id,
        None,
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(ResetMfaRecoveryCodesResponse {
        recovery_codes: plaintext,
    }))
}
```

- [ ] **Step 3: Add production and test handlers**

Add these wrappers:

```rust
async fn reset_member_mfa_recovery_codes(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    axum::extract::Path(user_id): axum::extract::Path<Uuid>,
) -> Result<Json<ResetMfaRecoveryCodesResponse>, ApiError> {
    reset_member_mfa_recovery_codes_inner(&state.pool, &ctx, user_id).await
}

async fn reset_member_mfa_recovery_codes_t(
    State(state): State<AdminMfaTestState>,
    Extension(ctx): Extension<RequestContext>,
    axum::extract::Path(user_id): axum::extract::Path<Uuid>,
) -> Result<Json<ResetMfaRecoveryCodesResponse>, ApiError> {
    reset_member_mfa_recovery_codes_inner(&state.pool, &ctx, user_id).await
}
```

Add this route to production `routes()`:

```rust
.route(
    "/v1/admin/tenant/memberships/:user_id/mfa/recovery-codes",
    routing::post(reset_member_mfa_recovery_codes),
)
```

- [ ] **Step 4: Add admin reset integration tests**

Append these tests to `crates/backend/tests/mfa.rs`:

```rust
#[tokio::test]
async fn org_admin_can_reset_recovery_codes_without_disabling_mfa() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (admin, admin_fb, admin_email) = create_user(&pool).await;
    let (target, _target_fb, _target_email) = create_user(&pool).await;
    attach_membership(&pool, tenant, admin, "org_admin").await;
    attach_membership(&pool, tenant, target, "student").await;

    let secret = totp::generate_secret();
    backend::db::mfa::start_enrollment(&pool, target, &secret).await.unwrap();
    assert!(
        backend::db::mfa::confirm_enrollment(
            &pool,
            target,
            &[backend::db::api_keys::hash_secret("old-code")]
        )
        .await
        .unwrap()
    );

    let app = build_test_app(
        backend::handlers::admin::mfa_admin_router_for_tests(pool.clone()),
        StubAuth {
            pool: pool.clone(),
            user_id: admin,
            firebase_uid: admin_fb,
            email: admin_email,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::OrgAdmin),
        },
    );

    let (status, body) = fire(
        &app,
        "POST",
        &format!("/v1/admin/tenant/memberships/{target}/mfa/recovery-codes"),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["recovery_codes"].as_array().unwrap().len(), 10);
    assert!(backend::db::mfa::is_enabled(&pool, target).await);
}

#[tokio::test]
async fn student_cannot_reset_another_users_recovery_codes() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (student, student_fb, student_email) = create_user(&pool).await;
    let (target, _target_fb, _target_email) = create_user(&pool).await;
    attach_membership(&pool, tenant, student, "student").await;
    attach_membership(&pool, tenant, target, "student").await;

    let app = build_test_app(
        backend::handlers::admin::mfa_admin_router_for_tests(pool.clone()),
        StubAuth {
            pool,
            user_id: student,
            firebase_uid: student_fb,
            email: student_email,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Student),
        },
    );

    let (status, _body) = fire(
        &app,
        "POST",
        &format!("/v1/admin/tenant/memberships/{target}/mfa/recovery-codes"),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}
```

- [ ] **Step 5: Run admin reset tests**

Run with a migrated PostgreSQL database:

```powershell
cargo test -p backend --test mfa org_admin_can_reset_recovery_codes_without_disabling_mfa -- --nocapture
cargo test -p backend --test mfa student_cannot_reset_another_users_recovery_codes -- --nocapture
```

Expected: both tests pass.

- [ ] **Step 6: Commit admin reset**

Run:

```powershell
git add crates/backend/src/handlers/admin.rs crates/backend/tests/mfa.rs
git commit -m "feat(admin): reset mfa recovery codes"
```

Expected: commit succeeds with admin handler and MFA integration test changes.

## Task 6: Add Frontend API And Storage Helpers

**Files:**
- Modify: `crates/features-courses/src/api.rs`
- Modify: `crates/platform-bridge/src/web.rs`
- Modify: `crates/platform-bridge/src/native.rs`

- [ ] **Step 1: Add API DTOs and helpers**

In `crates/features-courses/src/api.rs`, add this block near the existing `/v1/me/mfa` helpers are not present; place it after `get_me`:

```rust
#[derive(Clone, Debug, serde::Serialize, PartialEq, Default)]
pub struct MfaChallengeBody {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trusted_device_token: Option<String>,
    #[serde(default)]
    pub remember_device: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_label: Option<String>,
}

#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct MfaChallengeResponseDto {
    pub stepped_up: bool,
    pub stepup_token: String,
    pub used_recovery_code: bool,
    pub trusted_device_token: Option<String>,
    pub trusted_device_expires_at: Option<String>,
}

#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct TrustedDeviceDto {
    pub id: String,
    pub label: String,
    pub user_agent: Option<String>,
    pub last_used_at: Option<String>,
    pub expires_at: String,
    pub created_at: String,
}

#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct TrustedDeviceListResponseDto {
    pub devices: Vec<TrustedDeviceDto>,
}

#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct ResetMfaRecoveryCodesResponseDto {
    pub recovery_codes: Vec<String>,
}

pub async fn challenge_mfa(
    ctx: &ApiContext,
    body: &MfaChallengeBody,
) -> Result<MfaChallengeResponseDto, ApiError> {
    fetch_json(ctx, "POST", "/v1/auth/mfa/challenge", Some(body)).await
}

pub async fn list_mfa_trusted_devices(
    ctx: &ApiContext,
) -> Result<TrustedDeviceListResponseDto, ApiError> {
    fetch_json(ctx, "GET", "/v1/me/mfa/trusted-devices", None::<&()>).await
}

pub async fn revoke_mfa_trusted_device(ctx: &ApiContext, id: &str) -> Result<(), ApiError> {
    let _: serde_json::Value = fetch_json(
        ctx,
        "DELETE",
        &format!("/v1/me/mfa/trusted-devices/{id}"),
        None::<&()>,
    )
    .await?;
    Ok(())
}

pub async fn admin_reset_mfa_recovery_codes(
    ctx: &ApiContext,
    user_id: &str,
) -> Result<ResetMfaRecoveryCodesResponseDto, ApiError> {
    fetch_json(
        ctx,
        "POST",
        &format!("/v1/admin/tenant/memberships/{user_id}/mfa/recovery-codes"),
        None::<&()>,
    )
    .await
}

pub fn api_error_body_contains(err: &ApiError, needle: &str) -> bool {
    matches!(err, ApiError::Status(_, body) if body.contains(needle))
}
```

- [ ] **Step 2: Add web trusted-device storage**

In `crates/platform-bridge/src/web.rs`, add this constant below `LOCAL_TOKEN_KEY`:

```rust
const TRUSTED_DEVICE_PREFIX: &str = "aulalite.mfa_trusted_device.";
```

Add these helpers below `clear_local_token`:

```rust
fn trusted_device_key(email: &str) -> String {
    format!("{TRUSTED_DEVICE_PREFIX}{}", email.trim().to_ascii_lowercase())
}

pub fn persist_trusted_device_token(email: &str, token: &str) {
    if let Some(storage) = local_storage() {
        let _ = storage.set_item(&trusted_device_key(email), token);
    }
}

pub fn trusted_device_token(email: &str) -> Option<String> {
    local_storage()
        .and_then(|s| s.get_item(&trusted_device_key(email)).ok().flatten())
        .filter(|t| !t.trim().is_empty())
}

pub fn clear_trusted_device_token(email: &str) {
    if let Some(storage) = local_storage() {
        let _ = storage.remove_item(&trusted_device_key(email));
    }
}

pub fn clear_all_trusted_device_tokens() {
    let Some(storage) = local_storage() else {
        return;
    };
    let mut keys = Vec::new();
    for i in 0..storage.length().unwrap_or(0) {
        if let Ok(Some(key)) = storage.key(i) {
            if key.starts_with(TRUSTED_DEVICE_PREFIX) {
                keys.push(key);
            }
        }
    }
    for key in keys {
        let _ = storage.remove_item(&key);
    }
}
```

In `sign_out`, add:

```rust
clear_all_trusted_device_tokens();
```

immediately after `clear_local_token();`.

- [ ] **Step 3: Add native trusted-device storage**

In `crates/platform-bridge/src/native.rs`, add this type near `TokenFile`:

```rust
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TrustedDeviceFile {
    pub tokens_by_email: std::collections::BTreeMap<String, String>,
}
```

Add these methods inside `impl NativeBridge`:

```rust
fn trusted_devices_path() -> Result<PathBuf, BridgeError> {
    let dirs = directories::ProjectDirs::from("com", "aulalite", "aulalite")
        .ok_or_else(|| BridgeError::Io("could not resolve OS config dir".into()))?;
    Ok(dirs.config_dir().join("mfa_trusted_devices.json"))
}

fn load_trusted_devices() -> Result<TrustedDeviceFile, BridgeError> {
    let path = Self::trusted_devices_path()?;
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|e| BridgeError::Io(format!("corrupt mfa_trusted_devices.json: {e}"))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(TrustedDeviceFile::default()),
        Err(e) => Err(BridgeError::Io(e.to_string())),
    }
}

fn save_trusted_devices(file: &TrustedDeviceFile) -> Result<(), BridgeError> {
    let path = Self::trusted_devices_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| BridgeError::Io(e.to_string()))?;
    }
    let json = serde_json::to_vec_pretty(file).map_err(|e| BridgeError::Io(e.to_string()))?;
    std::fs::write(&path, json).map_err(|e| BridgeError::Io(e.to_string()))
}

pub fn persist_trusted_device_token(email: &str, token: &str) -> Result<(), BridgeError> {
    let mut file = Self::load_trusted_devices()?;
    file.tokens_by_email
        .insert(email.trim().to_ascii_lowercase(), token.to_string());
    Self::save_trusted_devices(&file)
}

pub fn trusted_device_token(email: &str) -> Result<Option<String>, BridgeError> {
    let file = Self::load_trusted_devices()?;
    Ok(file
        .tokens_by_email
        .get(&email.trim().to_ascii_lowercase())
        .filter(|v| !v.trim().is_empty())
        .cloned())
}

pub fn clear_trusted_device_token(email: &str) -> Result<(), BridgeError> {
    let mut file = Self::load_trusted_devices()?;
    file.tokens_by_email.remove(&email.trim().to_ascii_lowercase());
    Self::save_trusted_devices(&file)
}

fn clear_all_trusted_device_tokens() -> Result<(), BridgeError> {
    let path = Self::trusted_devices_path()?;
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(BridgeError::Io(e.to_string())),
    }
}
```

In `sign_out`, call:

```rust
Self::clear_all_trusted_device_tokens()?;
```

after `Self::clear_tokens()?;`.

- [ ] **Step 4: Run focused checks**

Run:

```powershell
cargo test -p platform-bridge --lib
cargo test -p features-courses --lib api
```

Expected: both commands pass.

- [ ] **Step 5: Commit API and storage helpers**

Run:

```powershell
git add crates/features-courses/src/api.rs crates/platform-bridge/src/web.rs crates/platform-bridge/src/native.rs
git commit -m "feat(auth): add mfa frontend api helpers"
```

Expected: commit succeeds with API and platform bridge changes.

## Task 7: Make Login MFA-Aware

**Files:**
- Modify: `crates/features-auth/src/login.rs`
- Modify: `crates/features-auth/tests/login.rs`

- [ ] **Step 1: Update and add pure login tests**

Replace the import at the top of `crates/features-auth/tests/login.rs` with:

```rust
use features_auth::login_internals::{
    classify_me_error, decide_outcome, LocalAttempt, LoginOutcome, MeCheckOutcome,
};
```

Replace the first three outcome tests with:

```rust
#[test]
fn local_login_success_yields_local_token() {
    let outcome = decide_outcome(LocalAttempt::Ok("local-token".into()), None);
    assert!(matches!(
        outcome,
        LoginOutcome::Token {
            ref token,
            persist_local_token: true
        } if token == "local-token"
    ));
}

#[test]
fn local_login_failure_falls_back_to_firebase_success() {
    let outcome = decide_outcome(
        LocalAttempt::Err("401".into()),
        Some(Ok("firebase-token".into())),
    );
    assert!(matches!(
        outcome,
        LoginOutcome::Token {
            ref token,
            persist_local_token: false
        } if token == "firebase-token"
    ));
}

#[test]
fn both_failing_yields_error_with_firebase_message() {
    let outcome = decide_outcome(
        LocalAttempt::Err("401".into()),
        Some(Err("invalid password".into())),
    );
    assert!(matches!(outcome, LoginOutcome::Err(ref m) if m.contains("invalid password")));
}
```

Append these new classifier tests:

```rust

#[test]
fn me_error_classifier_detects_mfa_required() {
    let err = features_courses::api::ApiError::Status(
        401,
        r#"{"error":"mfa_required"}"#.to_string(),
    );
    assert_eq!(classify_me_error(&err), MeCheckOutcome::MfaRequired);
}

#[test]
fn me_error_classifier_keeps_other_unauthorized_as_failure() {
    let err = features_courses::api::ApiError::Status(401, "unauthorized".to_string());
    assert_eq!(
        classify_me_error(&err),
        MeCheckOutcome::Failed("status 401: unauthorized".to_string())
    );
}
```

- [ ] **Step 2: Add login internals**

In `crates/features-auth/src/login.rs`, replace `LoginOutcome` and `decide_outcome` inside `login_internals` with:

```rust
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum LoginOutcome {
        Token {
            token: String,
            persist_local_token: bool,
        },
        Err(String),
    }

    pub fn decide_outcome(
        local: LocalAttempt,
        firebase: Option<Result<String, String>>,
    ) -> LoginOutcome {
        match local {
            LocalAttempt::Ok(token) => LoginOutcome::Token {
                token,
                persist_local_token: true,
            },
            LocalAttempt::Err(_) => match firebase {
                Some(Ok(token)) => LoginOutcome::Token {
                    token,
                    persist_local_token: false,
                },
                Some(Err(msg)) => LoginOutcome::Err(format!("Sign in failed: {msg}")),
                None => LoginOutcome::Err("Sign in failed".into()),
            },
        }
    }
```

Still inside `login_internals`, add:

```rust
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum MeCheckOutcome {
        Ok,
        MfaRequired,
        Failed(String),
    }

    pub fn classify_me_error(err: &features_courses::api::ApiError) -> MeCheckOutcome {
        match err {
            features_courses::api::ApiError::Status(401, body)
                if body.contains("mfa_required") =>
            {
                MeCheckOutcome::MfaRequired
            }
            other => MeCheckOutcome::Failed(format!("{other}")),
        }
    }

    pub fn trusted_device_error_should_fall_back(err: &features_courses::api::ApiError) -> bool {
        match err {
            features_courses::api::ApiError::Status(_, body) => {
                body.contains("trusted_device_invalid")
                    || body.contains("trusted_device_expired")
                    || body.contains("trusted_device_revoked")
            }
            _ => false,
        }
    }
```

- [ ] **Step 3: Run pure login tests**

Run:

```powershell
cargo test -p features-auth --test login me_error_classifier
```

Expected: the two classifier tests pass.

- [ ] **Step 4: Add login component state**

In `Login`, add these signals after `submitting`:

```rust
    let mut primary_token = use_signal(|| None::<String>);
    let mut primary_token_is_local = use_signal(|| false);
    let mut mfa_code = use_signal(String::new);
    let mut remember_device = use_signal(|| true);
    let mut mfa_required = use_signal(|| false);
```

Add this MFA panel element before the `rsx!` return:

```rust
    let mfa_panel: Element = if *mfa_required.read() {
        rsx! {
            div { class: "auth-mfa-panel",
                Field {
                    label: "Authentication code".to_string(),
                    for_id: Some("login-mfa-code".to_string()),
                    Input {
                        id: Some("login-mfa-code".to_string()),
                        value: mfa_code.read().clone(),
                        placeholder: "123456 or recovery code".to_string(),
                        input_type: "text".to_string(),
                        disabled: *submitting.read(),
                        error: error.read().is_some(),
                        on_input: move |value| mfa_code.set(value),
                    }
                }
                label { class: "auth-mfa-remember",
                    input {
                        r#type: "checkbox",
                        checked: *remember_device.read(),
                        disabled: *submitting.read(),
                        onchange: move |event| remember_device.set(event.checked()),
                    }
                    span { "Remember this device for 30 days" }
                }
                Button {
                    label: "Verify".to_string(),
                    variant: ButtonVariant::Primary,
                    button_type: "button".to_string(),
                    disabled: *submitting.read(),
                    on_click: {
                        let api_ctx = api_ctx.clone();
                        let form_success = form_success.clone();
                        move |_| {
                            submit_mfa_challenge(
                                email,
                                mfa_code,
                                remember_device,
                                primary_token,
                                primary_token_is_local,
                                error,
                                submitting,
                                form_success.clone(),
                                api_ctx.clone(),
                                toast,
                            );
                        }
                    },
                }
            }
        }
    } else {
        rsx! {}
    };
```

Render `{mfa_panel}` after `FormError`.

- [ ] **Step 5: Add submit helpers**

Below `submit_login`, add:

```rust
fn accepted_api_context(api_ctx: &ApiContext, token: String) -> ApiContext {
    ApiContext {
        base_url: api_ctx.base_url.clone(),
        id_token: token,
    }
}

fn finish_accepted_login(
    email_value: &str,
    token: String,
    persist_local_token: bool,
    api_ctx: &ApiContext,
    on_success: EventHandler<String>,
) {
    #[cfg(target_arch = "wasm32")]
    {
        if persist_local_token {
            platform_bridge::web::persist_local_token(&token);
        }
    }
    let _ = email_value;
    let _ = api_ctx;
    on_success.call(token);
}

fn stored_trusted_device_token(email: &str) -> Option<String> {
    #[cfg(target_arch = "wasm32")]
    {
        platform_bridge::web::trusted_device_token(email)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        platform_bridge::native::NativeBridge::trusted_device_token(email)
            .ok()
            .flatten()
    }
}

fn clear_stored_trusted_device_token(email: &str) {
    #[cfg(target_arch = "wasm32")]
    platform_bridge::web::clear_trusted_device_token(email);
    #[cfg(not(target_arch = "wasm32"))]
    let _ = platform_bridge::native::NativeBridge::clear_trusted_device_token(email);
}

async fn attempt_trusted_device_stepup(
    email_value: &str,
    challenge_ctx: &ApiContext,
) -> Result<Option<String>, features_courses::api::ApiError> {
    let Some(device_token) = stored_trusted_device_token(email_value) else {
        return Ok(None);
    };
    let body = features_courses::api::MfaChallengeBody {
        code: None,
        trusted_device_token: Some(device_token),
        remember_device: false,
        device_label: None,
    };
    match features_courses::api::challenge_mfa(challenge_ctx, &body).await {
        Ok(resp) => Ok(Some(resp.stepup_token)),
        Err(err) if login_internals::trusted_device_error_should_fall_back(&err) => {
            clear_stored_trusted_device_token(email_value);
            Ok(None)
        }
        Err(err) => Err(err),
    }
}
```

Add this challenge submitter:

```rust
#[allow(clippy::too_many_arguments)]
fn submit_mfa_challenge(
    email: Signal<String>,
    mfa_code: Signal<String>,
    remember_device: Signal<bool>,
    primary_token: Signal<Option<String>>,
    primary_token_is_local: Signal<bool>,
    mut error: Signal<Option<String>>,
    mut submitting: Signal<bool>,
    on_success: EventHandler<String>,
    api_ctx: ApiContext,
    #[allow(unused_mut)] mut toast: ToastSender,
) {
    let Some(primary) = primary_token.read().clone() else {
        error.set(Some("Sign in again to verify MFA.".to_string()));
        return;
    };
    let entered = mfa_code.read().trim().to_string();
    if entered.is_empty() {
        error.set(Some("Enter your authentication code.".to_string()));
        return;
    }
    submitting.set(true);
    error.set(None);

    let email_value = email.read().clone();
    let challenge_ctx = accepted_api_context(&api_ctx, primary);
    spawn(async move {
        let body = features_courses::api::MfaChallengeBody {
            code: Some(entered),
            trusted_device_token: None,
            remember_device: *remember_device.read(),
            device_label: Some("This device".to_string()),
        };
        match features_courses::api::challenge_mfa(&challenge_ctx, &body).await {
            Ok(resp) => {
                if let Some(device_token) = resp.trusted_device_token.as_deref() {
                    #[cfg(target_arch = "wasm32")]
                    platform_bridge::web::persist_trusted_device_token(&email_value, device_token);
                    #[cfg(not(target_arch = "wasm32"))]
                    let _ = platform_bridge::native::NativeBridge::persist_trusted_device_token(
                        &email_value,
                        device_token,
                    );
                }
                finish_accepted_login(
                    &email_value,
                    resp.stepup_token,
                    *primary_token_is_local.read(),
                    &challenge_ctx,
                    on_success,
                );
            }
            Err(err) => {
                let msg = format!("{err}");
                toast.push(ToastLevel::Danger, "MFA verification failed", msg.clone());
                error.set(Some(msg));
            }
        }
        submitting.set(false);
    });
}
```

- [ ] **Step 6: Gate primary token acceptance on `/v1/me`**

In both wasm and native branches in `submit_login`, replace direct token handling with:

```rust
LoginOutcome::Token {
    token,
    persist_local_token,
} => {
    let check_ctx = accepted_api_context(&api_ctx, token.clone());
    match api::get_me(&check_ctx).await {
        Ok(_) => finish_accepted_login(
            &email_value,
            token,
            persist_local_token,
            &check_ctx,
            on_success,
        ),
        Err(err) => match login_internals::classify_me_error(&err) {
            login_internals::MeCheckOutcome::MfaRequired => {
                match attempt_trusted_device_stepup(&email_value, &check_ctx).await {
                    Ok(Some(stepup_token)) => finish_accepted_login(
                        &email_value,
                        stepup_token,
                        persist_local_token,
                        &check_ctx,
                        on_success,
                    ),
                    Ok(None) => {
                        primary_token.set(Some(token));
                        primary_token_is_local.set(persist_local_token);
                        mfa_required.set(true);
                        error.set(None);
                    }
                    Err(stepup_err) => {
                        let msg = format!("{stepup_err}");
                        toast.push(ToastLevel::Danger, "Sign-in failed", msg.clone());
                        error.set(Some(msg));
                    }
                }
            }
            login_internals::MeCheckOutcome::Failed(msg) => {
                toast.push(ToastLevel::Danger, "Sign-in failed", msg.clone());
                error.set(Some(msg));
            }
            login_internals::MeCheckOutcome::Ok => {}
        },
    }
}
```

Pass `primary_token`, `primary_token_is_local`, and `mfa_required` into `submit_login` by updating its function signature and call site.

- [ ] **Step 7: Run login tests**

Run:

```powershell
cargo test -p features-auth --test login
```

Expected: all login tests pass and the render test still contains email/password controls.

- [ ] **Step 8: Commit login flow**

Run:

```powershell
git add crates/features-auth/src/login.rs crates/features-auth/tests/login.rs
git commit -m "feat(auth): add mfa login challenge"
```

Expected: commit succeeds with login component and tests.

## Task 8: Improve Security Settings MFA UX

**Files:**
- Modify: `crates/features-courses/src/security_settings.rs`

- [ ] **Step 1: Add DTO aliases and API wrappers**

In `security_settings.rs`, import trusted-device DTOs:

```rust
use crate::api::{self, ApiContext, ApiError, TrustedDeviceDto};
```

Add wrappers below `disable_mfa`:

```rust
pub async fn list_trusted_devices(cx: &ApiContext) -> Result<Vec<TrustedDeviceDto>, ApiError> {
    Ok(api::list_mfa_trusted_devices(cx).await?.devices)
}

pub async fn revoke_trusted_device(cx: &ApiContext, id: &str) -> Result<(), ApiError> {
    api::revoke_mfa_trusted_device(cx, id).await
}
```

- [ ] **Step 2: Add recovery-code acknowledgement state**

Inside `SecuritySettings`, add signals:

```rust
    let mut recovery_ack = use_signal(|| false);
    let mut trusted_devices = use_signal(Vec::<TrustedDeviceDto>::new);
```

After successful `verify_mfa`, set:

```rust
                        recovery_ack.set(false);
```

- [ ] **Step 3: Load trusted devices when MFA is enabled**

Add a resource after the status load:

```rust
    use_resource({
        let api = api.clone();
        move || {
            let api = api.clone();
            async move {
                if let Ok(resp) = list_trusted_devices(&api).await {
                    trusted_devices.set(resp);
                }
            }
        }
    });
```

- [ ] **Step 4: Add copy/download helpers**

Add these platform helpers above the component:

```rust
#[cfg(target_arch = "wasm32")]
fn copy_recovery_codes(codes: &[String]) {
    if let Some(clipboard) = web_sys::window().and_then(|w| w.navigator().clipboard()) {
        let _ = clipboard.write_text(&codes.join("\n"));
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn copy_recovery_codes(_codes: &[String]) {}

#[cfg(target_arch = "wasm32")]
fn download_recovery_codes(codes: &[String]) {
    use wasm_bindgen::JsCast;
    let Some(window) = web_sys::window() else { return; };
    let Some(document) = window.document() else { return; };
    let blob_parts = js_sys::Array::new();
    blob_parts.push(&wasm_bindgen::JsValue::from_str(&codes.join("\n")));
    let Ok(blob) = web_sys::Blob::new_with_str_sequence(&blob_parts) else { return; };
    let Ok(url) = web_sys::Url::create_object_url_with_blob(&blob) else { return; };
    let Ok(a) = document.create_element("a") else { return; };
    let _ = a.set_attribute("href", &url);
    let _ = a.set_attribute("download", "aulalite-recovery-codes.txt");
    if let Some(a) = a.dyn_ref::<web_sys::HtmlElement>() {
        a.click();
    }
    let _ = web_sys::Url::revoke_object_url(&url);
}

#[cfg(not(target_arch = "wasm32"))]
fn download_recovery_codes(_codes: &[String]) {}
```

- [ ] **Step 5: Replace recovery panel**

Replace the `recovery_panel` block with:

```rust
    let recovery_panel: Element = if recovery_codes.is_empty() {
        rsx! {}
    } else {
        let can_dismiss = *recovery_ack.read();
        rsx! {
            div { class: "security-recovery",
                span { class: "security-mfa-row-title", "Your recovery codes" }
                span { class: "security-mfa-row-help",
                    "Each code works once. Store them now; they are shown only once."
                }
                ul { class: "security-recovery-list",
                    for rc in recovery_codes.iter() {
                        li { class: "security-recovery-code", "{rc}" }
                    }
                }
                div { class: "security-recovery-actions",
                    Button {
                        label: "Copy codes".to_string(),
                        variant: ButtonVariant::Secondary,
                        on_click: {
                            let codes = recovery_codes.clone();
                            move |_| copy_recovery_codes(&codes)
                        },
                    }
                    Button {
                        label: "Download".to_string(),
                        variant: ButtonVariant::Secondary,
                        on_click: {
                            let codes = recovery_codes.clone();
                            move |_| download_recovery_codes(&codes)
                        },
                    }
                }
                label { class: "security-recovery-ack",
                    input {
                        r#type: "checkbox",
                        checked: can_dismiss,
                        onchange: move |event| recovery_ack.set(event.checked()),
                    }
                    span { "I saved these codes" }
                }
                Button {
                    label: "Done".to_string(),
                    variant: ButtonVariant::Primary,
                    disabled: !can_dismiss,
                    on_click: move |_| {
                        recovery.set(Vec::new());
                        recovery_ack.set(false);
                    },
                }
            }
        }
    };
```

- [ ] **Step 6: Add trusted-device panel**

Add before final `rsx!`:

```rust
    let trusted_panel: Element = if !is_enabled {
        rsx! {}
    } else {
        let devices = trusted_devices.read().clone();
        rsx! {
            div { class: "security-trusted-devices",
                span { class: "security-mfa-row-title", "Remembered devices" }
                if devices.is_empty() {
                    span { class: "security-mfa-row-help", "No remembered devices." }
                } else {
                    ul { class: "security-trusted-device-list",
                        for device in devices.iter() {
                            {
                                let id = device.id.clone();
                                let label = device.label.clone();
                                let api = api.clone();
                                rsx! {
                                    li { class: "security-trusted-device", key: "{id}",
                                        span { class: "security-trusted-device-label", "{label}" }
                                        span { class: "security-mfa-row-help", "Expires {device.expires_at}" }
                                        Button {
                                            label: "Revoke".to_string(),
                                            variant: ButtonVariant::Danger,
                                            on_click: move |_| {
                                                let api = api.clone();
                                                let id = id.clone();
                                                let mut toast = toast;
                                                spawn(async move {
                                                    match revoke_trusted_device(&api, &id).await {
                                                        Ok(()) => {
                                                            trusted_devices.with_mut(|items| items.retain(|d| d.id != id));
                                                            toast.push(ToastLevel::Success, "Device revoked", "This device must use a code next sign-in.");
                                                        }
                                                        Err(err) => toast.push(ToastLevel::Danger, "Revoke failed", format!("{err}")),
                                                    }
                                                });
                                            },
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    };
```

Render `{ trusted_panel }` after `{ recovery_panel }`.

- [ ] **Step 7: Add SSR test for recovery panel helper text**

Extend the existing SSR test to assert:

```rust
assert!(html.contains("Two-factor authentication"));
```

Append this SSR test:

```rust
#[test]
fn panel_mentions_recovery_code_safety() {
    fn app() -> Element {
        use_context_provider(|| {
            Signal::new(ApiContext {
                base_url: String::new(),
                id_token: String::new(),
            })
        });
        use_context_provider(|| Signal::new(design_system::ToastQueue::new()));
        rsx! { SecuritySettings {} }
    }
    let mut vdom = VirtualDom::new(app);
    vdom.rebuild_in_place();
    let html = dioxus_ssr::render(&vdom);
    assert!(html.contains("Security"), "{html}");
}
```

- [ ] **Step 8: Run feature tests**

Run:

```powershell
cargo test -p features-courses --lib security_settings
```

Expected: security settings tests pass.

- [ ] **Step 9: Commit security settings**

Run:

```powershell
git add crates/features-courses/src/security_settings.rs
git commit -m "feat(settings): manage mfa recovery and trusted devices"
```

Expected: commit succeeds with security settings changes.

## Task 9: Add Admin Recovery Reset UI

**Files:**
- Modify: `crates/shell-web/src/routes/admin_tenant.rs`

- [ ] **Step 1: Add pure recovery-code panel helper**

Add this helper above `AdminTenant`:

```rust
fn recovery_reset_panel(member_name: &str, codes: &[String], acknowledged: bool) -> Element {
    rsx! {
        div { class: "admin-mfa-reset-panel",
            h3 { "Replacement recovery codes" }
            p { class: "muted", "Give these codes to {member_name}. They are shown only now." }
            ul { class: "security-recovery-list",
                for code in codes.iter() {
                    li { class: "security-recovery-code", "{code}" }
                }
            }
            label { class: "security-recovery-ack",
                input { r#type: "checkbox", checked: acknowledged, readonly: true }
                span { "Codes have been saved" }
            }
        }
    }
}
```

- [ ] **Step 2: Add UI state**

Inside `AdminTenant`, add:

```rust
    let mut reset_codes_for = use_signal(|| None::<String>);
    let mut reset_codes = use_signal(Vec::<String>::new);
    let mut reset_ack = use_signal(|| false);
```

- [ ] **Step 3: Add reset button in membership editor rows**

Inside each `membership-editor-row`, after the status select label, add:

```rust
Button {
    label: "Reset recovery codes".to_string(),
    variant: ButtonVariant::Secondary,
    on_click: {
        let api = api_for_member.clone();
        let uid = user_id.clone();
        let member_name = name.clone();
        move |_| {
            let api = api.clone();
            let uid = uid.clone();
            let member_name = member_name.clone();
            let mut toast = toast;
            spawn(async move {
                match api::admin_reset_mfa_recovery_codes(&api, &uid).await {
                    Ok(resp) => {
                        reset_codes_for.set(Some(member_name));
                        reset_codes.set(resp.recovery_codes);
                        reset_ack.set(false);
                        toast.push(ToastLevel::Success, "Recovery codes reset", "Share the replacement codes securely.");
                    }
                    Err(err) => {
                        toast.push(ToastLevel::Danger, "Reset failed", format!("{err}"));
                    }
                }
            });
        }
    },
}
```

- [ ] **Step 4: Render reset panel**

After `members_section`, add:

```rust
let reset_panel: Element = if reset_codes.read().is_empty() {
    rsx! {}
} else {
    let member_name = reset_codes_for
        .read()
        .clone()
        .unwrap_or_else(|| "this member".to_string());
    let codes = reset_codes.read().clone();
    let acknowledged = *reset_ack.read();
    rsx! {
        div { class: "admin-mfa-reset-panel-wrap",
            { recovery_reset_panel(&member_name, &codes, acknowledged) }
            label { class: "security-recovery-ack",
                input {
                    r#type: "checkbox",
                    checked: acknowledged,
                    onchange: move |event| reset_ack.set(event.checked()),
                }
                span { "I saved these replacement codes" }
            }
            Button {
                label: "Close".to_string(),
                variant: ButtonVariant::Primary,
                disabled: !acknowledged,
                on_click: move |_| {
                    reset_codes.set(Vec::new());
                    reset_codes_for.set(None);
                    reset_ack.set(false);
                },
            }
        }
    }
};
```

Render `{reset_panel}` after `{members_section}` in the route body.

- [ ] **Step 5: Add SSR test**

Append to the `ssr_tests` module:

```rust
#[test]
fn recovery_reset_panel_requires_acknowledgement_copy() {
    fn app_inner() -> Element {
        recovery_reset_panel(
            "Ada",
            &vec!["abcde-23456".to_string(), "fghjk-789pq".to_string()],
            false,
        )
    }
    let mut vdom = VirtualDom::new(app_inner);
    vdom.rebuild_in_place();
    let html = dioxus_ssr::render(&vdom);
    assert!(html.contains("Replacement recovery codes"), "{html}");
    assert!(html.contains("Ada"), "{html}");
    assert!(html.contains("abcde-23456"), "{html}");
    assert!(html.contains("Codes have been saved"), "{html}");
}
```

- [ ] **Step 6: Run shell-web tests**

Run:

```powershell
cargo test -p shell-web --lib admin_tenant
```

Expected: admin tenant SSR tests pass.

- [ ] **Step 7: Commit admin UI**

Run:

```powershell
git add crates/shell-web/src/routes/admin_tenant.rs
git commit -m "feat(admin): add mfa recovery reset ui"
```

Expected: commit succeeds with admin tenant UI changes.

## Task 10: Phase 2 Verification

**Files:**
- Verify: full Phase 2 change set

- [ ] **Step 1: Run backend DB-free library tests**

Run:

```powershell
cargo test -p backend --lib
```

Expected: exits successfully without requiring PostgreSQL.

- [ ] **Step 2: Run backend MFA integration tests**

Run with a migrated PostgreSQL database:

```powershell
cargo test -p backend --test mfa -- --nocapture
```

Expected: all MFA integration tests pass.

- [ ] **Step 3: Run frontend library and SSR tests**

Run:

```powershell
cargo test -p features-auth --test login
cargo test -p features-courses --lib security_settings
cargo test -p shell-web --lib admin_tenant
```

Expected: all three commands pass.

- [ ] **Step 4: Run compile checks**

Run:

```powershell
cargo check -p backend --all-targets
cargo check -p shell-web --target wasm32-unknown-unknown
```

Expected: both checks pass. Existing warnings are acceptable unless they come from Phase 2 files.

- [ ] **Step 5: Review final status**

Run:

```powershell
git status --short
git log --oneline -10
```

Expected: working tree is clean or contains only unrelated user changes. Recent commits include Phase 2 schema, DB helpers, route extension, backend tests, admin reset, frontend API/storage, login flow, settings UI, and admin UI.
