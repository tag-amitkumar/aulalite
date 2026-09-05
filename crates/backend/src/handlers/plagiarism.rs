// crates/backend/src/handlers/plagiarism.rs
//! Plagiarism similarity report for an assignment's submissions.
//!
//! Compares every submission's text answer against the OTHER submissions of the
//! SAME assignment using the pure shingle-fingerprint similarity in
//! `services::plagiarism`. Staff-gated end to end.
//!
//! Routes (all under the authed router):
//!   * GET  /v1/assignments/{id}/plagiarism            — pairs with similarity
//!         >= `min_score` (default 0.3), each with the two student ids + score,
//!         sorted by score descending. Staff-only.
//!   * POST /v1/assignments/{id}/plagiarism/recompute   — (re)build fingerprints
//!         for every submission of the assignment from its current text answer.
//!         Staff-only. Returns how many fingerprints were written.
//!
//! Fingerprints are normally kept fresh by a best-effort hook in the submission
//! handler (see `db::plagiarism::upsert_fingerprint`); recompute is the
//! authoritative rebuild for backfills or after bulk edits.
use axum::extract::{Extension, Path, Query, State};
use axum::{routing, Json, Router};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::context::RequestContext;
use crate::db;
use crate::error::ApiError;
use crate::services::plagiarism;
use crate::AppState;

/// Default similarity threshold for a pair to appear in the report.
const DEFAULT_MIN_SCORE: f64 = 0.3;

#[derive(Deserialize)]
pub struct ReportQuery {
    /// Minimum similarity (0.0..=1.0) for a pair to be reported. Clamped to the
    /// valid range; defaults to `DEFAULT_MIN_SCORE`.
    pub min_score: Option<f64>,
}

#[derive(Serialize)]
pub struct PairDto {
    pub submission_a_id: Uuid,
    pub student_a_id: Uuid,
    pub submission_b_id: Uuid,
    pub student_b_id: Uuid,
    /// Similarity in `0.0..=1.0` (max of Jaccard / containment).
    pub score: f64,
}

#[derive(Serialize)]
pub struct ReportDto {
    pub assignment_id: Uuid,
    pub min_score: f64,
    /// Number of submissions that had a stored fingerprint and were compared.
    pub compared: usize,
    pub pairs: Vec<PairDto>,
}

#[derive(Serialize)]
pub struct RecomputeDto {
    pub assignment_id: Uuid,
    /// How many submissions had a non-empty text answer and were fingerprinted.
    pub fingerprinted: usize,
    /// How many submissions were skipped (no/empty text answer).
    pub skipped: usize,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/assignments/{id}/plagiarism", routing::get(report))
        .route(
            "/v1/assignments/{id}/plagiarism/recompute",
            routing::post(recompute),
        )
}

fn is_org_admin(ctx: &RequestContext) -> bool {
    ctx.can_manage_organization()
}

/// Course-scoped staff gate (course owner / active teacher-ta member / org_admin
/// / platform_admin). `platform_admin` always passes.
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

/// Resolve the assignment's course (under RLS) so we can staff-gate, returning
/// the course id. 404 if the assignment doesn't exist in this tenant.
async fn assignment_course(
    state: &AppState,
    ctx: &RequestContext,
    tenant: Uuid,
    assignment_id: Uuid,
) -> Result<Uuid, ApiError> {
    let mut tx = db::begin_with_context(&state.pool, ctx.user_id, Some(tenant))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let assn = db::assignments::fetch_by_id(&mut tx, assignment_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::NotFound)?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(assn.course_id)
}

async fn report(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(assignment_id): Path<Uuid>,
    Query(q): Query<ReportQuery>,
) -> Result<Json<ReportDto>, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    let course_id = assignment_course(&s, &ctx, tenant, assignment_id).await?;
    require_course_staff(&s, &ctx, course_id).await?;

    let min_score = q.min_score.unwrap_or(DEFAULT_MIN_SCORE).clamp(0.0, 1.0);

    let fps = db::plagiarism::list_fingerprints_for_assignment(&s.pool, tenant, assignment_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let mut pairs: Vec<PairDto> = Vec::new();
    for i in 0..fps.len() {
        for j in (i + 1)..fps.len() {
            let a = &fps[i];
            let b = &fps[j];
            // Two submissions from the same student would only happen if data
            // drifted; skip self-pairs defensively.
            if a.student_user_id == b.student_user_id {
                continue;
            }
            let score = plagiarism::similarity(&a.shingles, &b.shingles);
            if score >= min_score {
                pairs.push(PairDto {
                    submission_a_id: a.submission_id,
                    student_a_id: a.student_user_id,
                    submission_b_id: b.submission_id,
                    student_b_id: b.student_user_id,
                    score,
                });
            }
        }
    }
    // Highest similarity first; tie-break stably on the submission ids so the
    // ordering is deterministic across calls.
    pairs.sort_by(|x, y| {
        y.score
            .partial_cmp(&x.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| x.submission_a_id.cmp(&y.submission_a_id))
            .then_with(|| x.submission_b_id.cmp(&y.submission_b_id))
    });

    Ok(Json(ReportDto {
        assignment_id,
        min_score,
        compared: fps.len(),
        pairs,
    }))
}

async fn recompute(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(assignment_id): Path<Uuid>,
) -> Result<Json<RecomputeDto>, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    let course_id = assignment_course(&s, &ctx, tenant, assignment_id).await?;
    require_course_staff(&s, &ctx, course_id).await?;

    // One tx for the whole rebuild: read every submission, fingerprint the ones
    // with text, and upsert. All under the tenant GUC for RLS.
    let mut tx = db::begin_with_context(&s.pool, ctx.user_id, Some(tenant))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let submissions = db::submissions::list_for_assignment(&mut tx, assignment_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let mut fingerprinted = 0usize;
    let mut skipped = 0usize;
    for sub in &submissions {
        let text = sub.text_answer.as_deref().unwrap_or("");
        let fp = plagiarism::fingerprint(text);
        if fp.is_empty() {
            skipped += 1;
            continue;
        }
        db::plagiarism::upsert_fingerprint_tx(
            &mut tx,
            tenant,
            sub.id,
            assignment_id,
            sub.student_user_id,
            &fp,
        )
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
        fingerprinted += 1;
    }

    db::audit::emit_audit_event(
        &mut tx,
        tenant,
        ctx.user_id,
        "plagiarism.recompute",
        "assignment",
        assignment_id,
        Some(serde_json::json!({
            "fingerprinted": fingerprinted,
            "skipped": skipped,
        })),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;

    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(RecomputeDto {
        assignment_id,
        fingerprinted,
        skipped,
    }))
}
