// crates/backend/src/db/notes.rs
//! Student personal notes + lesson bookmarks data layer.
//!
//! Two tables, both tenant-scoped under RLS and further scoped to the CALLER:
//!
//!  * `student_notes` — one row per `(user_id, lesson_id)`; the caller's own
//!    free-text note on a lesson. Upserted via PUT, read via GET.
//!  * `lesson_bookmarks` — one row per `(user_id, lesson_id)`; a simple
//!    "saved this lesson" flag the caller toggles.
//!
//! These are PRIVATE to the owner: every query here additionally filters on
//! `user_id = $caller`, so even another user in the same tenant cannot read or
//! mutate them. Tenant isolation is enforced by the `tenant_id` predicate AND
//! the `tenant_isolation` RLS policy (keyed on `app.tenant_id`), mirroring
//! `db::announcements`. The handler resolves the lesson's `course_id` and gates
//! on `db::courses::caller_can_read_course` before any write so notes/bookmarks
//! can only attach to lessons the caller may actually see.
//!
//! Uses ONLY runtime sqlx (no compile-time macros — no DATABASE_URL at build).
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

/// The caller's own note on a lesson.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct NoteRow {
    pub lesson_id: Uuid,
    pub body: String,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// One bookmarked lesson, joined to `lessons`/`courses` so the client can build
/// a deep link and show a label without a second round-trip.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct BookmarkRow {
    pub lesson_id: Uuid,
    pub course_id: Uuid,
    pub course_slug: String,
    pub lesson_title: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
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

/// Resolve a lesson's `course_id` within the active tenant. Returns None when
/// the lesson does not exist (or belongs to another tenant). Tenant-scoped under
/// RLS. Used by the handler to authorize note/bookmark access via the course
/// read gate.
pub async fn lesson_course_id(
    pool: &PgPool,
    tenant_id: Uuid,
    lesson_id: Uuid,
) -> sqlx::Result<Option<Uuid>> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    let course_id: Option<Uuid> =
        sqlx::query_scalar("SELECT course_id FROM lessons WHERE id = $1 AND tenant_id = $2")
            .bind(lesson_id)
            .bind(tenant_id)
            .fetch_optional(&mut *tx)
            .await?;
    tx.commit().await?;
    Ok(course_id)
}

// ─── Notes ───────────────────────────────────────────────────────────────────

/// Fetch the caller's own note for `lesson_id`. Returns None when they haven't
/// written one. Tenant-scoped under RLS; filtered to `user_id`.
pub async fn fetch_note(
    pool: &PgPool,
    tenant_id: Uuid,
    user_id: Uuid,
    lesson_id: Uuid,
) -> sqlx::Result<Option<NoteRow>> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    let row = sqlx::query_as::<_, NoteRow>(
        "SELECT lesson_id, body, updated_at
           FROM student_notes
          WHERE tenant_id = $1 AND user_id = $2 AND lesson_id = $3",
    )
    .bind(tenant_id)
    .bind(user_id)
    .bind(lesson_id)
    .fetch_optional(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(row)
}

/// Upsert the caller's note for `lesson_id` (one row per user+lesson) and return
/// the persisted row. An empty body is a valid note (the handler may instead
/// choose to delete; see `delete_note`). Tenant-scoped under RLS.
pub async fn upsert_note(
    pool: &PgPool,
    tenant_id: Uuid,
    user_id: Uuid,
    lesson_id: Uuid,
    body: &str,
) -> sqlx::Result<NoteRow> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    let row = sqlx::query_as::<_, NoteRow>(
        "INSERT INTO student_notes (tenant_id, user_id, lesson_id, body, updated_at)
              VALUES ($1, $2, $3, $4, now())
         ON CONFLICT (user_id, lesson_id)
              DO UPDATE SET body = EXCLUDED.body, updated_at = now()
         RETURNING lesson_id, body, updated_at",
    )
    .bind(tenant_id)
    .bind(user_id)
    .bind(lesson_id)
    .bind(body)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(row)
}

/// Delete the caller's note for `lesson_id`. Used when the PUT body is empty so
/// we don't keep blank rows around. Returns true if a row was removed.
pub async fn delete_note(
    pool: &PgPool,
    tenant_id: Uuid,
    user_id: Uuid,
    lesson_id: Uuid,
) -> sqlx::Result<bool> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    let res = sqlx::query(
        "DELETE FROM student_notes WHERE tenant_id = $1 AND user_id = $2 AND lesson_id = $3",
    )
    .bind(tenant_id)
    .bind(user_id)
    .bind(lesson_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(res.rows_affected() > 0)
}

// ─── Bookmarks ─────────────────────────────────────────────────────────────

/// List the caller's bookmarks, newest first, joined for slug/title. Lessons (or
/// courses) the caller can no longer read are still owned rows; visibility of
/// the deep link is the client's concern. Tenant-scoped under RLS.
pub async fn list_bookmarks(
    pool: &PgPool,
    tenant_id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<Vec<BookmarkRow>> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    let rows = sqlx::query_as::<_, BookmarkRow>(
        "SELECT b.lesson_id,
                l.course_id,
                c.slug AS course_slug,
                l.title AS lesson_title,
                b.created_at
           FROM lesson_bookmarks b
           JOIN lessons l ON l.id = b.lesson_id
           JOIN courses c ON c.id = l.course_id
          WHERE b.tenant_id = $1 AND b.user_id = $2
          ORDER BY b.created_at DESC, b.lesson_id DESC",
    )
    .bind(tenant_id)
    .bind(user_id)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(rows)
}

/// True if the caller has bookmarked `lesson_id`. Tenant-scoped under RLS.
pub async fn is_bookmarked(
    pool: &PgPool,
    tenant_id: Uuid,
    user_id: Uuid,
    lesson_id: Uuid,
) -> sqlx::Result<bool> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (
             SELECT 1 FROM lesson_bookmarks
              WHERE tenant_id = $1 AND user_id = $2 AND lesson_id = $3)",
    )
    .bind(tenant_id)
    .bind(user_id)
    .bind(lesson_id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(exists)
}

/// Add a bookmark for the caller on `lesson_id`. Idempotent (ON CONFLICT DO
/// NOTHING). Tenant-scoped under RLS.
pub async fn add_bookmark(
    pool: &PgPool,
    tenant_id: Uuid,
    user_id: Uuid,
    lesson_id: Uuid,
) -> sqlx::Result<()> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    sqlx::query(
        "INSERT INTO lesson_bookmarks (tenant_id, user_id, lesson_id)
              VALUES ($1, $2, $3)
         ON CONFLICT (user_id, lesson_id) DO NOTHING",
    )
    .bind(tenant_id)
    .bind(user_id)
    .bind(lesson_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Remove the caller's bookmark on `lesson_id`. Idempotent. Returns true if a
/// row was removed. Tenant-scoped under RLS.
pub async fn remove_bookmark(
    pool: &PgPool,
    tenant_id: Uuid,
    user_id: Uuid,
    lesson_id: Uuid,
) -> sqlx::Result<bool> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    let res = sqlx::query(
        "DELETE FROM lesson_bookmarks WHERE tenant_id = $1 AND user_id = $2 AND lesson_id = $3",
    )
    .bind(tenant_id)
    .bind(user_id)
    .bind(lesson_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(res.rows_affected() > 0)
}
