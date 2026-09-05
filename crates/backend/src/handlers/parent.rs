// crates/backend/src/handlers/parent.rs
//
// Parent-role HTTP endpoints + admin parent-invitation endpoints.
//
// The parent role is STRICTLY READ-ONLY. A parent may only read their linked
// children's RELEASED grades, attendance, and upcoming schedule — never live
// data, never another student, never teacher-only notes.
//
// Authorization (enforced in every handler):
//   * `/v1/parent/*`              — caller must be a Parent (tenant_role) OR a
//                                   platform admin.
//   * child-specific endpoints    — ALSO require `db::parent::is_linked` to be
//                                   true for (caller, student_id); otherwise the
//                                   request is Forbidden.
//   * `/v1/admin/parent-invitations*` — org-admin or platform-admin only.
use axum::extract::{Extension, Path, Query, State};
use axum::{routing, Json, Router};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::context::RequestContext;
use crate::db;
use crate::error::ApiError;
use crate::AppState;

// ===========================================================================
// DTOs
// ===========================================================================

#[derive(Serialize)]
pub struct ChildDto {
    pub student_user_id: String,
    pub display_name: Option<String>,
    pub email: Option<String>,
}

#[derive(Serialize)]
pub struct ChildGradeDto {
    pub assignment_id: Uuid,
    pub assignment_title: String,
    pub course_title: Option<String>,
    pub status: String,
    pub numeric_grade: Option<f64>,
    pub letter_grade: Option<String>,
    pub passed: Option<bool>,
    pub student_visible_feedback: Option<String>,
    pub released_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Serialize)]
pub struct ChildAttendanceDto {
    pub session_id: Uuid,
    pub session_title: Option<String>,
    pub course_title: Option<String>,
    pub first_joined_at: chrono::DateTime<chrono::Utc>,
    pub last_left_at: Option<chrono::DateTime<chrono::Utc>>,
    pub total_seconds: i32,
    pub reconnect_count: i32,
    pub starts_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Serialize)]
pub struct ChildScheduleItemDto {
    pub session_id: Uuid,
    pub course_id: Uuid,
    pub course_title: String,
    pub title: String,
    pub starts_at: chrono::DateTime<chrono::Utc>,
    pub duration_minutes: i32,
    pub status: String,
}

#[derive(Deserialize, Default)]
pub struct ScheduleQuery {
    pub days: Option<i64>,
}

#[derive(Serialize)]
pub struct ChildGamificationDto {
    pub total_xp: i64,
    pub level: u32,
    pub current_streak_days: i32,
    pub longest_streak_days: i32,
    pub streak_active_today: bool,
}

#[derive(Serialize)]
pub struct ParentInvitationDto {
    pub id: Uuid,
    pub parent_email: String,
    pub student_user_id: Uuid,
    pub relationship: Option<String>,
    pub status: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Deserialize)]
pub struct CreateParentInvitation {
    pub parent_email: String,
    pub student_user_id: Uuid,
    pub relationship: Option<String>,
}

#[derive(Deserialize, Default)]
pub struct ListInvitationsQuery {
    pub student_id: Option<Uuid>,
}

// ===========================================================================
// Authorization helpers
// ===========================================================================

/// True for the linked-family read capability in the selected workspace.
fn is_parent(ctx: &RequestContext) -> bool {
    ctx.has_capability(core_types::Capability::ParentRead)
}

/// True for the admin gate on parent-invitation endpoints: org_admin OR
/// platform admin.
fn is_org_admin(ctx: &RequestContext) -> bool {
    ctx.can_manage_organization()
}

/// Gate a `/v1/parent/*` request to a caller with linked-family read access.
fn require_parent(ctx: &RequestContext) -> Result<(), ApiError> {
    if is_parent(ctx) {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
    }
}

