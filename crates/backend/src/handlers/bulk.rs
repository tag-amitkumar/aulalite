// crates/backend/src/handlers/bulk.rs
//! Bulk roster + grade import for course staff.
//!
//! Routes (course-scoped, staff-only — mirror the gate in
//! `handlers::announcements::require_course_staff`):
//!   * POST /v1/courses/{cid}/bulk/enroll  — CSV of (email, role) rows. For each
//!     row we either directly enroll an existing tenant member into the course
//!     (reusing the role semantics of the single-invite accept path) or, when
//!     the email has no platform user / tenant seat yet, create a pending
//!     `course_invitations` row + email (reusing
//!     `db::enrollments::insert_invitation` + the EmailLinkSender), exactly like
//!     `handlers::enrollments::create_invitation_inner`.
//!   * POST /v1/courses/{cid}/bulk/grades  — CSV of
//!     (student_email_or_id, assignment_id_or_title, grade) rows. Each row is
//!     applied through the same grade write the single-grade path uses
//!     (`db::submissions::save_grade`) honoring the assignment's grading_mode,
//!     max_points, late-penalty, and release_mode.
//!
//! Both endpoints accept either a raw `text/csv` body OR a JSON body
//! `{ "csv": "<...>" }`, cap the row count at `MAX_ROWS`, and return a per-row
//! result summary (`created` / `invited` / `enrolled` / `graded` / `skipped` /
//! `error`) so the importer can render a results table.
//!
//! CSV is hand-parsed (no `csv` crate) with RFC-4180-ish quote handling: fields
//! may be double-quoted, quotes inside a quoted field are doubled (`""`), and
//! commas/newlines inside quotes are literal. A leading header row whose first
//! cell looks like a header (`email` / `student`) is skipped.

use axum::extract::{Extension, Path, State};
use axum::{routing, Json, Router};
use serde::{Deserialize, Serialize};
use sqlx::types::BigDecimal;
use std::str::FromStr;
use uuid::Uuid;

use crate::context::RequestContext;
use crate::db;
use crate::error::ApiError;
use crate::AppState;

/// Hard cap on rows processed per request (excludes a skipped header row).
const MAX_ROWS: usize = 1000;
const VALID_ROLES: &[&str] = &["teacher", "ta", "student"];

// ---------------------------------------------------------------------------
// Request / response DTOs
// ---------------------------------------------------------------------------

/// JSON variant of the request body. The endpoints also accept a raw
/// `text/csv` body; see `extract_csv`.
#[derive(Deserialize)]
pub struct CsvBody {
    pub csv: String,
}

#[derive(Serialize)]
pub struct RowResult {
    /// 1-based line number within the CSV (after any header skip), for display.
    pub row: usize,
    /// One of: created | invited | enrolled | graded | skipped | error.
    pub outcome: String,
    /// The primary identifier echoed back (email or student id) for the row.
    pub subject: String,
    /// Human-readable detail (why skipped / what error / what was applied).
    pub detail: String,
}

#[derive(Serialize)]
pub struct BulkSummary {
    pub total: usize,
    pub succeeded: usize,
    pub skipped: usize,
    pub errored: usize,
    pub results: Vec<RowResult>,
}

impl BulkSummary {
    fn from_rows(results: Vec<RowResult>) -> Self {
        let mut succeeded = 0;
        let mut skipped = 0;
        let mut errored = 0;
        for r in &results {
            match r.outcome.as_str() {
                "skipped" => skipped += 1,
                "error" => errored += 1,
                _ => succeeded += 1,
            }
        }
        BulkSummary {
            total: results.len(),
            succeeded,
            skipped,
            errored,
            results,
        }
    }
}

// ---------------------------------------------------------------------------
// Routes + auth gate
// ---------------------------------------------------------------------------

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/courses/{cid}/bulk/enroll", routing::post(enroll))
        .route("/v1/courses/{cid}/bulk/grades", routing::post(grades))
}

