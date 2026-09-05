// crates/backend/src/error.rs
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    // Phase 0 variants (unchanged)
    #[error("unauthorized: {0}")]
    Unauthorized(String),
    #[error("forbidden")]
    Forbidden,
    #[error("not found")]
    NotFound,
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("internal: {0}")]
    Internal(String),

    // Phase 1a variants
    #[error("course not found")]
    CourseNotFound,
    #[error("enrollment code is invalid, expired, or fully used")]
    EnrollmentCodeInvalid,
    #[error("invitation is invalid, expired, or already accepted")]
    InvitationInvalid,
    #[error("lesson type not yet supported: {0}")]
    LessonTypeNotSupported(String),
    #[error("recurrence shape invalid: {0}")]
    RecurrenceShapeInvalid(String),

    // Phase 1b-alpha variants
    #[error("file asset not found")]
    FileAssetNotFound,
    #[error("upload validation failed: {0}")]
    UploadValidationFailed(String),
    #[error("upload object missing or size mismatch")]
    UploadObjectMissing,
    #[error("object storage temporarily unavailable")]
    StorageUnavailable,

    // Phase 1b-beta variants
    #[error("session not in valid state for this transition: {0}")]
    SessionStateInvalid(String),
    #[error("session window not open: {0}")]
    SessionWindowClosed(String),
    #[error("publish nonce invalid or expired")]
    PublishNonceInvalid,
    #[error("media server unreachable")]
    MediaServerUnreachable,
    #[error("rate limited")]
    RateLimited,
    #[error("monthly live-class minutes limit reached")]
    ClassMinutesLimitReached,
    #[error("recording storage limit reached")]
    RecordingStorageLimitReached,

    // Phase 1c variants
    #[error("validation: {0}")]
    Validation(String),
    #[error("conflict: {0}")]
    Conflict(String),
}

