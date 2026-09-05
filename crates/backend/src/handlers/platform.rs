// crates/backend/src/handlers/platform.rs
//
// PLATFORM SUPER-ADMIN endpoints — cross-tenant management + production tenant
// provisioning for the Elementors staff role.
//
// AUTH MODEL: every endpoint here is gated solely on `PlatformManage`
// (the bool on RequestContext, NOT a TenantRole). These operate ACROSS ALL
// tenants, so — unlike the org-admin endpoints in handlers/admin.rs — they do
// NOT require a `ctx.tenant_id`: a platform admin may have no tenant membership.
// The reads/writes bypass per-tenant RLS via the SECURITY DEFINER functions in
// migration 20260530000001_platform_admin.sql (see db::platform).
//
// TENANT PROVISIONING: POST atomically creates the tenant and a privileged,
// pending `org_owner` invitation. Ordinary invitation APIs cannot grant that
// role. JIT acceptance turns it into the tenant's active owner membership.
use axum::extract::{Extension, Path, State};
use axum::{routing, Json, Router};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::context::RequestContext;
use crate::db;
use crate::error::ApiError;
use crate::AppState;

// ===========================================================================
// DTOs — FIXED CONTRACT shared with the platform-admin frontend. Do NOT
// reorder/rename fields.
// ===========================================================================

#[derive(Serialize)]
pub struct TenantSummaryDto {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub status: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub member_count: i64,
    pub plan_id: Option<String>,
}

impl From<db::platform::TenantSummaryRow> for TenantSummaryDto {
    fn from(r: db::platform::TenantSummaryRow) -> Self {
        Self {
            id: r.id,
            slug: r.slug,
            name: r.name,
            status: r.status,
            created_at: r.created_at,
            member_count: r.member_count,
            plan_id: r.plan_id,
        }
    }
}

#[derive(Deserialize)]
pub struct CreateTenantBody {
    pub slug: String,
    pub name: String,
    pub admin_email: String,
}

#[derive(Deserialize)]
pub struct PatchTenantStatusBody {
    pub status: String,
}

#[derive(Deserialize)]
pub struct RecoverOwnerBody {
    pub new_owner_user_id: Uuid,
    pub confirmation: String,
}

#[derive(Serialize)]
pub struct OwnerRecoveryDto {
    pub previous_owner_user_id: Option<Uuid>,
    pub new_owner_user_id: Uuid,
    pub transferred_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Serialize)]
pub struct OwnerCandidateDto {
    pub user_id: Uuid,
    pub email: String,
    pub display_name: Option<String>,
    pub role: String,
}

#[derive(Deserialize)]
pub struct ReplacePendingOwnerInvitationBody {
    pub email: String,
    pub confirmation: String,
}

#[derive(Serialize)]
pub struct PendingOwnerInvitationDto {
    pub invitation_id: Uuid,
    pub email: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

// ===========================================================================
// Authorization + validation helpers
// ===========================================================================

/// Platform super-admin ONLY. Org-admins (or any TenantRole) are NOT enough.
fn require_platform_admin(ctx: &RequestContext) -> Result<(), ApiError> {
    if ctx.can_manage_platform() {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
    }
}

/// Tenant lifecycle statuses settable via PATCH. Matches the tenants CHECK.
fn valid_status(status: &str) -> bool {
    matches!(status, "active" | "suspended" | "trialing")
}

// ===========================================================================
// Routers
// ===========================================================================

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/v1/platform/tenants",
            routing::get(list_tenants).post(create_tenant),
        )
        .route(
            "/v1/platform/tenants/{id}",
            routing::patch(patch_tenant_status),
        )
        .route(
            "/v1/platform/tenants/{id}/recover-owner",
            routing::post(recover_owner),
        )
        .route(
            "/v1/platform/tenants/{id}/ownership-candidates",
            routing::get(list_owner_candidates),
        )
        .route(
            "/v1/platform/tenants/{id}/pending-owner-invitation",
            routing::patch(replace_pending_owner_invitation),
        )
}

/// Test-only router mirroring [`routes`] with a plain `PgPool` state so
/// integration tests can drive the endpoints under StubAuth.
#[doc(hidden)]
pub fn router_for_tests(pool: PgPool) -> Router {
    Router::new()
        .route(
            "/v1/platform/tenants",
            routing::get(list_tenants_t).post(create_tenant_t),
        )
        .route(
            "/v1/platform/tenants/{id}",
            routing::patch(patch_tenant_status_t),
        )
        .route(
            "/v1/platform/tenants/{id}/recover-owner",
            routing::post(recover_owner_t),
        )
        .route(
            "/v1/platform/tenants/{id}/ownership-candidates",
            routing::get(list_owner_candidates_t),
        )
        .route(
            "/v1/platform/tenants/{id}/pending-owner-invitation",
            routing::patch(replace_pending_owner_invitation_t),
        )
        .with_state(TestState { pool })
}

#[derive(Clone)]
struct TestState {
    pool: PgPool,
}

// ===========================================================================
// Production handlers
// ===========================================================================

