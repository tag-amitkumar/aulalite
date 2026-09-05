//! Certificate endpoints (learning-suite Cycle 5).
//!
//! - `GET  /v1/courses/{cid}/certificates`                — staff: eligible + issued list
//! - `POST /v1/courses/{cid}/certificates/{user_id}/issue` — staff: issue (or re-issue)
//! - `POST /v1/courses/{cid}/certificates/{user_id}/revoke`— staff: revoke
//! - `GET  /v1/me/certificates`                          — student: own certificates
//! - `GET  /v1/verify/{credential_id}`                    — PUBLIC, unauthenticated
//!
//! Eligibility rows are created by `db::certificates::sync_eligibility` from
//! the lesson-completion and quiz-submission flows; these endpoints only read
//! and transition state.

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
        .route(
            "/v1/courses/{cid}/certificates",
            routing::get(list_for_course),
        )
        .route(
            "/v1/courses/{cid}/certificates/{user_id}/issue",
            routing::post(issue),
        )
        .route(
            "/v1/courses/{cid}/certificates/{user_id}/revoke",
            routing::post(revoke),
        )
        .route("/v1/me/certificates", routing::get(my_certificates))
}

/// Unauthenticated verification route. Merged OUTSIDE the require_auth layer.
pub fn public_routes() -> Router<AppState> {
    Router::new().route("/v1/verify/{credential_id}", routing::get(verify))
}

#[derive(Clone)]
struct TestState {
    pool: PgPool,
}

#[doc(hidden)]
pub fn router_for_tests(pool: PgPool) -> Router {
    Router::new()
        .route(
            "/v1/courses/{cid}/certificates",
            routing::get(list_for_course_t),
        )
        .route(
            "/v1/courses/{cid}/certificates/{user_id}/issue",
            routing::post(issue_t),
        )
        .route(
            "/v1/courses/{cid}/certificates/{user_id}/revoke",
            routing::post(revoke_t),
        )
        .route("/v1/me/certificates", routing::get(my_certificates_t))
        .route("/v1/verify/{credential_id}", routing::get(verify_t))
        .with_state(TestState { pool })
}

fn internal(e: impl std::fmt::Display) -> ApiError {
    ApiError::Internal(e.to_string())
}

fn is_org_admin(ctx: &RequestContext) -> bool {
    ctx.can_manage_organization()
}

fn require_tenant(ctx: &RequestContext) -> Result<Uuid, ApiError> {
    ctx.tenant_id
        .ok_or_else(|| ApiError::BadRequest("no active tenant".into()))
}

/// Course-staff gate shared by the three course-scoped endpoints.
async fn require_staff(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
) -> Result<(), ApiError> {
    if ctx.can_manage_organization() {
        return Ok(());
    }
    let allowed = db::courses::caller_can_staff_course(
        pool,
        course_id,
        ctx.user_id,
        ctx.tenant_id,
        is_org_admin(ctx),
    )
    .await
    .map_err(internal)?;
    if allowed {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
    }
}

#[derive(serde::Serialize)]
pub struct CertificateDto {
    pub id: Uuid,
    pub course_id: Uuid,
    pub user_id: Uuid,
    pub credential_id: Option<String>,
    pub status: String,
    pub recipient_name: Option<String>,
    pub course_title: Option<String>,
    pub student_display_name: Option<String>,
    pub student_email: String,
    pub issued_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl From<db::certificates::CertificateRow> for CertificateDto {
    fn from(r: db::certificates::CertificateRow) -> Self {
        Self {
            id: r.id,
            course_id: r.course_id,
            user_id: r.user_id,
            credential_id: r.credential_id,
            status: r.status,
            recipient_name: r.recipient_name,
            course_title: r.course_title,
            student_display_name: r.display_name,
            student_email: r.email,
            issued_at: r.issued_at,
            created_at: r.created_at,
        }
    }
}

async fn list_for_course_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
) -> Result<Json<Vec<CertificateDto>>, ApiError> {
    let tenant_id = require_tenant(ctx)?;
    require_staff(pool, ctx, course_id).await?;
    let mut tx = db::begin_with_context(pool, ctx.user_id, Some(tenant_id))
        .await
        .map_err(internal)?;
    let rows = db::certificates::list_for_course(&mut tx, course_id)
        .await
        .map_err(internal)?;
    Ok(Json(rows.into_iter().map(CertificateDto::from).collect()))
}

async fn my_certificates_inner(
    pool: &PgPool,
    ctx: &RequestContext,
) -> Result<Json<Vec<CertificateDto>>, ApiError> {
    let tenant_id = require_tenant(ctx)?;
    let mut tx = db::begin_with_context(pool, ctx.user_id, Some(tenant_id))
        .await
        .map_err(internal)?;
    let rows = db::certificates::list_mine(&mut tx, tenant_id, ctx.user_id)
        .await
        .map_err(internal)?;
    Ok(Json(rows.into_iter().map(CertificateDto::from).collect()))
}

