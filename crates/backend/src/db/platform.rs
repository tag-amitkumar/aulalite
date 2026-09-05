// crates/backend/src/db/platform.rs
//! Platform super-admin data layer: CROSS-TENANT reads + production tenant
//! provisioning for the Elementors staff role.
//!
//! Unlike every other `db` module, these functions operate ACROSS ALL tenants
//! and so cannot rely on a single per-request `app.tenant_id` GUC: a platform
//! admin may have no tenant membership at all, and the listing spans every
//! tenant. They therefore delegate to the SECURITY DEFINER SQL functions added
//! in `migrations/20260530000001_platform_admin.sql`, which run with the
//! migration role's privileges and bypass the per-tenant RLS — exactly the
//! mechanism used by `accept_*_invitations_for_email` (db::parent /
//! db::member_invitations). Because the heavy lifting is server-side in those
//! functions. Actor-bearing operations run in a transaction with
//! `app.user_id` set; each newer definer function requires that context to
//! match its explicit actor argument before crossing the RLS boundary.
//!
//! Initial ownership is provisioned atomically by `platform_provision_tenant`;
//! ordinary tenant invitation APIs cannot grant `org_owner`.
use sqlx::PgPool;
use uuid::Uuid;

/// One row of the platform tenant directory. Matches the column list (and
/// order) returned by `platform_list_tenants()` / `platform_get_tenant_summary`.
#[derive(Debug, sqlx::FromRow)]
pub struct TenantSummaryRow {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub status: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub member_count: i64,
    pub plan_id: Option<String>,
}

