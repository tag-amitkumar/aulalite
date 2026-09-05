// crates/backend/src/db/submissions.rs
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::types::BigDecimal;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

#[derive(Debug, Serialize, sqlx::FromRow, Clone)]
pub struct SubmissionRow {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub assignment_id: Uuid,
    pub course_id: Uuid,
    pub student_user_id: Uuid,
    pub status: String,
    pub text_answer: Option<String>,
    pub attachment_asset_ids: Vec<Uuid>,
    pub submitted_at: Option<DateTime<Utc>>,
    pub is_late: bool,
    /// Which attempt this submission represents. Starts at `1`; each
    /// `return_for_resubmit` increments it (capped by the assignment's
    /// `max_resubmissions` at the handler layer).
    pub attempt_number: i32,
    /// The late-penalty percentage actually deducted at grade-release time.
    /// `NULL` until graded; `Some(0)` when no penalty applied (on time, or the
    /// assignment carries no penalty); `Some(p)` (p > 0) when a penalty hit.
    pub applied_late_penalty_percent: Option<i32>,
    pub numeric_grade: Option<BigDecimal>,
    pub letter_grade: Option<String>,
    pub passed: Option<bool>,
    pub student_visible_feedback: Option<String>,
    pub teacher_only_notes: Option<String>,
    pub graded_by_user_id: Option<Uuid>,
    pub graded_at: Option<DateTime<Utc>>,
    pub released_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Column list with the `status` enum coerced to TEXT so sqlx can
/// decode it into Rust `String`. Use anywhere we'd otherwise write `*`.
const SUBMISSION_COLS: &str = "id, tenant_id, assignment_id, course_id, student_user_id, \
     status::text AS status, text_answer, attachment_asset_ids, submitted_at, is_late, \
     attempt_number, applied_late_penalty_percent, \
     numeric_grade, letter_grade, passed, student_visible_feedback, teacher_only_notes, \
     graded_by_user_id, graded_at, released_at, created_at, updated_at";

pub async fn upsert_for_student(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    assignment_id: Uuid,
    course_id: Uuid,
    student_user_id: Uuid,
) -> sqlx::Result<SubmissionRow> {
    let sql = format!(
        "WITH ins AS (
             INSERT INTO submissions
                (tenant_id, assignment_id, course_id, student_user_id)
             VALUES ($1,$2,$3,$4)
             ON CONFLICT (assignment_id, student_user_id)
                DO UPDATE SET updated_at = submissions.updated_at
             RETURNING *
         )
         SELECT {SUBMISSION_COLS} FROM ins"
    );
    sqlx::query_as::<_, SubmissionRow>(sqlx::AssertSqlSafe(sql.as_str()))
        .bind(tenant_id)
        .bind(assignment_id)
        .bind(course_id)
        .bind(student_user_id)
        .fetch_one(&mut **tx)
        .await
}

pub async fn fetch_by_id(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
) -> sqlx::Result<Option<SubmissionRow>> {
    let sql = format!("SELECT {SUBMISSION_COLS} FROM submissions WHERE id=$1");
    sqlx::query_as::<_, SubmissionRow>(sqlx::AssertSqlSafe(sql.as_str()))
        .bind(id)
        .fetch_optional(&mut **tx)
        .await
}

pub async fn list_for_assignment(
    tx: &mut Transaction<'_, Postgres>,
    assignment_id: Uuid,
) -> sqlx::Result<Vec<SubmissionRow>> {
    let sql = format!(
        "SELECT {SUBMISSION_COLS} FROM submissions WHERE assignment_id=$1
         ORDER BY submitted_at NULLS LAST, created_at"
    );
    sqlx::query_as::<_, SubmissionRow>(sqlx::AssertSqlSafe(sql.as_str()))
        .bind(assignment_id)
        .fetch_all(&mut **tx)
        .await
}

pub async fn patch_draft_fields(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    text_answer: Option<Option<&str>>,
    attachment_asset_ids: Option<&[Uuid]>,
) -> sqlx::Result<SubmissionRow> {
    let sql = format!(
        "WITH upd AS (
             UPDATE submissions SET
                text_answer = CASE WHEN $2::bool THEN $3 ELSE text_answer END,
                attachment_asset_ids = COALESCE($4, attachment_asset_ids),
                updated_at = now()
             WHERE id = $1
             RETURNING *
         )
         SELECT {SUBMISSION_COLS} FROM upd"
    );
    sqlx::query_as::<_, SubmissionRow>(sqlx::AssertSqlSafe(sql.as_str()))
        .bind(id)
        .bind(text_answer.is_some())
        .bind(text_answer.flatten())
        .bind(attachment_asset_ids)
        .fetch_one(&mut **tx)
        .await
}

