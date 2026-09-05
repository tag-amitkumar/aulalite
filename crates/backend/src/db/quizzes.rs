//! Quiz queries (learning-suite Cycle 3). Callers run inside a
//! `begin_with_context` transaction so the tenant-isolation RLS applies.

use core_types::quiz::QuizPrompt;
use sqlx::{PgConnection, Postgres, Transaction};
use uuid::Uuid;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct QuizRow {
    pub id: Uuid,
    pub course_id: Uuid,
    pub module_id: Option<Uuid>,
    pub title: String,
    pub description: Option<String>,
    pub mode: String,
    pub time_limit_seconds: Option<i32>,
    pub max_attempts: Option<i32>,
    pub status: String,
    pub sort_order: i32,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

const QUIZ_COLS: &str = "id, course_id, module_id, title, description, mode, \
     time_limit_seconds, max_attempts, status, sort_order, created_at";

#[allow(clippy::too_many_arguments)]
pub async fn create_quiz(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    course_id: Uuid,
    module_id: Option<Uuid>,
    title: &str,
    description: Option<&str>,
    mode: &str,
    time_limit_seconds: Option<i32>,
    max_attempts: Option<i32>,
    created_by: Uuid,
) -> sqlx::Result<QuizRow> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "INSERT INTO quizzes (tenant_id, course_id, module_id, title, description, mode, \
         time_limit_seconds, max_attempts, created_by, sort_order) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, \
                 COALESCE((SELECT max(sort_order) + 10 FROM quizzes WHERE course_id = $2), 10)) \
         RETURNING {QUIZ_COLS}"
    )))
    .bind(tenant_id)
    .bind(course_id)
    .bind(module_id)
    .bind(title)
    .bind(description)
    .bind(mode)
    .bind(time_limit_seconds)
    .bind(max_attempts)
    .bind(created_by)
    .fetch_one(&mut **tx)
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn patch_quiz(
    tx: &mut Transaction<'_, Postgres>,
    quiz_id: Uuid,
    title: Option<&str>,
    description: Option<&str>,
    mode: Option<&str>,
    time_limit_seconds: Option<Option<i32>>,
    max_attempts: Option<Option<i32>>,
    module_id: Option<Option<Uuid>>,
    status: Option<&str>,
) -> sqlx::Result<Option<QuizRow>> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "UPDATE quizzes SET \
            title = COALESCE($2, title), \
            description = COALESCE($3, description), \
            mode = COALESCE($4, mode), \
            time_limit_seconds = CASE WHEN $5 THEN $6 ELSE time_limit_seconds END, \
            max_attempts = CASE WHEN $7 THEN $8 ELSE max_attempts END, \
            module_id = CASE WHEN $9 THEN $10 ELSE module_id END, \
            status = COALESCE($11, status), \
            updated_at = now() \
         WHERE id = $1 RETURNING {QUIZ_COLS}"
    )))
    .bind(quiz_id)
    .bind(title)
    .bind(description)
    .bind(mode)
    .bind(time_limit_seconds.is_some())
    .bind(time_limit_seconds.flatten())
    .bind(max_attempts.is_some())
    .bind(max_attempts.flatten())
    .bind(module_id.is_some())
    .bind(module_id.flatten())
    .bind(status)
    .fetch_optional(&mut **tx)
    .await
}

pub async fn delete_quiz(tx: &mut Transaction<'_, Postgres>, quiz_id: Uuid) -> sqlx::Result<bool> {
    let res = sqlx::query("DELETE FROM quizzes WHERE id = $1")
        .bind(quiz_id)
        .execute(&mut **tx)
        .await?;
    Ok(res.rows_affected() > 0)
}

pub async fn get_quiz(conn: &mut PgConnection, quiz_id: Uuid) -> sqlx::Result<Option<QuizRow>> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {QUIZ_COLS} FROM quizzes WHERE id = $1"
    )))
    .bind(quiz_id)
    .fetch_optional(conn)
    .await
}

/// All quizzes in a course (staff) or only published ones (students).
pub async fn list_quizzes(
    conn: &mut PgConnection,
    course_id: Uuid,
    published_only: bool,
) -> sqlx::Result<Vec<QuizRow>> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {QUIZ_COLS} FROM quizzes WHERE course_id = $1 \
         AND ($2 = false OR status = 'published') \
         ORDER BY module_id NULLS LAST, sort_order, created_at"
    )))
    .bind(course_id)
    .bind(published_only)
    .fetch_all(conn)
    .await
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct QuestionRow {
    pub id: Uuid,
    pub position: i32,
    pub prompt_text: String,
    pub prompt: serde_json::Value,
    pub explanation: String,
    pub points: i32,
}

pub async fn list_questions(
    conn: &mut PgConnection,
    quiz_id: Uuid,
) -> sqlx::Result<Vec<QuestionRow>> {
    sqlx::query_as(
        "SELECT id, position, prompt_text, prompt, explanation, points \
         FROM quiz_questions WHERE quiz_id = $1 ORDER BY position",
    )
    .bind(quiz_id)
    .fetch_all(conn)
    .await
}

