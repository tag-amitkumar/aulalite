// crates/backend/src/handlers/gradebook.rs
//! Weighted course gradebook + CSV export. All routes are course-scoped and
//! STAFF-ONLY (owner / active teacher|ta / org_admin / platform_admin); a
//! non-staff caller gets 403, so the frontend panel can self-hide students.
//!
//! Routes (authed router):
//!   * GET    /v1/courses/{cid}/grade-categories                 — list
//!   * POST   /v1/courses/{cid}/grade-categories                 — create
//!   * DELETE /v1/courses/{cid}/grade-categories/{id}             — delete
//!   * PUT    /v1/courses/{cid}/assignments/{aid}/category        — (un)assign
//!   * GET    /v1/courses/{cid}/gradebook                        — JSON matrix
//!   * GET    /v1/courses/{cid}/gradebook.csv                    — CSV download
//!
//! The JSON matrix is `students × assignments` with each released numeric grade
//! PLUS a per-student weighted total. The weighting math lives in
//! `compute_weighted_total` (pure, unit-tested) so the JSON and the CSV use the
//! exact same numbers — they can never drift.
use axum::extract::{Extension, Path, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{routing, Json, Router};
use serde::{Deserialize, Serialize};
use sqlx::types::BigDecimal;
use std::collections::HashMap;
use uuid::Uuid;

use crate::context::RequestContext;
use crate::db;
use crate::error::ApiError;
use crate::AppState;

const MAX_CATEGORY_NAME_LEN: usize = 100;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/v1/courses/{cid}/grade-categories",
            routing::get(list_categories).post(create_category),
        )
        .route(
            "/v1/courses/{cid}/grade-categories/{id}",
            routing::delete(delete_category),
        )
        .route(
            "/v1/courses/{cid}/assignments/{aid}/category",
            routing::put(set_assignment_category),
        )
        .route("/v1/courses/{cid}/gradebook", routing::get(get_gradebook))
        .route(
            "/v1/courses/{cid}/gradebook.csv",
            routing::get(get_gradebook_csv),
        )
}

fn is_org_admin(ctx: &RequestContext) -> bool {
    ctx.can_manage_organization()
}

/// Course-scoped staff gate. `platform_admin` always passes.
async fn require_course_staff(
    state: &AppState,
    ctx: &RequestContext,
    course_id: Uuid,
) -> Result<(), ApiError> {
    if !ctx.can_grade() {
        return Err(ApiError::Forbidden);
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

// ---------------------------------------------------------------------------
// Categories
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct CategoryDto {
    pub id: Uuid,
    pub course_id: Uuid,
    pub name: String,
    pub weight_percent: i32,
}

impl From<db::gradebook::CategoryRow> for CategoryDto {
    fn from(r: db::gradebook::CategoryRow) -> Self {
        Self {
            id: r.id,
            course_id: r.course_id,
            name: r.name,
            weight_percent: r.weight_percent,
        }
    }
}

#[derive(Deserialize)]
pub struct CreateCategory {
    pub name: String,
    pub weight_percent: i32,
}

async fn list_categories(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(cid): Path<Uuid>,
) -> Result<Json<Vec<CategoryDto>>, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    require_course_staff(&s, &ctx, cid).await?;
    let rows = db::gradebook::list_categories(&s.pool, tenant, cid)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(rows.into_iter().map(CategoryDto::from).collect()))
}

async fn create_category(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(cid): Path<Uuid>,
    Json(body): Json<CreateCategory>,
) -> Result<Json<CategoryDto>, ApiError> {
    if !ctx.can_teach() {
        return Err(ApiError::Forbidden);
    }
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    require_course_staff(&s, &ctx, cid).await?;

    let name = body.name.trim();
    if name.is_empty() {
        return Err(ApiError::Validation("name_required".into()));
    }
    if name.chars().count() > MAX_CATEGORY_NAME_LEN {
        return Err(ApiError::Validation("name_too_long".into()));
    }
    if !(0..=100).contains(&body.weight_percent) {
        return Err(ApiError::Validation("weight_out_of_range".into()));
    }

    let row = db::gradebook::insert_category(&s.pool, tenant, cid, name, body.weight_percent)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(CategoryDto::from(row)))
}

