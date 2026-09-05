// crates/backend/src/handlers/member_invitations.rs
//
// Admin tenant member-invitation endpoints (org-admin / platform-admin only).
//
// A member invitation issues (or re-activates) a `tenant_memberships` seat for
// an email under a role. The TENANT-WIDE seat cap check and pending-invitation
// reservation happen atomically behind the tenant row lock in
// `db::member_invitations::create_invitation_with_seat_reservation`.
//
// The invited account is JIT-provisioned on first sign-in, at which point
// `accept_tenant_invitations_for_email` (re)activates the membership.
use axum::extract::{Extension, Path, State};
use axum::{routing, Json, Router};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::context::RequestContext;
use crate::db;
use crate::error::ApiError;
use crate::AppState;

/// Stable machine code returned (as the `ApiError::Conflict` reason) when the
/// tenant-wide seat cap blocks a seat-consuming action. Shared so both this
/// handler and `handlers::admin::patch_membership` surface the SAME code.
pub use crate::db::seats::SEAT_LIMIT_REACHED;

// ===========================================================================
// DTOs
// ===========================================================================

#[derive(Serialize)]
pub struct InvitationDto {
    pub id: Uuid,
    pub email: String,
    pub role: String,
    pub status: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Deserialize)]
pub struct CreateMemberInvitation {
    pub email: String,
    pub role: String,
}

// ===========================================================================
// Authorization
// ===========================================================================

/// org_admin OR platform admin.
fn is_org_admin(ctx: &RequestContext) -> bool {
    ctx.can_manage_members()
}

/// Validate the requested role against the tenant_invitations CHECK constraint.
fn valid_role(role: &str) -> bool {
    matches!(role, "org_admin" | "teacher" | "ta" | "student" | "parent")
}

// ===========================================================================
// Routers
// ===========================================================================

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/v1/admin/member-invitations",
            routing::post(create_member_invitation).get(list_member_invitations),
        )
        .route(
            "/v1/admin/member-invitations/{id}",
            routing::delete(revoke_member_invitation),
        )
}

#[doc(hidden)]
pub fn router_for_tests(
    pool: PgPool,
    sender: std::sync::Arc<dyn crate::services::invitations::EmailLinkSender>,
    app_origin: String,
) -> Router {
    let state = TestState {
        pool,
        sender,
        app_origin,
    };
    Router::new()
        .route(
            "/v1/admin/member-invitations",
            routing::post(create_member_invitation_t).get(list_member_invitations_t),
        )
        .route(
            "/v1/admin/member-invitations/{id}",
            routing::delete(revoke_member_invitation_t),
        )
        .with_state(state)
}

#[derive(Clone)]
struct TestState {
    pool: PgPool,
    sender: std::sync::Arc<dyn crate::services::invitations::EmailLinkSender>,
    app_origin: String,
}

// ===========================================================================
// Production handlers
// ===========================================================================

async fn create_member_invitation(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Json(b): Json<CreateMemberInvitation>,
) -> Result<Json<InvitationDto>, ApiError> {
    create_inner(
        &s.pool,
        s.email_link_sender.as_ref(),
        &s.app_origin,
        &ctx,
        b,
    )
    .await
}

