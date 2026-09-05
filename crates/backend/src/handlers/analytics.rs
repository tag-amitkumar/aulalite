// crates/backend/src/handlers/analytics.rs
//! Read-only analytics endpoints.
//!
//! * `GET /v1/analytics/overview` — tenant-wide rollup. Org-admin (or platform
//!   admin) only.
//! * `GET /v1/courses/{cid}/analytics` — course rollup + per-assignment progress.
//!   Course-staff (platform admin / org_admin / owner / assigned teacher|ta).
//!
//! Both delegate the SQL to `db::analytics`, which opens its own tx and sets the
//! `app.tenant_id` GUC so RLS applies. Authorization happens here, before any
//! aggregation runs.
use axum::extract::{Extension, Path, State};
use axum::{routing, Json, Router};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::context::RequestContext;
use crate::db;
use crate::error::ApiError;
use crate::AppState;

// --- DTOs ---

#[derive(Serialize)]
pub struct OverviewDto {
    pub courses_total: i64,
    pub courses_published: i64,
    pub members_students: i64,
    pub members_teachers: i64,
    pub members_tas: i64,
    pub members_parents: i64,
    pub sessions_last_30d: i64,
    pub sessions_ended_total: i64,
    pub recordings_available: i64,
    pub assignments_total: i64,
    pub submissions_total: i64,
    pub submissions_graded: i64,
}

impl From<db::analytics::OverviewRow> for OverviewDto {
    fn from(r: db::analytics::OverviewRow) -> Self {
        Self {
            courses_total: r.courses_total,
            courses_published: r.courses_published,
            members_students: r.members_students,
            members_teachers: r.members_teachers,
            members_tas: r.members_tas,
            members_parents: r.members_parents,
            sessions_last_30d: r.sessions_last_30d,
            sessions_ended_total: r.sessions_ended_total,
            recordings_available: r.recordings_available,
            assignments_total: r.assignments_total,
            submissions_total: r.submissions_total,
            submissions_graded: r.submissions_graded,
        }
    }
}

#[derive(Serialize)]
pub struct AssignmentProgressDto {
    pub assignment_id: Uuid,
    pub title: String,
    pub status: String,
    pub submitted_count: i64,
    pub graded_count: i64,
    pub avg_numeric_grade: Option<f64>,
}

impl From<db::analytics::AssignmentProgressRow> for AssignmentProgressDto {
    fn from(r: db::analytics::AssignmentProgressRow) -> Self {
        Self {
            assignment_id: r.assignment_id,
            title: r.title,
            status: r.status,
            submitted_count: r.submitted_count,
            graded_count: r.graded_count,
            avg_numeric_grade: r.avg_numeric_grade,
        }
    }
}

#[derive(Serialize)]
pub struct DailyActivityDto {
    /// ISO date (YYYY-MM-DD).
    pub day: String,
    pub sessions: i64,
    pub attendance_joins: i64,
    pub submissions: i64,
    pub lessons_completed: i64,
}

#[derive(Serialize)]
pub struct ProgressFunnelDto {
    pub enrolled: i64,
    pub started: i64,
    pub half: i64,
    pub completed: i64,
}

#[derive(Serialize)]
pub struct QuizScoreDistributionDto {
    pub under_50: i64,
    pub from_50_to_69: i64,
    pub from_70_to_89: i64,
    pub from_90_up: i64,
}

/// Per-assignment grade distribution (percentage-based) over graded numeric
/// submissions. Mirrors `db::analytics::GradeDistributionRow`.
#[derive(Serialize)]
pub struct GradeDistributionDto {
    pub assignment_id: Uuid,
    pub title: String,
    pub graded_count: i64,
    pub under_50: i64,
    pub from_50_to_69: i64,
    pub from_70_to_89: i64,
    pub from_90_up: i64,
    /// Median percentage (0.0–1.0+), None when no graded work.
    pub median_pct: Option<f64>,
    /// Population stddev of the percentage, None when no graded work.
    pub stddev_pct: Option<f64>,
}

impl From<db::analytics::GradeDistributionRow> for GradeDistributionDto {
    fn from(r: db::analytics::GradeDistributionRow) -> Self {
        Self {
            assignment_id: r.assignment_id,
            title: r.title,
            graded_count: r.graded_count,
            under_50: r.under_50,
            from_50_to_69: r.from_50_to_69,
            from_70_to_89: r.from_70_to_89,
            from_90_up: r.from_90_up,
            median_pct: r.median_pct,
            stddev_pct: r.stddev_pct,
        }
    }
}

