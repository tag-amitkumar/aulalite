// crates/backend/src/db/mfa.rs
//! Per-user TOTP multi-factor-authentication enrollment.
//!
//! One row per user in `user_mfa` (PK = `user_id`). The flow is two-phase:
//!   1. ENROLL  — generate a secret, store it `enabled = false`, return the
//!                otpauth URI + base32 secret for the authenticator app.
//!   2. VERIFY  — the user enters a current code; on success we flip
//!                `enabled = true`, stamp `verified_at`, and persist a set of
//!                one-time recovery codes (stored only as SHA-256 hashes).
//!   3. DISABLE — clears enrollment entirely (deletes the row).
//!
//! NOT tenant-scoped: MFA is a property of the global `users` account (which is
//! itself tenant-agnostic — a user may belong to several tenants), mirroring how
//! `users` rows are keyed. Access is therefore scoped to the OWNER via the
//! `app.user_id` GUC (the same bootstrap key `tenant_memberships` uses), PLUS a
//! `system_context` SELECT path so the auth middleware can read `enabled`
//! cross-context (it runs before tenant resolution). All writes flow through the
//! owner-scoped path.
//!
//! TOTP seeds are authenticated-encrypted with an environment-held key. Recovery
//! codes are stored as lowercase-hex SHA-256 hashes (reusing
//! `db::api_keys::hash_secret`); the plaintext is shown exactly once at verify
//! time and never persisted.
//!
//! Uses ONLY runtime sqlx (no compile-time macros — there is no DATABASE_URL at
//! build time).
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::RngCore;
use sqlx::PgPool;
use uuid::Uuid;

const TRUSTED_DEVICE_TOKEN_BYTES: usize = 32;
const TRUSTED_DEVICE_LIFETIME_DAYS: i64 = 30;
const TRUSTED_DEVICE_LABEL_MAX: usize = 80;
const LOCK_ENABLED_MFA_SQL: &str = "SELECT enabled FROM user_mfa WHERE user_id = $1 FOR UPDATE";
const REVOKE_ACTIVE_TRUSTED_DEVICES_SQL: &str = "UPDATE user_mfa_trusted_devices
            SET revoked_at = now(), updated_at = now()
          WHERE user_id = $1
            AND revoked_at IS NULL";
const CREATE_TRUSTED_DEVICE_SQL: &str = "INSERT INTO user_mfa_trusted_devices
             (user_id, token_hash, label, user_agent, expires_at)
         VALUES ($1, $2, $3, $4, now() + ($5 * interval '1 day'))
         RETURNING id, user_id, label, user_agent, last_used_at, expires_at, created_at";
const VALIDATE_TRUSTED_DEVICE_SQL: &str = "UPDATE user_mfa_trusted_devices
            SET last_used_at = now(), updated_at = now()
          WHERE user_id = $1
            AND token_hash = $2
            AND revoked_at IS NULL
            AND expires_at > now()
      RETURNING id, user_id, label, user_agent, last_used_at, expires_at, created_at";
const LOOKUP_TRUSTED_DEVICE_STATUS_SQL: &str = "SELECT revoked_at, expires_at <= now() AS expired
           FROM user_mfa_trusted_devices
          WHERE user_id = $1 AND token_hash = $2";

pub(crate) fn secret_context(user_id: Uuid) -> String {
    format!("user-mfa:{user_id}:totp-secret")
}

fn secret_error(error: crate::services::secret_box::SecretBoxError) -> sqlx::Error {
    sqlx::Error::Protocol(format!("MFA secret unavailable: {error}"))
}

/// MFA enrollment state for a user, as surfaced to the owner / middleware.
pub struct UserMfa {
    pub user_id: Uuid,
    pub secret: Vec<u8>,
    pub enabled: bool,
    pub verified_at: Option<chrono::DateTime<chrono::Utc>>,
    /// SHA-256 hashes of the unused recovery codes.
    pub recovery_codes: Vec<String>,
}

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

#[derive(Debug, Clone, PartialEq)]
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
    rand::rng().fill_bytes(&mut bytes);
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

