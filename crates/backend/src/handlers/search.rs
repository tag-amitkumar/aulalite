// crates/backend/src/handlers/search.rs
//! Structured search endpoint: `GET /v1/search`.
//!
//! Any authenticated user may call it. Results are scoped to the caller's
//! active tenant and to content they are already allowed to see — course
//! visibility and assignment draft gating are enforced in `db::search`
//! (which reuses the semantics of the course-list and assignment-list paths).

use axum::extract::{Extension, Query, State};
use axum::{routing, Json, Router};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::context::RequestContext;
use crate::db;
use crate::error::ApiError;
use crate::AppState;

const DEFAULT_LIMIT: u32 = 20;
const MAX_LIMIT: u32 = 50;

#[derive(Deserialize)]
pub struct SearchQuery {
    #[serde(default)]
    pub q: String,
    pub limit: Option<u32>,
}

#[derive(Serialize)]
pub struct CourseHitDto {
    pub id: Uuid,
    pub slug: String,
    pub title: String,
    pub status: String,
}

impl From<db::search::CourseHitRow> for CourseHitDto {
    fn from(r: db::search::CourseHitRow) -> Self {
        Self {
            id: r.id,
            slug: r.slug,
            title: r.title,
            status: r.status,
        }
    }
}

#[derive(Serialize)]
pub struct AssignmentHitDto {
    pub id: Uuid,
    pub course_id: Uuid,
    pub course_slug: String,
    pub title: String,
    pub status: String,
}

impl From<db::search::AssignmentHitRow> for AssignmentHitDto {
    fn from(r: db::search::AssignmentHitRow) -> Self {
        Self {
            id: r.id,
            course_id: r.course_id,
            course_slug: r.course_slug,
            title: r.title,
            status: r.status,
        }
    }
}

#[derive(Serialize)]
pub struct LessonHitDto {
    pub id: Uuid,
    pub course_id: Uuid,
    pub course_slug: String,
    pub module_id: Uuid,
    pub title: String,
    pub snippet: Option<String>,
}

impl From<db::search::LessonHitRow> for LessonHitDto {
    fn from(r: db::search::LessonHitRow) -> Self {
        Self {
            id: r.id,
            course_id: r.course_id,
            course_slug: r.course_slug,
            module_id: r.module_id,
            title: r.title,
            snippet: r.snippet,
        }
    }
}

#[derive(Serialize)]
pub struct SearchResponseDto {
    pub courses: Vec<CourseHitDto>,
    pub assignments: Vec<AssignmentHitDto>,
    // Added additively: existing clients ignore unknown fields.
    pub lessons: Vec<LessonHitDto>,
}

pub fn routes() -> Router<AppState> {
    Router::new().route("/v1/search", routing::get(search))
}

#[doc(hidden)]
pub fn router_for_tests(pool: PgPool) -> Router {
    Router::new()
        .route("/v1/search", routing::get(search_t))
        .with_state(TestState { pool })
}

#[derive(Clone)]
struct TestState {
    pool: PgPool,
}

fn clamp_limit(limit: Option<u32>) -> i64 {
    limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT) as i64
}

/// org_admin for search purposes: an OrgAdmin tenant role, or a platform admin
/// (who can act across the tenant). Mirrors `handlers::courses::is_org_admin`
/// plus the platform-admin bypass used in `handlers::assignments`.
fn is_org_admin(ctx: &RequestContext) -> bool {
    ctx.can_manage_organization()
}

async fn search_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    q: SearchQuery,
) -> Result<Json<SearchResponseDto>, ApiError> {
    let term = q.q.trim();
    // Empty query is not an error — return empty results. Also require an active
    // tenant; without one there is nothing the caller can see.
    let tenant_id = match ctx.tenant_id {
        Some(t) if !term.is_empty() => t,
        _ => {
            return Ok(Json(SearchResponseDto {
                courses: Vec::new(),
                assignments: Vec::new(),
                lessons: Vec::new(),
            }))
        }
    };

    let limit = clamp_limit(q.limit);
    let org_admin = is_org_admin(ctx);

    let courses = db::search::search_courses(pool, tenant_id, ctx.user_id, org_admin, term, limit)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let assignments =
        db::search::search_assignments(pool, tenant_id, ctx.user_id, org_admin, term, limit)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
    let lessons = db::search::search_lessons(pool, tenant_id, ctx.user_id, org_admin, term, limit)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(SearchResponseDto {
        courses: courses.into_iter().map(Into::into).collect(),
        assignments: assignments.into_iter().map(Into::into).collect(),
        lessons: lessons.into_iter().map(Into::into).collect(),
    }))
}

// Production handler
async fn search(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Query(q): Query<SearchQuery>,
) -> Result<Json<SearchResponseDto>, ApiError> {
    search_inner(&state.pool, &ctx, q).await
}

// Test mirror
async fn search_t(
    State(state): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Query(q): Query<SearchQuery>,
) -> Result<Json<SearchResponseDto>, ApiError> {
    search_inner(&state.pool, &ctx, q).await
}

#[cfg(test)]
mod tests {
    use super::clamp_limit;

    #[test]
    fn limit_defaults_to_twenty() {
        assert_eq!(clamp_limit(None), 20);
    }

    #[test]
    fn limit_clamps_to_bounds() {
        assert_eq!(clamp_limit(Some(0)), 1);
        assert_eq!(clamp_limit(Some(1)), 1);
        assert_eq!(clamp_limit(Some(50)), 50);
        assert_eq!(clamp_limit(Some(9999)), 50);
    }
}