/// Gate a child-specific request to the selected workspace: the caller must
/// have an active parent entitlement there and a link to the child there.
/// Returning the selected tenant makes it impossible for a link in another
/// (including suspended) workspace to redirect the subsequent read.
async fn require_linked_child(
    pool: &PgPool,
    ctx: &RequestContext,
    student_id: Uuid,
) -> Result<Uuid, ApiError> {
    require_parent(ctx)?;
    let tenant_id = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    let linked = db::parent::is_linked(pool, tenant_id, ctx.user_id, student_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !linked {
        return Err(ApiError::Forbidden);
    }
    Ok(tenant_id)
}

fn normalize_schedule_days(days: Option<i64>) -> Result<i32, ApiError> {
    let days = days.unwrap_or(30);
    if !(1..=180).contains(&days) {
        return Err(ApiError::BadRequest(
            "days must be between 1 and 180".into(),
        ));
    }
    Ok(days as i32)
}

// ===========================================================================
// Routers
// ===========================================================================

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/parent/children", routing::get(children))
        .route(
            "/v1/parent/children/{student_id}/grades",
            routing::get(child_grades),
        )
        .route(
            "/v1/parent/children/{student_id}/attendance",
            routing::get(child_attendance),
        )
        .route(
            "/v1/parent/children/{student_id}/schedule",
            routing::get(child_schedule),
        )
        .route(
            "/v1/parent/children/{student_id}/gamification",
            routing::get(child_gamification),
        )
        // Admin parent-invitation endpoints (org-admin / platform-admin).
        .route(
            "/v1/admin/parent-invitations",
            routing::post(create_parent_invitation).get(list_parent_invitations),
        )
        .route(
            "/v1/admin/parent-invitations/{id}",
            routing::delete(revoke_parent_invitation),
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
        .route("/v1/parent/children", routing::get(children_t))
        .route(
            "/v1/parent/children/{student_id}/grades",
            routing::get(child_grades_t),
        )
        .route(
            "/v1/parent/children/{student_id}/attendance",
            routing::get(child_attendance_t),
        )
        .route(
            "/v1/parent/children/{student_id}/schedule",
            routing::get(child_schedule_t),
        )
        .route(
            "/v1/parent/children/{student_id}/gamification",
            routing::get(child_gamification_t),
        )
        .route(
            "/v1/admin/parent-invitations",
            routing::post(create_parent_invitation_t).get(list_parent_invitations_t),
        )
        .route(
            "/v1/admin/parent-invitations/{id}",
            routing::delete(revoke_parent_invitation_t),
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

async fn children(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<Vec<ChildDto>>, ApiError> {
    children_inner(&s.pool, &ctx).await
}

async fn child_grades(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(student_id): Path<Uuid>,
) -> Result<Json<Vec<ChildGradeDto>>, ApiError> {
    child_grades_inner(&s.pool, &ctx, student_id).await
}

async fn child_attendance(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(student_id): Path<Uuid>,
) -> Result<Json<Vec<ChildAttendanceDto>>, ApiError> {
    child_attendance_inner(&s.pool, &ctx, student_id).await
}

async fn child_schedule(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(student_id): Path<Uuid>,
    Query(q): Query<ScheduleQuery>,
) -> Result<Json<Vec<ChildScheduleItemDto>>, ApiError> {
    child_schedule_inner(&s.pool, &ctx, student_id, q).await
}

async fn child_gamification(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(student_id): Path<Uuid>,
) -> Result<Json<ChildGamificationDto>, ApiError> {
    child_gamification_inner(&s.pool, &ctx, student_id).await
}

async fn create_parent_invitation(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Json(b): Json<CreateParentInvitation>,
) -> Result<Json<ParentInvitationDto>, ApiError> {
    create_parent_invitation_inner(
        &s.pool,
        s.email_link_sender.as_ref(),
        &s.app_origin,
        &ctx,
        b,
    )
    .await
}

async fn list_parent_invitations(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Query(q): Query<ListInvitationsQuery>,
) -> Result<Json<Vec<ParentInvitationDto>>, ApiError> {
    list_parent_invitations_inner(&s.pool, &ctx, q).await
}

async fn revoke_parent_invitation(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<axum::http::StatusCode, ApiError> {
    revoke_parent_invitation_inner(&s.pool, &ctx, id).await
}

// ===========================================================================
// Test wrappers
// ===========================================================================

async fn children_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<Vec<ChildDto>>, ApiError> {
    children_inner(&s.pool, &ctx).await
}

async fn child_grades_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(student_id): Path<Uuid>,
) -> Result<Json<Vec<ChildGradeDto>>, ApiError> {
    child_grades_inner(&s.pool, &ctx, student_id).await
}

async fn child_attendance_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(student_id): Path<Uuid>,
) -> Result<Json<Vec<ChildAttendanceDto>>, ApiError> {
    child_attendance_inner(&s.pool, &ctx, student_id).await
}

async fn child_schedule_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(student_id): Path<Uuid>,
    Query(q): Query<ScheduleQuery>,
) -> Result<Json<Vec<ChildScheduleItemDto>>, ApiError> {
    child_schedule_inner(&s.pool, &ctx, student_id, q).await
}

async fn child_gamification_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(student_id): Path<Uuid>,
) -> Result<Json<ChildGamificationDto>, ApiError> {
    child_gamification_inner(&s.pool, &ctx, student_id).await
}

async fn create_parent_invitation_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Json(b): Json<CreateParentInvitation>,
) -> Result<Json<ParentInvitationDto>, ApiError> {
    create_parent_invitation_inner(&s.pool, s.sender.as_ref(), &s.app_origin, &ctx, b).await
}

