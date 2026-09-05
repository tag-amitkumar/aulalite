//! Gamification endpoints (learning-suite Cycle 4).
//!
//! - `GET  /v1/me/gamification`          — XP/level/streak + unlocks
//! - `POST /v1/me/gamification/seen`     — mark unlock celebrations seen
//! - `GET  /v1/me/leaderboard-opt-out`   — current opt-out state
//! - `PUT  /v1/me/leaderboard-opt-out`   — set opt-out state
//! - `GET  /v1/courses/{cid}/leaderboard` — per-course standings
//!
//! Awards themselves happen inside the lesson/quiz/attendance flows via
//! `db::gamification::award`.

use axum::extract::{Extension, Path, State};
use axum::{routing, Json, Router};
use sqlx::PgPool;
use uuid::Uuid;

use crate::context::RequestContext;
use crate::db;
use crate::error::ApiError;
use crate::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/me/gamification", routing::get(me_gamification))
        .route("/v1/me/gamification/seen", routing::post(mark_seen))
        .route(
            "/v1/me/leaderboard-opt-out",
            routing::get(get_opt_out).put(put_opt_out),
        )
        .route(
            "/v1/courses/{cid}/leaderboard",
            routing::get(course_leaderboard),
        )
}

#[derive(Clone)]
struct TestState {
    pool: PgPool,
}

#[doc(hidden)]
pub fn router_for_tests(pool: PgPool) -> Router {
    Router::new()
        .route("/v1/me/gamification", routing::get(me_gamification_t))
        .route("/v1/me/gamification/seen", routing::post(mark_seen_t))
        .route(
            "/v1/me/leaderboard-opt-out",
            routing::get(get_opt_out_t).put(put_opt_out_t),
        )
        .route(
            "/v1/courses/{cid}/leaderboard",
            routing::get(course_leaderboard_t),
        )
        .with_state(TestState { pool })
}

fn internal(e: impl std::fmt::Display) -> ApiError {
    ApiError::Internal(e.to_string())
}

fn require_tenant(ctx: &RequestContext) -> Result<Uuid, ApiError> {
    ctx.tenant_id
        .ok_or_else(|| ApiError::BadRequest("no active tenant".into()))
}

fn is_org_admin(ctx: &RequestContext) -> bool {
    ctx.can_manage_organization()
}

#[derive(serde::Serialize)]
pub struct UnlockDto {
    pub id: String,
    pub title: String,
    pub description: String,
    pub seen: bool,
}

#[derive(serde::Serialize)]
pub struct GamificationResponse {
    pub total_xp: i64,
    pub level: u32,
    pub xp_into_level: i64,
    pub level_span: i64,
    pub current_streak_days: i32,
    pub longest_streak_days: i32,
    pub streak_active_today: bool,
    pub unlocks: Vec<UnlockDto>,
}

async fn me_gamification_inner(
    pool: &PgPool,
    ctx: &RequestContext,
) -> Result<Json<GamificationResponse>, ApiError> {
    let tenant_id = require_tenant(ctx)?;
    let mut tx = db::begin_with_context(pool, ctx.user_id, ctx.tenant_id)
        .await
        .map_err(internal)?;
    let stats = db::gamification::learner_stats(&mut tx, tenant_id, ctx.user_id)
        .await
        .map_err(internal)?;
    let active_today = db::gamification::active_today(&mut tx, tenant_id, ctx.user_id)
        .await
        .map_err(internal)?;
    let unlocks = db::gamification::unlocks(&mut tx, tenant_id, ctx.user_id)
        .await
        .map_err(internal)?;

    let total_xp = stats.as_ref().map(|s| s.total_xp).unwrap_or(0);
    let (level, xp_into_level, level_span) = db::gamification::level_for_xp(total_xp);
    Ok(Json(GamificationResponse {
        total_xp,
        level,
        xp_into_level,
        level_span,
        current_streak_days: stats.as_ref().map(|s| s.current_streak_days).unwrap_or(0),
        longest_streak_days: stats.as_ref().map(|s| s.longest_streak_days).unwrap_or(0),
        streak_active_today: active_today,
        unlocks: unlocks
            .into_iter()
            .map(|(id, title, description, seen)| UnlockDto {
                id,
                title,
                description,
                seen,
            })
            .collect(),
    }))
}

