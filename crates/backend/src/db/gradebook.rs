// crates/backend/src/db/gradebook.rs
//! Weighted-gradebook data layer.
//!
//! Two tenant-scoped tables back this feature (see
//! `migrations/20260614000040_grade_categories.sql` and
//! `..41_assignment_category_links.sql`):
//!   * `assignment_categories`        — named weight buckets per course.
//!   * `assignment_category_links`    — at-most-one category per assignment.
//!
//! The LINK approach means we never touch the `assignments` table, so this
//! lands without colliding with concurrent assignment work. The gradebook
//! matrix is assembled here from our own SELECTs against `course_memberships`,
//! `assignments`, `submissions`, and the two new tables — we deliberately do
//! NOT reach into `db::submissions` / `db::assignments` (their reads run on a
//! `&mut Transaction`; ours run on the pool inside a tenant-GUC tx).
//!
//! Every read/write runs inside a tx with the `app.tenant_id` GUC set so the
//! strict `tenant_isolation` RLS policy applies under the non-bypass
//! `aulalite_app` role, mirroring `db::announcements` / `db::attendance`. Uses
//! ONLY runtime sqlx (no compile-time macros — there is no DATABASE_URL at
//! build time).
use serde::Serialize;
use sqlx::types::BigDecimal;
use sqlx::PgPool;
use uuid::Uuid;

/// One grade category (weight bucket) for a course.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct CategoryRow {
    pub id: Uuid,
    pub course_id: Uuid,
    pub name: String,
    pub weight_percent: i32,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// One published assignment column in the gradebook matrix, with its linked
/// category id (NULL when uncategorized).
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct GradebookAssignmentRow {
    pub id: Uuid,
    pub title: String,
    pub max_points: Option<i32>,
    pub category_id: Option<Uuid>,
}

/// One enrolled student row (the matrix rows).
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct GradebookStudentRow {
    pub user_id: Uuid,
    pub display_name: Option<String>,
    pub email: Option<String>,
}

/// One graded cell: a released numeric grade for (student, assignment). Only
/// rows with a non-NULL `numeric_grade` AND a non-NULL `released_at` are
/// returned — pass/fail and ungraded/unreleased submissions are simply absent
/// from the matrix, and the weighted total treats them as not-counted.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct GradebookGradeRow {
    pub assignment_id: Uuid,
    pub student_user_id: Uuid,
    pub numeric_grade: BigDecimal,
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

// ---------------------------------------------------------------------------
// Categories
// ---------------------------------------------------------------------------

const CATEGORY_COLS: &str = "id, course_id, name, weight_percent, created_at, updated_at";

/// List a course's grade categories, alphabetical by name then id (stable).
pub async fn list_categories(
    pool: &PgPool,
    tenant_id: Uuid,
    course_id: Uuid,
) -> sqlx::Result<Vec<CategoryRow>> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    let rows = sqlx::query_as::<_, CategoryRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {CATEGORY_COLS} FROM assignment_categories
          WHERE course_id = $1
          ORDER BY lower(name) ASC, id ASC"
    )))
    .bind(course_id)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(rows)
}

/// Insert a new grade category and return the persisted row. Tenant-scoped.
pub async fn insert_category(
    pool: &PgPool,
    tenant_id: Uuid,
    course_id: Uuid,
    name: &str,
    weight_percent: i32,
) -> sqlx::Result<CategoryRow> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    let row = sqlx::query_as::<_, CategoryRow>(sqlx::AssertSqlSafe(format!(
        "WITH ins AS (
             INSERT INTO assignment_categories (tenant_id, course_id, name, weight_percent)
             VALUES ($1, $2, $3, $4)
             RETURNING {CATEGORY_COLS}
         )
         SELECT {CATEGORY_COLS} FROM ins"
    )))
    .bind(tenant_id)
    .bind(course_id)
    .bind(name)
    .bind(weight_percent)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(row)
}

/// Delete a category by id within `course_id`. Returns true if a row was
/// removed. The link FK is ON DELETE CASCADE, so removing a category cleanly
/// unlinks any assignments it held. Tenant-scoped.
pub async fn delete_category(
    pool: &PgPool,
    tenant_id: Uuid,
    course_id: Uuid,
    id: Uuid,
) -> sqlx::Result<bool> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    let res = sqlx::query("DELETE FROM assignment_categories WHERE id = $1 AND course_id = $2")
        .bind(id)
        .bind(course_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(res.rows_affected() > 0)
}

/// Confirm a category exists within `course_id` (and this tenant). Used by the
/// "assign assignment to category" path to reject a bogus category id with a
/// 404 before writing the link.
pub async fn category_exists_in_course(
    pool: &PgPool,
    tenant_id: Uuid,
    course_id: Uuid,
    category_id: Uuid,
) -> sqlx::Result<bool> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM assignment_categories WHERE id = $1 AND course_id = $2",
    )
    .bind(category_id)
    .bind(course_id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(count > 0)
}

