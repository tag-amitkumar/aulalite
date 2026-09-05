//! Privileged startup rewrap for legacy plaintext and previous-key secrets.
//!
//! This runs through the owner-level migration connection immediately after
//! schema migrations. Discovery uses the existing read-only system policy;
//! each compare-and-swap write then uses its exact tenant/user RLS context.

use sqlx::PgPool;
use uuid::Uuid;

const MAX_COMPARE_AND_SWAP_ATTEMPTS: usize = 4;
const DISCOVERY_BATCH_SIZE: i64 = 500;

#[derive(Debug, Default, Clone, Copy)]
pub struct SecretRewrapReport {
    pub sso_client_secrets_scanned: u64,
    pub sso_client_secrets: u64,
    pub mfa_totp_secrets_scanned: u64,
    pub mfa_totp_secrets: u64,
}

pub async fn rewrap_stored_secrets(pool: &PgPool) -> anyhow::Result<SecretRewrapReport> {
    // Authenticate every existing ciphertext before committing any rewrite.
    // Without this preflight, one unreadable row late in the scan could leave a
    // partially-rotated database that needs three keys to recover (old, intended
    // new, and the accidentally configured key).
    let (sso_scanned, mfa_scanned) = preflight_stored_secrets(pool).await?;

    let mut report = SecretRewrapReport {
        sso_client_secrets_scanned: sso_scanned,
        mfa_totp_secrets_scanned: mfa_scanned,
        ..SecretRewrapReport::default()
    };

    let mut after_tenant_id = None;
    loop {
        let sso_rows: Vec<(Uuid, String)> = {
            let mut discovery = super::begin_system_context(pool).await?;
            let rows = sqlx::query_as(
                "SELECT tenant_id, client_secret
                   FROM tenant_sso_configs
                  WHERE ($1::uuid IS NULL OR tenant_id > $1)
                  ORDER BY tenant_id
                  LIMIT $2",
            )
            .bind(after_tenant_id)
            .bind(DISCOVERY_BATCH_SIZE)
            .fetch_all(&mut *discovery)
            .await?;
            discovery.commit().await?;
            rows
        };
        let Some(last_tenant_id) = sso_rows.last().map(|row| row.0) else {
            break;
        };
        for (tenant_id, stored) in sso_rows {
            report.sso_client_secrets += rewrap_sso_secret(pool, tenant_id, stored).await?;
        }
        after_tenant_id = Some(last_tenant_id);
    }

    let mut after_user_id = None;
    loop {
        let mfa_rows: Vec<(Uuid, Vec<u8>)> = {
            let mut discovery = super::begin_system_context(pool).await?;
            let rows = sqlx::query_as(
                "SELECT user_id, secret
                   FROM user_mfa
                  WHERE ($1::uuid IS NULL OR user_id > $1)
                  ORDER BY user_id
                  LIMIT $2",
            )
            .bind(after_user_id)
            .bind(DISCOVERY_BATCH_SIZE)
            .fetch_all(&mut *discovery)
            .await?;
            discovery.commit().await?;
            rows
        };
        let Some(last_user_id) = mfa_rows.last().map(|row| row.0) else {
            break;
        };
        for (user_id, stored) in mfa_rows {
            report.mfa_totp_secrets += rewrap_mfa_secret(pool, user_id, stored).await?;
        }
        after_user_id = Some(last_user_id);
    }

    Ok(report)
}

async fn preflight_stored_secrets(pool: &PgPool) -> anyhow::Result<(u64, u64)> {
    let mut sso_scanned = 0u64;
    let mut after_tenant_id = None;
    loop {
        let rows: Vec<(Uuid, String)> = {
            let mut discovery = super::begin_system_context(pool).await?;
            let rows = sqlx::query_as(
                "SELECT tenant_id, client_secret
                   FROM tenant_sso_configs
                  WHERE ($1::uuid IS NULL OR tenant_id > $1)
                  ORDER BY tenant_id
                  LIMIT $2",
            )
            .bind(after_tenant_id)
            .bind(DISCOVERY_BATCH_SIZE)
            .fetch_all(&mut *discovery)
            .await?;
            discovery.commit().await?;
            rows
        };
        let Some(last_id) = rows.last().map(|row| row.0) else {
            break;
        };
        for (tenant_id, stored) in rows {
            crate::services::secret_box::open_text(
                &stored,
                super::sso::secret_context(tenant_id).as_bytes(),
            )
            .map_err(|error| {
                anyhow::anyhow!(
                    "tenant SSO secret failed data-key preflight ({tenant_id}): {error}"
                )
            })?;
            sso_scanned += 1;
        }
        after_tenant_id = Some(last_id);
    }

    let mut mfa_scanned = 0u64;
    let mut after_user_id = None;
    loop {
        let rows: Vec<(Uuid, Vec<u8>)> = {
            let mut discovery = super::begin_system_context(pool).await?;
            let rows = sqlx::query_as(
                "SELECT user_id, secret
                   FROM user_mfa
                  WHERE ($1::uuid IS NULL OR user_id > $1)
                  ORDER BY user_id
                  LIMIT $2",
            )
            .bind(after_user_id)
            .bind(DISCOVERY_BATCH_SIZE)
            .fetch_all(&mut *discovery)
            .await?;
            discovery.commit().await?;
            rows
        };
        let Some(last_id) = rows.last().map(|row| row.0) else {
            break;
        };
        for (user_id, stored) in rows {
            crate::services::secret_box::open_bytes(
                &stored,
                super::mfa::secret_context(user_id).as_bytes(),
            )
            .map_err(|error| {
                anyhow::anyhow!("MFA secret failed data-key preflight ({user_id}): {error}")
            })?;
            mfa_scanned += 1;
        }
        after_user_id = Some(last_id);
    }

    Ok((sso_scanned, mfa_scanned))
}

