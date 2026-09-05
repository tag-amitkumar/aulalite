use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::auth::jit_provision::ensure_user;
use crate::auth::local_login::LocalLoginConfig;
use crate::error::ApiError;

#[derive(Clone)]
pub struct DevLoginState {
    pub pool: PgPool,
    pub config: LocalLoginConfig,
}

#[derive(Deserialize)]
struct LocalLoginRequest {
    email: Option<String>,
    password: Option<String>,
}

#[derive(Serialize)]
struct LocalLoginResponse {
    id_token: String,
}

pub fn routes(state: DevLoginState) -> Router {
    Router::new()
        .route("/v1/auth/local-login", post(local_login))
        .with_state(state)
}

async fn local_login(
    State(state): State<DevLoginState>,
    Json(body): Json<LocalLoginRequest>,
) -> Result<Json<LocalLoginResponse>, ApiError> {
    if !state.config.is_enabled() {
        return Err(ApiError::Forbidden);
    }
    let email = body
        .email
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ApiError::BadRequest("email is required".into()))?;
    let password = body
        .password
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ApiError::BadRequest("password is required".into()))?;

    let profile = match state.config.profile_for_email_password(email, password) {
        Some(p) => p,
        None => {
            // Brief delay on failure. The local-login bypass is dev-only and
            // does not have a per-IP brute-force tracker; the 250ms penalty
            // makes naive credential-stuffing impractical without blocking
            // legitimate retries for a noticeable period.
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            return Err(ApiError::Unauthorized("invalid credentials".into()));
        }
    };

    ensure_user(&state.pool, &profile.claims())
        .await
        .map_err(|err| ApiError::Internal(format!("local user provisioning failed: {err}")))?;

    Ok(Json(LocalLoginResponse {
        id_token: profile.token.clone(),
    }))
}