/// One point on the class-average trend across recently-graded assignments.
/// Mirrors `db::analytics::ClassAverageTrendRow`. Ordered oldest→newest.
#[derive(Serialize)]
pub struct ClassAverageTrendDto {
    pub assignment_id: Uuid,
    pub title: String,
    pub graded_count: i64,
    /// Mean percentage (0.0–1.0+) across graded submissions.
    pub avg_pct: f64,
}

impl From<db::analytics::ClassAverageTrendRow> for ClassAverageTrendDto {
    fn from(r: db::analytics::ClassAverageTrendRow) -> Self {
        Self {
            assignment_id: r.assignment_id,
            title: r.title,
            graded_count: r.graded_count,
            avg_pct: r.avg_pct,
        }
    }
}

/// A student flagged as at-risk (average below the threshold across graded
/// numeric work). Mirrors `db::analytics::AtRiskStudentRow`.
#[derive(Serialize)]
pub struct AtRiskStudentDto {
    pub student_user_id: Uuid,
    pub display_name: String,
    /// Mean percentage (0.0–1.0+) across the student's graded numeric work.
    pub avg_pct: f64,
    pub graded_count: i64,
}

impl From<db::analytics::AtRiskStudentRow> for AtRiskStudentDto {
    fn from(r: db::analytics::AtRiskStudentRow) -> Self {
        Self {
            student_user_id: r.student_user_id,
            display_name: r.display_name,
            avg_pct: r.avg_pct,
            graded_count: r.graded_count,
        }
    }
}

/// Grade-analytics threshold used for at-risk flagging: students averaging
/// below 60% across graded numeric work are surfaced.
const AT_RISK_THRESHOLD: f64 = 0.60;
/// How many of the most-recently-graded assignments feed the class-average
/// trend line.
const TREND_LAST_N: i64 = 10;

#[derive(Serialize)]
pub struct CourseAnalyticsDto {
    pub course_id: Uuid,
    pub enrolled_students: i64,
    pub sessions_total: i64,
    pub sessions_ended: i64,
    pub unique_attendees: i64,
    pub avg_attendance_seconds: f64,
    pub assignments: Vec<AssignmentProgressDto>,
    pub funnel: ProgressFunnelDto,
    pub quiz_scores: QuizScoreDistributionDto,
    // --- Grade analytics (additive; new optional fields) ---
    /// Per-assignment grade distribution over graded numeric submissions.
    pub grade_distribution: Vec<GradeDistributionDto>,
    /// Class-average trend across the last N graded numeric assignments.
    pub grade_trend: Vec<ClassAverageTrendDto>,
    /// Students averaging below the at-risk threshold across graded work.
    pub at_risk: Vec<AtRiskStudentDto>,
    /// The threshold (0.0–1.0) used to compute `at_risk`, so the UI can label it.
    pub at_risk_threshold: f64,
}

#[derive(serde::Deserialize, Default)]
pub struct ActivityQuery {
    pub days: Option<i32>,
}

// --- routers ---

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/analytics/overview", routing::get(overview))
        .route("/v1/analytics/activity", routing::get(activity))
        .route("/v1/courses/{cid}/analytics", routing::get(course))
}

#[doc(hidden)]
pub fn router_for_tests(pool: PgPool) -> Router {
    Router::new()
        .route("/v1/analytics/overview", routing::get(overview_t))
        .route("/v1/analytics/activity", routing::get(activity_t))
        .route("/v1/courses/{cid}/analytics", routing::get(course_t))
        .with_state(TestState { pool })
}

#[derive(Clone)]
struct TestState {
    pool: PgPool,
}

// --- authz helpers (mirror handlers/assignments.rs) ---

fn is_org_admin(ctx: &RequestContext) -> bool {
    ctx.can_manage_organization()
}

// --- shared inner functions ---

async fn overview_inner(
    pool: &PgPool,
    ctx: &RequestContext,
) -> Result<Json<OverviewDto>, ApiError> {
    // Org-admin (or platform admin) only.
    if !ctx.can_manage_organization() {
        return Err(ApiError::Forbidden);
    }
    let tenant_id = ctx
        .tenant_id
        .ok_or_else(|| ApiError::BadRequest("no tenant".into()))?;

    let row = db::analytics::tenant_overview(pool, tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(OverviewDto::from(row)))
}

async fn activity_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    q: ActivityQuery,
) -> Result<Json<Vec<DailyActivityDto>>, ApiError> {
    if !is_org_admin(ctx) {
        return Err(ApiError::Forbidden);
    }
    let tenant_id = ctx
        .tenant_id
        .ok_or_else(|| ApiError::BadRequest("no tenant".into()))?;
    let days = q.days.unwrap_or(30);
    if !(1..=90).contains(&days) {
        return Err(ApiError::BadRequest("days must be between 1 and 90".into()));
    }
    let rows = db::analytics::daily_activity(pool, tenant_id, days)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(
        rows.into_iter()
            .map(|r| DailyActivityDto {
                day: r.day.to_string(),
                sessions: r.sessions,
                attendance_joins: r.attendance_joins,
                submissions: r.submissions,
                lessons_completed: r.lessons_completed,
            })
            .collect(),
    ))
}

