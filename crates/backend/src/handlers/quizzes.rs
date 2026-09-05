//! Quiz endpoints (learning-suite Cycle 3).
//!
//! Teacher/staff:
//! - `POST   /v1/courses/{cid}/quizzes`                    — create
//! - `PATCH  /v1/courses/{cid}/quizzes/{qid}`               — update / publish
//! - `DELETE /v1/courses/{cid}/quizzes/{qid}`               — delete
//! - `PUT    /v1/courses/{cid}/quizzes/{qid}/questions`     — replace questions
//! - `GET    /v1/courses/{cid}/quizzes/{qid}/results`       — per-student rollup
//!
//! Everyone in the course:
//! - `GET /v1/courses/{cid}/quizzes`        — list (students: published only)
//! - `GET /v1/courses/{cid}/quizzes/{qid}`   — detail (students: keys stripped)
//!
//! Students:
//! - `POST /v1/courses/{cid}/quizzes/{qid}/attempts`              — start/resume
//! - `POST /v1/courses/{cid}/quizzes/{qid}/attempts/{aid}/submit`  — grade + store
//! - `GET  /v1/courses/{cid}/quizzes/{qid}/attempts`              — my attempts
//!
//! Grading is server-authoritative via `core_types::quiz::grade_answer`.

use axum::extract::{Extension, Path, State};
use axum::{routing, Json, Router};
use core_types::quiz::{grade_answer, QuizAnswer, QuizPrompt, StudentQuizPrompt};
use sqlx::PgPool;
use uuid::Uuid;

use crate::context::RequestContext;
use crate::db;
use crate::error::ApiError;
use crate::AppState;

/// Grace window past `started_at + time_limit` before a submission is
/// rejected (network latency, clock skew between tick and submit).
const TIME_LIMIT_GRACE_SECONDS: i64 = 30;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/courses/{cid}/quizzes", routing::post(create).get(list))
        .route(
            "/v1/courses/{cid}/quizzes/{qid}",
            routing::get(detail).patch(patch).delete(delete_one),
        )
        .route(
            "/v1/courses/{cid}/quizzes/{qid}/questions",
            routing::put(replace_questions),
        )
        .route(
            "/v1/courses/{cid}/quizzes/{qid}/results",
            routing::get(results),
        )
        .route(
            "/v1/courses/{cid}/quizzes/{qid}/attempts",
            routing::post(start_attempt).get(my_attempts),
        )
        .route(
            "/v1/courses/{cid}/quizzes/{qid}/attempts/{aid}/submit",
            routing::post(submit_attempt),
        )
}

#[derive(Clone)]
struct TestState {
    pool: PgPool,
}

#[doc(hidden)]
pub fn router_for_tests(pool: PgPool) -> Router {
    Router::new()
        .route(
            "/v1/courses/{cid}/quizzes",
            routing::post(create_t).get(list_t),
        )
        .route(
            "/v1/courses/{cid}/quizzes/{qid}",
            routing::get(detail_t).patch(patch_t).delete(delete_one_t),
        )
        .route(
            "/v1/courses/{cid}/quizzes/{qid}/questions",
            routing::put(replace_questions_t),
        )
        .route(
            "/v1/courses/{cid}/quizzes/{qid}/results",
            routing::get(results_t),
        )
        .route(
            "/v1/courses/{cid}/quizzes/{qid}/attempts",
            routing::post(start_attempt_t).get(my_attempts_t),
        )
        .route(
            "/v1/courses/{cid}/quizzes/{qid}/attempts/{aid}/submit",
            routing::post(submit_attempt_t),
        )
        .with_state(TestState { pool })
}

// --- auth helpers (same model as progress.rs) ---

fn is_org_admin(ctx: &RequestContext) -> bool {
    ctx.can_manage_organization()
}

fn require_tenant(ctx: &RequestContext) -> Result<Uuid, ApiError> {
    ctx.tenant_id
        .ok_or_else(|| ApiError::BadRequest("no active tenant".into()))
}

