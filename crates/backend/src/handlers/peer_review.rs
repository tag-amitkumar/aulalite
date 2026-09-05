// crates/backend/src/handlers/peer_review.rs
//! Assignment peer-review: staff enable peer review + allocate reviewers;
//! students see their queue and file reviews; authors see received (optionally
//! anonymized) reviews; staff see an aggregate.
//!
//! Routes (all under the authed router):
//!   * PUT  /v1/assignments/{id}/peer-review            — config (staff)
//!   * POST /v1/assignments/{id}/peer-review/allocate   — round-robin (staff)
//!   * GET  /v1/assignments/{id}/peer-review/mine       — reviewer queue (student)
//!   * POST /v1/peer-review/allocations/{id}            — submit a review (reviewer)
//!   * GET  /v1/assignments/{id}/peer-review/received   — author's received reviews
//!   * GET  /v1/assignments/{id}/peer-review/summary    — staff aggregate
//!
//! Allocation is a deterministic round-robin: each eligible (turned-in)
//! submission is assigned to N distinct reviewers drawn from the OTHER authors,
//! never the submission's own author. Everything is tenant-scoped under RLS (see
//! `db::peer_review`). Reuses the existing rubric (`rubric_id`) for scoring; the
//! actual scores are a free-form `scores_json` the handler validates lightly.
use axum::extract::{Extension, Path, State};
use axum::{routing, Json, Router};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

use crate::context::RequestContext;
use crate::db;
use crate::error::ApiError;
use crate::AppState;

const MAX_REVIEWS_PER_STUDENT: i32 = 10;
const MAX_COMMENT_LEN: usize = 10_000;

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct ConfigInput {
    pub reviews_per_student: i32,
    #[serde(default)]
    pub rubric_id: Option<Uuid>,
    #[serde(default)]
    pub anonymous: bool,
    #[serde(default)]
    pub due_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Serialize)]
