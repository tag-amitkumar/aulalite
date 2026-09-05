// crates/backend/src/db/scorm.rs
//! SCORM package registry + per-user CMI runtime store (Tool side).
//!
//! A SCORM `.zip` is uploaded via the standard presigned-upload path (purpose
//! `"scorm"`, linked_entity_type `"course"`) producing a `file_assets` row. Staff
//! then REGISTER it against a course: we record the package metadata + the parsed
//! launch href (relative to the package root) in `scorm_packages`. At play time
//! the player loads the launch URL in an iframe and the JS runtime bridge
//! (`window.API` / `window.API_1484_11`) reads/writes CMI data through
//! `GET/PUT /v1/scorm/:id/cmi`, persisted here per `(package, user)` as a JSON
//! blob in `scorm_cmi`.
//!
//! Both tables are TENANT-SCOPED under RLS exactly like `db::announcements`
//! (policy keys solely on `app.tenant_id`). Uses ONLY runtime sqlx.
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

/// One registered SCORM package, scoped to `(tenant_id, course_id)`.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct ScormPackageRow {
    pub id: Uuid,
    pub course_id: Uuid,
    pub title: String,
    /// The `file_assets.id` of the uploaded + extracted `.zip`.
    pub asset_id: Uuid,
    /// SCORM version we detected from the manifest: "1.2" or "2004".
    pub scorm_version: String,
    /// Launch href from `imsmanifest.xml`, relative to the package root (e.g.
    /// "index.html" or "shared/launchpage.html?course=1"). The player resolves
    /// this against the extracted-package base URL.
    pub launch_href: String,
    pub created_by: Uuid,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

const SELECT_COLS: &str = "id, course_id, title, asset_id, scorm_version, \
     launch_href, created_by, created_at, updated_at";

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

/// Register a parsed SCORM package against a course. Tenant-scoped under RLS.
#[allow(clippy::too_many_arguments)]
pub async fn insert(
    pool: &PgPool,
    tenant_id: Uuid,
    course_id: Uuid,
    title: &str,
    asset_id: Uuid,
    scorm_version: &str,
    launch_href: &str,
    created_by: Uuid,
) -> sqlx::Result<ScormPackageRow> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    let row = sqlx::query_as::<_, ScormPackageRow>(sqlx::AssertSqlSafe(format!(
        "INSERT INTO scorm_packages
             (tenant_id, course_id, title, asset_id, scorm_version, launch_href, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         RETURNING {SELECT_COLS}"
    )))
    .bind(tenant_id)
    .bind(course_id)
    .bind(title)
    .bind(asset_id)
    .bind(scorm_version)
    .bind(launch_href)
    .bind(created_by)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(row)
}

/// List a course's registered SCORM packages, newest first. Tenant-scoped.
pub async fn list_for_course(
    pool: &PgPool,
    tenant_id: Uuid,
    course_id: Uuid,
) -> sqlx::Result<Vec<ScormPackageRow>> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    let rows = sqlx::query_as::<_, ScormPackageRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {SELECT_COLS} FROM scorm_packages
          WHERE course_id = $1
          ORDER BY created_at DESC, id DESC"
    )))
    .bind(course_id)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(rows)
}

/// Fetch a single package by id within `tenant_id`. Tenant-scoped under RLS.
/// Returns None when it does not exist (or is in another tenant).
pub async fn fetch(
    pool: &PgPool,
    tenant_id: Uuid,
    id: Uuid,
) -> sqlx::Result<Option<ScormPackageRow>> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    let row = sqlx::query_as::<_, ScormPackageRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {SELECT_COLS} FROM scorm_packages WHERE id = $1"
    )))
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(row)
}

/// Delete a package by id within `tenant_id`. Returns true if removed.
/// Tenant-scoped. Also cascades the related `scorm_cmi` rows (FK ON DELETE CASCADE).
pub async fn delete(pool: &PgPool, tenant_id: Uuid, id: Uuid) -> sqlx::Result<bool> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    let res = sqlx::query("DELETE FROM scorm_packages WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(res.rows_affected() > 0)
}

/// Load the caller's CMI JSON blob for a package, or `None` if the learner has
/// never committed any state. Tenant-scoped under RLS.
pub async fn load_cmi(
    pool: &PgPool,
    tenant_id: Uuid,
    package_id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<Option<serde_json::Value>> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    let blob: Option<serde_json::Value> =
        sqlx::query_scalar("SELECT cmi FROM scorm_cmi WHERE package_id = $1 AND user_id = $2")
            .bind(package_id)
            .bind(user_id)
            .fetch_optional(&mut *tx)
            .await?;
    tx.commit().await?;
    Ok(blob)
}

/// Upsert the caller's CMI JSON blob for a package (LMSCommit / LMSFinish).
/// Tenant-scoped under RLS. The unique `(package_id, user_id)` constraint backs
/// the ON CONFLICT.
pub async fn save_cmi(
    pool: &PgPool,
    tenant_id: Uuid,
    package_id: Uuid,
    user_id: Uuid,
    cmi: &serde_json::Value,
) -> sqlx::Result<()> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    sqlx::query(
        "INSERT INTO scorm_cmi (tenant_id, package_id, user_id, cmi, updated_at)
         VALUES ($1, $2, $3, $4, now())
         ON CONFLICT (package_id, user_id)
         DO UPDATE SET cmi = EXCLUDED.cmi, updated_at = now()",
    )
    .bind(tenant_id)
    .bind(package_id)
    .bind(user_id)
    .bind(cmi)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}