async fn list_tenants(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<Vec<TenantSummaryDto>>, ApiError> {
    list_inner(&s.pool, &ctx).await
}

async fn create_tenant(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Json(b): Json<CreateTenantBody>,
) -> Result<Json<TenantSummaryDto>, ApiError> {
    create_inner(
        &s.pool,
        Some(s.email_link_sender.as_ref()),
        &s.app_origin,
        &ctx,
        b,
    )
    .await
}

async fn patch_tenant_status(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    Json(b): Json<PatchTenantStatusBody>,
) -> Result<Json<TenantSummaryDto>, ApiError> {
    patch_status_inner(&s.pool, &ctx, id, b).await
}

async fn recover_owner(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    Json(body): Json<RecoverOwnerBody>,
) -> Result<Json<OwnerRecoveryDto>, ApiError> {
    recover_owner_inner(&s.pool, &ctx, id, body).await
}

async fn list_owner_candidates(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<OwnerCandidateDto>>, ApiError> {
    list_owner_candidates_inner(&s.pool, &ctx, id).await
}

async fn replace_pending_owner_invitation(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    Json(body): Json<ReplacePendingOwnerInvitationBody>,
) -> Result<Json<PendingOwnerInvitationDto>, ApiError> {
    replace_pending_owner_invitation_inner(
        &s.pool,
        Some(s.email_link_sender.as_ref()),
        &s.app_origin,
        &ctx,
        id,
        body,
    )
    .await
}

// ===========================================================================
// Test wrappers
// ===========================================================================

async fn list_tenants_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<Vec<TenantSummaryDto>>, ApiError> {
    list_inner(&s.pool, &ctx).await
}

async fn create_tenant_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Json(b): Json<CreateTenantBody>,
) -> Result<Json<TenantSummaryDto>, ApiError> {
    create_inner(&s.pool, None, "http://localhost", &ctx, b).await
}

async fn patch_tenant_status_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    Json(b): Json<PatchTenantStatusBody>,
) -> Result<Json<TenantSummaryDto>, ApiError> {
    patch_status_inner(&s.pool, &ctx, id, b).await
}

async fn recover_owner_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    Json(body): Json<RecoverOwnerBody>,
) -> Result<Json<OwnerRecoveryDto>, ApiError> {
    recover_owner_inner(&s.pool, &ctx, id, body).await
}

async fn list_owner_candidates_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<OwnerCandidateDto>>, ApiError> {
    list_owner_candidates_inner(&s.pool, &ctx, id).await
}

async fn replace_pending_owner_invitation_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    Json(body): Json<ReplacePendingOwnerInvitationBody>,
) -> Result<Json<PendingOwnerInvitationDto>, ApiError> {
    replace_pending_owner_invitation_inner(&s.pool, None, "http://localhost", &ctx, id, body).await
}

// ===========================================================================
// Inner logic
// ===========================================================================

async fn list_inner(
    pool: &PgPool,
    ctx: &RequestContext,
) -> Result<Json<Vec<TenantSummaryDto>>, ApiError> {
    require_platform_admin(ctx)?;
    let rows = db::platform::list_tenants(pool, ctx.user_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(rows.into_iter().map(TenantSummaryDto::from).collect()))
}

async fn create_inner(
    pool: &PgPool,
    email_sender: Option<&dyn crate::services::invitations::EmailLinkSender>,
    app_origin: &str,
    ctx: &RequestContext,
    b: CreateTenantBody,
) -> Result<Json<TenantSummaryDto>, ApiError> {
    require_platform_admin(ctx)?;

    let slug = b.slug.trim();
    let name = b.name.trim();
    let admin_email = b.admin_email.trim();
    if !valid_email(admin_email) {
        return Err(ApiError::Validation("invalid_admin_email".into()));
    }

    // Create the tenant (slug format validated in Rust first; duplicate slug
    // surfaces as a Conflict).
    let tenant_id = match db::platform::create_tenant(pool, slug, name, admin_email, ctx.user_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
    {
        db::platform::CreateTenantOutcome::Created(id) => id,
        db::platform::CreateTenantOutcome::Invalid(reason) => {
            return Err(ApiError::Validation(reason.into()))
        }
        db::platform::CreateTenantOutcome::Conflict(reason) => {
            return Err(ApiError::Conflict(reason.into()))
        }
    };

    if let Some(sender) = email_sender {
        let continue_url = format!("{}/accept-invite", app_origin.trim_end_matches('/'));
        sender
            .send_invite(admin_email, &continue_url)
            .await
            .map_err(|error| ApiError::Internal(format!("email send failed: {error}")))?;
    }

    // Re-read the summary for the response (member_count = 0, plan_id = None on
    // a brand-new tenant).
    let row = db::platform::get_tenant_summary(pool, tenant_id, ctx.user_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(row.into()))
}

async fn patch_status_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    id: Uuid,
    b: PatchTenantStatusBody,
) -> Result<Json<TenantSummaryDto>, ApiError> {
    require_platform_admin(ctx)?;

    let status = b.status.trim();
    if !valid_status(status) {
        return Err(ApiError::Validation(format!("invalid status: {status}")));
    }

    let updated = db::platform::set_tenant_status(pool, id, status, ctx.user_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !updated {
        return Err(ApiError::NotFound);
    }

    // Status mutation and audit event commit atomically inside the definer.
    let row = db::platform::get_tenant_summary(pool, id, ctx.user_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(row.into()))
}

async fn recover_owner_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    tenant_id: Uuid,
    body: RecoverOwnerBody,
) -> Result<Json<OwnerRecoveryDto>, ApiError> {
    require_platform_admin(ctx)?;
    if body.confirmation.trim() != "RECOVER OWNERSHIP" {
        return Err(ApiError::Validation(
            "ownership_recovery_confirmation_required".into(),
        ));
    }

    let row =
        db::platform::recover_tenant_owner(pool, tenant_id, body.new_owner_user_id, ctx.user_id)
            .await
            .map_err(map_owner_recovery_error)?;

    Ok(Json(OwnerRecoveryDto {
        previous_owner_user_id: row.previous_owner_user_id,
        new_owner_user_id: row.new_owner_user_id,
        transferred_at: row.transferred_at,
    }))
}