fn require_learner(ctx: &RequestContext) -> Result<(), ApiError> {
    ctx.has_capability(core_types::Capability::Learn)
        .then_some(())
        .ok_or(ApiError::Forbidden)
}

/// Quiz attempts are learner records, not a generic course-read feature.
/// Require both active student roles so staff/parents cannot create synthetic
/// attempts and a stale course membership cannot survive a tenant demotion.
async fn require_active_student_course_membership(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: Uuid,
    course_id: Uuid,
    user_id: Uuid,
) -> Result<(), ApiError> {
    db::courses::has_active_course_membership_roles(
        tx, tenant_id, course_id, user_id, "student", "student",
    )
    .await
    .map_err(internal)?
    .then_some(())
    .ok_or(ApiError::Forbidden)
}

async fn require_course_read(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
) -> Result<(), ApiError> {
    if db::courses::caller_can_read_course(
        pool,
        course_id,
        ctx.user_id,
        ctx.tenant_id,
        is_org_admin(ctx),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?
    {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
    }
}

async fn course_staff(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
) -> Result<bool, ApiError> {
    if !ctx.can_assist() {
        return Ok(false);
    }
    db::courses::caller_can_staff_course(
        pool,
        course_id,
        ctx.user_id,
        ctx.tenant_id,
        is_org_admin(ctx),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))
}

async fn require_course_author(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
) -> Result<(), ApiError> {
    if !ctx.can_teach() {
        return Err(ApiError::Forbidden);
    }
    require_course_staff(pool, ctx, course_id).await
}

async fn require_course_staff(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
) -> Result<(), ApiError> {
    if course_staff(pool, ctx, course_id).await? {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
    }
}

fn internal(e: impl std::fmt::Display) -> ApiError {
    ApiError::Internal(e.to_string())
}

// --- DTOs ---

#[derive(serde::Serialize)]
pub struct QuizDto {
    pub id: Uuid,
    pub course_id: Uuid,
    pub module_id: Option<Uuid>,
    pub title: String,
    pub description: Option<String>,
    pub mode: String,
    pub time_limit_seconds: Option<i32>,
    pub max_attempts: Option<i32>,
    pub status: String,
    pub question_count: i64,
    pub my_submitted_attempts: i64,
    pub my_best_score: Option<i32>,
    pub my_best_max: Option<i32>,
}

impl QuizDto {
    fn from_row(row: db::quizzes::QuizRow) -> Self {
        Self {
            id: row.id,
            course_id: row.course_id,
            module_id: row.module_id,
            title: row.title,
            description: row.description,
            mode: row.mode,
            time_limit_seconds: row.time_limit_seconds,
            max_attempts: row.max_attempts,
            status: row.status,
            question_count: 0,
            my_submitted_attempts: 0,
            my_best_score: None,
            my_best_max: None,
        }
    }
}

/// Authoring view of a question — includes the key. Staff only.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct AuthoringQuestion {
    #[serde(default)]
    pub id: Option<Uuid>,
    pub prompt_text: String,
    pub prompt: QuizPrompt,
    #[serde(default)]
    pub explanation: String,
    #[serde(default = "default_points")]
    pub points: i32,
}

fn default_points() -> i32 {
    1
}

/// Student-safe question: key stripped.
#[derive(serde::Serialize)]
pub struct StudentQuestion {
    pub id: Uuid,
    pub prompt_text: String,
    pub prompt: StudentQuizPrompt,
    pub points: i32,
}

#[derive(serde::Serialize)]
pub struct QuizDetail {
    #[serde(flatten)]
    pub quiz: QuizDto,
    /// Staff only — `None` for students.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub questions: Option<Vec<AuthoringQuestion>>,
    /// Students only — `None` for staff.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub student_questions: Option<Vec<StudentQuestion>>,
}

#[derive(serde::Deserialize)]
pub struct CreateQuizBody {
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default = "default_mode")]
    pub mode: String,
    #[serde(default)]
    pub module_id: Option<Uuid>,
    #[serde(default)]
    pub time_limit_seconds: Option<i32>,
    #[serde(default)]
    pub max_attempts: Option<i32>,
}