fn is_org_admin(ctx: &RequestContext) -> bool {
    ctx.can_manage_organization()
}

/// Course-scoped staff gate (course owner / active teacher-ta member / org_admin
/// / platform_admin). `platform_admin` always passes. Mirrors
/// `handlers::announcements::require_course_staff`.
async fn require_course_staff(
    state: &AppState,
    ctx: &RequestContext,
    course_id: Uuid,
) -> Result<(), ApiError> {
    if ctx.can_manage_organization() {
        return Ok(());
    }
    if !db::courses::caller_can_staff_course(
        &state.pool,
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

/// Accept the request body either as raw `text/csv` (any non-JSON content type)
/// or as JSON `{ "csv": "..." }`. We sniff the leading byte: a JSON object body
/// starts with `{`. Everything else is treated as raw CSV text.
fn extract_csv(content_type: Option<&str>, body: &str) -> Result<String, ApiError> {
    let looks_json = content_type
        .map(|c| c.contains("application/json"))
        .unwrap_or(false)
        || body.trim_start().starts_with('{');
    if looks_json {
        let parsed: CsvBody = serde_json::from_str(body)
            .map_err(|e| ApiError::BadRequest(format!("invalid JSON body: {e}")))?;
        Ok(parsed.csv)
    } else {
        Ok(body.to_string())
    }
}

// ---------------------------------------------------------------------------
// CSV parsing (hand-rolled, RFC-4180-ish)
// ---------------------------------------------------------------------------

/// Parse `input` into rows of string fields. Handles double-quoted fields with
/// embedded commas/newlines and doubled `""` escapes. Blank lines are dropped.
/// Pure + unit-tested.
fn parse_csv(input: &str) -> Vec<Vec<String>> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut field = String::new();
    let mut record: Vec<String> = Vec::new();
    let mut in_quotes = false;
    let mut field_started_quoted = false;
    let mut any_field_on_record = false;

    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if in_quotes {
            if c == '"' {
                // Doubled quote => literal quote; otherwise close the quote.
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                } else {
                    in_quotes = false;
                }
            } else {
                field.push(c);
            }
            continue;
        }
        match c {
            '"' if field.is_empty() && !field_started_quoted => {
                in_quotes = true;
                field_started_quoted = true;
                any_field_on_record = true;
            }
            ',' => {
                record.push(std::mem::take(&mut field));
                field_started_quoted = false;
                any_field_on_record = true;
            }
            '\r' => { /* swallow; handled by the following \n or EOF */ }
            '\n' => {
                if any_field_on_record || !field.is_empty() {
                    record.push(std::mem::take(&mut field));
                    rows.push(std::mem::take(&mut record));
                } else {
                    // Truly blank line — drop it.
                    record.clear();
                }
                field_started_quoted = false;
                any_field_on_record = false;
            }
            _ => {
                field.push(c);
                any_field_on_record = true;
            }
        }
    }
    // Flush the trailing record (no final newline).
    if any_field_on_record || !field.is_empty() {
        record.push(field);
        rows.push(record);
    }
    rows
}

/// Drop a leading header row when its first cell looks like a header label.
/// Returns the data rows. Pure + unit-tested.
fn strip_header(rows: Vec<Vec<String>>, header_hints: &[&str]) -> Vec<Vec<String>> {
    if let Some(first) = rows.first() {
        if let Some(cell0) = first.first() {
            let c = cell0.trim().to_ascii_lowercase();
            if header_hints.iter().any(|h| c == *h) {
                return rows.into_iter().skip(1).collect();
            }
        }
    }
    rows
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn enroll(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(cid): Path<Uuid>,
    headers: axum::http::HeaderMap,
    body: String,
) -> Result<Json<BulkSummary>, ApiError> {
    let content_type = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok());
    let csv = extract_csv(content_type, &body)?;
    enroll_inner(&s, &ctx, cid, &csv).await
}

