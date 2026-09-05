// crates/backend/src/handlers/submissions.rs
use axum::extract::{Extension, Path, State};
use axum::{routing, Json, Router};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::types::BigDecimal;
use sqlx::PgPool;
use std::str::FromStr;
use uuid::Uuid;

use crate::context::RequestContext;
use crate::db;
use crate::error::ApiError;
use crate::AppState;

#[derive(Deserialize, Default)]
pub struct PatchSubmission {
    #[serde(default, with = "::serde_with::rust::double_option")]
    pub text_answer: Option<Option<String>>,
    pub attachment_asset_ids: Option<Vec<Uuid>>,
}

#[derive(Deserialize, Default)]
pub struct GradeBody {
    pub numeric_grade: Option<f64>,
    pub letter_grade: Option<String>,
    pub passed: Option<bool>,
    pub student_visible_feedback: Option<String>,
    pub teacher_only_notes: Option<String>,
    /// Optional rubric-driven per-criterion scores. When present (and
    /// non-empty), the assignment must be in numeric grading mode and carry a
    /// rubric; the supplied scores are validated against that rubric's criteria
    /// (each `criterion_id` must belong to it and `points` must be within the
    /// criterion's `max_points`), summed to form the numeric grade (any
    /// explicit `numeric_grade` is ignored in favor of the sum), and persisted.
    /// Absent or empty → fully backward-compatible numeric/letter/pass grading.
    #[serde(default)]
    pub criteria: Vec<CriterionScoreInput>,
}

#[derive(Deserialize)]
pub struct CriterionScoreInput {
    pub criterion_id: Uuid,
    pub points: f64,
}