fn default_mode() -> String {
    "graded".into()
}

#[derive(serde::Deserialize)]
pub struct PatchQuizBody {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub mode: Option<String>,
    /// Double-option: outer = field present in the patch, inner = new value.
    #[serde(default, with = "::serde_with::rust::double_option")]
    pub time_limit_seconds: Option<Option<i32>>,
    #[serde(default, with = "::serde_with::rust::double_option")]
    pub max_attempts: Option<Option<i32>>,
    #[serde(default, with = "::serde_with::rust::double_option")]
    pub module_id: Option<Option<Uuid>>,
    #[serde(default)]
    pub status: Option<String>,
}

fn validate_mode(mode: &str) -> Result<(), ApiError> {
    if matches!(mode, "graded" | "practice") {
        Ok(())
    } else {
        Err(ApiError::BadRequest(format!(
            "invalid quiz mode `{mode}`; expected graded|practice"
        )))
    }
}

fn validate_status(status: &str) -> Result<(), ApiError> {
    if matches!(status, "draft" | "published" | "archived") {
        Ok(())
    } else {
        Err(ApiError::BadRequest(format!(
            "invalid quiz status `{status}`"
        )))
    }
}

// --- create / patch / delete ---

async fn create_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
    body: CreateQuizBody,
) -> Result<Json<QuizDto>, ApiError> {
    let tenant_id = require_tenant(ctx)?;
    require_course_author(pool, ctx, course_id).await?;
    validate_mode(&body.mode)?;
    if body.title.trim().is_empty() {
        return Err(ApiError::BadRequest("title must not be empty".into()));
    }

    let mut tx = db::begin_with_context(pool, ctx.user_id, ctx.tenant_id)
        .await
        .map_err(internal)?;
    let row = db::quizzes::create_quiz(
        &mut tx,
        tenant_id,
        course_id,
        body.module_id,
        body.title.trim(),
        body.description.as_deref(),
        &body.mode,
        body.time_limit_seconds,
        body.max_attempts,
        ctx.user_id,
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok(Json(QuizDto::from_row(row)))
}

async fn patch_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
    quiz_id: Uuid,
    body: PatchQuizBody,
) -> Result<Json<QuizDto>, ApiError> {
    require_course_author(pool, ctx, course_id).await?;
    if let Some(mode) = &body.mode {
        validate_mode(mode)?;
    }
    if let Some(status) = &body.status {
        validate_status(status)?;
    }

    let mut tx = db::begin_with_context(pool, ctx.user_id, ctx.tenant_id)
        .await
        .map_err(internal)?;
    let existing = db::quizzes::get_quiz(&mut tx, quiz_id)
        .await
        .map_err(internal)?
        .filter(|q| q.course_id == course_id)
        .ok_or(ApiError::NotFound)?;
    // Publishing requires at least one question.
    if body.status.as_deref() == Some("published") && existing.status != "published" {
        let questions = db::quizzes::list_questions(&mut tx, quiz_id)
            .await
            .map_err(internal)?;
        if questions.is_empty() {
            return Err(ApiError::BadRequest(
                "add at least one question before publishing".into(),
            ));
        }
    }
    let row = db::quizzes::patch_quiz(
        &mut tx,
        quiz_id,
        body.title.as_deref(),
        body.description.as_deref(),
        body.mode.as_deref(),
        body.time_limit_seconds,
        body.max_attempts,
        body.module_id,
        body.status.as_deref(),
    )
    .await
    .map_err(internal)?
    .ok_or(ApiError::NotFound)?;
    tx.commit().await.map_err(internal)?;
    Ok(Json(QuizDto::from_row(row)))
}

async fn delete_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
    quiz_id: Uuid,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_course_author(pool, ctx, course_id).await?;
    let mut tx = db::begin_with_context(pool, ctx.user_id, ctx.tenant_id)
        .await
        .map_err(internal)?;
    let existing = db::quizzes::get_quiz(&mut tx, quiz_id)
        .await
        .map_err(internal)?
        .filter(|q| q.course_id == course_id);
    if existing.is_none() {
        return Err(ApiError::NotFound);
    }
    db::quizzes::delete_quiz(&mut tx, quiz_id)
        .await
        .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok(Json(serde_json::json!({ "deleted": true })))
}

