// crates/backend/src/db/assignments.rs
use serde::Serialize;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

#[derive(Debug, Serialize, sqlx::FromRow, Clone)]
pub struct AssignmentRow {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub course_id: Uuid,
    pub lesson_id: Option<Uuid>,
    pub title: String,
    pub instructions_md: String,
    pub grading_mode: String,
    pub max_points: Option<i32>,
    pub allow_late: bool,
    pub lock_on_submit: bool,
    pub accepts_text: bool,
    pub accepts_files: bool,
    pub release_mode: String,
    /// Percentage (0..=100) deducted from a numeric grade when the submission
    /// was turned in after `due_at`. `0` (default) disables the penalty.
    pub late_penalty_percent: i32,
    /// How many times a student may resubmit after a `returned` action.
    /// `0` (default) means no resubmission is allowed beyond the first submit.
    pub max_resubmissions: i32,
    pub attachment_asset_ids: Vec<Uuid>,
    pub due_at: Option<chrono::DateTime<chrono::Utc>>,
    pub status: String,
    pub published_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_by: Uuid,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Column list with custom-enum columns coerced to TEXT so sqlx can
/// decode them into Rust `String`. Use anywhere we'd otherwise write `*`.
const ASSIGNMENT_COLS: &str = "id, tenant_id, course_id, lesson_id, title, instructions_md, \
     grading_mode::text AS grading_mode, max_points, allow_late, lock_on_submit, \
     accepts_text, accepts_files, release_mode::text AS release_mode, \
     late_penalty_percent, max_resubmissions, \
     attachment_asset_ids, due_at, status::text AS status, published_at, \
     created_by, created_at, updated_at";

pub struct InsertAssignment<'a> {
    pub tenant_id: Uuid,
    pub course_id: Uuid,
    pub lesson_id: Option<Uuid>,
    pub title: &'a str,
    pub instructions_md: &'a str,
    pub grading_mode: &'a str,
    pub max_points: Option<i32>,
    pub allow_late: bool,
    pub lock_on_submit: bool,
    pub accepts_text: bool,
    pub accepts_files: bool,
    pub release_mode: &'a str,
    pub late_penalty_percent: i32,
    pub max_resubmissions: i32,
    pub due_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_by: Uuid,
}

pub async fn insert(
    tx: &mut Transaction<'_, Postgres>,
    a: InsertAssignment<'_>,
) -> sqlx::Result<AssignmentRow> {
    let sql = format!(
        "WITH ins AS (
             INSERT INTO assignments
                (tenant_id, course_id, lesson_id, title, instructions_md,
                 grading_mode, max_points, allow_late, lock_on_submit,
                 accepts_text, accepts_files, release_mode, due_at, created_by,
                 late_penalty_percent, max_resubmissions)
             VALUES ($1,$2,$3,$4,$5,$6::assignment_grading_mode,$7,$8,$9,$10,$11,
                     $12::assignment_release_mode,$13,$14,$15,$16)
             RETURNING *
         )
         SELECT {ASSIGNMENT_COLS} FROM ins"
    );
    sqlx::query_as::<_, AssignmentRow>(sqlx::AssertSqlSafe(sql.as_str()))
        .bind(a.tenant_id)
        .bind(a.course_id)
        .bind(a.lesson_id)
        .bind(a.title)
        .bind(a.instructions_md)
        .bind(a.grading_mode)
        .bind(a.max_points)
        .bind(a.allow_late)
        .bind(a.lock_on_submit)
        .bind(a.accepts_text)
        .bind(a.accepts_files)
        .bind(a.release_mode)
        .bind(a.due_at)
        .bind(a.created_by)
        .bind(a.late_penalty_percent)
        .bind(a.max_resubmissions)
        .fetch_one(&mut **tx)
        .await
}

pub async fn fetch_by_id(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
) -> sqlx::Result<Option<AssignmentRow>> {
    let sql = format!("SELECT {ASSIGNMENT_COLS} FROM assignments WHERE id = $1");
    sqlx::query_as::<_, AssignmentRow>(sqlx::AssertSqlSafe(sql.as_str()))
        .bind(id)
        .fetch_optional(&mut **tx)
        .await
}

pub async fn list_by_course(
    tx: &mut Transaction<'_, Postgres>,
    course_id: Uuid,
    include_drafts: bool,
) -> sqlx::Result<Vec<AssignmentRow>> {
    let sql = if include_drafts {
        format!(
            "SELECT {ASSIGNMENT_COLS} FROM assignments WHERE course_id = $1
             ORDER BY COALESCE(due_at, 'infinity'), created_at"
        )
    } else {
        format!(
            "SELECT {ASSIGNMENT_COLS} FROM assignments WHERE course_id = $1 AND status = 'published'
             ORDER BY COALESCE(due_at, 'infinity'), created_at"
        )
    };
    sqlx::query_as::<_, AssignmentRow>(sqlx::AssertSqlSafe(sql.as_str()))
        .bind(course_id)
        .fetch_all(&mut **tx)
        .await
}