#[derive(Serialize)]
pub struct SubmissionDto {
    pub id: Uuid,
    pub assignment_id: Uuid,
    pub course_id: Uuid,
    pub student_user_id: Uuid,
    pub status: String,
    pub text_answer: Option<String>,
    pub attachment_asset_ids: Vec<Uuid>,
    pub submitted_at: Option<DateTime<Utc>>,
    pub is_late: bool,
    pub attempt_number: i32,
    /// Late-penalty percentage actually deducted at grade time. Visible only
    /// once the grade is visible to the viewer (teacher always; student after
    /// release). `None` until graded.
    pub applied_late_penalty_percent: Option<i32>,
    pub numeric_grade: Option<f64>,
    pub letter_grade: Option<String>,
    pub passed: Option<bool>,
    pub student_visible_feedback: Option<String>,
    pub graded_at: Option<DateTime<Utc>>,
    pub released_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

fn dto_for_viewer(row: db::submissions::SubmissionRow, is_teacher: bool) -> SubmissionDto {
    let released = row.released_at.is_some();
    let show_grade = is_teacher || released;
    // On manual-release assignments the student must not observe the grade
    // event itself: a `graded` status before release reveals that (and, with
    // timing, roughly when) grading happened. Collapse it to `submitted` for
    // non-privileged viewers; teachers and post-release viewers see the truth.
    let viewer_status = if show_grade {
        row.status
    } else if row.status == "graded" {
        "submitted".to_string()
    } else {
        row.status
    };
    SubmissionDto {
        id: row.id,
        assignment_id: row.assignment_id,
        course_id: row.course_id,
        student_user_id: row.student_user_id,
        status: viewer_status,
        text_answer: row.text_answer,
        attachment_asset_ids: row.attachment_asset_ids,
        submitted_at: row.submitted_at,
        is_late: row.is_late,
        attempt_number: row.attempt_number,
        applied_late_penalty_percent: if show_grade {
            row.applied_late_penalty_percent
        } else {
            None
        },
        numeric_grade: if show_grade {
            row.numeric_grade
                .and_then(|d| d.to_string().parse::<f64>().ok())
        } else {
            None
        },
        letter_grade: if show_grade { row.letter_grade } else { None },
        passed: if show_grade { row.passed } else { None },
        student_visible_feedback: if show_grade {
            row.student_visible_feedback
        } else {
            None
        },
        graded_at: if show_grade { row.graded_at } else { None },
        released_at: if show_grade { row.released_at } else { None },
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/v1/assignments/{aid}/submissions",
            routing::post(create_or_get).get(list_for_assignment),
        )
        .route("/v1/submissions/{id}", routing::get(get_one).patch(patch))
        .route("/v1/submissions/{id}/submit", routing::post(submit))
        .route("/v1/submissions/{id}/grade", routing::post(grade))
        .route("/v1/submissions/{id}/release", routing::post(release))
        .route(
            "/v1/submissions/{id}/return",
            routing::post(return_resubmit),
        )
}

#[doc(hidden)]
pub fn router_for_tests(pool: PgPool) -> Router {
    Router::new()
        .route(
            "/v1/assignments/{aid}/submissions",
            routing::post(create_or_get_t).get(list_for_assignment_t),
        )
        .route(
            "/v1/submissions/{id}",
            routing::get(get_one_t).patch(patch_t),
        )
        .route("/v1/submissions/{id}/submit", routing::post(submit_t))
        .route("/v1/submissions/{id}/grade", routing::post(grade_t))
        .route("/v1/submissions/{id}/release", routing::post(release_t))
        .route(
            "/v1/submissions/{id}/return",
            routing::post(return_resubmit_t),
        )
        .with_state(TestState { pool })
}

#[derive(Clone)]
struct TestState {
    pool: PgPool,
}

fn is_teacher(ctx: &RequestContext) -> bool {
    ctx.can_grade()
}

fn is_org_admin(ctx: &RequestContext) -> bool {
    ctx.can_manage_organization()
}

fn require_learner(ctx: &RequestContext) -> Result<(), ApiError> {
    if ctx.has_capability(core_types::Capability::Learn) {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
    }
}

/// Lock and verify the caller's active student membership in both the selected
/// workspace and the concrete course. Holding `FOR SHARE` locks until the
/// submission transaction commits prevents a concurrent suspension/removal
/// from racing the authorized write.
async fn require_active_student_course_membership(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: Uuid,
    course_id: Uuid,
    user_id: Uuid,
) -> Result<(), ApiError> {
    if db::courses::has_active_course_membership_roles(
        tx, tenant_id, course_id, user_id, "student", "student",
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?
    {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
    }
}

/// Course-scoped staff gate for teacher-only submission actions.
/// `platform_admin` always bypasses; everyone else must own the course or be
/// an active teacher/ta member of it.
async fn require_course_staff(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
) -> Result<(), ApiError> {
    if ctx.can_manage_organization() {
        return Ok(());
    }
    if !db::courses::caller_can_staff_course(
        pool,
        course_id,
        ctx.user_id,
        ctx.tenant_id,
        is_org_admin(ctx),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?
    {
        return Err(ApiError::Forbidden);
    }
    Ok(())
}

async fn set_tenant(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: Uuid,
    tenant: Uuid,
) -> sqlx::Result<()> {
    db::set_request_guc(tx, user_id, Some(tenant)).await
}

// --- shared inner functions ---

async fn create_or_get_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    aid: Uuid,
) -> Result<Json<SubmissionDto>, ApiError> {
    require_learner(ctx)?;
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    set_tenant(&mut tx, ctx.user_id, tenant)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let assignment = db::assignments::fetch_by_id(&mut tx, aid)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::NotFound)?;
    if assignment.status != "published" {
        return Err(ApiError::NotFound);
    }
    require_active_student_course_membership(&mut tx, tenant, assignment.course_id, ctx.user_id)
        .await?;
    let row = db::submissions::upsert_for_student(
        &mut tx,
        tenant,
        aid,
        assignment.course_id,
        ctx.user_id,
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(dto_for_viewer(row, is_teacher(ctx))))
}

async fn patch_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    id: Uuid,
    body: PatchSubmission,
) -> Result<Json<SubmissionDto>, ApiError> {
    require_learner(ctx)?;
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    set_tenant(&mut tx, ctx.user_id, tenant)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let row = db::submissions::fetch_by_id(&mut tx, id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::NotFound)?;
    if row.student_user_id != ctx.user_id {
        return Err(ApiError::NotFound);
    }
    require_active_student_course_membership(&mut tx, tenant, row.course_id, ctx.user_id).await?;
    let assn = db::assignments::fetch_by_id(&mut tx, row.assignment_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::NotFound)?;
    let editable = match row.status.as_str() {
        "draft" | "returned" => true,
        "submitted" => !assn.lock_on_submit,
        _ => false,
    };
    if !editable {
        return Err(ApiError::Conflict("submission_locked".into()));
    }
    let text_arg: Option<Option<&str>> = body.text_answer.as_ref().map(|opt| opt.as_deref());
    let updated = db::submissions::patch_draft_fields(
        &mut tx,
        id,
        text_arg,
        body.attachment_asset_ids.as_deref(),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    db::audit::emit_audit_event(
        &mut tx,
        tenant,
        ctx.user_id,
        "submission.update",
        "submission",
        updated.id,
        None,
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(dto_for_viewer(updated, is_teacher(ctx))))
}

async fn submit_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    id: Uuid,
) -> Result<Json<SubmissionDto>, ApiError> {
    require_learner(ctx)?;
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    set_tenant(&mut tx, ctx.user_id, tenant)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let row = db::submissions::fetch_by_id(&mut tx, id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::NotFound)?;
    if row.student_user_id != ctx.user_id {
        return Err(ApiError::NotFound);
    }
    require_active_student_course_membership(&mut tx, tenant, row.course_id, ctx.user_id).await?;
    if !matches!(row.status.as_str(), "draft" | "returned") {
        return Err(ApiError::Conflict("not_in_draft_or_returned".into()));
    }
    let assn = db::assignments::fetch_by_id(&mut tx, row.assignment_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::NotFound)?;
    let now = chrono::Utc::now();
    let is_late = assn.due_at.map(|d| now > d).unwrap_or(false);
    if is_late && !assn.allow_late {
        return Err(ApiError::Validation("submission_late_not_allowed".into()));
    }
    let updated = db::submissions::mark_submitted(&mut tx, id, is_late)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    db::audit::emit_audit_event(
        &mut tx,
        tenant,
        ctx.user_id,
        "submission.submit",
        "submission",
        updated.id,
        Some(serde_json::json!({ "is_late": is_late })),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(dto_for_viewer(updated, is_teacher(ctx))))
}