/// Set the owner GUC the RLS policy depends on, inside `tx`.
async fn set_user(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: Uuid,
) -> sqlx::Result<()> {
    sqlx::query("SELECT set_config('app.user_id', $1, true)")
        .bind(user_id.to_string())
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn revoke_active_trusted_devices(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: Uuid,
) -> sqlx::Result<()> {
    sqlx::query(REVOKE_ACTIVE_TRUSTED_DEVICES_SQL)
        .bind(user_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn lock_enabled_mfa(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: Uuid,
) -> sqlx::Result<bool> {
    let enabled: Option<bool> = sqlx::query_scalar(LOCK_ENABLED_MFA_SQL)
        .bind(user_id)
        .fetch_optional(&mut **tx)
        .await?;
    Ok(enabled.unwrap_or(false))
}

/// Fetch a user's MFA row (owner-scoped). `None` when not enrolled.
pub async fn get(pool: &PgPool, user_id: Uuid) -> sqlx::Result<Option<UserMfa>> {
    let mut tx = pool.begin().await?;
    set_user(&mut tx, user_id).await?;
    let row: Option<(
        Uuid,
        Vec<u8>,
        bool,
        Option<chrono::DateTime<chrono::Utc>>,
        Vec<String>,
    )> = sqlx::query_as(
        "SELECT user_id, secret, enabled, verified_at, recovery_codes
           FROM user_mfa
          WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await?;
    tx.commit().await?;
    row.map(|(user_id, secret, enabled, verified_at, recovery_codes)| {
        let secret =
            crate::services::secret_box::open_bytes(&secret, secret_context(user_id).as_bytes())
                .map_err(secret_error)?;
        Ok(UserMfa {
            user_id,
            secret,
            enabled,
            verified_at,
            recovery_codes,
        })
    })
    .transpose()
}

/// Owner-scoped status lookup that deliberately does not load/decrypt the TOTP
/// seed. UI status and enrollment guards need only this bit.
pub async fn enrollment_status(pool: &PgPool, user_id: Uuid) -> sqlx::Result<Option<bool>> {
    let mut tx = pool.begin().await?;
    set_user(&mut tx, user_id).await?;
    let enabled: Option<bool> =
        sqlx::query_scalar("SELECT enabled FROM user_mfa WHERE user_id = $1")
            .bind(user_id)
            .fetch_optional(&mut *tx)
            .await?;
    tx.commit().await?;
    Ok(enabled)
}

/// Whether `user_id` has a verified+enabled MFA enrollment. Runs WITHOUT an
/// owner GUC (the auth middleware calls this before tenant resolution), so it
/// elevates to `app.system='on'` for the read — guarded by the
/// `system_context_select` policy. Callers enforcing MFA must fail closed on a
/// lookup error; otherwise a transient database fault would become an MFA
/// bypass.
pub async fn is_enabled(pool: &PgPool, user_id: Uuid) -> sqlx::Result<bool> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.system', 'on', true)")
        .execute(&mut *tx)
        .await?;
    let enabled: Option<bool> =
        sqlx::query_scalar("SELECT enabled FROM user_mfa WHERE user_id = $1")
            .bind(user_id)
            .fetch_optional(&mut *tx)
            .await?;
    tx.commit().await?;
    Ok(enabled.unwrap_or(false))
}

/// Start (or restart) enrollment: upsert the row with a fresh `secret`, set
/// `enabled = false`, and clear any prior recovery codes. Idempotent — re-enroll
/// before verifying simply rotates the pending secret. An enabled enrollment is
/// never replaced: that guard lives in this statement (rather than only in the
/// handler) so a concurrent verification cannot be undone by a stale request.
/// Returns false when MFA was already enabled. Owner-scoped.
pub async fn start_enrollment(pool: &PgPool, user_id: Uuid, secret: &[u8]) -> sqlx::Result<bool> {
    let encrypted_secret =
        crate::services::secret_box::seal_bytes(secret, secret_context(user_id).as_bytes())
            .map_err(secret_error)?;
    let mut tx = pool.begin().await?;
    set_user(&mut tx, user_id).await?;
    let result = sqlx::query(
        "INSERT INTO user_mfa (user_id, secret, enabled, verified_at, recovery_codes)
         VALUES ($1, $2, false, NULL, '{}')
         ON CONFLICT (user_id) DO UPDATE
             SET secret = EXCLUDED.secret,
                 enabled = false,
                 verified_at = NULL,
                 recovery_codes = '{}',
                 updated_at = now()
           WHERE user_mfa.enabled = false",
    )
    .bind(user_id)
    .bind(encrypted_secret)
    .execute(&mut *tx)
    .await?;
    let changed = result.rows_affected() > 0;
    if changed {
        revoke_active_trusted_devices(&mut tx, user_id).await?;
    }
    tx.commit().await?;
    Ok(changed)
}

/// Confirm enrollment: flip `enabled = true`, stamp `verified_at`, and store the
/// hashed recovery codes. The pending row is locked and its decrypted seed must
/// still equal the seed the handler actually verified. This prevents a stale
/// valid code for enrollment A from enabling concurrently-created enrollment B.
/// Returns true if that exact pending enrollment was activated. Owner-scoped.
pub async fn confirm_enrollment(
    pool: &PgPool,
    user_id: Uuid,
    verified_secret: &[u8],
    recovery_code_hashes: &[String],
) -> sqlx::Result<bool> {
    let mut tx = pool.begin().await?;
    set_user(&mut tx, user_id).await?;
    let stored_secret: Option<Vec<u8>> = sqlx::query_scalar(
        "SELECT secret
           FROM user_mfa
          WHERE user_id = $1 AND enabled = false
          FOR UPDATE",
    )
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(stored_secret) = stored_secret else {
        tx.commit().await?;
        return Ok(false);
    };
    let current_secret =
        crate::services::secret_box::open_bytes(&stored_secret, secret_context(user_id).as_bytes())
            .map_err(secret_error)?;
    if current_secret.as_slice() != verified_secret {
        tx.commit().await?;
        return Ok(false);
    }

    let res = sqlx::query(
        "UPDATE user_mfa
            SET enabled = true,
                verified_at = now(),
                recovery_codes = $2,
                updated_at = now()
          WHERE user_id = $1 AND enabled = false AND secret = $3",
    )
    .bind(user_id)
    .bind(recovery_code_hashes)
    .bind(stored_secret)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(res.rows_affected() > 0)
}

/// Disable MFA: delete the enrollment row outright. Returns true if one existed.
/// Owner-scoped.
pub async fn disable(pool: &PgPool, user_id: Uuid) -> sqlx::Result<bool> {
    let mut tx = pool.begin().await?;
    set_user(&mut tx, user_id).await?;
    let res = sqlx::query("DELETE FROM user_mfa WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    revoke_active_trusted_devices(&mut tx, user_id).await?;
    tx.commit().await?;
    Ok(res.rows_affected() > 0)
}

/// Consume a one-time recovery code: if its hash is present in the row, remove it
/// and return true. Owner-scoped, atomic within the tx.
pub async fn consume_recovery_code(
    pool: &PgPool,
    user_id: Uuid,
    code_hash: &str,
) -> sqlx::Result<bool> {
    let mut tx = pool.begin().await?;
    set_user(&mut tx, user_id).await?;
    let res = sqlx::query(
        "UPDATE user_mfa
            SET recovery_codes = array_remove(recovery_codes, $2),
                updated_at = now()
          WHERE user_id = $1
            AND enabled = true
            AND $2 = ANY(recovery_codes)",
    )
    .bind(user_id)
    .bind(code_hash)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(res.rows_affected() > 0)
}

/// Atomically claim TOTP time-step `step` as consumed for step-up challenges
/// (RFC 6238 §5.2 replay prevention). Succeeds only when `step` is STRICTLY
/// newer than the last consumed step; a repeated or older code returns false
/// and must be rejected. Owner-scoped; the single conditional UPDATE makes the
/// check-and-set race-free across concurrent logins.
pub async fn claim_totp_step(pool: &PgPool, user_id: Uuid, step: i64) -> sqlx::Result<bool> {
    let mut tx = pool.begin().await?;
    set_user(&mut tx, user_id).await?;
    let res = sqlx::query(
        "UPDATE user_mfa
            SET last_used_totp_step = $2,
                updated_at = now()
          WHERE user_id = $1
            AND enabled = true
            AND last_used_totp_step < $2",
    )
    .bind(user_id)
    .bind(step)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(res.rows_affected() > 0)
}

pub async fn create_trusted_device(
    pool: &PgPool,
    user_id: Uuid,
    label: Option<String>,
    user_agent: Option<String>,
) -> sqlx::Result<(TrustedDevice, String)> {
    let plaintext = trusted_device_token();
    let hash = trusted_device_hash(&plaintext);
    let label = normalize_trusted_device_label(label);

    let mut tx = pool.begin().await?;
    set_user(&mut tx, user_id).await?;
    if !lock_enabled_mfa(&mut tx, user_id).await? {
        tx.commit().await?;
        return Err(sqlx::Error::RowNotFound);
    }

    let row: (
        Uuid,
        Uuid,
        String,
        Option<String>,
        Option<chrono::DateTime<chrono::Utc>>,
        chrono::DateTime<chrono::Utc>,
        chrono::DateTime<chrono::Utc>,
    ) = sqlx::query_as(CREATE_TRUSTED_DEVICE_SQL)
        .bind(user_id)
        .bind(hash)
        .bind(label)
        .bind(user_agent)
        .bind(TRUSTED_DEVICE_LIFETIME_DAYS)
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
    if !lock_enabled_mfa(&mut tx, user_id).await? {
        tx.commit().await?;
        return Ok(TrustedDeviceCheck::Invalid);
    }

    let row: Option<(
        Uuid,
        Uuid,
        String,
        Option<String>,
        Option<chrono::DateTime<chrono::Utc>>,
        chrono::DateTime<chrono::Utc>,
        chrono::DateTime<chrono::Utc>,
    )> = sqlx::query_as(VALIDATE_TRUSTED_DEVICE_SQL)
        .bind(user_id)
        .bind(&hash)
        .fetch_optional(&mut *tx)
        .await?;

    if let Some(row) = row {
        tx.commit().await?;
        return Ok(TrustedDeviceCheck::Valid(TrustedDevice {
            id: row.0,
            user_id: row.1,
            label: row.2,
            user_agent: row.3,
            last_used_at: row.4,
            expires_at: row.5,
            created_at: row.6,
        }));
    }

    let status: Option<(Option<chrono::DateTime<chrono::Utc>>, bool)> =
        sqlx::query_as(LOOKUP_TRUSTED_DEVICE_STATUS_SQL)
            .bind(user_id)
            .bind(&hash)
            .fetch_optional(&mut *tx)
            .await?;

    let check = match status {
        None => TrustedDeviceCheck::Invalid,
        Some((Some(_), _)) => TrustedDeviceCheck::Revoked,
        Some((None, true)) => TrustedDeviceCheck::Expired,
        Some((None, false)) => TrustedDeviceCheck::Invalid,
    };
    tx.commit().await?;
    Ok(check)
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

#[cfg(test)]
mod tests {
    use super::{
        normalize_trusted_device_label, trusted_device_hash, trusted_device_token, TrustedDevice,
        TrustedDeviceCheck, CREATE_TRUSTED_DEVICE_SQL, LOCK_ENABLED_MFA_SQL,
        REVOKE_ACTIVE_TRUSTED_DEVICES_SQL, VALIDATE_TRUSTED_DEVICE_SQL,
    };

    fn normalized_sql(sql: &str) -> String {
        sql.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    #[test]
    fn trusted_device_token_is_random_and_url_safe() {
        let a = trusted_device_token();
        let b = trusted_device_token();

        assert_ne!(a, b);
        assert!(a.len() >= 43, "{a}");
        assert!(a
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
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
        let now = chrono::Utc::now();
        let trusted_device = TrustedDevice {
            id: uuid::Uuid::new_v4(),
            user_id: uuid::Uuid::new_v4(),
            label: "This device".to_string(),
            user_agent: None,
            last_used_at: Some(now),
            expires_at: now,
            created_at: now,
        };

        assert_eq!(
            TrustedDeviceCheck::Valid(trusted_device).error_code(),
            "trusted_device_valid"
        );
        assert_eq!(
            TrustedDeviceCheck::Invalid.error_code(),
            "trusted_device_invalid"
        );
        assert_eq!(
            TrustedDeviceCheck::Expired.error_code(),
            "trusted_device_expired"
        );
        assert_eq!(
            TrustedDeviceCheck::Revoked.error_code(),
            "trusted_device_revoked"
        );
    }

    #[test]
    fn trusted_device_sql_guards_lifecycle_and_validation() {
        let revoke = normalized_sql(REVOKE_ACTIVE_TRUSTED_DEVICES_SQL);
        assert!(
            revoke.contains(
                "UPDATE user_mfa_trusted_devices SET revoked_at = now(), updated_at = now() \
                 WHERE user_id = $1 AND revoked_at IS NULL"
            ),
            "{revoke}"
        );

        let validate = normalized_sql(VALIDATE_TRUSTED_DEVICE_SQL);
        assert!(
            validate.contains(
                "UPDATE user_mfa_trusted_devices SET last_used_at = now(), updated_at = now() \
                 WHERE user_id = $1 AND token_hash = $2 AND revoked_at IS NULL \
                 AND expires_at > now() RETURNING id, user_id, label, user_agent, \
                 last_used_at, expires_at, created_at"
            ),
            "{validate}"
        );
    }

    #[test]
    fn trusted_device_sql_locks_enabled_mfa_and_uses_db_expiry() {
        let lock = normalized_sql(LOCK_ENABLED_MFA_SQL);
        assert_eq!(
            lock,
            "SELECT enabled FROM user_mfa WHERE user_id = $1 FOR UPDATE"
        );

        let create = normalized_sql(CREATE_TRUSTED_DEVICE_SQL);
        assert!(
            create.contains(
                "INSERT INTO user_mfa_trusted_devices (user_id, token_hash, label, user_agent, \
                 expires_at) VALUES ($1, $2, $3, $4, now() + ($5 * interval '1 day')) \
                 RETURNING id, user_id, label, user_agent, last_used_at, expires_at, created_at"
            ),
            "{create}"
        );
    }
}