/// List EVERY tenant (newest first) with its active member count + plan id.
/// Delegates to the cross-tenant SECURITY DEFINER `platform_list_tenants()`.
pub async fn list_tenants(
    pool: &PgPool,
    actor_user_id: Uuid,
) -> sqlx::Result<Vec<TenantSummaryRow>> {
    let mut tx = crate::db::begin_with_context(pool, actor_user_id, None).await?;
    let rows = sqlx::query_as::<_, TenantSummaryRow>("SELECT * FROM platform_list_tenants()")
        .fetch_all(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(rows)
}

/// Re-read a single tenant's summary (same shape as the list). `None` if the
/// id does not exist. Used to build the mutation responses.
pub async fn get_tenant_summary(
    pool: &PgPool,
    id: Uuid,
    actor_user_id: Uuid,
) -> sqlx::Result<Option<TenantSummaryRow>> {
    let mut tx = crate::db::begin_with_context(pool, actor_user_id, None).await?;
    let row =
        sqlx::query_as::<_, TenantSummaryRow>("SELECT * FROM platform_get_tenant_summary($1)")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
    tx.commit().await?;
    Ok(row)
}

/// Stable reason returned by [`create_tenant`] when the slug is malformed. The
/// handler maps this to a 4xx.
pub const INVALID_SLUG: &str = "invalid_slug";
/// Stable reason returned by [`create_tenant`] when the slug already exists.
pub const SLUG_TAKEN: &str = "slug_taken";

/// Outcome of [`create_tenant`]: either the new tenant id, or a stable reason
/// string the handler maps to a validation / conflict error.
pub enum CreateTenantOutcome {
    Created(Uuid),
    Invalid(&'static str),
    Conflict(&'static str),
}

/// Validate that `slug` is a well-formed tenant slug: non-empty, <= 63 chars,
/// lowercase `[a-z0-9-]`, no leading/trailing/`--` dashes. Mirrors the shape
/// produced by `services::slugger::slugify` so a slug round-trips unchanged.
pub fn is_valid_slug(slug: &str) -> bool {
    if slug.is_empty() || slug.len() > 63 {
        return false;
    }
    if slug.starts_with('-') || slug.ends_with('-') || slug.contains("--") {
        return false;
    }
    slug.bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// Provision a new tenant. Validates the slug format in Rust FIRST (so a bad
/// slug never reaches the DB), then delegates the insert to the cross-tenant
/// SECURITY DEFINER `platform_create_tenant`. A duplicate slug surfaces as a
/// `Conflict` outcome rather than a raw DB error.
pub async fn create_tenant(
    pool: &PgPool,
    slug: &str,
    name: &str,
    admin_email: &str,
    actor_user_id: Uuid,
) -> sqlx::Result<CreateTenantOutcome> {
    if !is_valid_slug(slug) {
        return Ok(CreateTenantOutcome::Invalid(INVALID_SLUG));
    }
    if name.trim().is_empty() {
        return Ok(CreateTenantOutcome::Invalid("invalid_name"));
    }
    if admin_email.trim().is_empty() {
        return Ok(CreateTenantOutcome::Invalid("invalid_admin_email"));
    }

    // Tenant, trial entitlement, initial owner invitation and audit event are
    // committed atomically. An identical retry returns the same tenant so the
    // handler can retry a failed email-provider handoff safely.
    let mut tx = crate::db::begin_with_context(pool, actor_user_id, None).await?;
    let result: Result<Uuid, sqlx::Error> =
        sqlx::query_scalar("SELECT platform_provision_tenant($1, $2, $3, $4)")
            .bind(slug)
            .bind(name)
            .bind(admin_email)
            .bind(actor_user_id)
            .fetch_one(&mut *tx)
            .await;

    match result {
        Ok(id) => {
            tx.commit().await?;
            Ok(CreateTenantOutcome::Created(id))
        }
        Err(sqlx::Error::Database(db)) if is_unique_violation(db.as_ref()) => {
            let _ = tx.rollback().await;
            Ok(CreateTenantOutcome::Conflict(SLUG_TAKEN))
        }
        Err(e) => {
            let _ = tx.rollback().await;
            Err(e)
        }
    }
}

/// True if this DB error is a Postgres `unique_violation` (SQLSTATE 23505).
/// The `platform_create_tenant` fn raises it explicitly on a duplicate slug,
/// and the underlying UNIQUE index would too.
fn is_unique_violation(db: &dyn sqlx::error::DatabaseError) -> bool {
    db.code().as_deref() == Some("23505")
}

/// Set a tenant's lifecycle status (`active` | `suspended` | `trialing`).
/// Returns `true` if the tenant existed and was updated, `false` otherwise.
/// Delegates to the cross-tenant SECURITY DEFINER `platform_set_tenant_status`.
pub async fn set_tenant_status(
    pool: &PgPool,
    id: Uuid,
    status: &str,
    actor_user_id: Uuid,
) -> sqlx::Result<bool> {
    let mut tx = crate::db::begin_with_context(pool, actor_user_id, None).await?;
    let updated = sqlx::query_scalar("SELECT platform_set_tenant_status($1, $2)")
        .bind(id)
        .bind(status)
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(updated)
}

#[derive(Debug, sqlx::FromRow)]
pub struct OwnerRecoveryRow {
    pub previous_owner_user_id: Option<Uuid>,
    pub new_owner_user_id: Uuid,
    pub transferred_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, sqlx::FromRow)]
pub struct OwnerCandidateRow {
    pub user_id: Uuid,
    pub email: String,
    pub display_name: Option<String>,
    pub role: String,
}

#[derive(Debug, sqlx::FromRow)]
pub struct PendingOwnerInvitationRow {
    pub invitation_id: Uuid,
    pub email: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

pub async fn replace_pending_owner_invitation(
    pool: &PgPool,
    tenant_id: Uuid,
    new_email: &str,
    actor_user_id: Uuid,
) -> sqlx::Result<PendingOwnerInvitationRow> {
    let mut tx = crate::db::begin_with_context(pool, actor_user_id, None).await?;
    let row = sqlx::query_as("SELECT * FROM platform_replace_pending_owner_invitation($1, $2, $3)")
        .bind(tenant_id)
        .bind(new_email)
        .bind(actor_user_id)
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(row)
}

pub async fn list_tenant_owner_candidates(
    pool: &PgPool,
    tenant_id: Uuid,
    actor_user_id: Uuid,
) -> sqlx::Result<Vec<OwnerCandidateRow>> {
    let mut tx = crate::db::begin_with_context(pool, actor_user_id, None).await?;
    let rows = sqlx::query_as("SELECT * FROM platform_list_tenant_owner_candidates($1, $2)")
        .bind(tenant_id)
        .bind(actor_user_id)
        .fetch_all(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(rows)
}

/// Explicit platform-owner break-glass recovery. The SECURITY DEFINER function
/// independently verifies the actor and performs the tenant-scoped transfer,
/// because this caller deliberately has no ordinary tenant RLS context.
pub async fn recover_tenant_owner(
    pool: &PgPool,
    tenant_id: Uuid,
    target_user_id: Uuid,
    actor_user_id: Uuid,
) -> sqlx::Result<OwnerRecoveryRow> {
    let mut tx = crate::db::begin_with_context(pool, actor_user_id, None).await?;
    let row = sqlx::query_as("SELECT * FROM platform_recover_tenant_owner($1, $2, $3)")
        .bind(tenant_id)
        .bind(target_user_id)
        .bind(actor_user_id)
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(row)
}

#[cfg(test)]
mod tests {
    use super::is_valid_slug;

    #[test]
    fn accepts_well_formed_slugs() {
        assert!(is_valid_slug("acme"));
        assert!(is_valid_slug("acme-school"));
        assert!(is_valid_slug("a1-b2-c3"));
    }

    #[test]
    fn rejects_malformed_slugs() {
        assert!(!is_valid_slug(""));
        assert!(!is_valid_slug("-acme"));
        assert!(!is_valid_slug("acme-"));
        assert!(!is_valid_slug("ac--me"));
        assert!(!is_valid_slug("Acme")); // uppercase
        assert!(!is_valid_slug("acme school")); // space
        assert!(!is_valid_slug("acme_school")); // underscore
        assert!(!is_valid_slug(&"a".repeat(64))); // too long
    }
}
