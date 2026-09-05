// crates/backend/src/db/api_keys.rs
//! Tenant-scoped programmatic API keys for the read-only public API (`/api/v1/*`).
//!
//! A key is minted by an org-admin and the PLAINTEXT secret is returned EXACTLY
//! ONCE (we store only its SHA-256 hash). Authentication of an inbound
//! `Authorization: Bearer ak_<prefix>_<secret>` happens in
//! `db::api_keys::authenticate`, which resolves the key by its public `prefix`
//! (an indexed equality lookup), constant-time compares the SHA-256 of the
//! presented secret against the stored `key_hash`, and rejects revoked keys.
//!
//! TENANT-SCOPED under RLS exactly like `db::announcements`: every read/write
//! runs inside a tx with the `app.tenant_id` GUC set so the strict
//! `tenant_isolation` policy applies under the non-bypass `aulalite_app` role.
//! The ONE exception is `authenticate`, which runs with NO tenant GUC (the
//! caller has not been resolved to a tenant yet) and therefore relies on the
//! `system_context` SELECT/UPDATE policies (migration 053) to look the key up
//! cross-tenant; it sets `app.system='on'` for exactly that lookup.
//!
//! Uses ONLY runtime sqlx (no compile-time macros — there is no DATABASE_URL at
//! build time).
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