// --- list / detail ---

async fn list_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
) -> Result<Json<Vec<QuizDto>>, ApiError> {
    require_course_read(pool, ctx, course_id).await?;
    let staff = course_staff(pool, ctx, course_id).await?;

    let mut tx = db::begin_with_context(pool, ctx.user_id, ctx.tenant_id)
        .await
        .map_err(internal)?;
    let rows = db::quizzes::list_quizzes(&mut tx, course_id, !staff)
        .await
        .map_err(internal)?;
    // Per-quiz stats arrive in one grouped query (question counts + the
    // caller's submitted attempts/best score) instead of 2N+1 per-quiz lookups.
    let stats: std::collections::HashMap<uuid::Uuid, db::quizzes::QuizListStats> =
        db::quizzes::list_stats_for_course(&mut tx, course_id, ctx.user_id)
            .await
            .map_err(internal)?
            .into_iter()
            .map(|s| (s.quiz_id, s))
            .collect();
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let quiz_id = row.id;
        let mut dto = QuizDto::from_row(row);
        if let Some(s) = stats.get(&quiz_id) {
            dto.question_count = s.question_count;
            dto.my_submitted_attempts = s.my_submitted_attempts;
            dto.my_best_score = s.my_best_score;
            dto.my_best_max = s.my_best_max;
        }
        out.push(dto);
    }
    Ok(Json(out))
}

async fn detail_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
    quiz_id: Uuid,
) -> Result<Json<QuizDetail>, ApiError> {
    require_course_read(pool, ctx, course_id).await?;
    let staff = course_staff(pool, ctx, course_id).await?;

    let mut tx = db::begin_with_context(pool, ctx.user_id, ctx.tenant_id)
        .await
        .map_err(internal)?;
    let row = db::quizzes::get_quiz(&mut tx, quiz_id)
        .await
        .map_err(internal)?
        .filter(|q| q.course_id == course_id)
        .ok_or(ApiError::NotFound)?;
    if !staff && row.status != "published" {
        return Err(ApiError::NotFound);
    }
    let questions = db::quizzes::list_questions(&mut tx, quiz_id)
        .await
        .map_err(internal)?;
    let attempts = db::quizzes::list_my_attempts(&mut tx, quiz_id, ctx.user_id)
        .await
        .map_err(internal)?;

    let mut dto = QuizDto::from_row(row);
    dto.question_count = questions.len() as i64;
    dto.my_submitted_attempts = attempts.len() as i64;
    dto.my_best_score = attempts.iter().filter_map(|a| a.score_points).max();
    dto.my_best_max = attempts.iter().filter_map(|a| a.max_points).max();

    let detail = if staff {
        let qs = questions
            .into_iter()
            .map(|q| {
                Ok(AuthoringQuestion {
                    id: Some(q.id),
                    prompt_text: q.prompt_text,
                    prompt: serde_json::from_value(q.prompt).map_err(internal)?,
                    explanation: q.explanation,
                    points: q.points,
                })
            })
            .collect::<Result<Vec<_>, ApiError>>()?;
        QuizDetail {
            quiz: dto,
            questions: Some(qs),
            student_questions: None,
        }
    } else {
        let qs = questions
            .into_iter()
            .map(|q| {
                let prompt: QuizPrompt = serde_json::from_value(q.prompt).map_err(internal)?;
                Ok(StudentQuestion {
                    id: q.id,
                    prompt_text: q.prompt_text,
                    prompt: prompt.student_view(),
                    points: q.points,
                })
            })
            .collect::<Result<Vec<_>, ApiError>>()?;
        QuizDetail {
            quiz: dto,
            questions: None,
            student_questions: Some(qs),
        }
    };
    Ok(Json(detail))
}

// --- questions authoring ---