async fn list_member_invitations(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<Vec<InvitationDto>>, ApiError> {
    list_inner(&s.pool, &ctx).await
}

async fn revoke_member_invitation(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<axum::http::StatusCode, ApiError> {
    revoke_inner(&s.pool, &ctx, id).await
}

// ===========================================================================
// Test wrappers
// ===========================================================================

async fn create_member_invitation_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Json(b): Json<CreateMemberInvitation>,
) -> Result<Json<InvitationDto>, ApiError> {
    create_inner(&s.pool, s.sender.as_ref(), &s.app_origin, &ctx, b).await
}

async fn list_member_invitations_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<Vec<InvitationDto>>, ApiError> {
    list_inner(&s.pool, &ctx).await
}

async fn revoke_member_invitation_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<axum::http::StatusCode, ApiError> {
    revoke_inner(&s.pool, &ctx, id).await
}

// ===========================================================================
// Inner logic
// ===========================================================================

async fn create_inner(
    pool: &PgPool,
    sender: &dyn crate::services::invitations::EmailLinkSender,
    app_origin: &str,
    ctx: &RequestContext,
    b: CreateMemberInvitation,
) -> Result<Json<InvitationDto>, ApiError> {
    if !is_org_admin(ctx) {
        return Err(ApiError::Forbidden);
    }
    let tenant_id = ctx
        .tenant_id
        .ok_or_else(|| ApiError::BadRequest("no tenant".into()))?;

    // Validate the role against the CHECK constraint.
    if !valid_role(&b.role) {
        return Err(ApiError::BadRequest(format!("invalid role: {}", b.role)));
    }
    if b.role == "org_admin" && !ctx.owns_organization() {
        return Err(ApiError::Forbidden);
    }

    let invitation = match db::member_invitations::create_invitation_with_seat_reservation(
        pool,
        tenant_id,
        &b.email,
        &b.role,
        ctx.user_id,
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?
    {
        db::member_invitations::SeatReservationOutcome::Reserved(invitation) => invitation,
        db::member_invitations::SeatReservationOutcome::SeatLimitReached => {
            return Err(ApiError::Conflict(SEAT_LIMIT_REACHED.into()));
        }
    };
    if invitation.role != b.role {
        return Err(ApiError::Conflict(
            "pending_invitation_role_conflict".into(),
        ));
    }

    // Audit-log the creation in its own GUC-scoped tx.
    {
        let mut tx = db::begin_with_context(pool, ctx.user_id, Some(tenant_id))
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        db::audit::emit_audit_event(
            &mut tx,
            tenant_id,
            ctx.user_id,
            "member_invitation.create",
            "tenant_invitation",
            invitation.id,
            None,
        )
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
        tx.commit()
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
    }

    // Send the Firebase email-link sign-in, mirroring parent/course invitations:
    // the continue URL is built from `app_origin`.
    let continue_url = format!("{}/accept-invite", app_origin.trim_end_matches('/'));
    sender
        .send_invite(&b.email, &continue_url)
        .await
        .map_err(|e| ApiError::Internal(format!("email send failed: {e}")))?;

    Ok(Json(InvitationDto {
        id: invitation.id,
        email: invitation.email,
        role: invitation.role,
        status: invitation.status,
        created_at: invitation.created_at,
    }))
}

async fn list_inner(
    pool: &PgPool,
    ctx: &RequestContext,
) -> Result<Json<Vec<InvitationDto>>, ApiError> {
    if !is_org_admin(ctx) {
        return Err(ApiError::Forbidden);
    }
    let tenant_id = ctx
        .tenant_id
        .ok_or_else(|| ApiError::BadRequest("no tenant".into()))?;
    let rows = db::member_invitations::list_invitations(pool, tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(
        rows.into_iter()
            .map(|r| InvitationDto {
                id: r.id,
                email: r.email,
                role: r.role,
                status: r.status,
                created_at: r.created_at,
            })
            .collect(),
    ))
}

async fn revoke_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    id: Uuid,
) -> Result<axum::http::StatusCode, ApiError> {
    if !is_org_admin(ctx) {
        return Err(ApiError::Forbidden);
    }
    let tenant_id = ctx
        .tenant_id
        .ok_or_else(|| ApiError::BadRequest("no tenant".into()))?;
    let outcome = db::member_invitations::revoke_invitation(pool, tenant_id, id, ctx.user_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    match outcome {
        db::member_invitations::RevokeInvitationOutcome::Revoked => {}
        db::member_invitations::RevokeInvitationOutcome::NotFound => {
            return Err(ApiError::NotFound)
        }
        db::member_invitations::RevokeInvitationOutcome::PrivilegedRole => {
            return Err(ApiError::Forbidden)
        }
    }
    // Audit-log the revoke in its own GUC-scoped tx.
    let mut tx = db::begin_with_context(pool, ctx.user_id, Some(tenant_id))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    db::audit::emit_audit_event(
        &mut tx,
        tenant_id,
        ctx.user_id,
        "member_invitation.revoke",
        "tenant_invitation",
        id,
        None,
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::valid_role;

    #[test]
    fn valid_role_accepts_known_roles() {
        for r in ["org_admin", "teacher", "ta", "student", "parent"] {
            assert!(valid_role(r), "{r} should be valid");
        }
    }

    #[test]
    fn valid_role_rejects_unknown() {
        assert!(!valid_role("org_owner"));
        assert!(!valid_role("superuser"));
        assert!(!valid_role(""));
    }
}