pub struct ConfigDto {
    pub assignment_id: Uuid,
    pub course_id: Uuid,
    pub reviews_per_student: i32,
    pub rubric_id: Option<Uuid>,
    pub anonymous: bool,
    pub due_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl From<db::peer_review::PeerReviewConfigRow> for ConfigDto {
    fn from(r: db::peer_review::PeerReviewConfigRow) -> Self {
        Self {
            assignment_id: r.assignment_id,
            course_id: r.course_id,
            reviews_per_student: r.reviews_per_student,
            rubric_id: r.rubric_id,
            anonymous: r.anonymous,
            due_at: r.due_at,
        }
    }
}

#[derive(Serialize)]
pub struct AllocateResultDto {
    pub allocations_created: i64,
    pub eligible_submissions: i64,
    pub reviews_per_student: i32,
}

#[derive(Deserialize)]
pub struct SubmitReviewInput {
    /// Free-form per-criterion / numeric scores. Validated against the rubric's
    /// criteria when a rubric is configured; otherwise a plain object.
    pub scores: serde_json::Value,
    #[serde(default)]
    pub comment_md: String,
}

/// One allocation as seen by a reviewer / author / staff. `reviewer_user_id` is
/// `None` when the config is anonymous and the caller is the reviewed author.
#[derive(Serialize)]
pub struct AllocationDto {
    pub id: Uuid,
    pub assignment_id: Uuid,
    pub submission_id: Uuid,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reviewer_user_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author_user_id: Option<Uuid>,
    pub submitted_at: Option<chrono::DateTime<chrono::Utc>>,
    pub scores: Option<serde_json::Value>,
    pub comment_md: Option<String>,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/assignments/{id}/peer-review", routing::put(put_config))
        .route(
            "/v1/assignments/{id}/peer-review/allocate",
            routing::post(allocate),
        )
        .route(
            "/v1/assignments/{id}/peer-review/mine",
            routing::get(my_queue),
        )
        .route(
            "/v1/peer-review/allocations/{id}",
            routing::post(submit_review),
        )
        .route(
            "/v1/assignments/{id}/peer-review/received",
            routing::get(received),
        )
        .route(
            "/v1/assignments/{id}/peer-review/summary",
            routing::get(summary),
        )
}

// ---------------------------------------------------------------------------
// Authorization helpers (mirror handlers::announcements / rubrics)
// ---------------------------------------------------------------------------

fn is_org_admin(ctx: &RequestContext) -> bool {
    ctx.can_manage_organization()
}

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

async fn require_course_read(
    state: &AppState,
    ctx: &RequestContext,
    course_id: Uuid,
) -> Result<(), ApiError> {
    if ctx.can_manage_organization() {
        return Ok(());
    }
    if !db::courses::caller_can_read_course(
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

/// Resolve an assignment's course id (404 if missing), tenant-scoped under RLS.
async fn assignment_course_id(
    state: &AppState,
    tenant: Uuid,
    user_id: Uuid,
    assignment_id: Uuid,
) -> Result<Uuid, ApiError> {
    let mut tx = db::begin_with_context(&state.pool, user_id, Some(tenant))
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

// ---------------------------------------------------------------------------
// Round-robin allocator (pure — unit-tested)
// ---------------------------------------------------------------------------

/// Build the (reviewer -> [submission]) plan: assign each submission to
/// `reviews_per_student` distinct reviewers chosen from the OTHER authors, never
/// the submission's own author. Deterministic given the input order.
///
/// Strategy: order authors stably; for each submission, walk the author ring
/// starting just after the author's index and pick the next `n` distinct authors
/// (skipping the author themselves). With `k` authors, at most `k-1` reviewers
/// can be assigned per submission, so `n` is implicitly capped at `k-1`.
///
/// Returns the flat allocation list. Empty when fewer than 2 authors exist (no
/// peer can review).
fn build_allocations(
    submissions: &[db::peer_review::EligibleSubmission],
    reviews_per_student: i32,
) -> Vec<db::peer_review::NewAllocation> {
    // Distinct authors, in first-seen order, each mapped to a ring index.
    let mut authors: Vec<Uuid> = Vec::new();
    let mut seen: HashSet<Uuid> = HashSet::new();
    for s in submissions {
        if seen.insert(s.author_user_id) {
            authors.push(s.author_user_id);
        }
    }
    let k = authors.len();
    if k < 2 {
        return Vec::new();
    }
    let index_of: HashMap<Uuid, usize> = authors.iter().enumerate().map(|(i, a)| (*a, i)).collect();
    // Cap reviewers-per-submission at the number of available peers.
    let n = (reviews_per_student.max(0) as usize).min(k - 1);
    if n == 0 {
        return Vec::new();
    }

    let mut out = Vec::with_capacity(submissions.len() * n);
    for s in submissions {
        let owner_idx = index_of[&s.author_user_id];
        let mut assigned = 0usize;
        let mut step = 1usize;
        while assigned < n && step <= k {
            let cand_idx = (owner_idx + step) % k;
            step += 1;
            let reviewer = authors[cand_idx];
            if reviewer == s.author_user_id {
                continue;
            }
            out.push(db::peer_review::NewAllocation {
                reviewer_user_id: reviewer,
                submission_id: s.submission_id,
            });
            assigned += 1;
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn put_config(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    Json(body): Json<ConfigInput>,
) -> Result<Json<ConfigDto>, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    let course_id = assignment_course_id(&s, tenant, ctx.user_id, id).await?;
    require_course_staff(&s, &ctx, course_id).await?;

    if body.reviews_per_student < 1 || body.reviews_per_student > MAX_REVIEWS_PER_STUDENT {
        return Err(ApiError::Validation(
            "reviews_per_student_out_of_range".into(),
        ));
    }

    // If a rubric is referenced, it must belong to THIS assignment (reuse of the
    // existing rubric grid). Cheap guard via the rubric data layer.
    if let Some(rid) = body.rubric_id {
        let rubric = db::rubrics::fetch_for_assignment(&s.pool, tenant, id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        match rubric {
            Some(r) if r.id == rid => {}
            _ => return Err(ApiError::Validation("rubric_not_for_assignment".into())),
        }
    }

    let row = db::peer_review::upsert_config(
        &s.pool,
        tenant,
        course_id,
        id,
        body.reviews_per_student,
        body.rubric_id,
        body.anonymous,
        body.due_at,
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(ConfigDto::from(row)))
}

async fn allocate(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<AllocateResultDto>, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    let config = db::peer_review::fetch_config(&s.pool, tenant, id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::BadRequest("peer_review_not_configured".into()))?;
    require_course_staff(&s, &ctx, config.course_id).await?;

    let submissions = db::peer_review::list_eligible_submissions(&s.pool, tenant, id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if submissions.len() < 2 {
        return Err(ApiError::Validation("not_enough_submissions".into()));
    }

    let plan = build_allocations(&submissions, config.reviews_per_student);
    if plan.is_empty() {
        return Err(ApiError::Validation("not_enough_reviewers".into()));
    }
    let created =
        db::peer_review::replace_allocations(&s.pool, tenant, config.course_id, id, &plan)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(AllocateResultDto {
        allocations_created: created,
        eligible_submissions: submissions.len() as i64,
        reviews_per_student: config.reviews_per_student,
    }))
}

async fn my_queue(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<AllocationDto>>, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    let course_id = assignment_course_id(&s, tenant, ctx.user_id, id).await?;
    require_course_read(&s, &ctx, course_id).await?;
    let rows = db::peer_review::list_for_reviewer(&s.pool, tenant, id, ctx.user_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    // The reviewer sees who they review only as an opaque submission; we never
    // leak the author identity to the reviewer (peer review is single-blind on
    // the reviewer side regardless of the anonymous flag).
    Ok(Json(
        rows.into_iter().map(reviewer_view).collect::<Vec<_>>(),
    ))
}

async fn submit_review(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(allocation_id): Path<Uuid>,
    Json(body): Json<SubmitReviewInput>,
) -> Result<Json<AllocationDto>, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    let alloc = db::peer_review::fetch_allocation(&s.pool, tenant, allocation_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::NotFound)?;

    // Only the assigned reviewer may file this review.
    if alloc.reviewer_user_id != ctx.user_id {
        return Err(ApiError::Forbidden);
    }
    // Defense-in-depth against a stale author == reviewer allocation.
    if alloc.author_user_id == ctx.user_id {
        return Err(ApiError::Forbidden);
    }

    // Respect the due date when configured: no new/updated reviews past due.
    let config = db::peer_review::fetch_config(&s.pool, tenant, alloc.assignment_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::BadRequest("peer_review_not_configured".into()))?;
    if let Some(due) = config.due_at {
        if chrono::Utc::now() > due {
            return Err(ApiError::Validation("peer_review_closed".into()));
        }
    }

    // `scores` must be a JSON object. When a rubric is configured, every key must
    // be one of its criterion ids and every value a number within [0, max].
    let scores_obj = body
        .scores
        .as_object()
        .ok_or(ApiError::Validation("scores_must_be_object".into()))?;
    if let Some(rubric_id) = config.rubric_id {
        let criteria = db::rubrics::list_criteria(&s.pool, tenant, rubric_id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        let caps: HashMap<String, f64> = criteria
            .iter()
            .map(|c| (c.id.to_string(), c.max_points as f64))
            .collect();
        for (key, val) in scores_obj {
            let Some(cap) = caps.get(key) else {
                return Err(ApiError::Validation("unknown_criterion".into()));
            };
            let n = val
                .as_f64()
                .ok_or(ApiError::Validation("score_must_be_number".into()))?;
            if n < 0.0 || n > *cap {
                return Err(ApiError::Validation("score_out_of_range".into()));
            }
        }
    }

    let comment = body.comment_md.trim();
    if comment.chars().count() > MAX_COMMENT_LEN {
        return Err(ApiError::Validation("comment_too_long".into()));
    }
    let comment = crate::services::sanitize::clean_markdown(comment, MAX_COMMENT_LEN);

    // Serialize the validated scores object to a compact JSON string; the data
    // layer binds it to the JSONB column (avoids the sqlx `json` decode feature).
    let scores_str =
        serde_json::to_string(&body.scores).map_err(|e| ApiError::Internal(e.to_string()))?;
    let row = db::peer_review::upsert_review(&s.pool, tenant, allocation_id, &scores_str, &comment)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(reviewer_view(row)))
}

async fn received(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<AllocationDto>>, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    let course_id = assignment_course_id(&s, tenant, ctx.user_id, id).await?;
    require_course_read(&s, &ctx, course_id).await?;
    let config = db::peer_review::fetch_config(&s.pool, tenant, id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let anonymous = config.map(|c| c.anonymous).unwrap_or(true);

    let rows = db::peer_review::list_received_for_author(&s.pool, tenant, id, ctx.user_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(
        rows.into_iter()
            .map(|r| author_view(r, anonymous))
            .collect::<Vec<_>>(),
    ))
}

async fn summary(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<AllocationDto>>, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    let course_id = assignment_course_id(&s, tenant, ctx.user_id, id).await?;
    require_course_staff(&s, &ctx, course_id).await?;
    // Staff see everything un-redacted (reviewer + author both visible).
    let rows = db::peer_review::list_all_for_assignment(&s.pool, tenant, id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(rows.into_iter().map(staff_view).collect::<Vec<_>>()))
}

// ---------------------------------------------------------------------------
// View mappers (control identity disclosure)
// ---------------------------------------------------------------------------

/// Parse the stored JSON-string scores into a `serde_json::Value`. Malformed
/// stored JSON (should never happen — we only ever store serialized objects)
/// degrades to `None` rather than failing the read.
fn parse_scores(raw: Option<String>) -> Option<serde_json::Value> {
    raw.and_then(|s| serde_json::from_str(&s).ok())
}

/// Reviewer view: shows the reviewer (themselves) but never the reviewed author.
fn reviewer_view(r: db::peer_review::AllocationRow) -> AllocationDto {
    AllocationDto {
        id: r.id,
        assignment_id: r.assignment_id,
        submission_id: r.submission_id,
        status: r.status,
        reviewer_user_id: Some(r.reviewer_user_id),
        author_user_id: None,
        submitted_at: r.submitted_at,
        scores: parse_scores(r.scores_json),
        comment_md: r.comment_md,
    }
}

/// Author view: shows the review they received; the reviewer identity is hidden
/// when the config is anonymous.
fn author_view(r: db::peer_review::AllocationRow, anonymous: bool) -> AllocationDto {
    AllocationDto {
        id: r.id,
        assignment_id: r.assignment_id,
        submission_id: r.submission_id,
        status: r.status,
        reviewer_user_id: if anonymous {
            None
        } else {
            Some(r.reviewer_user_id)
        },
        author_user_id: None,
        submitted_at: r.submitted_at,
        scores: parse_scores(r.scores_json),
        comment_md: r.comment_md,
    }
}

/// Staff view: everything visible (reviewer + author).
fn staff_view(r: db::peer_review::AllocationRow) -> AllocationDto {
    AllocationDto {
        id: r.id,
        assignment_id: r.assignment_id,
        submission_id: r.submission_id,
        status: r.status,
        reviewer_user_id: Some(r.reviewer_user_id),
        author_user_id: Some(r.author_user_id),
        submitted_at: r.submitted_at,
        scores: parse_scores(r.scores_json),
        comment_md: r.comment_md,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sub(author: Uuid, submission: Uuid) -> db::peer_review::EligibleSubmission {
        db::peer_review::EligibleSubmission {
            submission_id: submission,
            author_user_id: author,
        }
    }

    #[test]
    fn allocation_never_assigns_author_to_own_submission() {
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        let c = Uuid::from_u128(3);
        let subs = vec![
            sub(a, Uuid::from_u128(10)),
            sub(b, Uuid::from_u128(20)),
            sub(c, Uuid::from_u128(30)),
        ];
        let plan = build_allocations(&subs, 2);
        // 3 submissions * 2 reviewers each = 6 allocations.
        assert_eq!(plan.len(), 6);
        for alloc in &plan {
            let owner = subs
                .iter()
                .find(|s| s.submission_id == alloc.submission_id)
                .unwrap()
                .author_user_id;
            assert_ne!(alloc.reviewer_user_id, owner, "author reviewed own work");
        }
    }

    #[test]
    fn allocation_assigns_distinct_reviewers() {
        let authors: Vec<Uuid> = (1..=4).map(Uuid::from_u128).collect();
        let subs: Vec<_> = authors
            .iter()
            .enumerate()
            .map(|(i, a)| sub(*a, Uuid::from_u128(100 + i as u128)))
            .collect();
        let plan = build_allocations(&subs, 2);
        // Group reviewers per submission; each set must be distinct + size 2.
        let mut by_sub: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
        for a in &plan {
            by_sub
                .entry(a.submission_id)
                .or_default()
                .push(a.reviewer_user_id);
        }
        for (_, reviewers) in by_sub {
            assert_eq!(reviewers.len(), 2);
            let distinct: HashSet<_> = reviewers.iter().collect();
            assert_eq!(distinct.len(), 2, "duplicate reviewer for a submission");
        }
    }

    #[test]
    fn allocation_caps_at_available_peers() {
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        // Two authors, requested 5 reviews: capped at k-1 = 1 each.
        let subs = vec![sub(a, Uuid::from_u128(10)), sub(b, Uuid::from_u128(20))];
        let plan = build_allocations(&subs, 5);
        assert_eq!(plan.len(), 2);
    }

    #[test]
    fn allocation_empty_when_single_author() {
        let a = Uuid::from_u128(1);
        let subs = vec![sub(a, Uuid::from_u128(10))];
        assert!(build_allocations(&subs, 2).is_empty());
    }
}