async fn replace_questions_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
    quiz_id: Uuid,
    body: Vec<AuthoringQuestion>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let tenant_id = require_tenant(ctx)?;
    require_course_author(pool, ctx, course_id).await?;
    if body.len() > 100 {
        return Err(ApiError::BadRequest("too many questions (max 100)".into()));
    }
    for (i, q) in body.iter().enumerate() {
        if q.prompt_text.trim().is_empty() {
            return Err(ApiError::BadRequest(format!(
                "question {} is missing its prompt text",
                i + 1
            )));
        }
        if q.points <= 0 {
            return Err(ApiError::BadRequest(format!(
                "question {} must be worth at least 1 point",
                i + 1
            )));
        }
        q.prompt
            .validate()
            .map_err(|e| ApiError::BadRequest(format!("question {}: {e}", i + 1)))?;
    }

    let mut tx = db::begin_with_context(pool, ctx.user_id, ctx.tenant_id)
        .await
        .map_err(internal)?;
    db::quizzes::get_quiz(&mut tx, quiz_id)
        .await
        .map_err(internal)?
        .filter(|q| q.course_id == course_id)
        .ok_or(ApiError::NotFound)?;
    let questions: Vec<(String, QuizPrompt, String, i32)> = body
        .into_iter()
        .map(|q| (q.prompt_text, q.prompt, q.explanation, q.points))
        .collect();
    let rows = db::quizzes::replace_questions(&mut tx, tenant_id, quiz_id, &questions)
        .await
        .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok(Json(serde_json::json!({ "count": rows.len() })))
}

// --- attempts ---

#[derive(serde::Serialize)]
pub struct AttemptDto {
    pub id: Uuid,
    pub quiz_id: Uuid,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub submitted_at: Option<chrono::DateTime<chrono::Utc>>,
    pub score_points: Option<i32>,
    pub max_points: Option<i32>,
    /// Absolute deadline for timed quizzes (started_at + time limit).
    pub deadline: Option<chrono::DateTime<chrono::Utc>>,
}

fn attempt_dto(row: db::quizzes::AttemptRow, time_limit_seconds: Option<i32>) -> AttemptDto {
    let deadline = time_limit_seconds.map(|s| row.started_at + chrono::Duration::seconds(s as i64));
    AttemptDto {
        id: row.id,
        quiz_id: row.quiz_id,
        started_at: row.started_at,
        submitted_at: row.submitted_at,
        score_points: row.score_points,
        max_points: row.max_points,
        deadline,
    }
}

async fn start_attempt_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
    quiz_id: Uuid,
) -> Result<Json<AttemptDto>, ApiError> {
    require_learner(ctx)?;
    let tenant_id = require_tenant(ctx)?;
    require_course_read(pool, ctx, course_id).await?;

    let mut tx = db::begin_with_context(pool, ctx.user_id, ctx.tenant_id)
        .await
        .map_err(internal)?;
    require_active_student_course_membership(&mut tx, tenant_id, course_id, ctx.user_id).await?;
    let quiz = db::quizzes::get_quiz(&mut tx, quiz_id)
        .await
        .map_err(internal)?
        .filter(|q| q.course_id == course_id)
        .ok_or(ApiError::NotFound)?;
    if quiz.status != "published" {
        return Err(ApiError::NotFound);
    }

    // Resume an open attempt rather than burning a new one.
    if let Some(open) = db::quizzes::open_attempt(&mut tx, quiz_id, ctx.user_id)
        .await
        .map_err(internal)?
    {
        return Ok(Json(attempt_dto(open, quiz.time_limit_seconds)));
    }

    // Graded quizzes enforce the attempt cap; practice is unlimited.
    if quiz.mode == "graded" {
        if let Some(cap) = quiz.max_attempts {
            let used = db::quizzes::count_submitted_attempts(&mut tx, quiz_id, ctx.user_id)
                .await
                .map_err(internal)?;
            if used >= cap as i64 {
                return Err(ApiError::BadRequest(format!(
                    "attempt limit reached ({cap})"
                )));
            }
        }
    }

    let row = db::quizzes::start_attempt(&mut tx, tenant_id, quiz_id, ctx.user_id)
        .await
        .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok(Json(attempt_dto(row, quiz.time_limit_seconds)))
}

