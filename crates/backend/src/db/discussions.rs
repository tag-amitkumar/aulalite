// crates/backend/src/db/discussions.rs
//! Course discussion forums / Q&A data layer.
//!
//! Two tables, both scoped to a `(tenant_id, course_id)`:
//!   * `discussions`        — one row per thread (title + body_md, pinned/locked).
//!   * `discussion_posts`   — replies, optionally nested via `parent_post_id`.
//!
//! Any enrolled course member may open a thread or reply (the handler gates on
//! `db::courses::caller_can_read_course`); pin/lock and delete-any are
//! staff-only (the handler gates on `db::courses::caller_can_staff_course`); an
//! author may always delete their own thread/post.
//!
//! TENANT-SCOPED under RLS: every read/write runs inside a tx with the
//! `app.tenant_id` GUC set, mirroring `db::announcements` and `db::attendance`,
//! so the strict `tenant_isolation` policy applies under the non-bypass
//! `aulalite_app` role (see `migrations/20260517000020_app_role.sql`). Uses ONLY
//! runtime sqlx (no compile-time macros — there is no DATABASE_URL at build).
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

/// One discussion thread, joined with `users` for the author's display
/// name/email, plus a denormalized reply count for the list view.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct DiscussionRow {
    pub id: Uuid,
    pub course_id: Uuid,
    pub author_user_id: Uuid,
    pub author_display_name: Option<String>,
    pub author_email: Option<String>,
    pub title: String,
    pub body_md: String,
    pub pinned: bool,
    pub locked: bool,
    pub reply_count: i64,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// One reply within a thread, joined with `users` for the author label.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct DiscussionPostRow {
    pub id: Uuid,
    pub discussion_id: Uuid,
    pub parent_post_id: Option<Uuid>,
    pub author_user_id: Uuid,
    pub author_display_name: Option<String>,
    pub author_email: Option<String>,
    pub body_md: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

const THREAD_COLS: &str = "d.id,
        d.course_id,
        d.author_user_id,
        u.display_name AS author_display_name,
        u.email::text AS author_email,
        d.title,
        d.body_md,
        d.pinned,
        d.locked,
        (SELECT count(*) FROM discussion_posts p WHERE p.discussion_id = d.id) AS reply_count,
        d.created_at,
        d.updated_at";

const POST_COLS: &str = "p.id,
        p.discussion_id,
        p.parent_post_id,
        p.author_user_id,
        u.display_name AS author_display_name,
        u.email::text AS author_email,
        p.body_md,
        p.created_at";

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

// ---------------------------------------------------------------------------
// Threads
// ---------------------------------------------------------------------------

/// Insert a new thread for `course_id` and return the persisted row (joined with
/// the author's user record). Tenant-scoped under RLS.
pub async fn insert_thread(
    pool: &PgPool,
    tenant_id: Uuid,
    course_id: Uuid,
    author_user_id: Uuid,
    title: &str,
    body_md: &str,
) -> sqlx::Result<DiscussionRow> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    let row = sqlx::query_as::<_, DiscussionRow>(sqlx::AssertSqlSafe(format!(
        "WITH ins AS (
             INSERT INTO discussions
                 (tenant_id, course_id, author_user_id, title, body_md)
             VALUES ($1, $2, $3, $4, $5)
             RETURNING id, course_id, author_user_id, title, body_md,
                       pinned, locked, created_at, updated_at
         )
         SELECT {THREAD_COLS}
           FROM ins d
           LEFT JOIN users u ON u.id = d.author_user_id"
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

/// List a course's threads, pinned-first then newest. Tenant-scoped under RLS;
/// the caller has already been authorized to read the course.
pub async fn list_threads_for_course(
    pool: &PgPool,
    tenant_id: Uuid,
    course_id: Uuid,
) -> sqlx::Result<Vec<DiscussionRow>> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    let rows = sqlx::query_as::<_, DiscussionRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {THREAD_COLS}
           FROM discussions d
           LEFT JOIN users u ON u.id = d.author_user_id
          WHERE d.course_id = $1
          ORDER BY d.pinned DESC, d.created_at DESC, d.id DESC"
    )))
    .bind(course_id)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(rows)
}

/// Fetch a single thread by id (any course in the tenant). Tenant-scoped under
/// RLS. Returns None when it does not exist (or is in another tenant).
pub async fn fetch_thread(
    pool: &PgPool,
    tenant_id: Uuid,
    id: Uuid,
) -> sqlx::Result<Option<DiscussionRow>> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    let row = sqlx::query_as::<_, DiscussionRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {THREAD_COLS}
           FROM discussions d
           LEFT JOIN users u ON u.id = d.author_user_id
          WHERE d.id = $1"
    )))
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(row)
}