async fn list_owner_candidates_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    tenant_id: Uuid,
) -> Result<Json<Vec<OwnerCandidateDto>>, ApiError> {
    require_platform_admin(ctx)?;
    let rows = db::platform::list_tenant_owner_candidates(pool, tenant_id, ctx.user_id)
        .await
        .map_err(map_owner_recovery_error)?;
    Ok(Json(
        rows.into_iter()
            .map(|row| OwnerCandidateDto {
                user_id: row.user_id,
                email: row.email,
                display_name: row.display_name,
                role: row.role,
            })
            .collect(),
    ))
}

fn map_owner_recovery_error(error: sqlx::Error) -> ApiError {
    if let sqlx::Error::Database(database_error) = &error {
        if database_error
            .message()
            .contains("target_must_be_active_org_admin")
        {
            return ApiError::Conflict("target_must_be_active_org_admin".into());
        }
        if database_error
            .message()
            .contains("target_must_be_active_member")
        {
            return ApiError::Conflict("target_must_be_active_member".into());
        }
        if database_error.message().contains("tenant_not_found") {
            return ApiError::NotFound;
        }
    }
    ApiError::Internal(error.to_string())
}

async fn replace_pending_owner_invitation_inner(
    pool: &PgPool,
    email_sender: Option<&dyn crate::services::invitations::EmailLinkSender>,
    app_origin: &str,
    ctx: &RequestContext,
    tenant_id: Uuid,
    body: ReplacePendingOwnerInvitationBody,
) -> Result<Json<PendingOwnerInvitationDto>, ApiError> {
    require_platform_admin(ctx)?;
    if body.confirmation.trim() != "REPLACE OWNER INVITATION" {
        return Err(ApiError::Validation(
            "owner_invitation_replacement_confirmation_required".into(),
        ));
    }
    let email = body.email.trim();
    if !valid_email(email) {
        return Err(ApiError::Validation("invalid_owner_email".into()));
    }

    let row = db::platform::replace_pending_owner_invitation(pool, tenant_id, email, ctx.user_id)
        .await
        .map_err(map_pending_owner_invitation_error)?;

    if let Some(sender) = email_sender {
        let continue_url = format!("{}/accept-invite", app_origin.trim_end_matches('/'));
        sender
            .send_invite(email, &continue_url)
            .await
            .map_err(|error| ApiError::Internal(format!("email send failed: {error}")))?;
    }

    Ok(Json(PendingOwnerInvitationDto {
        invitation_id: row.invitation_id,
        email: row.email,
        created_at: row.created_at,
    }))
}

fn valid_email(email: &str) -> bool {
    if email.is_empty()
        || email.len() > 254
        || email
            .bytes()
            .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control())
    {
        return false;
    }
    let mut parts = email.split('@');
    let local = parts.next().unwrap_or_default();
    let domain = parts.next().unwrap_or_default();
    !local.is_empty()
        && local.len() <= 64
        && !domain.is_empty()
        && domain.len() <= 253
        && parts.next().is_none()
}

fn map_pending_owner_invitation_error(error: sqlx::Error) -> ApiError {
    if let sqlx::Error::Database(database_error) = &error {
        let message = database_error.message();
        if message.contains("tenant_not_found")
            || message.contains("pending_owner_invitation_not_found")
        {
            return ApiError::NotFound;
        }
        if message.contains("organization_owner_already_claimed") {
            return ApiError::Conflict("organization_owner_already_claimed".into());
        }
        if database_error.code().as_deref() == Some("23505") {
            return ApiError::Conflict("pending_invitation_email_conflict".into());
        }
        if message.contains("invalid_owner_email") {
            return ApiError::Validation("invalid_owner_email".into());
        }
    }
    ApiError::Internal(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::valid_status;

    #[test]
    fn valid_status_accepts_lifecycle_states() {
        for s in ["active", "suspended", "trialing"] {
            assert!(valid_status(s), "{s} should be valid");
        }
    }

    #[test]
    fn valid_status_rejects_unknown() {
        assert!(!valid_status("past_due"));
        assert!(!valid_status(""));
        assert!(!valid_status("Active"));
    }
}