#[derive(serde::Deserialize)]
pub struct SubmitBody {
    pub answers: Vec<SubmittedAnswer>,
}

#[derive(serde::Deserialize)]
pub struct SubmittedAnswer {
    pub question_id: Uuid,
    pub answer: QuizAnswer,
}

#[derive(serde::Serialize)]
pub struct SubmitResult {
    pub attempt_id: Uuid,
    pub score_points: i32,
    pub max_points: i32,
    pub per_question: Vec<PerQuestionResult>,
}

#[derive(serde::Serialize)]
pub struct PerQuestionResult {
    pub question_id: Uuid,
    pub correct: bool,
    pub points_awarded: i32,
    pub points: i32,
}

async fn submit_attempt_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
    quiz_id: Uuid,
    attempt_id: Uuid,
    body: SubmitBody,
) -> Result<Json<SubmitResult>, ApiError> {
    require_learner(ctx)?;
    require_course_read(pool, ctx, course_id).await?;
    let tenant_id = require_tenant(ctx)?;

    let mut tx = db::begin_with_context(pool, ctx.user_id, ctx.tenant_id)
        .await
        .map_err(internal)?;
    require_active_student_course_membership(&mut tx, tenant_id, course_id, ctx.user_id).await?;
    let quiz = db::quizzes::get_quiz(&mut tx, quiz_id)
        .await
        .map_err(internal)?
        .filter(|q| q.course_id == course_id)
        .ok_or(ApiError::NotFound)?;
    let attempt = db::quizzes::get_attempt(&mut tx, attempt_id)
        .await
        .map_err(internal)?
        .filter(|a| a.quiz_id == quiz_id)
        .ok_or(ApiError::NotFound)?;
    if attempt.user_id != ctx.user_id {
        return Err(ApiError::Forbidden);
    }
    if attempt.submitted_at.is_some() {
        return Err(ApiError::BadRequest(
            "this attempt was already submitted".into(),
        ));
    }
    if let Some(limit) = quiz.time_limit_seconds {
        let deadline =
            attempt.started_at + chrono::Duration::seconds(limit as i64 + TIME_LIMIT_GRACE_SECONDS);
        if chrono::Utc::now() > deadline {
            return Err(ApiError::BadRequest("the time limit has expired".into()));
        }
    }

    let questions = db::quizzes::list_questions(&mut tx, quiz_id)
        .await
        .map_err(internal)?;
    let mut max_points = 0;
    let mut score = 0;
    let mut stored = Vec::with_capacity(questions.len());
    let mut per_question = Vec::with_capacity(questions.len());
    for q in &questions {
        max_points += q.points;
        let prompt: QuizPrompt = serde_json::from_value(q.prompt.clone()).map_err(internal)?;
        let submitted = body.answers.iter().find(|a| a.question_id == q.id);
        let (correct, answer_json) = match submitted {
            Some(a) => {
                let correct = grade_answer(&prompt, &a.answer).ok_or_else(|| {
                    ApiError::BadRequest(format!("answer shape does not match question {}", q.id))
                })?;
                (correct, serde_json::to_value(&a.answer).map_err(internal)?)
            }
            // Unanswered questions grade as incorrect.
            None => (false, serde_json::Value::Null),
        };
        let awarded = if correct { q.points } else { 0 };
        score += awarded;
        stored.push((q.id, answer_json, correct, awarded));
        per_question.push(PerQuestionResult {
            question_id: q.id,
            correct,
            points_awarded: awarded,
            points: q.points,
        });
    }

    let finalized = db::quizzes::submit_attempt(&mut tx, attempt_id, score, max_points, &stored)
        .await
        .map_err(internal)?;
    if !finalized {
        // A concurrent submit won the guarded UPDATE; this request changes
        // nothing and must not fall through to answer inserts.
        return Err(ApiError::Conflict("attempt_already_submitted".into()));
    }

    // Best-effort XP awards: first submission on this quiz, plus a perfect
    // bonus (each once per quiz/user via dedup keys).
    if let Some(tenant_id) = ctx.tenant_id {
        if let Err(e) = db::gamification::award(
            &mut tx,
            tenant_id,
            ctx.user_id,
            Some(course_id),
            "quiz_submitted",
            db::gamification::XP_QUIZ_SUBMITTED,
            &format!("quiz:{quiz_id}:{}", ctx.user_id),
        )
        .await
        {
            tracing::warn!(?e, %quiz_id, "quiz XP award failed");
        }
        if max_points > 0 && score == max_points {
            if let Err(e) = db::gamification::award(
                &mut tx,
                tenant_id,
                ctx.user_id,
                Some(course_id),
                "quiz_perfect",
                db::gamification::XP_QUIZ_PERFECT,
                &format!("quizperfect:{quiz_id}:{}", ctx.user_id),
            )
            .await
            {
                tracing::warn!(?e, %quiz_id, "perfect-quiz XP award failed");
            }
        }
        // Best-effort certificate eligibility (idempotent): a graded-quiz
        // submission may have been the last completion requirement.
        if let Err(e) =
            db::certificates::sync_eligibility(&mut tx, tenant_id, course_id, ctx.user_id).await
        {
            tracing::warn!(?e, %course_id, "certificate eligibility sync failed");
        }
    }
    tx.commit().await.map_err(internal)?;

    Ok(Json(SubmitResult {
        attempt_id,
        score_points: score,
        max_points,
        per_question,
    }))
}