async fn list_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    aid: Uuid,
) -> Result<Json<Vec<SubmissionDto>>, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    if !is_teacher(ctx) {
        return Err(ApiError::Forbidden);
    }
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    set_tenant(&mut tx, ctx.user_id, tenant)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let assn = db::assignments::fetch_by_id(&mut tx, aid)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::NotFound)?;
    require_course_staff(pool, ctx, assn.course_id).await?;
    let rows = db::submissions::list_for_assignment(&mut tx, aid)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(
        rows.into_iter().map(|r| dto_for_viewer(r, true)).collect(),
    ))
}

async fn get_one_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    id: Uuid,
) -> Result<Json<SubmissionDto>, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    set_tenant(&mut tx, ctx.user_id, tenant)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let row = db::submissions::fetch_by_id(&mut tx, id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::NotFound)?;
    // Teacher visibility (grades + teacher_only_notes) is course-scoped: only
    // course owners / active teacher-ta members / org_admin / platform_admin
    // get the teacher view. A tenant-wide teacher who is not staff on this
    // submission's course is treated as a non-staff viewer.
    let teacher = is_teacher(ctx)
        && require_course_staff(pool, ctx, row.course_id)
            .await
            .map(|_| true)
            .or_else(|e| match e {
                ApiError::Forbidden => Ok(false),
                other => Err(other),
            })?;
    if !teacher && row.student_user_id != ctx.user_id {
        return Err(ApiError::NotFound);
    }
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(dto_for_viewer(row, teacher)))
}