async fn delete_category(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path((cid, id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    if !ctx.can_teach() {
        return Err(ApiError::Forbidden);
    }
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    require_course_staff(&s, &ctx, cid).await?;
    let deleted = db::gradebook::delete_category(&s.pool, tenant, cid, id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !deleted {
        return Err(ApiError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Assign an assignment to a category
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct SetCategoryBody {
    /// `None` clears the assignment's category link.
    pub category_id: Option<Uuid>,
}

async fn set_assignment_category(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path((cid, aid)): Path<(Uuid, Uuid)>,
    Json(body): Json<SetCategoryBody>,
) -> Result<StatusCode, ApiError> {
    if !ctx.can_teach() {
        return Err(ApiError::Forbidden);
    }
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    require_course_staff(&s, &ctx, cid).await?;

    // The assignment must live in this course.
    if !db::gradebook::assignment_in_course(&s.pool, tenant, cid, aid)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
    {
        return Err(ApiError::NotFound);
    }
    // If assigning (not clearing), the category must live in this course too.
    if let Some(cat) = body.category_id {
        if !db::gradebook::category_exists_in_course(&s.pool, tenant, cid, cat)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?
        {
            return Err(ApiError::NotFound);
        }
    }

    db::gradebook::set_assignment_category(&s.pool, tenant, cid, aid, body.category_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Gradebook matrix (JSON)
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct GradebookAssignmentDto {
    pub id: Uuid,
    pub title: String,
    pub max_points: Option<i32>,
    pub category_id: Option<Uuid>,
}

/// One graded cell in a student's row: the released numeric grade for an
/// assignment. Absent assignments (no released numeric grade) are simply not
/// present in `grades`.
#[derive(Serialize)]
pub struct GradeCellDto {
    pub assignment_id: Uuid,
    pub numeric_grade: f64,
}

#[derive(Serialize)]
pub struct GradebookStudentDto {
    pub user_id: Uuid,
    pub display_name: Option<String>,
    pub email: Option<String>,
    pub grades: Vec<GradeCellDto>,
    /// Weighted total as a 0..=100 percentage. `None` when the student has no
    /// counted (released, numerically-graded, scorable) coursework yet.
    pub weighted_total: Option<f64>,
}

#[derive(Serialize)]
pub struct GradebookDto {
    pub course_id: Uuid,
    pub categories: Vec<CategoryDto>,
    pub assignments: Vec<GradebookAssignmentDto>,
    pub students: Vec<GradebookStudentDto>,
}

/// A single graded data point used by the weighting math: the grade earned,
/// the assignment's max points, and the (optional) category it belongs to.
#[derive(Clone, Copy)]
pub struct ScoredItem {
    pub earned: f64,
    pub max_points: f64,
    /// `None` = uncategorized (pooled into a synthetic "uncategorized" bucket).
    pub category: Option<Uuid>,
}

/// Compute a student's weighted total as a 0..=100 percentage from their graded
/// items and the category weights.
///
/// Model: within each category, the student's category percentage is
/// `sum(earned) / sum(max_points)` across that category's graded, scorable
/// items (max_points > 0). The overall total is the category percentages
/// combined by weight, normalized by the sum of the weights of the categories
/// that actually contributed — so partial coursework still yields a sensible
/// running average rather than collapsing toward zero.
///
/// Uncategorized items are pooled into one synthetic bucket. Its weight is the
/// remainder `max(0, 100 - sum(category weights))`; if that remainder is zero
/// (categories already sum to ≥ 100) the uncategorized bucket falls back to an
/// equal-share weight of 1 so uncategorized graded work is never silently
/// dropped from a course that has categories.
///
/// Returns `None` when there is no counted coursework (no scorable graded
/// items, or every contributing weight is zero).
pub fn compute_weighted_total(
    items: &[ScoredItem],
    category_weights: &[(Uuid, i32)],
) -> Option<f64> {
    // Aggregate earned/max per bucket (None = uncategorized).
    let mut earned: HashMap<Option<Uuid>, f64> = HashMap::new();
    let mut maxsum: HashMap<Option<Uuid>, f64> = HashMap::new();
    for it in items {
        if it.max_points <= 0.0 {
            continue; // unscorable (no max points) — can't form a percentage.
        }
        *earned.entry(it.category).or_insert(0.0) += it.earned;
        *maxsum.entry(it.category).or_insert(0.0) += it.max_points;
    }
    if maxsum.is_empty() {
        return None;
    }

    let weight_of: HashMap<Uuid, i32> = category_weights.iter().copied().collect();
    let weight_sum: i32 = category_weights.iter().map(|(_, w)| *w).sum();
    let uncategorized_weight: f64 = {
        let remainder = 100 - weight_sum;
        if remainder > 0 {
            remainder as f64
        } else {
            // Categories already fill (or overflow) 100% — give any
            // uncategorized work a minimal equal share rather than dropping it.
            1.0
        }
    };

    let mut weighted_sum = 0.0;
    let mut total_weight = 0.0;
    for (bucket, max_total) in &maxsum {
        if *max_total <= 0.0 {
            continue;
        }
        let earned_total = earned.get(bucket).copied().unwrap_or(0.0);
        let pct = (earned_total / max_total) * 100.0;
        let weight = match bucket {
            Some(cat) => weight_of.get(cat).copied().unwrap_or(0) as f64,
            None => uncategorized_weight,
        };
        if weight <= 0.0 {
            continue; // a zero-weight category never moves the total.
        }
        weighted_sum += pct * weight;
        total_weight += weight;
    }
    if total_weight <= 0.0 {
        return None;
    }
    Some(weighted_sum / total_weight)
}

/// Convert a `BigDecimal` grade to `f64`, defaulting to 0.0 on the (practically
/// impossible) parse failure so a single odd row can't 500 the whole gradebook.
fn bd_to_f64(v: &BigDecimal) -> f64 {
    v.to_string().parse::<f64>().unwrap_or(0.0)
}

/// Build the JSON-ready gradebook from the raw DB pieces. Pure (no IO) so the
/// matrix assembly + weighting are unit-testable and shared with the CSV path.
fn build_gradebook(course_id: Uuid, data: db::gradebook::GradebookData) -> GradebookDto {
    let categories: Vec<CategoryDto> = data.categories.into_iter().map(CategoryDto::from).collect();
    let category_weights: Vec<(Uuid, i32)> = categories
        .iter()
        .map(|c| (c.id, c.weight_percent))
        .collect();

    // assignment_id -> (max_points, category)
    let assignment_meta: HashMap<Uuid, (Option<i32>, Option<Uuid>)> = data
        .assignments
        .iter()
        .map(|a| (a.id, (a.max_points, a.category_id)))
        .collect();

    // student -> list of (assignment_id, grade)
    let mut grades_by_student: HashMap<Uuid, Vec<(Uuid, f64)>> = HashMap::new();
    for g in &data.grades {
        // Only count grades for assignments that are in our column set (i.e.
        // currently published). Submissions for unpublished/removed work are
        // skipped so the matrix stays internally consistent.
        if assignment_meta.contains_key(&g.assignment_id) {
            grades_by_student
                .entry(g.student_user_id)
                .or_default()
                .push((g.assignment_id, bd_to_f64(&g.numeric_grade)));
        }
    }

    let assignments: Vec<GradebookAssignmentDto> = data
        .assignments
        .into_iter()
        .map(|a| GradebookAssignmentDto {
            id: a.id,
            title: a.title,
            max_points: a.max_points,
            category_id: a.category_id,
        })
        .collect();

    let students: Vec<GradebookStudentDto> = data
        .students
        .into_iter()
        .map(|st| {
            let cells = grades_by_student.remove(&st.user_id).unwrap_or_default();
            let scored: Vec<ScoredItem> = cells
                .iter()
                .filter_map(|(aid, earned)| {
                    let (max_points, category) = assignment_meta.get(aid).copied()?;
                    Some(ScoredItem {
                        earned: *earned,
                        max_points: max_points.unwrap_or(0) as f64,
                        category,
                    })
                })
                .collect();
            let weighted_total = compute_weighted_total(&scored, &category_weights);
            let grades = cells
                .into_iter()
                .map(|(assignment_id, numeric_grade)| GradeCellDto {
                    assignment_id,
                    numeric_grade,
                })
                .collect();
            GradebookStudentDto {
                user_id: st.user_id,
                display_name: st.display_name,
                email: st.email,
                grades,
                weighted_total,
            }
        })
        .collect();

    GradebookDto {
        course_id,
        categories,
        assignments,
        students,
    }
}

async fn get_gradebook(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(cid): Path<Uuid>,
) -> Result<Json<GradebookDto>, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    require_course_staff(&s, &ctx, cid).await?;
    let data = db::gradebook::load_gradebook(&s.pool, tenant, cid)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(build_gradebook(cid, data)))
}

// ---------------------------------------------------------------------------
// Gradebook CSV export
// ---------------------------------------------------------------------------

/// RFC 4180 field escaping: quote the field (doubling embedded quotes) iff it
/// contains a character that would otherwise break the row/column framing.
/// Mirrors `handlers::attendance_export::csv_escape`.
fn csv_escape(field: &str) -> String {
    if field.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_string()
    }
}

/// The displayed student label for the CSV `student` column: display_name, else
/// email, else the raw user id — so the cell is never blank.
fn student_label(st: &GradebookStudentDto) -> String {
    st.display_name
        .as_deref()
        .filter(|s| !s.is_empty())
        .or(st.email.as_deref().filter(|s| !s.is_empty()))
        .map(|s| s.to_string())
        .unwrap_or_else(|| st.user_id.to_string())
}

/// Render the gradebook to an RFC 4180 CSV string. Header:
/// `student,user_id,<assignment title>…,weighted_total`. Each grade cell is the
/// numeric grade (one decimal), empty when ungraded. The weighted total is a
/// one-decimal percentage, empty when there is no counted coursework.
fn gradebook_to_csv(gb: &GradebookDto) -> String {
    let mut out = String::from("student,user_id");
    for a in &gb.assignments {
        out.push(',');
        out.push_str(&csv_escape(&a.title));
    }
    out.push_str(",weighted_total\r\n");

    for st in &gb.students {
        // assignment_id -> grade for this student, for O(1) column lookup.
        let by_assignment: HashMap<Uuid, f64> = st
            .grades
            .iter()
            .map(|g| (g.assignment_id, g.numeric_grade))
            .collect();
        out.push_str(&csv_escape(&student_label(st)));
        out.push(',');
        out.push_str(&csv_escape(&st.user_id.to_string()));
        for a in &gb.assignments {
            out.push(',');
            if let Some(grade) = by_assignment.get(&a.id) {
                out.push_str(&format!("{grade:.1}"));
            }
            // else: empty cell (ungraded).
        }
        out.push(',');
        if let Some(total) = st.weighted_total {
            out.push_str(&format!("{total:.1}"));
        }
        out.push_str("\r\n");
    }
    out
}

async fn get_gradebook_csv(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(cid): Path<Uuid>,
) -> Result<Response, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    require_course_staff(&s, &ctx, cid).await?;
    let data = db::gradebook::load_gradebook(&s.pool, tenant, cid)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let gb = build_gradebook(cid, data);
    let body = gradebook_to_csv(&gb);
    let filename = format!("gradebook-{cid}.csv");
    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{filename}\""),
            ),
        ],
        body,
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cat(n: u8) -> Uuid {
        Uuid::from_u128(n as u128)
    }

    #[test]
    fn weighted_total_combines_categories_by_weight() {
        // Two categories: Homework (weight 30) at 80%, Exams (weight 70) at 90%.
        let hw = cat(1);
        let ex = cat(2);
        let items = vec![
            ScoredItem {
                earned: 8.0,
                max_points: 10.0,
                category: Some(hw),
            },
            ScoredItem {
                earned: 90.0,
                max_points: 100.0,
                category: Some(ex),
            },
        ];
        let weights = vec![(hw, 30), (ex, 70)];
        let total = compute_weighted_total(&items, &weights).unwrap();
        // (80*30 + 90*70) / 100 = (2400 + 6300)/100 = 87.0
        assert!((total - 87.0).abs() < 1e-9, "got {total}");
    }

    #[test]
    fn weighted_total_normalizes_when_a_category_has_no_grades() {
        // Only the Exams category has a graded item; the total normalizes by the
        // contributing weight (70) so it equals that category's own percentage.
        let hw = cat(1);
        let ex = cat(2);
        let items = vec![ScoredItem {
            earned: 90.0,
            max_points: 100.0,
            category: Some(ex),
        }];
        let weights = vec![(hw, 30), (ex, 70)];
        let total = compute_weighted_total(&items, &weights).unwrap();
        assert!((total - 90.0).abs() < 1e-9, "got {total}");
    }

    #[test]
    fn weighted_total_pools_assignments_within_a_category() {
        // Two assignments in the same category aggregate points (not average of
        // percentages): 8/10 + 30/40 = 38/50 = 76%.
        let hw = cat(1);
        let items = vec![
            ScoredItem {
                earned: 8.0,
                max_points: 10.0,
                category: Some(hw),
            },
            ScoredItem {
                earned: 30.0,
                max_points: 40.0,
                category: Some(hw),
            },
        ];
        let weights = vec![(hw, 100)];
        let total = compute_weighted_total(&items, &weights).unwrap();
        assert!((total - 76.0).abs() < 1e-9, "got {total}");
    }

    #[test]
    fn weighted_total_none_without_scorable_coursework() {
        // No items at all.
        assert!(compute_weighted_total(&[], &[]).is_none());
        // An item with zero max_points is unscorable → still None.
        let items = vec![ScoredItem {
            earned: 5.0,
            max_points: 0.0,
            category: None,
        }];
        assert!(compute_weighted_total(&items, &[]).is_none());
    }

    #[test]
    fn uncategorized_uses_weight_remainder() {
        // One category (weight 60) at 50%, plus uncategorized work at 100%.
        // Uncategorized weight = 100 - 60 = 40.
        // (50*60 + 100*40)/100 = (3000 + 4000)/100 = 70.
        let c = cat(1);
        let items = vec![
            ScoredItem {
                earned: 5.0,
                max_points: 10.0,
                category: Some(c),
            },
            ScoredItem {
                earned: 10.0,
                max_points: 10.0,
                category: None,
            },
        ];
        let weights = vec![(c, 60)];
        let total = compute_weighted_total(&items, &weights).unwrap();
        assert!((total - 70.0).abs() < 1e-9, "got {total}");
    }

    #[test]
    fn uncategorized_only_is_pure_percentage() {
        // With no categories defined, the whole grade is just the pooled
        // uncategorized percentage.
        let items = vec![
            ScoredItem {
                earned: 18.0,
                max_points: 20.0,
                category: None,
            },
            ScoredItem {
                earned: 7.0,
                max_points: 10.0,
                category: None,
            },
        ];
        let total = compute_weighted_total(&items, &[]).unwrap();
        // 25/30 = 83.333…
        assert!((total - (25.0 / 30.0 * 100.0)).abs() < 1e-9, "got {total}");
    }

    fn sample_data() -> db::gradebook::GradebookData {
        let course = cat(99);
        let hw = cat(1);
        let a1 = cat(10);
        let a2 = cat(11);
        let s1 = cat(20);
        let now = chrono::Utc::now();
        db::gradebook::GradebookData {
            students: vec![db::gradebook::GradebookStudentRow {
                user_id: s1,
                display_name: Some("Ada, \"Lovelace\"".into()),
                email: Some("ada@example.com".into()),
            }],
            assignments: vec![
                db::gradebook::GradebookAssignmentRow {
                    id: a1,
                    title: "Essay, part 1".into(),
                    max_points: Some(10),
                    category_id: Some(hw),
                },
                db::gradebook::GradebookAssignmentRow {
                    id: a2,
                    title: "Quiz".into(),
                    max_points: Some(20),
                    category_id: None,
                },
            ],
            grades: vec![db::gradebook::GradebookGradeRow {
                assignment_id: a1,
                student_user_id: s1,
                numeric_grade: "8".parse().unwrap(),
            }],
            categories: vec![db::gradebook::CategoryRow {
                id: hw,
                course_id: course,
                name: "Homework".into(),
                weight_percent: 100,
                created_at: now,
                updated_at: now,
            }],
        }
    }

    #[test]
    fn build_gradebook_shapes_matrix_and_total() {
        let gb = build_gradebook(cat(99), sample_data());
        assert_eq!(gb.assignments.len(), 2);
        assert_eq!(gb.students.len(), 1);
        let st = &gb.students[0];
        assert_eq!(st.grades.len(), 1);
        // Only the Homework category (weight 100) contributed: 8/10 = 80%.
        assert!((st.weighted_total.unwrap() - 80.0).abs() < 1e-9);
    }

    #[test]
    fn csv_has_header_columns_and_escapes_fields() {
        let gb = build_gradebook(cat(99), sample_data());
        let csv = gradebook_to_csv(&gb);
        let mut lines = csv.lines();
        let header = lines.next().unwrap();
        // Assignment titles become columns; the comma-bearing title is quoted.
        assert!(header.starts_with("student,user_id,"));
        assert!(header.contains("\"Essay, part 1\""));
        assert!(header.ends_with("weighted_total"));
        let row = lines.next().unwrap();
        // Student label with embedded comma+quotes is RFC-4180 escaped.
        assert!(row.contains("\"Ada, \"\"Lovelace\"\"\""));
        // a1 graded (8.0), a2 ungraded (empty), weighted total 80.0.
        assert!(row.contains(",8.0,,80.0"));
    }

    #[test]
    fn csv_empty_when_no_students() {
        let mut data = sample_data();
        data.students.clear();
        let gb = build_gradebook(cat(99), data);
        let csv = gradebook_to_csv(&gb);
        // Just the header row; no data rows.
        assert_eq!(csv.lines().count(), 1);
    }
}