async fn list_parent_invitations_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Query(q): Query<ListInvitationsQuery>,
) -> Result<Json<Vec<ParentInvitationDto>>, ApiError> {
    list_parent_invitations_inner(&s.pool, &ctx, q).await
}

async fn revoke_parent_invitation_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<axum::http::StatusCode, ApiError> {
    revoke_parent_invitation_inner(&s.pool, &ctx, id).await
}

// ===========================================================================
// Inner logic
// ===========================================================================

async fn children_inner(
    pool: &PgPool,
    ctx: &RequestContext,
) -> Result<Json<Vec<ChildDto>>, ApiError> {
    require_parent(ctx)?;
    let tenant_id = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    let rows = db::parent::list_children(pool, tenant_id, ctx.user_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(
        rows.into_iter()
            .map(|c| ChildDto {
                student_user_id: c.student_user_id.to_string(),
                display_name: c.display_name,
                email: c.email,
            })
            .collect(),
    ))
}

async fn child_grades_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    student_id: Uuid,
) -> Result<Json<Vec<ChildGradeDto>>, ApiError> {
    let tenant_id = require_linked_child(pool, ctx, student_id).await?;
    let rows = db::parent::list_released_grades_for_student(pool, tenant_id, student_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(
        rows.into_iter()
            .map(|r| ChildGradeDto {
                assignment_id: r.assignment_id,
                assignment_title: r.assignment_title,
                course_title: r.course_title,
                status: r.status,
                numeric_grade: r
                    .numeric_grade
                    .and_then(|d| d.to_string().parse::<f64>().ok()),
                letter_grade: r.letter_grade,
                passed: r.passed,
                student_visible_feedback: r.student_visible_feedback,
                released_at: r.released_at,
            })
            .collect(),
    ))
}

async fn child_attendance_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    student_id: Uuid,
) -> Result<Json<Vec<ChildAttendanceDto>>, ApiError> {
    let tenant_id = require_linked_child(pool, ctx, student_id).await?;
    let rows = db::attendance::list_for_user(pool, tenant_id, student_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(
        rows.into_iter()
            .map(|r| ChildAttendanceDto {
                session_id: r.session_id,
                session_title: r.session_title,
                course_title: r.course_title,
                first_joined_at: r.first_joined_at,
                last_left_at: r.last_left_at,
                total_seconds: r.total_seconds,
                reconnect_count: r.reconnect_count,
                starts_at: r.starts_at,
            })
            .collect(),
    ))
}

async fn child_schedule_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    student_id: Uuid,
    q: ScheduleQuery,
) -> Result<Json<Vec<ChildScheduleItemDto>>, ApiError> {
    let tenant_id = require_linked_child(pool, ctx, student_id).await?;
    let days = normalize_schedule_days(q.days)?;
    let rows = db::parent::list_upcoming_schedule_for_student(pool, tenant_id, student_id, days)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(
        rows.into_iter()
            .map(|r| ChildScheduleItemDto {
                session_id: r.session_id,
                course_id: r.course_id,
                course_title: r.course_title,
                title: r.title,
                starts_at: r.starts_at,
                duration_minutes: r.duration_minutes,
                status: r.status,
            })
            .collect(),
    ))
}