async fn grade_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    id: Uuid,
    body: GradeBody,
) -> Result<Json<SubmissionDto>, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    if !is_teacher(ctx) {
        return Err(ApiError::Forbidden);
    }
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    set_tenant(&mut tx, ctx.user_id, tenant)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let row = db::submissions::fetch_by_id(&mut tx, id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::NotFound)?;
    require_course_staff(pool, ctx, row.course_id).await?;
    let assn = db::assignments::fetch_by_id(&mut tx, row.assignment_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::NotFound)?;

    // Effective numeric grade (post-penalty) + the penalty percent actually
    // applied. Only numeric, late submissions on an assignment carrying a
    // penalty are docked; everything else records a 0 applied penalty.
    let mut effective_numeric: Option<f64> = None;
    let mut applied_late_penalty_percent: i32 = 0;

    // Rubric-driven scoring: when the caller supplied per-criterion scores, we
    // resolve the assignment's rubric, validate every score against its real
    // criteria, sum them into the numeric grade, and persist the scores after
    // the grade is saved. Empty `criteria` ⇒ legacy path, fully unchanged.
    let use_rubric = !body.criteria.is_empty();
    let mut rubric_scores: Vec<db::rubrics::ScoreInput> = Vec::new();

    match assn.grading_mode.as_str() {
        "numeric" => {
            let n = if use_rubric {
                // The grade is the sum of the per-criterion points. Validate the
                // supplied scores against the assignment's rubric criteria.
                let rubric = db::rubrics::rubric_for_assignment_tx(&mut tx, row.assignment_id)
                    .await
                    .map_err(|e| ApiError::Internal(e.to_string()))?
                    .ok_or(ApiError::Validation("assignment_has_no_rubric".into()))?;
                let criteria = db::rubrics::criteria_for_rubric_tx(&mut tx, rubric.id)
                    .await
                    .map_err(|e| ApiError::Internal(e.to_string()))?;
                let mut sum = 0.0_f64;
                for s in &body.criteria {
                    let crit = criteria
                        .iter()
                        .find(|c| c.id == s.criterion_id)
                        .ok_or(ApiError::Validation("unknown_criterion".into()))?;
                    if s.points < 0.0 || s.points > crit.max_points as f64 {
                        return Err(ApiError::Validation("criterion_points_out_of_range".into()));
                    }
                    sum += s.points;
                    let points = BigDecimal::from_str(&s.points.to_string()).map_err(|_| {
                        ApiError::Validation("criterion_points_out_of_range".into())
                    })?;
                    rubric_scores.push(db::rubrics::ScoreInput {
                        criterion_id: s.criterion_id,
                        points,
                    });
                }
                sum
            } else {
                body.numeric_grade
                    .ok_or(ApiError::Validation("numeric_grade_required".into()))?
            };
            let max = assn.max_points.unwrap_or(0);
            if n < 0.0 || n > max as f64 {
                return Err(ApiError::Validation("numeric_grade_out_of_range".into()));
            }
            if body.passed.is_some() {
                return Err(ApiError::Validation(
                    "passed_not_allowed_for_numeric".into(),
                ));
            }
            // Late penalty: a submission turned in after the due date on an
            // assignment with a configured penalty is docked. We apply against
            // the raw grade the grader entered and clamp at 0. The `is_late`
            // flag was set at submit time; we also defensively re-check
            // submitted_at > due_at so a manually toggled flag can't over- or
            // under-charge.
            let submitted_late = row.is_late
                || match (row.submitted_at, assn.due_at) {
                    (Some(submitted), Some(due)) => submitted > due,
                    _ => false,
                };
            if submitted_late && assn.late_penalty_percent > 0 {
                applied_late_penalty_percent = assn.late_penalty_percent;
                let factor = 1.0 - (assn.late_penalty_percent as f64 / 100.0);
                effective_numeric = Some((n * factor).max(0.0));
            } else {
                effective_numeric = Some(n);
            }
        }
        "pass_fail" => {
            if use_rubric {
                return Err(ApiError::Validation(
                    "criteria_not_allowed_for_pass_fail".into(),
                ));
            }
            if body.passed.is_none() {
                return Err(ApiError::Validation("passed_required".into()));
            }
            if body.numeric_grade.is_some() || body.letter_grade.is_some() {
                return Err(ApiError::Validation(
                    "numeric_letter_not_allowed_for_pass_fail".into(),
                ));
            }
        }
        _ => return Err(ApiError::Internal("bad grading_mode".into())),
    }

    let release_now = assn.release_mode == "instant";
    let numeric = effective_numeric.and_then(|f| BigDecimal::from_str(&f.to_string()).ok());
    let updated = db::submissions::save_grade(
        &mut tx,
        id,
        db::submissions::GradeFields {
            numeric_grade: numeric,
            letter_grade: body.letter_grade.as_deref(),
            passed: body.passed,
            student_visible_feedback: body.student_visible_feedback.as_deref(),
            teacher_only_notes: body.teacher_only_notes.as_deref(),
            grader_id: ctx.user_id,
            release_now,
            applied_late_penalty_percent,
        },
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    // Persist the per-criterion scores (replacing any prior set) when rubric
    // grading was used. The numeric grade just saved is the sum of these.
    if use_rubric {
        db::rubrics::replace_scores_tx(&mut tx, tenant, updated.id, &rubric_scores)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
    }
    db::audit::emit_audit_event(
        &mut tx,
        tenant,
        ctx.user_id,
        "submission.grade",
        "submission",
        updated.id,
        Some(serde_json::json!({
            "grading_mode": assn.grading_mode,
            "numeric_grade": body.numeric_grade,
            "effective_numeric_grade": effective_numeric,
            "applied_late_penalty_percent": applied_late_penalty_percent,
            "letter_grade": body.letter_grade,
            "passed": body.passed,
            "released_now": release_now,
            "rubric_criteria_count": body.criteria.len(),
        })),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(dto_for_viewer(updated, true)))
}