/// Shared issue logic: gate, transition, audit. Returns the updated row;
/// notification fan-out happens only in the production wrapper, after commit.
async fn issue_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
    student_id: Uuid,
) -> Result<Json<CertificateDto>, ApiError> {
    let tenant_id = require_tenant(ctx)?;
    require_staff(pool, ctx, course_id).await?;
    let mut tx = db::begin_with_context(pool, ctx.user_id, Some(tenant_id))
        .await
        .map_err(internal)?;
    let row = db::certificates::issue(&mut tx, course_id, student_id, ctx.user_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| ApiError::BadRequest("student is not eligible".into()))?;
    db::audit::emit_audit_event(
        &mut tx,
        tenant_id,
        ctx.user_id,
        "certificate.issue",
        "certificate",
        row.id,
        None,
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok(Json(CertificateDto::from(row)))
}

async fn revoke_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
    student_id: Uuid,
) -> Result<axum::http::StatusCode, ApiError> {
    let tenant_id = require_tenant(ctx)?;
    require_staff(pool, ctx, course_id).await?;
    let mut tx = db::begin_with_context(pool, ctx.user_id, Some(tenant_id))
        .await
        .map_err(internal)?;
    let revoked = db::certificates::revoke(&mut tx, course_id, student_id)
        .await
        .map_err(internal)?;
    if !revoked {
        return Err(ApiError::NotFound);
    }
    db::audit::emit_audit_event(
        &mut tx,
        tenant_id,
        ctx.user_id,
        "certificate.revoke",
        "certificate",
        student_id,
        None,
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

#[derive(serde::Serialize)]
pub struct VerifyDto {
    pub credential_id: String,
    /// `issued` (valid) or `revoked`.
    pub status: String,
    pub recipient_name: Option<String>,
    pub course_title: Option<String>,
    pub issued_at: Option<chrono::DateTime<chrono::Utc>>,
}

async fn verify_inner(pool: &PgPool, credential_id: &str) -> Result<Json<VerifyDto>, ApiError> {
    let row = db::certificates::verify(pool, credential_id)
        .await
        .map_err(internal)?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(VerifyDto {
        credential_id: row.credential_id,
        status: row.status,
        recipient_name: row.recipient_name,
        course_title: row.course_title,
        issued_at: row.issued_at,
    }))
}

// --- production handlers ---

async fn list_for_course(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(cid): Path<Uuid>,
) -> Result<Json<Vec<CertificateDto>>, ApiError> {
    list_for_course_inner(&s.pool, &ctx, cid).await
}

async fn my_certificates(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<Vec<CertificateDto>>, ApiError> {
    my_certificates_inner(&s.pool, &ctx).await
}

async fn issue(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path((cid, student_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<CertificateDto>, ApiError> {
    let result = issue_inner(&s.pool, &ctx, cid, student_id).await?;
    // Best-effort notification AFTER the issue committed.
    if let Some(tenant_id) = ctx.tenant_id {
        let course_title = result
            .0
            .course_title
            .clone()
            .unwrap_or_else(|| "your course".to_string());
        let link = format!("{}/certificates", s.app_origin.trim_end_matches('/'));
        crate::services::notifications::notify(
            &s.pool,
            s.email_notifier.as_ref(),
            s.push_sender.as_ref(),
            tenant_id,
            student_id,
            "certificate_issued",
            "Certificate issued",
            Some(&format!(
                "Congratulations! Your certificate for \"{course_title}\" has been issued."
            )),
            Some(&link),
        )
        .await;
    }
    Ok(result)
}

async fn revoke(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path((cid, student_id)): Path<(Uuid, Uuid)>,
) -> Result<axum::http::StatusCode, ApiError> {
    revoke_inner(&s.pool, &ctx, cid, student_id).await
}

async fn verify(
    State(s): State<AppState>,
    Path(credential_id): Path<String>,
) -> Result<Json<VerifyDto>, ApiError> {
    verify_inner(&s.pool, &credential_id).await
}

// --- test wrappers ---

async fn list_for_course_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(cid): Path<Uuid>,
) -> Result<Json<Vec<CertificateDto>>, ApiError> {
    list_for_course_inner(&s.pool, &ctx, cid).await
}

async fn my_certificates_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<Vec<CertificateDto>>, ApiError> {
    my_certificates_inner(&s.pool, &ctx).await
}

async fn issue_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path((cid, student_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<CertificateDto>, ApiError> {
    issue_inner(&s.pool, &ctx, cid, student_id).await
}

async fn revoke_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path((cid, student_id)): Path<(Uuid, Uuid)>,
) -> Result<axum::http::StatusCode, ApiError> {
    revoke_inner(&s.pool, &ctx, cid, student_id).await
}

async fn verify_t(
    State(s): State<TestState>,
    Path(credential_id): Path<String>,
) -> Result<Json<VerifyDto>, ApiError> {
    verify_inner(&s.pool, &credential_id).await
}