/// Child XP/streak summary for the parent dashboard. Same linked-child gate as
/// the other child reads; the GUC tx is scoped to the child's tenant so the
/// RLS policies on `learner_stats` apply.
async fn child_gamification_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    student_id: Uuid,
) -> Result<Json<ChildGamificationDto>, ApiError> {
    let tenant_id = require_linked_child(pool, ctx, student_id).await?;
    let mut tx = db::begin_with_context(pool, ctx.user_id, Some(tenant_id))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let stats = db::gamification::learner_stats(&mut tx, tenant_id, student_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let active_today = db::gamification::active_today(&mut tx, tenant_id, student_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let total_xp = stats.as_ref().map(|s| s.total_xp).unwrap_or(0);
    let (level, _, _) = db::gamification::level_for_xp(total_xp);
    Ok(Json(ChildGamificationDto {
        total_xp,
        level,
        current_streak_days: stats.as_ref().map(|s| s.current_streak_days).unwrap_or(0),
        longest_streak_days: stats.as_ref().map(|s| s.longest_streak_days).unwrap_or(0),
        streak_active_today: active_today,
    }))
}

async fn create_parent_invitation_inner(
    pool: &PgPool,
    sender: &dyn crate::services::invitations::EmailLinkSender,
    app_origin: &str,
    ctx: &RequestContext,
    b: CreateParentInvitation,
) -> Result<Json<ParentInvitationDto>, ApiError> {
    if !is_org_admin(ctx) {
        return Err(ApiError::Forbidden);
    }
    let tenant_id = ctx
        .tenant_id
        .ok_or_else(|| ApiError::BadRequest("no tenant".into()))?;

    // The named student MUST be an ACTIVE member of the caller's tenant.
    let ok = db::parent::student_is_active_member(pool, tenant_id, b.student_user_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !ok {
        return Err(ApiError::BadRequest(
            "student is not an active member of this tenant".into(),
        ));
    }

    let invitation = match db::parent::create_invitation(
        pool,
        tenant_id,
        &b.parent_email,
        b.student_user_id,
        b.relationship.as_deref(),
        ctx.user_id,
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?
    {
        db::parent::ParentInvitationReservationOutcome::Reserved(invitation) => invitation,
        db::parent::ParentInvitationReservationOutcome::SeatLimitReached => {
            return Err(ApiError::Conflict(db::seats::SEAT_LIMIT_REACHED.into()));
        }
    };

    // Audit-log the creation in its own GUC-scoped tx.
    {
        let mut tx = db::begin_with_context(pool, ctx.user_id, Some(tenant_id))
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        db::audit::emit_audit_event(
            &mut tx,
            tenant_id,
            ctx.user_id,
            "parent_invitation.create",
            "parent_invitation",
            invitation.id,
            None,
        )
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
        tx.commit()
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
    }

    // Send the Firebase email-link sign-in, mirroring course invitations: the
    // continue URL is built from `app_origin`. The invited parent's account is
    // JIT-provisioned on first sign-in, at which point the pending invitation is
    // accepted by `accept_parent_invitations_for_email`.
    // Parent invitations return to the family-specific sign-in surface. JIT
    // provisioning still accepts the pending invitation by verified email;
    // the continue URL only controls the product experience around that flow.
    let continue_url = format!("{}/parent/login", app_origin.trim_end_matches('/'));
    sender
        .send_invite(&b.parent_email, &continue_url)
        .await
        .map_err(|e| ApiError::Internal(format!("email send failed: {e}")))?;

    Ok(Json(ParentInvitationDto {
        id: invitation.id,
        parent_email: invitation.parent_email,
        student_user_id: invitation.student_user_id,
        relationship: invitation.relationship,
        status: invitation.status,
        created_at: invitation.created_at,
    }))
}

async fn list_parent_invitations_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    q: ListInvitationsQuery,
) -> Result<Json<Vec<ParentInvitationDto>>, ApiError> {
    if !is_org_admin(ctx) {
        return Err(ApiError::Forbidden);
    }
    let tenant_id = ctx
        .tenant_id
        .ok_or_else(|| ApiError::BadRequest("no tenant".into()))?;
    let rows = db::parent::list_invitations(pool, tenant_id, q.student_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(
        rows.into_iter()
            .map(|r| ParentInvitationDto {
                id: r.id,
                parent_email: r.parent_email,
                student_user_id: r.student_user_id,
                relationship: r.relationship,
                status: r.status,
                created_at: r.created_at,
            })
            .collect(),
    ))
}

async fn revoke_parent_invitation_inner(
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
    let ok = db::parent::revoke_invitation(pool, tenant_id, id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !ok {
        return Err(ApiError::NotFound);
    }
    // Audit-log the revoke in its own GUC-scoped tx.
    let mut tx = db::begin_with_context(pool, ctx.user_id, Some(tenant_id))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    db::audit::emit_audit_event(
        &mut tx,
        tenant_id,
        ctx.user_id,
        "parent_invitation.revoke",
        "parent_invitation",
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
    use super::normalize_schedule_days;

    #[test]
    fn schedule_days_defaults_to_thirty() {
        assert_eq!(normalize_schedule_days(None).unwrap(), 30);
    }

    #[test]
    fn schedule_days_accepts_bounds() {
        assert_eq!(normalize_schedule_days(Some(1)).unwrap(), 1);
        assert_eq!(normalize_schedule_days(Some(180)).unwrap(), 180);
    }

    #[test]
    fn schedule_days_rejects_out_of_range() {
        assert!(normalize_schedule_days(Some(0)).is_err());
        assert!(normalize_schedule_days(Some(181)).is_err());
    }
}