/// Replace the full question list (authoring saves the whole set; positions
/// follow array order). Returns the new rows.
pub async fn replace_questions(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    quiz_id: Uuid,
    questions: &[(String, QuizPrompt, String, i32)],
) -> sqlx::Result<Vec<QuestionRow>> {
    sqlx::query("DELETE FROM quiz_questions WHERE quiz_id = $1")
        .bind(quiz_id)
        .execute(&mut **tx)
        .await?;
    let mut rows = Vec::with_capacity(questions.len());
    for (i, (prompt_text, prompt, explanation, points)) in questions.iter().enumerate() {
        let prompt_json =
            serde_json::to_value(prompt).map_err(|e| sqlx::Error::Encode(Box::new(e)))?;
        let row: QuestionRow = sqlx::query_as(
            "INSERT INTO quiz_questions \
             (tenant_id, quiz_id, position, prompt_text, prompt, explanation, points) \
             VALUES ($1, $2, $3, $4, $5, $6, $7) \
             RETURNING id, position, prompt_text, prompt, explanation, points",
        )
        .bind(tenant_id)
        .bind(quiz_id)
        .bind((i as i32 + 1) * 10)
        .bind(prompt_text)
        .bind(&prompt_json)
        .bind(explanation)
        .bind(points)
        .fetch_one(&mut **tx)
        .await?;
        rows.push(row);
    }
    Ok(rows)
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AttemptRow {
    pub id: Uuid,
    pub quiz_id: Uuid,
    pub user_id: Uuid,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub submitted_at: Option<chrono::DateTime<chrono::Utc>>,
    pub score_points: Option<i32>,
    pub max_points: Option<i32>,
}

const ATTEMPT_COLS: &str =
    "id, quiz_id, user_id, started_at, submitted_at, score_points, max_points";

/// The user's open (unsubmitted) attempt on a quiz, if any.
pub async fn open_attempt(
    conn: &mut PgConnection,
    quiz_id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<Option<AttemptRow>> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {ATTEMPT_COLS} FROM quiz_attempts \
         WHERE quiz_id = $1 AND user_id = $2 AND submitted_at IS NULL \
         ORDER BY started_at DESC LIMIT 1"
    )))
    .bind(quiz_id)
    .bind(user_id)
    .fetch_optional(conn)
    .await
}

pub async fn count_submitted_attempts(
    conn: &mut PgConnection,
    quiz_id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<i64> {
    sqlx::query_scalar(
        "SELECT count(*) FROM quiz_attempts \
         WHERE quiz_id = $1 AND user_id = $2 AND submitted_at IS NOT NULL",
    )
    .bind(quiz_id)
    .bind(user_id)
    .fetch_one(conn)
    .await
}

/// Per-quiz list-view stats: question count, the caller's submitted attempt
/// count, and their best score/max. One grouped query replaces the per-quiz
/// N+1 (list_questions + list_my_attempts per quiz) on the course quiz list;
/// it also avoids deserializing answer-key prompts server-side just to count
/// them.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct QuizListStats {
    pub quiz_id: Uuid,
    pub question_count: i64,
    pub my_submitted_attempts: i64,
    pub my_best_score: Option<i32>,
    pub my_best_max: Option<i32>,
}

pub async fn list_stats_for_course(
    conn: &mut PgConnection,
    course_id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<Vec<QuizListStats>> {
    sqlx::query_as(
        "SELECT q.id AS quiz_id,
                COUNT(qq.id) AS question_count,
                COALESCE(a.submitted_count, 0) AS my_submitted_attempts,
                a.best_score AS my_best_score,
                a.best_max AS my_best_max
           FROM quizzes q
           LEFT JOIN quiz_questions qq ON qq.quiz_id = q.id
           LEFT JOIN (
                SELECT quiz_id,
                       COUNT(*) AS submitted_count,
                       MAX(score_points) AS best_score,
                       MAX(max_points) AS best_max
                  FROM quiz_attempts
                 WHERE user_id = $2 AND submitted_at IS NOT NULL
                 GROUP BY quiz_id
           ) a ON a.quiz_id = q.id
          WHERE q.course_id = $1
          GROUP BY q.id, a.submitted_count, a.best_score, a.best_max",
    )
    .bind(course_id)
    .bind(user_id)
    .fetch_all(conn)
    .await
}

pub async fn start_attempt(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    quiz_id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<AttemptRow> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "INSERT INTO quiz_attempts (tenant_id, quiz_id, user_id) \
         VALUES ($1, $2, $3) RETURNING {ATTEMPT_COLS}"
    )))
    .bind(tenant_id)
    .bind(quiz_id)
    .bind(user_id)
    .fetch_one(&mut **tx)
    .await
}

