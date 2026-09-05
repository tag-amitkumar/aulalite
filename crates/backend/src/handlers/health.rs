// crates/backend/src/handlers/health.rs
use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;

use crate::AppState;

pub async fn healthz() -> &'static str {
    "ok"
}

#[derive(Debug, Serialize)]
struct ReadinessResponse {
    status: &'static str,
    dependencies: ReadinessDependencies,
}

#[derive(Debug, Serialize)]
struct ReadinessDependencies {
    database: &'static str,
    redis: &'static str,
    media: &'static str,
}

/// Dependency-aware rollout/readiness probe. It intentionally returns only
/// coarse states—never provider or connection details—because the route is
/// reachable by infrastructure without application authentication.
pub async fn readyz(State(state): State<AppState>) -> Response {
    let probe = async {
        let database = sqlx::query_scalar::<_, i32>("SELECT 1").fetch_one(&state.pool);
        let redis = state.live_room.healthz();
        let media = state.mediamtx.healthz();
        tokio::join!(database, redis, media)
    };

    let (database_ok, redis_ok, media_ok) =
        match tokio::time::timeout(Duration::from_secs(3), probe).await {
            Ok((database, redis, media)) => (database.is_ok(), redis.is_ok(), media.is_ok()),
            Err(_) => (false, false, false),
        };
    let ready = database_ok && redis_ok && media_ok;
    let body = ReadinessResponse {
        status: if ready { "ready" } else { "not_ready" },
        dependencies: ReadinessDependencies {
            database: if database_ok { "ok" } else { "unavailable" },
            redis: if redis_ok { "ok" } else { "unavailable" },
            media: if media_ok { "ok" } else { "unavailable" },
        },
    };
    (
        if ready {
            StatusCode::OK
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        },
        Json(body),
    )
        .into_response()
}