async fn course_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    cid: Uuid,
) -> Result<Json<CourseAnalyticsDto>, ApiError> {
    let tenant_id = ctx.tenant_id.ok_or(ApiError::CourseNotFound)?;

    // Resolve + verify the course is in the active tenant first, mirroring the
    // other course-scoped handlers (return CourseNotFound, not Forbidden, when
    // the course is missing or belongs to another tenant).
    {
        let mut tx = db::begin_with_context(pool, ctx.user_id, Some(tenant_id))
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        let course = db::courses::fetch_course(&mut *tx, cid)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?
            .ok_or(ApiError::CourseNotFound)?;
        tx.commit()
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        if course.tenant_id != tenant_id {
            return Err(ApiError::CourseNotFound);
        }
    }

    // Course-staff gate: platform admin bypasses; everyone else must own the
    // course or be an active teacher/ta member (org_admin via the helper).
    if !ctx.can_manage_organization() {
        let allowed = db::courses::caller_can_staff_course(
            pool,
            cid,
            ctx.user_id,
            ctx.tenant_id,
            is_org_admin(ctx),
        )
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
        if !allowed {
            return Err(ApiError::Forbidden);
        }
    }

    let rollup = db::analytics::course_analytics(pool, tenant_id, cid)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let progress = db::analytics::assignment_progress(pool, tenant_id, cid)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let funnel = db::analytics::progress_funnel(pool, tenant_id, cid)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let quiz_scores = db::analytics::quiz_score_distribution(pool, tenant_id, cid)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let grade_distribution = db::analytics::assignment_grade_distribution(pool, tenant_id, cid)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let grade_trend = db::analytics::class_average_trend(pool, tenant_id, cid, TREND_LAST_N)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let at_risk = db::analytics::at_risk_students(pool, tenant_id, cid, AT_RISK_THRESHOLD)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(CourseAnalyticsDto {
        course_id: rollup.course_id,
        enrolled_students: rollup.enrolled_students,
        sessions_total: rollup.sessions_total,
        sessions_ended: rollup.sessions_ended,
        unique_attendees: rollup.unique_attendees,
        avg_attendance_seconds: rollup.avg_attendance_seconds,
        assignments: progress
            .into_iter()
            .map(AssignmentProgressDto::from)
            .collect(),
        funnel: ProgressFunnelDto {
            enrolled: funnel.enrolled,
            started: funnel.started,
            half: funnel.half,
            completed: funnel.completed,
        },
        quiz_scores: QuizScoreDistributionDto {
            under_50: quiz_scores.under_50,
            from_50_to_69: quiz_scores.from_50_to_69,
            from_70_to_89: quiz_scores.from_70_to_89,
            from_90_up: quiz_scores.from_90_up,
        },
        grade_distribution: grade_distribution
            .into_iter()
            .map(GradeDistributionDto::from)
            .collect(),
        grade_trend: grade_trend
            .into_iter()
            .map(ClassAverageTrendDto::from)
            .collect(),
        at_risk: at_risk.into_iter().map(AtRiskStudentDto::from).collect(),
        at_risk_threshold: AT_RISK_THRESHOLD,
    }))
}

// --- production handlers ---

async fn overview(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<OverviewDto>, ApiError> {
    overview_inner(&s.pool, &ctx).await
}

async fn activity(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    axum::extract::Query(q): axum::extract::Query<ActivityQuery>,
) -> Result<Json<Vec<DailyActivityDto>>, ApiError> {
    activity_inner(&s.pool, &ctx, q).await
}

async fn course(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(cid): Path<Uuid>,
) -> Result<Json<CourseAnalyticsDto>, ApiError> {
    course_inner(&s.pool, &ctx, cid).await
}

// --- test wrappers ---

async fn overview_t(
    State(ts): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<OverviewDto>, ApiError> {
    overview_inner(&ts.pool, &ctx).await
}

async fn activity_t(
    State(ts): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    axum::extract::Query(q): axum::extract::Query<ActivityQuery>,
) -> Result<Json<Vec<DailyActivityDto>>, ApiError> {
    activity_inner(&ts.pool, &ctx, q).await
}

async fn course_t(
    State(ts): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(cid): Path<Uuid>,
) -> Result<Json<CourseAnalyticsDto>, ApiError> {
    course_inner(&ts.pool, &ctx, cid).await
}