/// One API key row as surfaced to the admin list (NEVER includes the secret or
/// the hash — those never leave this module after minting).
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct ApiKeyRow {
    pub id: Uuid,
    pub name: String,
    pub prefix: String,
    pub scopes: Vec<String>,
    pub created_by: Uuid,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub last_used_at: Option<chrono::DateTime<chrono::Utc>>,
    pub revoked_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// The authenticated principal a valid `ak_...` token resolves to. Returned by
/// `authenticate`; the public handlers scope every read to `tenant_id`.
#[derive(Debug, Clone)]
pub struct AuthedKey {
    pub key_id: Uuid,
    pub tenant_id: Uuid,
    pub scopes: Vec<String>,
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

/// Lowercase hex SHA-256 of `secret`. The same routine is used at mint time and
/// at authenticate time so the stored hash and the lookup hash always match.
pub fn hash_secret(secret: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(secret.as_bytes());
    let digest = hasher.finalize();
    let mut out = String::with_capacity(digest.len() * 2);
    for b in digest {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// Insert a freshly-minted key. The caller has already generated `prefix` +
/// `key_hash` (and holds the plaintext to return once). Tenant-scoped.
#[allow(clippy::too_many_arguments)]
pub async fn insert(
    pool: &PgPool,
    tenant_id: Uuid,
    name: &str,
    prefix: &str,
    key_hash: &str,
    scopes: &[String],
    created_by: Uuid,
) -> sqlx::Result<ApiKeyRow> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    let row = sqlx::query_as::<_, ApiKeyRow>(
        "INSERT INTO api_keys
             (tenant_id, name, prefix, key_hash, scopes, created_by)
         VALUES ($1, $2, $3, $4, $5, $6)
         RETURNING id, name, prefix, scopes, created_by, created_at, last_used_at, revoked_at",
    )
    .bind(tenant_id)
    .bind(name)
    .bind(prefix)
    .bind(key_hash)
    .bind(scopes)
    .bind(created_by)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(row)
}

/// List a tenant's API keys, newest first (active first, then revoked). Never
/// returns the hash/secret. Tenant-scoped.
pub async fn list(pool: &PgPool, tenant_id: Uuid) -> sqlx::Result<Vec<ApiKeyRow>> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    let rows = sqlx::query_as::<_, ApiKeyRow>(
        "SELECT id, name, prefix, scopes, created_by, created_at, last_used_at, revoked_at
           FROM api_keys
          ORDER BY (revoked_at IS NOT NULL), created_at DESC, id DESC",
    )
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(rows)
}

/// Revoke a key by id within `tenant_id` (idempotent — only stamps `revoked_at`
/// on a still-active row). Returns true if a row was newly revoked. Tenant-scoped.
pub async fn revoke(pool: &PgPool, tenant_id: Uuid, id: Uuid) -> sqlx::Result<bool> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    let res = sqlx::query(
        "UPDATE api_keys SET revoked_at = now()
          WHERE id = $1 AND revoked_at IS NULL",
    )
    .bind(id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(res.rows_affected() > 0)
}

/// Authenticate a presented `ak_<prefix>_<secret>` token. Returns the resolved
/// `AuthedKey` (tenant + scopes) on success, `Ok(None)` when the prefix is
/// unknown / the secret mismatches / the key is revoked.
///
/// This runs WITHOUT a tenant GUC (the tenant is unknown until the key is
/// resolved), so it elevates to `app.system='on'` for the cross-tenant lookup —
/// guarded by the `system_context_select` policy added in migration 053. The
/// `last_used_at` bump uses the matching `system_context_update` policy. The
/// secret comparison is constant-time (string compare over the fixed-width hex
/// SHA-256 of attacker-controlled input vs the stored hash).
pub async fn authenticate(pool: &PgPool, token: &str) -> sqlx::Result<Option<AuthedKey>> {
    // Token shape: `ak_<prefix>_<secret>`. The `prefix` is the indexed public
    // half; `secret` is the high-entropy half we hash.
    let Some((prefix, secret)) = parse_token(token) else {
        return Ok(None);
    };

    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.system', 'on', true)")
        .execute(&mut *tx)
        .await?;

    let row: Option<(Uuid, Uuid, String, Vec<String>)> = sqlx::query_as(
        "SELECT k.id, k.tenant_id, k.key_hash, k.scopes
           FROM api_keys k
          WHERE k.prefix = $1
            AND k.revoked_at IS NULL
            AND EXISTS (
                SELECT 1 FROM tenants t
                 WHERE t.id = k.tenant_id
                   AND tenant_access_allowed(t.id)
            )",
    )
    .bind(&prefix)
    .fetch_optional(&mut *tx)
    .await?;

    let Some((key_id, tenant_id, key_hash, scopes)) = row else {
        tx.commit().await?;
        return Ok(None);
    };

    let presented = hash_secret(&secret);
    if !ct_eq_hex(&presented, &key_hash) {
        tx.commit().await?;
        return Ok(None);
    }

    // Best-effort last-used bump in the same elevated tx.
    sqlx::query("UPDATE api_keys SET last_used_at = now() WHERE id = $1")
        .bind(key_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    Ok(Some(AuthedKey {
        key_id,
        tenant_id,
        scopes,
    }))
}

/// Split `ak_<prefix>_<secret>` into `(prefix, secret)`. Returns `None` if the
/// token is not in the expected shape. The prefix is everything between the
/// `ak_` marker and the final `_`; the secret is the trailing segment.
fn parse_token(token: &str) -> Option<(String, String)> {
    let rest = token.trim().strip_prefix("ak_")?;
    let (prefix, secret) = rest.rsplit_once('_')?;
    if prefix.is_empty() || secret.is_empty() {
        return None;
    }
    Some((format!("ak_{prefix}"), secret.to_string()))
}

/// Constant-time equality over two equal-length hex strings. Returns false fast
/// only on a length mismatch (which leaks nothing — both inputs are SHA-256 hex,
/// always 64 chars). Avoids a new crate by folding the XOR like `services::billing`.
fn ct_eq_hex(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.bytes().zip(b.bytes()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_secret_is_stable_lowercase_hex() {
        let h = hash_secret("hello");
        assert_eq!(h.len(), 64);
        assert!(h
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        // Known SHA-256("hello").
        assert_eq!(
            h,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn parse_token_splits_prefix_and_secret() {
        let (p, s) = parse_token("ak_abcd1234_supersecretvalue").unwrap();
        assert_eq!(p, "ak_abcd1234");
        assert_eq!(s, "supersecretvalue");
    }

    #[test]
    fn parse_token_rejects_garbage() {
        assert!(parse_token("nope").is_none());
        assert!(parse_token("ak_onlyprefix").is_none());
        assert!(parse_token("ak__").is_none());
    }

    #[test]
    fn ct_eq_hex_matches_and_rejects() {
        assert!(ct_eq_hex("deadbeef", "deadbeef"));
        assert!(!ct_eq_hex("deadbeef", "deadbeee"));
        assert!(!ct_eq_hex("dead", "deadbeef"));
    }
}