/// Delete a thread by id (cascades its posts via FK). Tenant-scoped under RLS.
/// Returns true if a row was removed. Authorization (author-or-staff) is
/// enforced by the handler before this is called.
pub async fn delete_thread(pool: &PgPool, tenant_id: Uuid, id: Uuid) -> sqlx::Result<bool> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    let res = sqlx::query("DELETE FROM discussions WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(res.rows_affected() > 0)
}

/// Update the pinned and/or locked flags on a thread and return the refreshed
/// row. Both are optional; `updated_at` is bumped. Tenant-scoped under RLS.
/// Staff authorization is enforced by the handler. Returns None when the thread
/// does not exist in this tenant.
pub async fn set_thread_flags(
    pool: &PgPool,
    tenant_id: Uuid,
    id: Uuid,
    pinned: Option<bool>,
    locked: Option<bool>,
) -> sqlx::Result<Option<DiscussionRow>> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    // COALESCE keeps the existing value when a flag is left None. Casting the
    // bind to bool keeps the runtime-sqlx type inference happy for NULLs.
    let row = sqlx::query_as::<_, DiscussionRow>(sqlx::AssertSqlSafe(format!(
        "WITH upd AS (
             UPDATE discussions
                SET pinned = COALESCE($2::bool, pinned),
                    locked = COALESCE($3::bool, locked),
                    updated_at = now()
              WHERE id = $1
             RETURNING id, course_id, author_user_id, title, body_md,
                       pinned, locked, created_at, updated_at
         )
         SELECT {THREAD_COLS}
           FROM upd d
           LEFT JOIN users u ON u.id = d.author_user_id"
    )))
    .bind(id)
    .bind(pinned)
    .bind(locked)
    .fetch_optional(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(row)
}

// ---------------------------------------------------------------------------
// Posts (replies)
// ---------------------------------------------------------------------------

/// Insert a reply into `discussion_id` (optionally nested under
/// `parent_post_id`) and return the persisted row joined with its author. Also
/// bumps the parent thread's `updated_at`. Tenant-scoped under RLS.
pub async fn insert_post(
    pool: &PgPool,
    tenant_id: Uuid,
    discussion_id: Uuid,
    parent_post_id: Option<Uuid>,
    author_user_id: Uuid,
    body_md: &str,
) -> sqlx::Result<DiscussionPostRow> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    let row = sqlx::query_as::<_, DiscussionPostRow>(sqlx::AssertSqlSafe(format!(
        "WITH ins AS (
             INSERT INTO discussion_posts
                 (tenant_id, discussion_id, parent_post_id, author_user_id, body_md)
             VALUES ($1, $2, $3, $4, $5)
             RETURNING id, discussion_id, parent_post_id, author_user_id, body_md, created_at
         )
         SELECT {POST_COLS}
           FROM ins p
           LEFT JOIN users u ON u.id = p.author_user_id"
    )))
    .bind(tenant_id)
    .bind(discussion_id)
    .bind(parent_post_id)
    .bind(author_user_id)
    .bind(body_md)
    .fetch_one(&mut *tx)
    .await?;
    // Keep the thread's updated_at in step with new activity (best within the
    // same tx so the list ordering can later key off it if desired).
    sqlx::query("UPDATE discussions SET updated_at = now() WHERE id = $1")
        .bind(discussion_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(row)
}

/// List every post in a thread, oldest-first. The handler builds the nested
/// tree from `parent_post_id`. Tenant-scoped under RLS.
pub async fn list_posts_for_thread(
    pool: &PgPool,
    tenant_id: Uuid,
    discussion_id: Uuid,
) -> sqlx::Result<Vec<DiscussionPostRow>> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    let rows = sqlx::query_as::<_, DiscussionPostRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {POST_COLS}
           FROM discussion_posts p
           LEFT JOIN users u ON u.id = p.author_user_id
          WHERE p.discussion_id = $1
          ORDER BY p.created_at ASC, p.id ASC"
    )))
    .bind(discussion_id)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(rows)
}

/// Fetch a single post by id (used to authorize a delete). Tenant-scoped under
/// RLS. Returns None when it does not exist in this tenant.
pub async fn fetch_post(
    pool: &PgPool,
    tenant_id: Uuid,
    id: Uuid,
) -> sqlx::Result<Option<DiscussionPostRow>> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    let row = sqlx::query_as::<_, DiscussionPostRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {POST_COLS}
           FROM discussion_posts p
           LEFT JOIN users u ON u.id = p.author_user_id
          WHERE p.id = $1"
    )))
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(row)
}

/// Delete a post by id. Child posts cascade via the self-FK. Tenant-scoped
/// under RLS. Returns true if a row was removed. Authorization (author-or-staff)
/// is enforced by the handler before this is called.
pub async fn delete_post(pool: &PgPool, tenant_id: Uuid, id: Uuid) -> sqlx::Result<bool> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    let res = sqlx::query("DELETE FROM discussion_posts WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(res.rows_affected() > 0)
}