async fn mark_seen_inner(
    pool: &PgPool,
    ctx: &RequestContext,
) -> Result<Json<serde_json::Value>, ApiError> {
    let tenant_id = require_tenant(ctx)?;
    let mut tx = db::begin_with_context(pool, ctx.user_id, ctx.tenant_id)
        .await
        .map_err(internal)?;
    db::gamification::mark_unlocks_seen(&mut tx, tenant_id, ctx.user_id)
        .await
        .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct OptOutDto {
    pub opted_out: bool,
}

async fn get_opt_out_inner(
    pool: &PgPool,
    ctx: &RequestContext,
) -> Result<Json<OptOutDto>, ApiError> {
    let tenant_id = require_tenant(ctx)?;
    let mut tx = db::begin_with_context(pool, ctx.user_id, ctx.tenant_id)
        .await
        .map_err(internal)?;
    let opted_out = db::gamification::is_opted_out(&mut tx, tenant_id, ctx.user_id)
        .await
        .map_err(internal)?;
    Ok(Json(OptOutDto { opted_out }))
}

async fn put_opt_out_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    body: OptOutDto,
) -> Result<Json<OptOutDto>, ApiError> {
    let tenant_id = require_tenant(ctx)?;
    let mut tx = db::begin_with_context(pool, ctx.user_id, ctx.tenant_id)
        .await
        .map_err(internal)?;
    db::gamification::set_leaderboard_opt_out(&mut tx, tenant_id, ctx.user_id, body.opted_out)
        .await
        .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok(Json(body))
}

#[derive(serde::Serialize)]
pub struct LeaderboardRow {
    pub user_id: Uuid,
    pub display_name: String,
    pub course_xp: i64,
    pub is_me: bool,
}

async fn course_leaderboard_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
) -> Result<Json<Vec<LeaderboardRow>>, ApiError> {
    let tenant_id = require_tenant(ctx)?;
    if !db::courses::caller_can_read_course(
        pool,
        course_id,
        ctx.user_id,
        ctx.tenant_id,
        is_org_admin(ctx),
    )
    .await
    .map_err(internal)?
    {
        return Err(ApiError::Forbidden);
    }
    let mut tx = db::begin_with_context(pool, ctx.user_id, ctx.tenant_id)
        .await
        .map_err(internal)?;
    let rows = db::gamification::course_leaderboard(&mut tx, tenant_id, course_id, 50)
        .await
        .map_err(internal)?;
    Ok(Json(
        rows.into_iter()
            .map(|(user_id, display_name, email, course_xp)| LeaderboardRow {
                user_id,
                display_name: display_name
                    .filter(|n| !n.trim().is_empty())
                    .unwrap_or(email),
                course_xp,
                is_me: user_id == ctx.user_id,
            })
            .collect(),
    ))
}

// --- wrappers ---

macro_rules! me_wrappers {
    ($name:ident, $name_t:ident, $inner:ident, $ret:ty) => {
        async fn $name(
            State(state): State<AppState>,
            Extension(ctx): Extension<RequestContext>,
        ) -> Result<$ret, ApiError> {
            $inner(&state.pool, &ctx).await
        }
        async fn $name_t(
            State(s): State<TestState>,
            Extension(ctx): Extension<RequestContext>,
        ) -> Result<$ret, ApiError> {
            $inner(&s.pool, &ctx).await
        }
    };
}

me_wrappers!(
    me_gamification,
    me_gamification_t,
    me_gamification_inner,
    Json<GamificationResponse>
);
me_wrappers!(
    mark_seen,
    mark_seen_t,
    mark_seen_inner,
    Json<serde_json::Value>
);
me_wrappers!(
    get_opt_out,
    get_opt_out_t,
    get_opt_out_inner,
    Json<OptOutDto>
);

async fn put_opt_out(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Json(body): Json<OptOutDto>,
) -> Result<Json<OptOutDto>, ApiError> {
    put_opt_out_inner(&state.pool, &ctx, body).await
}

async fn put_opt_out_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Json(body): Json<OptOutDto>,
) -> Result<Json<OptOutDto>, ApiError> {
    put_opt_out_inner(&s.pool, &ctx, body).await
}

async fn course_leaderboard(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(cid): Path<Uuid>,
) -> Result<Json<Vec<LeaderboardRow>>, ApiError> {
    course_leaderboard_inner(&state.pool, &ctx, cid).await
}

async fn course_leaderboard_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(cid): Path<Uuid>,
) -> Result<Json<Vec<LeaderboardRow>>, ApiError> {
    course_leaderboard_inner(&s.pool, &ctx, cid).await
}
