// crates/backend/src/handlers/privacy.rs
//! Self-service GDPR + auth-hardening endpoints for the calling user.
//!
//! Routes (all under the authed router; the caller acts only on themselves):
//!   * GET  /v1/me/export              — download a JSON bundle of the caller's
//!                                       own data across the active tenant.
//!   * POST /v1/me/delete              — right-to-erasure: ANONYMIZE the caller
//!                                       (tombstone display_name/email, null
//!                                       avatar/locale, stamp deleted_at) and
//!                                       revoke all tokens. Gradebook /
//!                                       authorship / attendance rows are kept
//!                                       intact (anonymized, never cascaded).
//!   * POST /v1/me/sessions/revoke-all — "sign out everywhere": bump
//!                                       `tokens_valid_after = now()` so all
//!                                       previously-issued tokens are rejected.
//!
//! There is no per-resource authorization to do here: the only subject is the
//! caller (`ctx.user_id`), and `db::privacy` pins every query to that id.
use axum::extract::{Extension, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{routing, Json, Router};
use serde::Serialize;

use crate::context::RequestContext;
use crate::db;
use crate::error::ApiError;
use crate::AppState;

/// `GET /v1/me/export` — stream the caller's data bundle as a JSON attachment.
///
/// The bundle is gathered under the caller's tenant/RLS context and contains
/// only their own rows (profile, enrollments, submissions, quiz attempts,
/// discussion posts, notes, bookmarks, lesson completions, attendance). We set
/// `Content-Disposition: attachment` so browsers offer a download rather than
/// rendering it inline.
async fn export_me(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Response, ApiError> {
    let bundle = db::privacy::export_for_user(&s.pool, ctx.user_id, ctx.tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let body = serde_json::to_vec_pretty(&bundle)
        .map_err(|e| ApiError::Internal(format!("serialize export: {e}")))?;

    // Date-stamped, filesystem-safe download name.
    let filename = format!(
        "aulalite-data-export-{}.json",
        bundle.generated_at.format("%Y%m%d")
    );

    Ok((
        StatusCode::OK,
        [
            (
                header::CONTENT_TYPE,
                "application/json; charset=utf-8".to_string(),
            ),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{filename}\""),
            ),
        ],
        body,
    )
        .into_response())
}

#[derive(Serialize)]
pub struct DeleteAccountResponse {
    pub anonymized: bool,
    pub anonymized_at: Option<chrono::DateTime<chrono::Utc>>,
    pub deleted_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// `POST /v1/me/delete` — right-to-erasure. Anonymizes the caller's `users` row
/// in place (preserving FK-referenced gradebook/authorship/attendance rows) and
/// revokes every outstanding token. Idempotent: re-invoking on an already
/// anonymized account succeeds and echoes the original stamps.
async fn delete_me(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<DeleteAccountResponse>, ApiError> {
    let status = db::privacy::anonymize_user(&s.pool, ctx.user_id)
        .await
        .map_err(map_erasure_error)?;

    Ok(Json(DeleteAccountResponse {
        anonymized: status.anonymized_at.is_some(),
        anonymized_at: status.anonymized_at,
        deleted_at: status.deleted_at,
    }))
}

fn map_erasure_error(error: sqlx::Error) -> ApiError {
    if let sqlx::Error::Database(database_error) = &error {
        match database_error.constraint() {
            Some("tenant_memberships_active_org_owner_required") => {
                return ApiError::Conflict(
                    "transfer organization ownership before deleting this account".into(),
                );
            }
            Some("users_last_platform_admin") => {
                return ApiError::Conflict(
                    "assign another platform owner before deleting this account".into(),
                );
            }
            _ => {}
        }
    }
    ApiError::Internal(error.to_string())
}

#[derive(Serialize)]
pub struct RevokeAllResponse {
    pub tokens_valid_after: chrono::DateTime<chrono::Utc>,
}

/// `POST /v1/me/sessions/revoke-all` — "sign out everywhere". Bumps the caller's
/// `tokens_valid_after` cutoff to now, so the auth middleware rejects any token
/// whose `iat` predates it. The current request still completes (its token was
/// already verified); the next request with an old token gets a 401.
async fn revoke_all(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<RevokeAllResponse>, ApiError> {
    let cutoff = db::privacy::revoke_all_tokens(&s.pool, ctx.user_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(RevokeAllResponse {
        tokens_valid_after: cutoff,
    }))
}

/// Routes for the privacy/auth-hardening endpoints. Merged into the authed
/// router by `lib.rs` so they sit behind `require_auth`.
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/me/export", routing::get(export_me))
        .route("/v1/me/delete", routing::post(delete_me))
        .route("/v1/me/sessions/revoke-all", routing::post(revoke_all))
}