pub async fn list_by_lesson(
    tx: &mut Transaction<'_, Postgres>,
    lesson_id: Uuid,
    include_drafts: bool,
) -> sqlx::Result<Vec<AssignmentRow>> {
    let sql = if include_drafts {
        format!(
            "SELECT {ASSIGNMENT_COLS} FROM assignments WHERE lesson_id = $1
             ORDER BY COALESCE(due_at, 'infinity'), created_at"
        )
    } else {
        format!(
            "SELECT {ASSIGNMENT_COLS} FROM assignments WHERE lesson_id = $1 AND status = 'published'
             ORDER BY COALESCE(due_at, 'infinity'), created_at"
        )
    };
    sqlx::query_as::<_, AssignmentRow>(sqlx::AssertSqlSafe(sql.as_str()))
        .bind(lesson_id)
        .fetch_all(&mut **tx)
        .await
}

pub struct PatchAssignment<'a> {
    pub title: Option<&'a str>,
    pub instructions_md: Option<&'a str>,
    pub grading_mode: Option<&'a str>,
    pub max_points: Option<Option<i32>>,
    pub allow_late: Option<bool>,
    pub lock_on_submit: Option<bool>,
    pub accepts_text: Option<bool>,
    pub accepts_files: Option<bool>,
    pub release_mode: Option<&'a str>,
    pub late_penalty_percent: Option<i32>,
    pub max_resubmissions: Option<i32>,
    pub attachment_asset_ids: Option<&'a [Uuid]>,
    pub due_at: Option<Option<chrono::DateTime<chrono::Utc>>>,
    pub lesson_id: Option<Option<Uuid>>,
}

pub async fn patch(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    p: PatchAssignment<'_>,
) -> sqlx::Result<AssignmentRow> {
    let sql = format!(
        "WITH upd AS (
             UPDATE assignments SET
                title = COALESCE($2, title),
                instructions_md = COALESCE($3, instructions_md),
                grading_mode = COALESCE($4::assignment_grading_mode, grading_mode),
                max_points = CASE WHEN $5::bool THEN $6 ELSE max_points END,
                allow_late = COALESCE($7, allow_late),
                lock_on_submit = COALESCE($8, lock_on_submit),
                accepts_text = COALESCE($9, accepts_text),
                accepts_files = COALESCE($10, accepts_files),
                release_mode = COALESCE($11::assignment_release_mode, release_mode),
                attachment_asset_ids = COALESCE($12, attachment_asset_ids),
                due_at = CASE WHEN $13::bool THEN $14 ELSE due_at END,
                lesson_id = CASE WHEN $15::bool THEN $16 ELSE lesson_id END,
                late_penalty_percent = COALESCE($17, late_penalty_percent),
                max_resubmissions = COALESCE($18, max_resubmissions),
                updated_at = now()
             WHERE id = $1
             RETURNING *
         )
         SELECT {ASSIGNMENT_COLS} FROM upd"
    );
    sqlx::query_as::<_, AssignmentRow>(sqlx::AssertSqlSafe(sql.as_str()))
        .bind(id)
        .bind(p.title)
        .bind(p.instructions_md)
        .bind(p.grading_mode)
        .bind(p.max_points.is_some())
        .bind(p.max_points.flatten())
        .bind(p.allow_late)
        .bind(p.lock_on_submit)
        .bind(p.accepts_text)
        .bind(p.accepts_files)
        .bind(p.release_mode)
        .bind(p.attachment_asset_ids)
        .bind(p.due_at.is_some())
        .bind(p.due_at.flatten())
        .bind(p.lesson_id.is_some())
        .bind(p.lesson_id.flatten())
        .bind(p.late_penalty_percent)
        .bind(p.max_resubmissions)
        .fetch_one(&mut **tx)
        .await
}

pub async fn publish(tx: &mut Transaction<'_, Postgres>, id: Uuid) -> sqlx::Result<AssignmentRow> {
    let sql = format!(
        "WITH upd AS (
             UPDATE assignments SET status='published', published_at=now(), updated_at=now()
             WHERE id=$1 AND status='draft' RETURNING *
         )
         SELECT {ASSIGNMENT_COLS} FROM upd"
    );
    sqlx::query_as::<_, AssignmentRow>(sqlx::AssertSqlSafe(sql.as_str()))
        .bind(id)
        .fetch_one(&mut **tx)
        .await
}

pub async fn unpublish(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
) -> sqlx::Result<AssignmentRow> {
    let sql = format!(
        "WITH upd AS (
             UPDATE assignments SET status='draft', published_at=NULL, updated_at=now()
             WHERE id=$1 AND status='published' RETURNING *
         )
         SELECT {ASSIGNMENT_COLS} FROM upd"
    );
    sqlx::query_as::<_, AssignmentRow>(sqlx::AssertSqlSafe(sql.as_str()))
        .bind(id)
        .fetch_one(&mut **tx)
        .await
}

pub async fn delete(tx: &mut Transaction<'_, Postgres>, id: Uuid) -> sqlx::Result<u64> {
    Ok(
        sqlx::query("DELETE FROM assignments WHERE id=$1 AND status='draft'")
            .bind(id)
            .execute(&mut **tx)
            .await?
            .rows_affected(),
    )
}

pub async fn count_submissions(
    tx: &mut Transaction<'_, Postgres>,
    assignment_id: Uuid,
) -> sqlx::Result<i64> {
    sqlx::query_scalar("SELECT COUNT(*) FROM submissions WHERE assignment_id = $1")
        .bind(assignment_id)
        .fetch_one(&mut **tx)
        .await
}