pub async fn mark_submitted(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    is_late: bool,
) -> sqlx::Result<SubmissionRow> {
    let sql = format!(
        "WITH upd AS (
             UPDATE submissions SET status='submitted', submitted_at=now(),
                  is_late=$2, updated_at=now()
             WHERE id=$1 AND status IN ('draft','returned')
             RETURNING *
         )
         SELECT {SUBMISSION_COLS} FROM upd"
    );
    sqlx::query_as::<_, SubmissionRow>(sqlx::AssertSqlSafe(sql.as_str()))
        .bind(id)
        .bind(is_late)
        .fetch_one(&mut **tx)
        .await
}

pub struct GradeFields<'a> {
    /// Effective numeric grade AFTER any late penalty has been applied by the
    /// handler. `None` for pass/fail.
    pub numeric_grade: Option<BigDecimal>,
    pub letter_grade: Option<&'a str>,
    pub passed: Option<bool>,
    pub student_visible_feedback: Option<&'a str>,
    pub teacher_only_notes: Option<&'a str>,
    pub grader_id: Uuid,
    pub release_now: bool,
    /// Late-penalty percentage actually applied to `numeric_grade`. `0` when no
    /// penalty hit (on time or no penalty configured / pass-fail).
    pub applied_late_penalty_percent: i32,
}

pub async fn save_grade(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    g: GradeFields<'_>,
) -> sqlx::Result<SubmissionRow> {
    let sql = format!(
        "WITH upd AS (
             UPDATE submissions SET
                status = 'graded',
                numeric_grade = $2,
                letter_grade = $3,
                passed = $4,
                student_visible_feedback = $5,
                teacher_only_notes = $6,
                graded_by_user_id = $7,
                graded_at = now(),
                released_at = CASE WHEN $8::bool THEN now() ELSE NULL END,
                applied_late_penalty_percent = $9,
                updated_at = now()
             WHERE id = $1
             RETURNING *
         )
         SELECT {SUBMISSION_COLS} FROM upd"
    );
    sqlx::query_as::<_, SubmissionRow>(sqlx::AssertSqlSafe(sql.as_str()))
        .bind(id)
        .bind(g.numeric_grade)
        .bind(g.letter_grade)
        .bind(g.passed)
        .bind(g.student_visible_feedback)
        .bind(g.teacher_only_notes)
        .bind(g.grader_id)
        .bind(g.release_now)
        .bind(g.applied_late_penalty_percent)
        .fetch_one(&mut **tx)
        .await
}

pub async fn mark_released(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
) -> sqlx::Result<SubmissionRow> {
    let sql = format!(
        "WITH upd AS (
             UPDATE submissions SET released_at=now(), updated_at=now()
             WHERE id=$1 AND status='graded' AND released_at IS NULL
             RETURNING *
         )
         SELECT {SUBMISSION_COLS} FROM upd"
    );
    sqlx::query_as::<_, SubmissionRow>(sqlx::AssertSqlSafe(sql.as_str()))
        .bind(id)
        .fetch_one(&mut **tx)
        .await
}

/// Return a submission to the student for resubmission. Bumps
/// `attempt_number` so the handler-level `max_resubmissions` cap can be
/// enforced, and clears the released grade. The handler must have already
/// validated that another attempt is allowed.
pub async fn return_for_resubmit(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
) -> sqlx::Result<SubmissionRow> {
    let sql = format!(
        "WITH upd AS (
             UPDATE submissions SET
                status='returned',
                attempt_number = attempt_number + 1,
                released_at=NULL,
                updated_at=now()
             WHERE id=$1 AND status IN ('submitted','graded')
             RETURNING *
         )
         SELECT {SUBMISSION_COLS} FROM upd"
    );
    sqlx::query_as::<_, SubmissionRow>(sqlx::AssertSqlSafe(sql.as_str()))
        .bind(id)
        .fetch_one(&mut **tx)
        .await
}