pub async fn get_attempt(
    conn: &mut PgConnection,
    attempt_id: Uuid,
) -> sqlx::Result<Option<AttemptRow>> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {ATTEMPT_COLS} FROM quiz_attempts WHERE id = $1"
    )))
    .bind(attempt_id)
    .fetch_optional(conn)
    .await
}

/// Finalize an attempt: store the score and per-question answers. Returns
/// false when the attempt was already submitted by a concurrent request
/// (the guarded UPDATE affected no rows) so callers can surface a clean
/// conflict instead of dying on the answer-insert primary key.
pub async fn submit_attempt(
    tx: &mut Transaction<'_, Postgres>,
    attempt_id: Uuid,
    score_points: i32,
    max_points: i32,
    answers: &[(Uuid, serde_json::Value, bool, i32)],
) -> sqlx::Result<bool> {
    let res = sqlx::query(
        "UPDATE quiz_attempts SET submitted_at = now(), score_points = $2, max_points = $3 \
         WHERE id = $1 AND submitted_at IS NULL",
    )
    .bind(attempt_id)
    .bind(score_points)
    .bind(max_points)
    .execute(&mut **tx)
    .await?;
    if res.rows_affected() == 0 {
        return Ok(false);
    }
    for (question_id, answer, correct, points_awarded) in answers {
        sqlx::query(
            "INSERT INTO quiz_attempt_answers (attempt_id, question_id, answer, correct, points_awarded) \
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(attempt_id)
        .bind(question_id)
        .bind(answer)
        .bind(correct)
        .bind(points_awarded)
        .execute(&mut **tx)
        .await?;
    }
    Ok(true)
}

/// A user's submitted attempts on one quiz, newest first.
pub async fn list_my_attempts(
    conn: &mut PgConnection,
    quiz_id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<Vec<AttemptRow>> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {ATTEMPT_COLS} FROM quiz_attempts \
         WHERE quiz_id = $1 AND user_id = $2 AND submitted_at IS NOT NULL \
         ORDER BY submitted_at DESC"
    )))
    .bind(quiz_id)
    .bind(user_id)
    .fetch_all(conn)
    .await
}

/// Per-question correctness for a submitted attempt, in question order.
pub async fn attempt_answers(
    conn: &mut PgConnection,
    attempt_id: Uuid,
) -> sqlx::Result<Vec<(Uuid, bool, i32)>> {
    sqlx::query_as(
        "SELECT qaa.question_id, qaa.correct, qaa.points_awarded \
         FROM quiz_attempt_answers qaa \
         JOIN quiz_questions qq ON qq.id = qaa.question_id \
         WHERE qaa.attempt_id = $1 ORDER BY qq.position",
    )
    .bind(attempt_id)
    .fetch_all(conn)
    .await
}

/// Staff rollup: per-student best score on one quiz:
/// (user_id, display_name, email, attempts, best_score, best_max).
pub async fn quiz_student_summary(
    conn: &mut PgConnection,
    quiz_id: Uuid,
) -> sqlx::Result<Vec<(Uuid, Option<String>, String, i64, Option<i32>, Option<i32>)>> {
    sqlx::query_as(
        "SELECT qa.user_id, u.display_name, u.email::text, \
                count(*) AS attempts, \
                max(qa.score_points) AS best_score, \
                max(qa.max_points) AS best_max \
         FROM quiz_attempts qa \
         JOIN users u ON u.id = qa.user_id \
         WHERE qa.quiz_id = $1 AND qa.submitted_at IS NOT NULL \
         GROUP BY qa.user_id, u.display_name, u.email \
         ORDER BY u.display_name NULLS LAST, u.email",
    )
    .bind(quiz_id)
    .fetch_all(conn)
    .await
}

/// Count of distinct published module-attached quizzes in a course the user
/// has submitted at least one attempt on — feeds course progress.
pub async fn completed_module_quiz_count(
    conn: &mut PgConnection,
    course_id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<i64> {
    sqlx::query_scalar(
        "SELECT count(DISTINCT q.id) FROM quizzes q \
         JOIN quiz_attempts qa ON qa.quiz_id = q.id \
         WHERE q.course_id = $1 AND q.module_id IS NOT NULL \
           AND q.status = 'published' \
           AND qa.user_id = $2 AND qa.submitted_at IS NOT NULL",
    )
    .bind(course_id)
    .bind(user_id)
    .fetch_one(conn)
    .await
}

/// Count of published module-attached quizzes in a course — feeds the
/// course-progress denominator.
pub async fn module_quiz_count(conn: &mut PgConnection, course_id: Uuid) -> sqlx::Result<i64> {
    sqlx::query_scalar(
        "SELECT count(*) FROM quizzes \
         WHERE course_id = $1 AND module_id IS NOT NULL AND status = 'published'",
    )
    .bind(course_id)
    .fetch_one(conn)
    .await
}