async fn grades(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(cid): Path<Uuid>,
    headers: axum::http::HeaderMap,
    body: String,
) -> Result<Json<BulkSummary>, ApiError> {
    let content_type = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok());
    let csv = extract_csv(content_type, &body)?;
    grades_inner(&s, &ctx, cid, &csv).await
}

async fn enroll_inner(
    s: &AppState,
    ctx: &RequestContext,
    course_id: Uuid,
    csv: &str,
) -> Result<Json<BulkSummary>, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    if !ctx.can_teach() {
        return Err(ApiError::Forbidden);
    }
    require_course_staff(s, ctx, course_id).await?;

    let rows = strip_header(parse_csv(csv), &["email", "e-mail"]);
    if rows.len() > MAX_ROWS {
        return Err(ApiError::Validation(format!(
            "too_many_rows: {} (max {})",
            rows.len(),
            MAX_ROWS
        )));
    }

    let mut results: Vec<RowResult> = Vec::with_capacity(rows.len());
    for (i, cols) in rows.iter().enumerate() {
        let row = i + 1;
        let email = cols.first().map(|c| c.trim()).unwrap_or("").to_string();
        // role defaults to "student" when the column is missing/empty.
        let role_raw = cols.get(1).map(|c| c.trim()).unwrap_or("");
        let role = if role_raw.is_empty() {
            "student".to_string()
        } else {
            role_raw.to_ascii_lowercase()
        };

        if email.is_empty() {
            results.push(RowResult {
                row,
                outcome: "skipped".into(),
                subject: String::new(),
                detail: "empty email".into(),
            });
            continue;
        }
        if !email.contains('@') {
            results.push(RowResult {
                row,
                outcome: "error".into(),
                subject: email,
                detail: "invalid email".into(),
            });
            continue;
        }
        if !VALID_ROLES.contains(&role.as_str()) {
            results.push(RowResult {
                row,
                outcome: "error".into(),
                subject: email,
                detail: format!("invalid role: {role}"),
            });
            continue;
        }

        match enroll_one(s, ctx, tenant, course_id, &email, &role).await {
            Ok(r) => results.push(RowResult { row, ..r }),
            Err(e) => results.push(RowResult {
                row,
                outcome: "error".into(),
                subject: email,
                detail: e.user_facing_detail(),
            }),
        }
    }

    Ok(Json(BulkSummary::from_rows(results)))
}