async fn release_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    id: Uuid,
) -> Result<Json<SubmissionDto>, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    if !is_teacher(ctx) {
        return Err(ApiError::Forbidden);
    }
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    set_tenant(&mut tx, ctx.user_id, tenant)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    // If the row exists, enforce course-scoped staff authorization. If it is
    // absent, fall through to mark_released so the existing Conflict semantics
    // are preserved.
    if let Some(row) = db::submissions::fetch_by_id(&mut tx, id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
    {
        require_course_staff(pool, ctx, row.course_id).await?;
    }
    let updated = db::submissions::mark_released(&mut tx, id)
        .await
        .map_err(|e| match e {
            sqlx::Error::RowNotFound => {
                ApiError::Conflict("not_in_graded_or_already_released".into())
            }
            other => ApiError::Internal(other.to_string()),
        })?;
    db::audit::emit_audit_event(
        &mut tx,
        tenant,
        ctx.user_id,
        "submission.release",
        "submission",
        updated.id,
        None,
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(dto_for_viewer(updated, true)))
}

async fn return_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    id: Uuid,
) -> Result<Json<SubmissionDto>, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    if !is_teacher(ctx) {
        return Err(ApiError::Forbidden);
    }
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    set_tenant(&mut tx, ctx.user_id, tenant)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    // If the row exists, enforce course-scoped staff authorization AND the
    // assignment's resubmission cap. If it is absent, fall through to
    // return_for_resubmit so the existing Conflict semantics are preserved.
    //
    // Resubmission accounting: `attempt_number` starts at 1 and each return
    // bumps it. An assignment's `max_resubmissions` is the number of *extra*
    // attempts allowed beyond the first, so the Nth return is permitted only
    // while the current attempt is still within budget
    // (`attempt_number <= max_resubmissions`). `max_resubmissions = 0` (the
    // default) therefore rejects every return with 409 resubmissions_exhausted.
    if let Some(row) = db::submissions::fetch_by_id(&mut tx, id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
    {
        require_course_staff(pool, ctx, row.course_id).await?;
        let assn = db::assignments::fetch_by_id(&mut tx, row.assignment_id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?
            .ok_or(ApiError::NotFound)?;
        if row.attempt_number > assn.max_resubmissions {
            return Err(ApiError::Conflict("resubmissions_exhausted".into()));
        }
    }
    let updated = db::submissions::return_for_resubmit(&mut tx, id)
        .await
        .map_err(|e| match e {
            sqlx::Error::RowNotFound => ApiError::Conflict("not_in_submitted_or_graded".into()),
            other => ApiError::Internal(other.to_string()),
        })?;
    db::audit::emit_audit_event(
        &mut tx,
        tenant,
        ctx.user_id,
        "submission.return",
        "submission",
        updated.id,
        None,
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(dto_for_viewer(updated, true)))
}

// --- production handlers ---
async fn create_or_get(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(aid): Path<Uuid>,
) -> Result<Json<SubmissionDto>, ApiError> {
    create_or_get_inner(&s.pool, &ctx, aid).await
}
async fn patch(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    Json(body): Json<PatchSubmission>,
) -> Result<Json<SubmissionDto>, ApiError> {
    patch_inner(&s.pool, &ctx, id, body).await
}
async fn submit(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<SubmissionDto>, ApiError> {
    let dto = submit_inner(&s.pool, &ctx, id).await?;
    // Best-effort plagiarism fingerprint of the submitted text (never fails the
    // submission). Compared within the assignment via /v1/assignments/{id}/plagiarism.
    if let (Some(tenant), Some(text)) = (ctx.tenant_id, dto.0.text_answer.as_deref()) {
        let fp = crate::services::plagiarism::fingerprint(text);
        if !fp.is_empty() {
            if let Err(e) = crate::db::plagiarism::upsert_fingerprint(
                &s.pool,
                tenant,
                dto.0.id,
                dto.0.assignment_id,
                dto.0.student_user_id,
                &fp,
            )
            .await
            {
                tracing::warn!(?e, submission_id = %dto.0.id, "fingerprint upsert failed");
            }
        }
    }
    Ok(dto)
}
async fn list_for_assignment(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(aid): Path<Uuid>,
) -> Result<Json<Vec<SubmissionDto>>, ApiError> {
    list_inner(&s.pool, &ctx, aid).await
}
async fn get_one(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<SubmissionDto>, ApiError> {
    get_one_inner(&s.pool, &ctx, id).await
}
async fn grade(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    Json(body): Json<GradeBody>,
) -> Result<Json<SubmissionDto>, ApiError> {
    let dto = grade_inner(&s.pool, &ctx, id, body).await?;
    // Instant-release mode makes the grade student-visible at grade time, so
    // notify on this path too. Best-effort: never fail the grade on notify err.
    if dto.0.released_at.is_some() {
        notify_grade_released(
            &s,
            dto.0.student_user_id,
            dto.0.course_id,
            dto.0.assignment_id,
            dto.0.id,
        )
        .await;
    }
    Ok(dto)
}
async fn release(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<SubmissionDto>, ApiError> {
    let dto = release_inner(&s.pool, &ctx, id).await?;
    // AFTER the release commit: notify the student their grade is released.
    // Best-effort — the release already succeeded; a notify failure is logged.
    notify_grade_released(
        &s,
        dto.0.student_user_id,
        dto.0.course_id,
        dto.0.assignment_id,
        dto.0.id,
    )
    .await;
    Ok(dto)
}

/// Best-effort "grade released" notification to the student. Looks up the
/// assignment title for a friendly body, builds a deep link to the submission,
/// and fans out via `state.email_notifier` + `state.push_sender` +
/// `state.pool` through the notify facade. NEVER returns/propagates an error.
async fn notify_grade_released(
    state: &AppState,
    student_user_id: Uuid,
    course_id: Uuid,
    assignment_id: Uuid,
    submission_id: Uuid,
) {
    // The release tx already committed; we need the tenant for the
    // tenant-scoped notifications/device_tokens writes. Resolve it from the
    // submission row under its own GUC-scoped tx (the grader's tenant context).
    // We re-read via the global users table for tenant via the submission's
    // tenant_id, which we fetch best-effort.
    let assignment_title = {
        // Read the assignment title best-effort (tenant GUC not strictly needed
        // for the title text, but we scope by the submission's tenant below).
        match sqlx::query_scalar::<_, String>("SELECT title FROM assignments WHERE id = $1")
            .bind(assignment_id)
            .fetch_optional(&state.pool)
            .await
        {
            Ok(Some(t)) => t,
            _ => "your assignment".to_string(),
        }
    };
    let tenant_id: Option<Uuid> =
        match sqlx::query_scalar::<_, Uuid>("SELECT tenant_id FROM submissions WHERE id = $1")
            .bind(submission_id)
            .fetch_optional(&state.pool)
            .await
        {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(?e, %submission_id, "notify_grade_released: tenant lookup failed");
                None
            }
        };
    let Some(tenant_id) = tenant_id else {
        return;
    };
    let title = "Grade released";
    let body = format!("Your grade for \"{assignment_title}\" has been released.");
    let link = format!(
        "{}/app/courses/{}/assignments/{}/submission",
        state.app_origin.trim_end_matches('/'),
        course_id,
        assignment_id
    );
    crate::services::notifications::notify(
        &state.pool,
        state.email_notifier.as_ref(),
        state.push_sender.as_ref(),
        tenant_id,
        student_user_id,
        "grade_released",
        title,
        Some(&body),
        Some(&link),
    )
    .await;
}
async fn return_resubmit(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<SubmissionDto>, ApiError> {
    return_inner(&s.pool, &ctx, id).await
}

// --- test wrappers ---
async fn create_or_get_t(
    State(ts): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(aid): Path<Uuid>,
) -> Result<Json<SubmissionDto>, ApiError> {
    create_or_get_inner(&ts.pool, &ctx, aid).await
}
async fn patch_t(
    State(ts): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    Json(body): Json<PatchSubmission>,
) -> Result<Json<SubmissionDto>, ApiError> {
    patch_inner(&ts.pool, &ctx, id, body).await
}
async fn submit_t(
    State(ts): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<SubmissionDto>, ApiError> {
    submit_inner(&ts.pool, &ctx, id).await
}
async fn list_for_assignment_t(
    State(ts): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(aid): Path<Uuid>,
) -> Result<Json<Vec<SubmissionDto>>, ApiError> {
    list_inner(&ts.pool, &ctx, aid).await
}
async fn get_one_t(
    State(ts): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<SubmissionDto>, ApiError> {
    get_one_inner(&ts.pool, &ctx, id).await
}
async fn grade_t(
    State(ts): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    Json(body): Json<GradeBody>,
) -> Result<Json<SubmissionDto>, ApiError> {
    grade_inner(&ts.pool, &ctx, id, body).await
}
async fn release_t(
    State(ts): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<SubmissionDto>, ApiError> {
    release_inner(&ts.pool, &ctx, id).await
}
async fn return_resubmit_t(
    State(ts): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<SubmissionDto>, ApiError> {
    return_inner(&ts.pool, &ctx, id).await
}
