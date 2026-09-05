// crates/backend/src/db/branding.rs
//! Tenant branding read/write.
//!
//! Branding lives in the pre-existing `tenants.branding JSONB` column (see
//! `migrations/20260503000002_tenants.sql`) as a flat object:
//!   { "logo_url": ..., "primary_color": ..., "accent_color": ... }
//! Any of the three keys may be absent/null. An entirely-unset column (NULL or
//! `{}`) decodes to an all-`None` [`Branding`].
//!
//! All reads/writes run inside a tenant-scoped tx so the `tenants_self_access`
//! RLS policy applies under the non-bypass `aulalite_app` role.

use serde::{Deserialize, Serialize};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

/// The three branding fields, as stored in / read from `tenants.branding`.
/// Serialized keys match the persisted JSON object shape exactly.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Branding {
    #[serde(default)]
    pub logo_url: Option<String>,
    #[serde(default)]
    pub primary_color: Option<String>,
    #[serde(default)]
    pub accent_color: Option<String>,
}

/// Read a tenant's branding. Returns `Ok(None)` when the tenant row is not
/// visible (e.g. wrong tenant / RLS), and an all-`None` [`Branding`] when the
/// row exists but `branding` is NULL or `{}`.
pub async fn get(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
) -> sqlx::Result<Option<Branding>> {
    let row: Option<(Option<serde_json::Value>,)> =
        sqlx::query_as("SELECT branding FROM tenants WHERE id = $1")
            .bind(tenant_id)
            .fetch_optional(&mut **tx)
            .await?;
    Ok(row.map(|(json,)| decode(json)))
}

/// Overwrite a tenant's branding with `branding`, serialized to the flat JSON
/// object. Returns the stored value (re-decoded) or `None` if the row is not
/// visible. Caller is responsible for merge semantics + emitting the audit row
/// in the same tx.
pub async fn set(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    branding: &Branding,
) -> sqlx::Result<Option<Branding>> {
    // serde_json::to_value on a struct of Options never fails.
    let json = serde_json::to_value(branding).unwrap_or(serde_json::Value::Null);
    let row: Option<(Option<serde_json::Value>,)> = sqlx::query_as(
        "UPDATE tenants
            SET branding = $2, updated_at = now()
          WHERE id = $1
        RETURNING branding",
    )
    .bind(tenant_id)
    .bind(json)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(|(json,)| decode(json)))
}

/// Resolve the caller's active tenant: the first `active` tenant_membership by
/// join time. This is used only while `/v1/me/branding` bootstraps before the
/// shell has selected a workspace; tenant-scoped course authorization always
/// requires the explicit workspace carried by `RequestContext`. Reuses the
/// caller's tx/connection so the `app.user_id` GUC applies under RLS.
pub async fn active_tenant_for_user(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
) -> sqlx::Result<Option<Uuid>> {
    sqlx::query_scalar(
        "SELECT tenant_id
           FROM tenant_memberships
          WHERE user_id = $1 AND status = 'active'
          ORDER BY joined_at ASC
          LIMIT 1",
    )
    .bind(user_id)
    .fetch_optional(&mut **tx)
    .await
}

/// Decode a possibly-NULL `branding` JSON value into a [`Branding`]. A NULL
/// column, an empty object, or a malformed payload all degrade to all-`None`.
fn decode(json: Option<serde_json::Value>) -> Branding {
    match json {
        Some(v) => serde_json::from_value(v).unwrap_or_default(),
        None => Branding::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_null_and_empty_are_all_none() {
        let b = decode(None);
        assert!(b.logo_url.is_none() && b.primary_color.is_none() && b.accent_color.is_none());
        let b = decode(Some(serde_json::json!({})));
        assert!(b.logo_url.is_none() && b.primary_color.is_none() && b.accent_color.is_none());
    }

    #[test]
    fn decode_roundtrips_fields() {
        let b = decode(Some(serde_json::json!({
            "logo_url": "https://cdn/logo.png",
            "primary_color": "#abc",
            "accent_color": "#112233"
        })));
        assert_eq!(b.logo_url.as_deref(), Some("https://cdn/logo.png"));
        assert_eq!(b.primary_color.as_deref(), Some("#abc"));
        assert_eq!(b.accent_color.as_deref(), Some("#112233"));
    }

    #[test]
    fn decode_ignores_unknown_and_partial() {
        let b = decode(Some(serde_json::json!({
            "primary_color": "#fff",
            "unrelated": 7
        })));
        assert_eq!(b.primary_color.as_deref(), Some("#fff"));
        assert!(b.logo_url.is_none());
    }
}