impl ApiError {
    /// Returns a short, user-safe string describing this error suitable for
    /// surfacing in UI toasts via the `command_failed` socket event. The text
    /// must never leak server internals; we map every variant to a stable
    /// short reason rather than reusing the raw `Display` impl (which can
    /// embed inner error strings).
    pub fn user_facing(&self) -> String {
        match self {
            ApiError::Unauthorized(_) => "unauthorized".into(),
            ApiError::Forbidden => "forbidden".into(),
            ApiError::NotFound => "not found".into(),
            ApiError::BadRequest(_) => "bad request".into(),
            ApiError::Internal(_) => "internal error".into(),
            ApiError::CourseNotFound => "course not found".into(),
            ApiError::EnrollmentCodeInvalid => "enrollment code invalid".into(),
            ApiError::InvitationInvalid => "invitation invalid".into(),
            ApiError::LessonTypeNotSupported(_) => "lesson type not supported".into(),
            ApiError::RecurrenceShapeInvalid(_) => "recurrence shape invalid".into(),
            ApiError::FileAssetNotFound => "file asset not found".into(),
            ApiError::UploadValidationFailed(_) => "upload validation failed".into(),
            ApiError::UploadObjectMissing => "upload missing".into(),
            ApiError::StorageUnavailable => "storage unavailable".into(),
            ApiError::SessionStateInvalid(_) => "session state invalid".into(),
            ApiError::SessionWindowClosed(_) => "session window closed".into(),
            ApiError::PublishNonceInvalid => "publish credential invalid".into(),
            ApiError::MediaServerUnreachable => "media server unreachable".into(),
            ApiError::RateLimited => "rate limited".into(),
            ApiError::ClassMinutesLimitReached => "class minutes limit reached".into(),
            ApiError::RecordingStorageLimitReached => "recording storage limit reached".into(),
            ApiError::Validation(_) => "validation error".into(),
            ApiError::Conflict(_) => "conflict".into(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            ApiError::Unauthorized(message) => (StatusCode::UNAUTHORIZED, message.clone()),
            ApiError::Forbidden => (StatusCode::FORBIDDEN, "forbidden".into()),
            ApiError::NotFound => (StatusCode::NOT_FOUND, "not found".into()),
            ApiError::BadRequest(message) => (StatusCode::BAD_REQUEST, message.clone()),
            ApiError::Internal(message) => {
                // Keep the actionable provider/DB detail in server telemetry,
                // but never serialize it into the public response. SQLx and
                // upstream errors can contain schema names, query fragments,
                // endpoints, and other operational details.
                tracing::error!(error = %message, "request failed with an internal error");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal error".into())
            }
            ApiError::CourseNotFound => (
                StatusCode::NOT_FOUND,
                "course not found or not accessible".into(),
            ),
            ApiError::EnrollmentCodeInvalid => (
                StatusCode::BAD_REQUEST,
                "enrollment code is invalid, expired, or fully used".into(),
            ),
            ApiError::InvitationInvalid => (
                StatusCode::BAD_REQUEST,
                "invitation is invalid, expired, or already accepted".into(),
            ),
            ApiError::LessonTypeNotSupported(t) => (
                StatusCode::BAD_REQUEST,
                format!("lesson type not yet supported: {t}"),
            ),
            ApiError::RecurrenceShapeInvalid(reason) => (
                StatusCode::BAD_REQUEST,
                format!("recurrence shape invalid: {reason}"),
            ),
            ApiError::FileAssetNotFound => (StatusCode::NOT_FOUND, "file asset not found".into()),
            ApiError::UploadValidationFailed(reason) => (
                StatusCode::BAD_REQUEST,
                format!("upload validation failed: {reason}"),
            ),
            ApiError::UploadObjectMissing => (
                StatusCode::BAD_REQUEST,
                "upload object missing or size mismatch".into(),
            ),
            ApiError::StorageUnavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "object storage temporarily unavailable; retry".into(),
            ),
            ApiError::SessionStateInvalid(reason) => {
                (StatusCode::CONFLICT, format!("session state: {reason}"))
            }
            ApiError::SessionWindowClosed(reason) => (
                StatusCode::BAD_REQUEST,
                format!("session window closed: {reason}"),
            ),
            ApiError::PublishNonceInvalid => (
                StatusCode::FORBIDDEN,
                "publish nonce invalid or expired".into(),
            ),
            ApiError::MediaServerUnreachable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "media server unreachable".into(),
            ),
            ApiError::RateLimited => (StatusCode::TOO_MANY_REQUESTS, "rate limited".into()),
            ApiError::ClassMinutesLimitReached => (
                StatusCode::PAYMENT_REQUIRED,
                "class_minutes_limit_reached".into(),
            ),
            ApiError::RecordingStorageLimitReached => (
                StatusCode::PAYMENT_REQUIRED,
                "recording_storage_limit_reached".into(),
            ),
            ApiError::Validation(reason) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("validation: {reason}"),
            ),
            ApiError::Conflict(reason) => (StatusCode::CONFLICT, format!("conflict: {reason}")),
        };

        (status, axum::Json(json!({ "error": message }))).into_response()
    }
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use http_body_util::BodyExt;

    use super::ApiError;

    #[tokio::test]
    async fn internal_error_response_is_stable_and_does_not_leak_detail() {
        let sensitive = "db error: duplicate key violates users_email_key at 10.0.0.5";
        let response = ApiError::Internal(sensitive.into()).into_response();

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body, serde_json::json!({ "error": "internal error" }));
        assert!(!String::from_utf8_lossy(&bytes).contains(sensitive));
    }

    #[tokio::test]
    async fn usage_limit_errors_are_stable_payment_required_responses() {
        for (error, code) in [
            (
                ApiError::ClassMinutesLimitReached,
                "class_minutes_limit_reached",
            ),
            (
                ApiError::RecordingStorageLimitReached,
                "recording_storage_limit_reached",
            ),
        ] {
            let response = error.into_response();
            assert_eq!(response.status(), StatusCode::PAYMENT_REQUIRED);
            let bytes = response.into_body().collect().await.unwrap().to_bytes();
            let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(body, serde_json::json!({ "error": code }));
        }
    }
}