#[derive(serde::Serialize)]
pub struct MyAttemptDto {
    #[serde(flatten)]
    pub attempt: AttemptDto,
    pub per_question: Vec<PerQuestionCorrect>,
}

#[derive(serde::Serialize)]
pub struct PerQuestionCorrect {
    pub question_id: Uuid,
    pub correct: bool,
}

async fn my_attempts_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
    quiz_id: Uuid,
) -> Result<Json<Vec<MyAttemptDto>>, ApiError> {
    require_learner(ctx)?;
    require_course_read(pool, ctx, course_id).await?;
    let tenant_id = require_tenant(ctx)?;
    let mut tx = db::begin_with_context(pool, ctx.user_id, ctx.tenant_id)
        .await
        .map_err(internal)?;
    require_active_student_course_membership(&mut tx, tenant_id, course_id, ctx.user_id).await?;
    let quiz = db::quizzes::get_quiz(&mut tx, quiz_id)
        .await
        .map_err(internal)?
        .filter(|q| q.course_id == course_id)
        .ok_or(ApiError::NotFound)?;
    let rows = db::quizzes::list_my_attempts(&mut tx, quiz_id, ctx.user_id)
        .await
        .map_err(internal)?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let answers = db::quizzes::attempt_answers(&mut tx, row.id)
            .await
            .map_err(internal)?;
        out.push(MyAttemptDto {
            attempt: attempt_dto(row, quiz.time_limit_seconds),
            per_question: answers
                .into_iter()
                .map(|(question_id, correct, _)| PerQuestionCorrect {
                    question_id,
                    correct,
                })
                .collect(),
        });
    }
    Ok(Json(out))
}

#[derive(serde::Serialize)]
pub struct QuizResultRow {
    pub user_id: Uuid,
    pub display_name: Option<String>,
    pub email: String,
    pub attempts: i64,
    pub best_score: Option<i32>,
    pub best_max: Option<i32>,
}

async fn results_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
    quiz_id: Uuid,
) -> Result<Json<Vec<QuizResultRow>>, ApiError> {
    require_course_staff(pool, ctx, course_id).await?;
    let mut tx = db::begin_with_context(pool, ctx.user_id, ctx.tenant_id)
        .await
        .map_err(internal)?;
    db::quizzes::get_quiz(&mut tx, quiz_id)
        .await
        .map_err(internal)?
        .filter(|q| q.course_id == course_id)
        .ok_or(ApiError::NotFound)?;
    let rows = db::quizzes::quiz_student_summary(&mut tx, quiz_id)
        .await
        .map_err(internal)?;
    Ok(Json(
        rows.into_iter()
            .map(
                |(user_id, display_name, email, attempts, best_score, best_max)| QuizResultRow {
                    user_id,
                    display_name,
                    email,
                    attempts,
                    best_score,
                    best_max,
                },
            )
            .collect(),
    ))
}