// ---------------------------------------------------------------------------
// Assignment ↔ category link
// ---------------------------------------------------------------------------

/// Confirm an assignment belongs to `course_id` in this tenant. Used to reject
/// a cross-course/bogus assignment id before linking. Reads `assignments`
/// directly (we never mutate it).
pub async fn assignment_in_course(
    pool: &PgPool,
    tenant_id: Uuid,
    course_id: Uuid,
    assignment_id: Uuid,
) -> sqlx::Result<bool> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM assignments WHERE id = $1 AND course_id = $2")
            .bind(assignment_id)
            .bind(course_id)
            .fetch_one(&mut *tx)
            .await?;
    tx.commit().await?;
    Ok(count > 0)
}

/// Link `assignment_id` to `category_id` (UPSERT — at most one category per
/// assignment). `category_id == None` clears any existing link. Tenant-scoped.
/// The caller has already validated that both ids belong to `course_id`.
pub async fn set_assignment_category(
    pool: &PgPool,
    tenant_id: Uuid,
    course_id: Uuid,
    assignment_id: Uuid,
    category_id: Option<Uuid>,
) -> sqlx::Result<()> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    match category_id {
        None => {
            sqlx::query("DELETE FROM assignment_category_links WHERE assignment_id = $1")
                .bind(assignment_id)
                .execute(&mut *tx)
                .await?;
        }
        Some(cat) => {
            sqlx::query(
                "INSERT INTO assignment_category_links
                    (assignment_id, category_id, tenant_id, course_id)
                 VALUES ($1, $2, $3, $4)
                 ON CONFLICT (assignment_id) DO UPDATE
                    SET category_id = EXCLUDED.category_id",
            )
            .bind(assignment_id)
            .bind(cat)
            .bind(tenant_id)
            .bind(course_id)
            .execute(&mut *tx)
            .await?;
        }
    }
    tx.commit().await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Matrix assembly
// ---------------------------------------------------------------------------

/// The raw pieces of a course gradebook matrix, fetched in one tenant-GUC tx so
/// they're internally consistent. The handler turns these into the JSON matrix
/// and computes the weighted totals (kept out of SQL so the math is pure and
/// unit-testable).
pub struct GradebookData {
    pub students: Vec<GradebookStudentRow>,
    pub assignments: Vec<GradebookAssignmentRow>,
    pub grades: Vec<GradebookGradeRow>,
    pub categories: Vec<CategoryRow>,
}

/// Assemble everything the gradebook needs for `course_id`:
///   * enrolled active students (matrix rows),
///   * published assignments with their linked category (matrix columns),
///   * released numeric grades (matrix cells),
///   * grade categories (for weighting + the category manager UI).
///
/// All four reads share one tx with the tenant GUC set so RLS applies and the
/// snapshot is coherent. Caller has already authorized staff access.
pub async fn load_gradebook(
    pool: &PgPool,
    tenant_id: Uuid,
    course_id: Uuid,
) -> sqlx::Result<GradebookData> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;

    let students = sqlx::query_as::<_, GradebookStudentRow>(
        "SELECT cm.user_id,
                u.display_name,
                u.email::text AS email
           FROM course_memberships cm
           LEFT JOIN users u ON u.id = cm.user_id
          WHERE cm.course_id = $1
            AND cm.tenant_id = $2
            AND cm.status = 'active'
            AND cm.role = 'student'
          ORDER BY lower(coalesce(u.display_name, u.email::text, cm.user_id::text)) ASC,
                   cm.user_id ASC",
    )
    .bind(course_id)
    .bind(tenant_id)
    .fetch_all(&mut *tx)
    .await?;

    let assignments = sqlx::query_as::<_, GradebookAssignmentRow>(
        "SELECT a.id,
                a.title,
                a.max_points,
                l.category_id
           FROM assignments a
           LEFT JOIN assignment_category_links l ON l.assignment_id = a.id
          WHERE a.course_id = $1
            AND a.status = 'published'
          ORDER BY COALESCE(a.due_at, 'infinity'), a.created_at, a.id",
    )
    .bind(course_id)
    .fetch_all(&mut *tx)
    .await?;

    // Only released, numerically-graded cells. We coerce away pass/fail and
    // ungraded/unreleased submissions in SQL so the handler matrix is sparse.
    let grades = sqlx::query_as::<_, GradebookGradeRow>(
        "SELECT s.assignment_id,
                s.student_user_id,
                s.numeric_grade
           FROM submissions s
          WHERE s.course_id = $1
            AND s.numeric_grade IS NOT NULL
            AND s.released_at IS NOT NULL",
    )
    .bind(course_id)
    .fetch_all(&mut *tx)
    .await?;

    let categories = sqlx::query_as::<_, CategoryRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {CATEGORY_COLS} FROM assignment_categories
          WHERE course_id = $1
          ORDER BY lower(name) ASC, id ASC"
    )))
    .bind(course_id)
    .fetch_all(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(GradebookData {
        students,
        assignments,
        grades,
        categories,
    })
}