/// Process one enroll row. Returns a `RowResult` with `row` left at 0 (the
/// caller sets the real row index).
async fn enroll_one(
    s: &AppState,
    ctx: &RequestContext,
    tenant: Uuid,
    course_id: Uuid,
    email: &str,
    role: &str,
) -> Result<RowResult, ApiError> {
    // Does this email belong to an existing platform user with a tenant seat?
    let mut tx = s
        .pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let resolved = db::bulk::resolve_user_by_email(&mut tx, tenant, email)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let resolved = match resolved {
        db::bulk::UserResolution::Ambiguous(_) => {
            tx.rollback().await.ok();
            return Err(ApiError::Conflict(
                "multiple workspace members use this email; import by user id".into(),
            ));
        }
        db::bulk::UserResolution::One(user) => Some(user),
        db::bulk::UserResolution::None => None,
    };

    if let Some(user) = resolved.filter(|u| u.is_active_tenant_member) {
        // Existing seat-holder: enroll directly, mirroring the single-invite
        // accept path's role-aware course-membership upsert.
        let existing = db::bulk::course_membership_status(&mut tx, tenant, course_id, user.user_id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        if existing.as_deref() == Some("active") {
            tx.rollback().await.ok();
            return Ok(RowResult {
                row: 0,
                outcome: "skipped".into(),
                subject: email.to_string(),
                detail: "already an active member".into(),
            });
        }
        db::bulk::upsert_course_membership(&mut tx, tenant, course_id, user.user_id, role)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        db::audit::emit_audit_event(
            &mut tx,
            tenant,
            ctx.user_id,
            "bulk_enroll.enrolled",
            "course_membership",
            user.user_id,
            Some(serde_json::json!({ "course_id": course_id, "role": role })),
        )
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
        tx.commit()
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        return Ok(RowResult {
            row: 0,
            outcome: "enrolled".into(),
            subject: email.to_string(),
            detail: format!("enrolled as {role}"),
        });
    }

    // No seat yet: create (or no-op against) a pending course invitation +
    // send the email link, exactly like create_invitation_inner.
    let token = db::enrollments::generate_invitation_token();
    let expires_at = chrono::Utc::now() + chrono::Duration::days(14);
    db::enrollments::lazy_expire_stale_pending(&mut tx, course_id, email)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let insert = db::enrollments::insert_invitation(
        &mut tx,
        tenant,
        course_id,
        email,
        role,
        &token,
        expires_at,
        ctx.user_id,
    )
    .await;
    let invitation_id = match insert {
        Ok(id) => id,
        Err(sqlx::Error::Database(dbe)) if dbe.is_unique_violation() => {
            // A pending invite already exists for this (course, email).
            tx.rollback().await.ok();
            return Ok(RowResult {
                row: 0,
                outcome: "skipped".into(),
                subject: email.to_string(),
                detail: "invite already pending".into(),
            });
        }
        Err(e) => return Err(ApiError::Internal(e.to_string())),
    };
    db::audit::emit_audit_event(
        &mut tx,
        tenant,
        ctx.user_id,
        "bulk_enroll.invited",
        "course_invitation",
        invitation_id,
        Some(serde_json::json!({ "course_id": course_id, "role": role })),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    // Best-effort email (the invite is already committed; a send failure must
    // not fail the row — surface it as a non-fatal detail instead).
    let continue_url = format!(
        "{}/accept-invite/{}",
        s.app_origin.trim_end_matches('/'),
        token
    );
    match s.email_link_sender.send_invite(email, &continue_url).await {
        Ok(()) => Ok(RowResult {
            row: 0,
            outcome: "invited".into(),
            subject: email.to_string(),
            detail: format!("invited as {role}"),
        }),
        Err(e) => Ok(RowResult {
            row: 0,
            outcome: "invited".into(),
            subject: email.to_string(),
            detail: format!("invited as {role} (email send failed: {e})"),
        }),
    }
}

async fn grades_inner(
    s: &AppState,
    ctx: &RequestContext,
    course_id: Uuid,
    csv: &str,
) -> Result<Json<BulkSummary>, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    if !ctx.can_grade() {
        return Err(ApiError::Forbidden);
    }
    require_course_staff(s, ctx, course_id).await?;

    let rows = strip_header(parse_csv(csv), &["student", "student_email", "email"]);
    if rows.len() > MAX_ROWS {
        return Err(ApiError::Validation(format!(
            "too_many_rows: {} (max {})",
            rows.len(),
            MAX_ROWS
        )));
    }

    let mut results: Vec<RowResult> = Vec::with_capacity(rows.len());
    for (i, cols) in rows.iter().enumerate() {
        let row = i + 1;
        let student = cols.first().map(|c| c.trim()).unwrap_or("").to_string();
        let assignment_ref = cols.get(1).map(|c| c.trim()).unwrap_or("").to_string();
        let grade_raw = cols.get(2).map(|c| c.trim()).unwrap_or("").to_string();

        if student.is_empty() && assignment_ref.is_empty() && grade_raw.is_empty() {
            results.push(RowResult {
                row,
                outcome: "skipped".into(),
                subject: String::new(),
                detail: "empty row".into(),
            });
            continue;
        }
        if student.is_empty() || assignment_ref.is_empty() || grade_raw.is_empty() {
            results.push(RowResult {
                row,
                outcome: "error".into(),
                subject: student,
                detail: "expected: student, assignment, grade".into(),
            });
            continue;
        }

        match grade_one(
            s,
            ctx,
            tenant,
            course_id,
            &student,
            &assignment_ref,
            &grade_raw,
        )
        .await
        {
            Ok(r) => results.push(RowResult { row, ..r }),
            Err(e) => results.push(RowResult {
                row,
                outcome: "error".into(),
                subject: student,
                detail: e.user_facing_detail(),
            }),
        }
    }

    Ok(Json(BulkSummary::from_rows(results)))
}

/// Process one grade row. Returns a `RowResult` with `row` left at 0.
async fn grade_one(
    s: &AppState,
    ctx: &RequestContext,
    tenant: Uuid,
    course_id: Uuid,
    student: &str,
    assignment_ref: &str,
    grade_raw: &str,
) -> Result<RowResult, ApiError> {
    let mut tx = s
        .pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    // Resolve the student: a UUID is taken literally; anything else is an email.
    let student_user_id = match Uuid::parse_str(student) {
        Ok(id) => id,
        Err(_) => {
            let resolved = db::bulk::resolve_user_by_email(&mut tx, tenant, student)
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?;
            match resolved {
                db::bulk::UserResolution::One(u) if u.is_active_tenant_member => u.user_id,
                db::bulk::UserResolution::Ambiguous(_) => {
                    tx.rollback().await.ok();
                    return Ok(RowResult {
                        row: 0,
                        outcome: "error".into(),
                        subject: student.to_string(),
                        detail: "multiple active members use that email; use a user id".into(),
                    });
                }
                _ => {
                    tx.rollback().await.ok();
                    return Ok(RowResult {
                        row: 0,
                        outcome: "error".into(),
                        subject: student.to_string(),
                        detail: "no active workspace member with that email".into(),
                    });
                }
            }
        }
    };

    // Resolve the assignment: a UUID is taken literally (and verified to be in
    // this course); anything else is a published-title lookup.
    let assignment_id = match Uuid::parse_str(assignment_ref) {
        Ok(id) => id,
        Err(_) => {
            match db::bulk::find_assignment_by_title(&mut tx, tenant, course_id, assignment_ref)
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?
            {
                db::bulk::TitleMatch::One(id) => id,
                db::bulk::TitleMatch::None => {
                    tx.rollback().await.ok();
                    return Ok(RowResult {
                        row: 0,
                        outcome: "error".into(),
                        subject: student.to_string(),
                        detail: format!("no assignment titled \"{assignment_ref}\""),
                    });
                }
                db::bulk::TitleMatch::Ambiguous(n) => {
                    tx.rollback().await.ok();
                    return Ok(RowResult {
                        row: 0,
                        outcome: "error".into(),
                        subject: student.to_string(),
                        detail: format!("ambiguous title \"{assignment_ref}\" ({n} matches)"),
                    });
                }
            }
        }
    };

    // Set the tenant GUC for the remaining writes in THIS tx (the resolve_* /
    // find_* helpers set it too, but make it explicit before save_grade).
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let assn = db::assignments::fetch_by_id(&mut tx, assignment_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::NotFound)?;
    if assn.course_id != course_id {
        tx.rollback().await.ok();
        return Ok(RowResult {
            row: 0,
            outcome: "error".into(),
            subject: student.to_string(),
            detail: "assignment not in this course".into(),
        });
    }

    // Find or create the submission for this (assignment, student), then apply
    // the grade through the same save_grade write the single-grade path uses.
    let submission = db::bulk::find_submission(&mut tx, tenant, assignment_id, student_user_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let (submission_id, sub_is_late, submitted_at) = match submission {
        Some((id, _course, _status)) => {
            // Re-read the full row for late/submitted_at penalty math.
            let full = db::submissions::fetch_by_id(&mut tx, id)
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?
                .ok_or(ApiError::NotFound)?;
            (id, full.is_late, full.submitted_at)
        }
        None => {
            // No submission yet: create one so the grade has a row to attach to
            // (mirrors create_or_get_inner's upsert_for_student).
            let created = db::submissions::upsert_for_student(
                &mut tx,
                tenant,
                assignment_id,
                course_id,
                student_user_id,
            )
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
            (created.id, created.is_late, created.submitted_at)
        }
    };

    // Build GradeFields the same way grade_inner does, honoring grading_mode.
    let mut effective_numeric: Option<f64> = None;
    let mut applied_late_penalty_percent: i32 = 0;
    let mut letter_for_pass: Option<bool> = None;
    let detail: String;

    match assn.grading_mode.as_str() {
        "numeric" => {
            let n: f64 = grade_raw
                .parse()
                .map_err(|_| ApiError::Validation(format!("grade not numeric: {grade_raw}")))?;
            let max = assn.max_points.unwrap_or(0);
            if n < 0.0 || n > max as f64 {
                return Err(ApiError::Validation(format!(
                    "grade {n} out of range 0..={max}"
                )));
            }
            let submitted_late = sub_is_late
                || match (submitted_at, assn.due_at) {
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
            detail = format!("graded {n}/{max}");
        }
        "pass_fail" => {
            let p = parse_pass_fail(grade_raw)
                .ok_or_else(|| ApiError::Validation(format!("grade not pass/fail: {grade_raw}")))?;
            letter_for_pass = Some(p);
            detail = format!("graded {}", if p { "pass" } else { "fail" });
        }
        other => return Err(ApiError::Internal(format!("bad grading_mode: {other}"))),
    }

    let release_now = assn.release_mode == "instant";
    let numeric = effective_numeric.and_then(|f| BigDecimal::from_str(&f.to_string()).ok());
    let updated = db::submissions::save_grade(
        &mut tx,
        submission_id,
        db::submissions::GradeFields {
            numeric_grade: numeric,
            letter_grade: None,
            passed: letter_for_pass,
            student_visible_feedback: None,
            teacher_only_notes: None,
            grader_id: ctx.user_id,
            release_now,
            applied_late_penalty_percent,
        },
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    db::audit::emit_audit_event(
        &mut tx,
        tenant,
        ctx.user_id,
        "bulk_grade.graded",
        "submission",
        updated.id,
        Some(serde_json::json!({
            "assignment_id": assignment_id,
            "grading_mode": assn.grading_mode,
            "applied_late_penalty_percent": applied_late_penalty_percent,
            "released_now": release_now,
        })),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    // Instant-release makes the grade student-visible now: notify best-effort,
    // mirroring the grade-released path in handlers::submissions. The grade tx
    // already committed; a notify failure must never fail the row.
    if updated.released_at.is_some() {
        notify_grade_released(s, tenant, &assn.title, &updated).await;
    }

    Ok(RowResult {
        row: 0,
        outcome: "graded".into(),
        subject: student.to_string(),
        detail,
    })
}

/// Parse a pass/fail cell. Accepts pass/fail, p/f, true/false, yes/no, 1/0.
fn parse_pass_fail(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "pass" | "p" | "true" | "yes" | "y" | "1" => Some(true),
        "fail" | "f" | "false" | "no" | "n" | "0" => Some(false),
        _ => None,
    }
}

/// Best-effort "grade released" notification, mirroring the title/body/link the
/// single-grade release path builds in `handlers::submissions`. NEVER errors.
async fn notify_grade_released(
    s: &AppState,
    tenant_id: Uuid,
    assignment_title: &str,
    updated: &db::submissions::SubmissionRow,
) {
    let title = "Grade released";
    let body = format!("Your grade for \"{assignment_title}\" has been released.");
    let link = format!(
        "{}/app/courses/{}/assignments/{}/submission",
        s.app_origin.trim_end_matches('/'),
        updated.course_id,
        updated.assignment_id
    );
    crate::services::notifications::notify(
        &s.pool,
        s.email_notifier.as_ref(),
        s.push_sender.as_ref(),
        tenant_id,
        updated.student_user_id,
        "grade_released",
        title,
        Some(&body),
        Some(&link),
    )
    .await;
}

// ---------------------------------------------------------------------------
// Small error-detail helper
// ---------------------------------------------------------------------------

impl ApiError {
    /// A row-level detail string for the bulk results table. For validation /
    /// bad-request errors we surface the inner reason (it is user-authored CSV
    /// data, not a server internal); for everything else we use the stable
    /// `user_facing` label so server internals never leak.
    fn user_facing_detail(&self) -> String {
        match self {
            ApiError::Validation(r) | ApiError::BadRequest(r) => r.clone(),
            other => other.user_facing(),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_csv_basic() {
        let rows = parse_csv("a@x.com,student\nb@x.com,teacher");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], vec!["a@x.com", "student"]);
        assert_eq!(rows[1], vec!["b@x.com", "teacher"]);
    }

    #[test]
    fn parse_csv_quoted_field_with_comma_and_quote() {
        let rows = parse_csv("\"Doe, Jane\",\"she said \"\"hi\"\"\",3");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0], vec!["Doe, Jane", "she said \"hi\"", "3"]);
    }

    #[test]
    fn parse_csv_crlf_and_trailing_no_newline() {
        let rows = parse_csv("a,1\r\nb,2");
        assert_eq!(rows, vec![vec!["a", "1"], vec!["b", "2"]]);
    }

    #[test]
    fn parse_csv_drops_blank_lines() {
        let rows = parse_csv("a,1\n\n\nb,2\n");
        assert_eq!(rows, vec![vec!["a", "1"], vec!["b", "2"]]);
    }

    #[test]
    fn parse_csv_quoted_newline_inside_field() {
        let rows = parse_csv("\"line1\nline2\",x");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0], vec!["line1\nline2", "x"]);
    }

    #[test]
    fn strip_header_removes_when_first_cell_is_hint() {
        let rows = vec![
            vec!["email".to_string(), "role".to_string()],
            vec!["a@x.com".to_string(), "student".to_string()],
        ];
        let data = strip_header(rows, &["email"]);
        assert_eq!(data.len(), 1);
        assert_eq!(data[0][0], "a@x.com");
    }

    #[test]
    fn strip_header_keeps_when_no_hint() {
        let rows = vec![vec!["a@x.com".to_string(), "student".to_string()]];
        let data = strip_header(rows.clone(), &["email"]);
        assert_eq!(data, rows);
    }

    #[test]
    fn extract_csv_json_vs_raw() {
        let raw = extract_csv(Some("text/csv"), "a@x.com,student").unwrap();
        assert_eq!(raw, "a@x.com,student");
        let json = extract_csv(Some("application/json"), "{\"csv\":\"a@x.com,student\"}").unwrap();
        assert_eq!(json, "a@x.com,student");
        // Content-type lies (says csv) but body is a JSON object — sniff it.
        let sniffed = extract_csv(Some("text/csv"), "  {\"csv\":\"x\"}").unwrap();
        assert_eq!(sniffed, "x");
    }

    #[test]
    fn parse_pass_fail_variants() {
        assert_eq!(parse_pass_fail("pass"), Some(true));
        assert_eq!(parse_pass_fail("F"), Some(false));
        assert_eq!(parse_pass_fail("Yes"), Some(true));
        assert_eq!(parse_pass_fail("0"), Some(false));
        assert_eq!(parse_pass_fail("maybe"), None);
    }

    #[test]
    fn summary_counts() {
        let results = vec![
            RowResult {
                row: 1,
                outcome: "enrolled".into(),
                subject: "a".into(),
                detail: String::new(),
            },
            RowResult {
                row: 2,
                outcome: "invited".into(),
                subject: "b".into(),
                detail: String::new(),
            },
            RowResult {
                row: 3,
                outcome: "skipped".into(),
                subject: "c".into(),
                detail: String::new(),
            },
            RowResult {
                row: 4,
                outcome: "error".into(),
                subject: "d".into(),
                detail: String::new(),
            },
        ];
        let s = BulkSummary::from_rows(results);
        assert_eq!(s.total, 4);
        assert_eq!(s.succeeded, 2);
        assert_eq!(s.skipped, 1);
        assert_eq!(s.errored, 1);
    }
}