// --- axum wrappers (AppState + TestState) ---

macro_rules! wrappers {
    ($name:ident, $name_t:ident, $inner:ident, ($($param:ident: $pty:ty),*), $ret:ty) => {
        async fn $name(
            State(state): State<AppState>,
            Extension(ctx): Extension<RequestContext>,
            Path(path): Path<($($pty,)*)>,
        ) -> Result<$ret, ApiError> {
            let ($($param,)*) = path;
            $inner(&state.pool, &ctx, $($param),*).await
        }
        async fn $name_t(
            State(s): State<TestState>,
            Extension(ctx): Extension<RequestContext>,
            Path(path): Path<($($pty,)*)>,
        ) -> Result<$ret, ApiError> {
            let ($($param,)*) = path;
            $inner(&s.pool, &ctx, $($param),*).await
        }
    };
}

wrappers!(list, list_t, list_inner, (cid: Uuid), Json<Vec<QuizDto>>);
wrappers!(detail, detail_t, detail_inner, (cid: Uuid, qid: Uuid), Json<QuizDetail>);
wrappers!(delete_one, delete_one_t, delete_inner, (cid: Uuid, qid: Uuid), Json<serde_json::Value>);
wrappers!(results, results_t, results_inner, (cid: Uuid, qid: Uuid), Json<Vec<QuizResultRow>>);
wrappers!(start_attempt, start_attempt_t, start_attempt_inner, (cid: Uuid, qid: Uuid), Json<AttemptDto>);
wrappers!(my_attempts, my_attempts_t, my_attempts_inner, (cid: Uuid, qid: Uuid), Json<Vec<MyAttemptDto>>);

async fn create(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(cid): Path<Uuid>,
    Json(body): Json<CreateQuizBody>,
) -> Result<Json<QuizDto>, ApiError> {
    create_inner(&state.pool, &ctx, cid, body).await
}

async fn create_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(cid): Path<Uuid>,
    Json(body): Json<CreateQuizBody>,
) -> Result<Json<QuizDto>, ApiError> {
    create_inner(&s.pool, &ctx, cid, body).await
}

async fn patch(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path((cid, qid)): Path<(Uuid, Uuid)>,
    Json(body): Json<PatchQuizBody>,
) -> Result<Json<QuizDto>, ApiError> {
    patch_inner(&state.pool, &ctx, cid, qid, body).await
}

async fn patch_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path((cid, qid)): Path<(Uuid, Uuid)>,
    Json(body): Json<PatchQuizBody>,
) -> Result<Json<QuizDto>, ApiError> {
    patch_inner(&s.pool, &ctx, cid, qid, body).await
}

async fn replace_questions(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path((cid, qid)): Path<(Uuid, Uuid)>,
    Json(body): Json<Vec<AuthoringQuestion>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    replace_questions_inner(&state.pool, &ctx, cid, qid, body).await
}

async fn replace_questions_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path((cid, qid)): Path<(Uuid, Uuid)>,
    Json(body): Json<Vec<AuthoringQuestion>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    replace_questions_inner(&s.pool, &ctx, cid, qid, body).await
}

async fn submit_attempt(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path((cid, qid, aid)): Path<(Uuid, Uuid, Uuid)>,
    Json(body): Json<SubmitBody>,
) -> Result<Json<SubmitResult>, ApiError> {
    submit_attempt_inner(&state.pool, &ctx, cid, qid, aid, body).await
}

async fn submit_attempt_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path((cid, qid, aid)): Path<(Uuid, Uuid, Uuid)>,
    Json(body): Json<SubmitBody>,
) -> Result<Json<SubmitResult>, ApiError> {
    submit_attempt_inner(&s.pool, &ctx, cid, qid, aid, body).await
}
