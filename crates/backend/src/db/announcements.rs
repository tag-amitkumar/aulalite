// crates/backend/src/db/announcements.rs
//! Course announcements data layer.
//!
//! One row per announcement, scoped to a `(tenant_id, course_id)`. Authored by
//! course staff (the handler gates on `db::courses::caller_can_staff_course`);
//! readable by anyone who can read the course (the handler gates that, too).
//!
//! TENANT-SCOPED under RLS: every read/write runs inside a tx with the
//! `app.tenant_id` GUC set, mirroring `db::attendance` and `db::notifications`,
//! so the strict `tenant_isolation` policy applies under the non-bypass
//! `aulalite_app` role (see `migrations/20260517000020_app_role.sql`). Uses ONLY
//! runtime sqlx (no compile-time macros — there is no DATABASE_URL at build).
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

/// One announcement, joined with `users` for the author's display name/email.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct AnnouncementRow {
    pub id: Uuid,
    pub course_id: Uuid,
    pub author_user_id: Uuid,
    pub author_display_name: Option<String>,
    pub author_email: Option<String>,
    pub title: String,
    pub body_md: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

const SELECT_COLS: &str = "a.id,
        a.course_id,
        a.author_user_id,
        u.display_name AS author_display_name,
        u.email::text AS author_email,
        a.title,
        a.body_md,
        a.created_at,
        a.updated_at";

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

/// Insert a new announcement for `course_id` and return the persisted row
/// (joined with the author's user record). Tenant-scoped under RLS.
pub async fn insert(
    pool: &PgPool,
    tenant_id: Uuid,
    course_id: Uuid,
    author_user_id: Uuid,
    title: &str,
    body_md: &str,
) -> sqlx::Result<AnnouncementRow> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    let row = sqlx::query_as::<_, AnnouncementRow>(sqlx::AssertSqlSafe(format!(
        "WITH ins AS (
             INSERT INTO announcements
                 (tenant_id, course_id, author_user_id, title, body_md)
             VALUES ($1, $2, $3, $4, $5)
             RETURNING id, course_id, author_user_id, title, body_md, created_at, updated_at
         )
         SELECT {SELECT_COLS}
           FROM ins a
           LEFT JOIN users u ON u.id = a.author_user_id"
    )))
    .bind(tenant_id)
    .bind(course_id)
    .bind(author_user_id)
    .bind(title)
    .bind(body_md)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(row)
}

/// List a course's announcements, newest first. Tenant-scoped under RLS; the
/// caller has already been authorized to read the course.
pub async fn list_for_course(
    pool: &PgPool,
    tenant_id: Uuid,
    course_id: Uuid,
) -> sqlx::Result<Vec<AnnouncementRow>> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    let rows = sqlx::query_as::<_, AnnouncementRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {SELECT_COLS}
           FROM announcements a
           LEFT JOIN users u ON u.id = a.author_user_id
          WHERE a.course_id = $1
          ORDER BY a.created_at DESC, a.id DESC"
    )))
    .bind(course_id)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(rows)
}

/// Fetch a single announcement by id within `course_id`. Tenant-scoped under
/// RLS. Returns None when it does not exist (or is in another course/tenant).
pub async fn fetch_in_course(
    pool: &PgPool,
    tenant_id: Uuid,
    course_id: Uuid,
    id: Uuid,
) -> sqlx::Result<Option<AnnouncementRow>> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    let row = sqlx::query_as::<_, AnnouncementRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {SELECT_COLS}
           FROM announcements a
           LEFT JOIN users u ON u.id = a.author_user_id
          WHERE a.id = $1 AND a.course_id = $2"
    )))
    .bind(id)
    .bind(course_id)
    .fetch_optional(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(row)
}

/// Delete an announcement by id within `course_id`. Tenant-scoped under RLS.
/// Returns true if a row was removed. Authorization (author-or-staff) is
/// enforced by the handler before this is called.
pub async fn delete_in_course(
    pool: &PgPool,
    tenant_id: Uuid,
    course_id: Uuid,
    id: Uuid,
) -> sqlx::Result<bool> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    let res = sqlx::query("DELETE FROM announcements WHERE id = $1 AND course_id = $2")
        .bind(id)
        .bind(course_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(res.rows_affected() > 0)
}

/// List the user_ids of every ACTIVE student member of `course_id` in
/// `tenant_id`. Used to fan out the "new announcement" notification to enrolled
/// students. Tenant-scoped under RLS.
pub async fn list_enrolled_student_ids(
    pool: &PgPool,
    tenant_id: Uuid,
    course_id: Uuid,
) -> sqlx::Result<Vec<Uuid>> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    let ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT user_id FROM course_memberships
          WHERE course_id = $1
            AND tenant_id = $2
            AND status = 'active'
            AND role = 'student'",
    )
    .bind(course_id)
    .bind(tenant_id)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(ids)
}