/// Re-read after a lost compare-and-swap instead of silently leaving a row on
/// the old key. This matters during rolling deploys, where an old replica can
/// legitimately update the same enrollment/config while startup is scanning.
async fn rewrap_sso_secret(
    pool: &PgPool,
    tenant_id: Uuid,
    mut stored: String,
) -> anyhow::Result<u64> {
    let context = super::sso::secret_context(tenant_id);
    for _ in 0..MAX_COMPARE_AND_SWAP_ATTEMPTS {
        let Some(rewrapped) =
            crate::services::secret_box::rewrap_text(&stored, context.as_bytes())?
        else {
            return Ok(0);
        };

        let mut tx = pool.begin().await?;
        sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
            .bind(tenant_id.to_string())
            .execute(&mut *tx)
            .await?;
        let updated = sqlx::query(
            "UPDATE tenant_sso_configs
                SET client_secret = $2, updated_at = now()
              WHERE tenant_id = $1 AND client_secret = $3",
        )
        .bind(tenant_id)
        .bind(rewrapped)
        .bind(&stored)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if updated == 1 {
            tx.commit().await?;
            return Ok(1);
        }

        let latest: Option<String> =
            sqlx::query_scalar("SELECT client_secret FROM tenant_sso_configs WHERE tenant_id = $1")
                .bind(tenant_id)
                .fetch_optional(&mut *tx)
                .await?;
        tx.commit().await?;
        let Some(latest) = latest else {
            if sso_secret_exists_in_system_context(pool, tenant_id).await? {
                anyhow::bail!(
                    "tenant SSO secret became invisible in its tenant RLS context during data-key rewrap ({tenant_id})"
                );
            }
            return Ok(0);
        };
        stored = latest;
    }
    anyhow::bail!("tenant SSO secret changed repeatedly during data-key rewrap ({tenant_id})")
}

async fn rewrap_mfa_secret(
    pool: &PgPool,
    user_id: Uuid,
    mut stored: Vec<u8>,
) -> anyhow::Result<u64> {
    let context = super::mfa::secret_context(user_id);
    for _ in 0..MAX_COMPARE_AND_SWAP_ATTEMPTS {
        let Some(rewrapped) =
            crate::services::secret_box::rewrap_bytes(&stored, context.as_bytes())?
        else {
            return Ok(0);
        };

        let mut tx = pool.begin().await?;
        super::set_request_guc(&mut tx, user_id, None).await?;
        let updated = sqlx::query(
            "UPDATE user_mfa
                SET secret = $2, updated_at = now()
              WHERE user_id = $1 AND secret = $3",
        )
        .bind(user_id)
        .bind(rewrapped)
        .bind(&stored)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if updated == 1 {
            tx.commit().await?;
            return Ok(1);
        }

        let latest: Option<Vec<u8>> =
            sqlx::query_scalar("SELECT secret FROM user_mfa WHERE user_id = $1")
                .bind(user_id)
                .fetch_optional(&mut *tx)
                .await?;
        tx.commit().await?;
        let Some(latest) = latest else {
            if mfa_secret_exists_in_system_context(pool, user_id).await? {
                anyhow::bail!(
                    "MFA secret became invisible in its owner RLS context during data-key rewrap ({user_id})"
                );
            }
            return Ok(0);
        };
        stored = latest;
    }
    anyhow::bail!("MFA secret changed repeatedly during data-key rewrap ({user_id})")
}

async fn sso_secret_exists_in_system_context(
    pool: &PgPool,
    tenant_id: Uuid,
) -> anyhow::Result<bool> {
    let mut tx = super::begin_system_context(pool).await?;
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM tenant_sso_configs WHERE tenant_id = $1)")
            .bind(tenant_id)
            .fetch_one(&mut *tx)
            .await?;
    tx.commit().await?;
    Ok(exists)
}

async fn mfa_secret_exists_in_system_context(pool: &PgPool, user_id: Uuid) -> anyhow::Result<bool> {
    let mut tx = super::begin_system_context(pool).await?;
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM user_mfa WHERE user_id = $1)")
            .bind(user_id)
            .fetch_one(&mut *tx)
            .await?;
    tx.commit().await?;
    Ok(exists)
}
