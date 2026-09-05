// crates/backend/src/handlers/enrollments.rs
use axum::extract::{Extension, Path, State};
use axum::{routing, Json, Router};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::context::RequestContext;
use crate::db;
use crate::error::ApiError;
use crate::AppState;

#[derive(Deserialize)]
pub struct CreateCode {
    pub max_uses: Option<i32>,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}
#[derive(Serialize)]
pub struct CodeCreatedDto {
    pub id: Uuid,
    pub code: String,
    pub max_uses: Option<i32>,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}
#[derive(Serialize)]
pub struct CodeSummaryDto {
    pub id: Uuid,
    pub last4: String,
    pub max_uses: Option<i32>,
    pub uses: i32,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}
#[derive(Deserialize)]
pub struct RedeemCode {
    pub code: String,
}
#[derive(Serialize)]
pub struct RedeemedDto {
    pub course_id: Uuid,
    pub course_title: String,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/v1/courses/{cid}/codes",
            routing::post(create_code).get(list_codes),
        )
        .route(
            "/v1/courses/{cid}/codes/{codeid}",
            routing::delete(revoke_code),
        )
        .route("/v1/codes/redeem", routing::post(redeem))
        .route("/v1/catalog", routing::get(catalog))
        .route("/v1/catalog/{cid}/enroll", routing::post(catalog_enroll))
}

#[doc(hidden)]
pub fn router_for_tests(pool: PgPool) -> Router {
    Router::new()
        .route(
            "/v1/courses/{cid}/codes",
            routing::post(create_code_t).get(list_codes_t),
        )
        .route(
            "/v1/courses/{cid}/codes/{codeid}",
            routing::delete(revoke_code_t),
        )
        .route("/v1/codes/redeem", routing::post(redeem_t))
        .with_state(TestState { pool })
}

#[derive(Clone)]
struct TestState {
    pool: PgPool,
}

fn is_org_admin(ctx: &RequestContext) -> bool {
    ctx.can_manage_organization()
}

// Production handlers
async fn create_code(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(cid): Path<Uuid>,
    Json(b): Json<CreateCode>,
) -> Result<Json<CodeCreatedDto>, ApiError> {
    create_code_inner(&s.pool, &ctx, cid, b).await
}
async fn list_codes(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(cid): Path<Uuid>,
) -> Result<Json<Vec<CodeSummaryDto>>, ApiError> {
    list_codes_inner(&s.pool, &ctx, cid).await
}
async fn revoke_code(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path((cid, codeid)): Path<(Uuid, Uuid)>,
) -> Result<axum::http::StatusCode, ApiError> {
    revoke_code_inner(&s.pool, &ctx, cid, codeid).await
}
async fn redeem(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Json(b): Json<RedeemCode>,
) -> Result<Json<RedeemedDto>, ApiError> {
    redeem_inner(&s.pool, &ctx, b).await
}

// Test mirrors
async fn create_code_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(cid): Path<Uuid>,
    Json(b): Json<CreateCode>,
) -> Result<Json<CodeCreatedDto>, ApiError> {
    create_code_inner(&s.pool, &ctx, cid, b).await
}
async fn list_codes_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(cid): Path<Uuid>,
) -> Result<Json<Vec<CodeSummaryDto>>, ApiError> {
    list_codes_inner(&s.pool, &ctx, cid).await
}
async fn revoke_code_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path((cid, codeid)): Path<(Uuid, Uuid)>,
) -> Result<axum::http::StatusCode, ApiError> {
    revoke_code_inner(&s.pool, &ctx, cid, codeid).await
}
async fn redeem_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Json(b): Json<RedeemCode>,
) -> Result<Json<RedeemedDto>, ApiError> {
    redeem_inner(&s.pool, &ctx, b).await
}

// Inner logic
async fn create_code_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
    b: CreateCode,
) -> Result<Json<CodeCreatedDto>, ApiError> {
    let allowed = db::courses::caller_can_admin_course(
        pool,
        course_id,
        ctx.user_id,
        ctx.tenant_id,
        is_org_admin(ctx),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !allowed {
        return Err(ApiError::Forbidden);
    }
    let tenant_id = ctx
        .tenant_id
        .ok_or_else(|| ApiError::BadRequest("no tenant".into()))?;

    let mut last_err: Option<sqlx::Error> = None;
    for _ in 0..5 {
        // A fresh transaction per attempt: a unique-violation aborts the
        // current Postgres transaction, so retrying the INSERT on the same
        // `tx` would fail with 25P02 (in aborted transaction) instead of
        // generating a new code.
        let mut tx = pool
            .begin()
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        let code = db::enrollments::generate_code();
        match db::enrollments::insert_code(
            &mut tx,
            tenant_id,
            course_id,
            &code,
            b.max_uses,
            b.expires_at,
            ctx.user_id,
        )
        .await
        {
            Ok(id) => {
                db::audit::emit_audit_event(
                    &mut tx,
                    tenant_id,
                    ctx.user_id,
                    "enrollment_code.create",
                    "enrollment_code",
                    id,
                    None,
                )
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?;
                tx.commit()
                    .await
                    .map_err(|e| ApiError::Internal(e.to_string()))?;
                return Ok(Json(CodeCreatedDto {
                    id,
                    code,
                    max_uses: b.max_uses,
                    expires_at: b.expires_at,
                }));
            }
            Err(sqlx::Error::Database(dbe)) if dbe.is_unique_violation() => {
                last_err = Some(sqlx::Error::Database(dbe));
                continue;
            }
            Err(e) => return Err(ApiError::Internal(e.to_string())),
        }
    }
    Err(ApiError::Internal(format!(
        "could not generate unique code after 5 retries: {:?}",
        last_err
    )))
}

async fn list_codes_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
) -> Result<Json<Vec<CodeSummaryDto>>, ApiError> {
    let allowed = db::courses::caller_can_admin_course(
        pool,
        course_id,
        ctx.user_id,
        ctx.tenant_id,
        is_org_admin(ctx),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !allowed {
        return Err(ApiError::Forbidden);
    }
    let mut tx = db::begin_with_context(pool, ctx.user_id, ctx.tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let rows = db::enrollments::list_codes_for_course(&mut *tx, course_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let dtos = rows
        .into_iter()
        .map(|(id, code, max_uses, uses, expires_at)| CodeSummaryDto {
            id,
            last4: code
                .chars()
                .rev()
                .take(4)
                .collect::<String>()
                .chars()
                .rev()
                .collect(),
            max_uses,
            uses,
            expires_at,
        })
        .collect();
    Ok(Json(dtos))
}

async fn revoke_code_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
    code_id: Uuid,
) -> Result<axum::http::StatusCode, ApiError> {
    let allowed = db::courses::caller_can_admin_course(
        pool,
        course_id,
        ctx.user_id,
        ctx.tenant_id,
        is_org_admin(ctx),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !allowed {
        return Err(ApiError::Forbidden);
    }
    let tenant_id = ctx
        .tenant_id
        .ok_or_else(|| ApiError::BadRequest("no tenant".into()))?;
    let mut tx = db::begin_with_context(pool, ctx.user_id, Some(tenant_id))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let ok = db::enrollments::revoke_code(&mut tx, tenant_id, course_id, code_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !ok {
        return Err(ApiError::NotFound);
    }
    Ok(axum::http::StatusCode::NO_CONTENT)
}

async fn redeem_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    b: RedeemCode,
) -> Result<Json<RedeemedDto>, ApiError> {
    // Bypass-RLS lookup
    let looked_up = db::enrollments::lookup_code_bypass_rls(pool, &b.code)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::EnrollmentCodeInvalid)?;

    if !ctx.can_target_tenant(looked_up.tenant_id) {
        return Err(ApiError::Forbidden);
    }

    if let Some(exp) = looked_up.expires_at {
        if exp <= chrono::Utc::now() {
            return Err(ApiError::EnrollmentCodeInvalid);
        }
    }
    if let Some(max) = looked_up.max_uses {
        if looked_up.uses >= max {
            return Err(ApiError::EnrollmentCodeInvalid);
        }
    }

    let mut tx = db::begin_with_context(pool, ctx.user_id, Some(looked_up.tenant_id))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    // Tenant first, then code: every seat-consuming path takes the shared
    // tenant row lock before its flow-specific row lock.
    db::seats::lock_tenant_for_seat_mutation(&mut tx, looked_up.tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let row = db::enrollments::lock_code_for_redeem(&mut tx, looked_up.code_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::EnrollmentCodeInvalid)?;
    let (max_uses, uses, expires_at) = row;
    if let Some(exp) = expires_at {
        if exp <= chrono::Utc::now() {
            return Err(ApiError::EnrollmentCodeInvalid);
        }
    }
    if let Some(m) = max_uses {
        if uses >= m {
            return Err(ApiError::EnrollmentCodeInvalid);
        }
    }

    let activation = db::enrollments::ensure_tenant_membership_student(
        &mut tx,
        looked_up.tenant_id,
        ctx.user_id,
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    if activation == db::seats::MembershipActivationOutcome::SeatLimitReached {
        return Err(ApiError::Conflict(db::seats::SEAT_LIMIT_REACHED.into()));
    }
    if activation == db::seats::MembershipActivationOutcome::Suspended {
        return Err(ApiError::Forbidden);
    }

    let membership_changed = db::enrollments::ensure_course_membership_student(
        &mut tx,
        looked_up.course_id,
        ctx.user_id,
        looked_up.tenant_id,
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    // Only consume a use when redemption changed something. A student who is
    // already actively enrolled (double-click, retry, or re-redeem) must not
    // burn a limited-use code.
    if membership_changed.is_some() {
        db::enrollments::increment_uses(&mut tx, looked_up.code_id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
    }
    db::audit::emit_audit_event(
        &mut tx,
        looked_up.tenant_id,
        ctx.user_id,
        "enrollment_code.redeem",
        "enrollment_code",
        looked_up.code_id,
        None,
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;

    let course_title: String = sqlx::query_scalar("SELECT title FROM courses WHERE id = $1")
        .bind(looked_up.course_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(RedeemedDto {
        course_id: looked_up.course_id,
        course_title,
    }))
}

// ---- Catalog / self-enrollment -------------------------------------------

#[derive(Serialize)]
pub struct CatalogCourseDto {
    pub course_id: Uuid,
    pub slug: String,
    pub title: String,
    pub description: Option<String>,
    pub cover_asset_id: Option<Uuid>,
    pub owner_name: Option<String>,
    /// True when the caller already holds an active membership.
    pub enrolled: bool,
}

/// `GET /v1/catalog` — published, self-enrollment-open courses in the active
/// tenant, annotated with the caller's enrollment state. Any authenticated
/// member of a workspace may browse it; only open courses appear.
async fn catalog(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<Vec<CatalogCourseDto>>, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    let mut tx = db::begin_with_context(&s.pool, ctx.user_id, Some(tenant))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let rows = db::courses::list_catalog(&mut tx, tenant, ctx.user_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(
        rows.into_iter()
            .map(|c| CatalogCourseDto {
                course_id: c.id,
                slug: c.slug,
                title: c.title,
                description: c.description,
                cover_asset_id: c.cover_asset_id,
                owner_name: c.owner_name,
                enrolled: c.enrolled,
            })
            .collect(),
    ))
}

/// `POST /v1/catalog/{cid}/enroll` — self-enroll into an open published course.
/// Seat caps, suspended memberships and tenant activation flow through exactly
/// the same helpers as code redemption; the only new gate is that the course
/// must be published AND have self-enrollment enabled at enroll time.
async fn catalog_enroll(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(cid): Path<Uuid>,
) -> Result<Json<RedeemedDto>, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    let mut tx = db::begin_with_context(&s.pool, ctx.user_id, Some(tenant))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let (status, open) = db::courses::catalog_course(&mut tx, tenant, cid)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::NotFound)?;
    if status != "published" || !open {
        // 404 (not 403): an unpublished/closed course is not visible to learners.
        return Err(ApiError::NotFound);
    }

    // Tenant first, then membership writes — same ordering contract as redeem.
    db::seats::lock_tenant_for_seat_mutation(&mut tx, tenant)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let activation =
        db::enrollments::ensure_tenant_membership_student(&mut tx, tenant, ctx.user_id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
    if activation == db::seats::MembershipActivationOutcome::SeatLimitReached {
        return Err(ApiError::Conflict(db::seats::SEAT_LIMIT_REACHED.into()));
    }
    if activation == db::seats::MembershipActivationOutcome::Suspended {
        return Err(ApiError::Forbidden);
    }

    db::enrollments::ensure_course_membership_student(&mut tx, cid, ctx.user_id, tenant)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    db::audit::emit_audit_event(
        &mut tx,
        tenant,
        ctx.user_id,
        "course.self_enroll",
        "course",
        cid,
        None,
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;

    let course_title: String = sqlx::query_scalar("SELECT title FROM courses WHERE id = $1")
        .bind(cid)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(RedeemedDto {
        course_id: cid,
        course_title,
    }))
}

// ---- Invitations ---------------------------------------------------------

#[derive(serde::Deserialize)]
pub struct CreateInvitation {
    pub email: String,
    pub role: String,
}
#[derive(serde::Serialize)]
pub struct InvitationCreatedDto {
    pub invitation_id: Uuid,
    pub email: String,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}
#[derive(serde::Serialize)]
pub struct InvitationDto {
    pub id: Uuid,
    pub email: String,
    pub role: String,
    pub status: String,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}
#[derive(serde::Deserialize)]
pub struct AcceptInvitationBody {}

pub fn invitation_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/v1/courses/{cid}/invitations",
            routing::post(create_invitation_h).get(list_invitations_h),
        )
        .route(
            "/v1/courses/{cid}/invitations/{iid}",
            routing::delete(revoke_invitation_h),
        )
        .route(
            "/v1/invitations/{token}/accept",
            routing::post(accept_invitation_h),
        )
}

#[doc(hidden)]
pub fn invitation_router_for_tests(
    pool: PgPool,
    sender: std::sync::Arc<dyn crate::services::invitations::EmailLinkSender>,
    app_origin: String,
) -> Router {
    let state = TestInviteState {
        pool,
        sender,
        app_origin,
    };
    Router::new()
        .route(
            "/v1/courses/{cid}/invitations",
            routing::post(create_invitation_t).get(list_invitations_t),
        )
        .route(
            "/v1/courses/{cid}/invitations/{iid}",
            routing::delete(revoke_invitation_t),
        )
        .route(
            "/v1/invitations/{token}/accept",
            routing::post(accept_invitation_t),
        )
        .with_state(state)
}

#[derive(Clone)]
struct TestInviteState {
    pool: PgPool,
    sender: std::sync::Arc<dyn crate::services::invitations::EmailLinkSender>,
    app_origin: String,
}

async fn create_invitation_h(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(cid): Path<Uuid>,
    Json(b): Json<CreateInvitation>,
) -> Result<Json<InvitationCreatedDto>, ApiError> {
    create_invitation_inner(
        &s.pool,
        s.email_link_sender.as_ref(),
        &s.app_origin,
        &ctx,
        cid,
        b,
    )
    .await
}
async fn list_invitations_h(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(cid): Path<Uuid>,
) -> Result<Json<Vec<InvitationDto>>, ApiError> {
    list_invitations_inner(&s.pool, &ctx, cid).await
}
async fn revoke_invitation_h(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path((cid, iid)): Path<(Uuid, Uuid)>,
) -> Result<axum::http::StatusCode, ApiError> {
    revoke_invitation_inner(&s.pool, &ctx, cid, iid).await
}
async fn accept_invitation_h(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(token): Path<String>,
    Json(_): Json<AcceptInvitationBody>,
) -> Result<Json<RedeemedDto>, ApiError> {
    accept_invitation_inner(&s.pool, &ctx, &token).await
}

async fn create_invitation_t(
    State(s): State<TestInviteState>,
    Extension(ctx): Extension<RequestContext>,
    Path(cid): Path<Uuid>,
    Json(b): Json<CreateInvitation>,
) -> Result<Json<InvitationCreatedDto>, ApiError> {
    create_invitation_inner(&s.pool, s.sender.as_ref(), &s.app_origin, &ctx, cid, b).await
}
async fn list_invitations_t(
    State(s): State<TestInviteState>,
    Extension(ctx): Extension<RequestContext>,
    Path(cid): Path<Uuid>,
) -> Result<Json<Vec<InvitationDto>>, ApiError> {
    list_invitations_inner(&s.pool, &ctx, cid).await
}
async fn revoke_invitation_t(
    State(s): State<TestInviteState>,
    Extension(ctx): Extension<RequestContext>,
    Path((cid, iid)): Path<(Uuid, Uuid)>,
) -> Result<axum::http::StatusCode, ApiError> {
    revoke_invitation_inner(&s.pool, &ctx, cid, iid).await
}
async fn accept_invitation_t(
    State(s): State<TestInviteState>,
    Extension(ctx): Extension<RequestContext>,
    Path(token): Path<String>,
    Json(_): Json<AcceptInvitationBody>,
) -> Result<Json<RedeemedDto>, ApiError> {
    accept_invitation_inner(&s.pool, &ctx, &token).await
}

async fn create_invitation_inner(
    pool: &PgPool,
    sender: &dyn crate::services::invitations::EmailLinkSender,
    app_origin: &str,
    ctx: &RequestContext,
    course_id: Uuid,
    b: CreateInvitation,
) -> Result<Json<InvitationCreatedDto>, ApiError> {
    let allowed = db::courses::caller_can_admin_course(
        pool,
        course_id,
        ctx.user_id,
        ctx.tenant_id,
        is_org_admin(ctx),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !allowed {
        return Err(ApiError::Forbidden);
    }
    if !["teacher", "ta", "student"].contains(&b.role.as_str()) {
        return Err(ApiError::BadRequest(format!("invalid role: {}", b.role)));
    }
    let tenant_id = ctx
        .tenant_id
        .ok_or_else(|| ApiError::BadRequest("no tenant".into()))?;

    let token = db::enrollments::generate_invitation_token();
    let expires_at = chrono::Utc::now() + chrono::Duration::days(14);

    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    // Lazily expire any stale pending invite for (course, email) so the new one
    // doesn't collide with the partial unique index.
    db::enrollments::lazy_expire_stale_pending(&mut tx, course_id, &b.email)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let invitation_id = db::enrollments::insert_invitation(
        &mut tx,
        tenant_id,
        course_id,
        &b.email,
        &b.role,
        &token,
        expires_at,
        ctx.user_id,
    )
    .await
    .map_err(|e| {
        if let sqlx::Error::Database(dbe) = &e {
            if dbe.is_unique_violation() {
                return ApiError::BadRequest(
                    "an invite for this email is already pending on this course".into(),
                );
            }
        }
        ApiError::Internal(e.to_string())
    })?;
    db::audit::emit_audit_event(
        &mut tx,
        tenant_id,
        ctx.user_id,
        "course_invitation.create",
        "course_invitation",
        invitation_id,
        None,
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let continue_url = format!(
        "{}/accept-invite/{}",
        app_origin.trim_end_matches('/'),
        token
    );
    sender
        .send_invite(&b.email, &continue_url)
        .await
        .map_err(|e| ApiError::Internal(format!("email send failed: {e}")))?;

    Ok(Json(InvitationCreatedDto {
        invitation_id,
        email: b.email,
        expires_at,
    }))
}

async fn list_invitations_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
) -> Result<Json<Vec<InvitationDto>>, ApiError> {
    let allowed = db::courses::caller_can_admin_course(
        pool,
        course_id,
        ctx.user_id,
        ctx.tenant_id,
        is_org_admin(ctx),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !allowed {
        return Err(ApiError::Forbidden);
    }
    let mut tx = db::begin_with_context(pool, ctx.user_id, ctx.tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let rows = db::enrollments::list_invitations_for_course(&mut *tx, course_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(
        rows.into_iter()
            .map(|(id, email, role, status, expires_at)| InvitationDto {
                id,
                email,
                role,
                status,
                expires_at,
            })
            .collect(),
    ))
}

async fn revoke_invitation_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
    invitation_id: Uuid,
) -> Result<axum::http::StatusCode, ApiError> {
    let allowed = db::courses::caller_can_admin_course(
        pool,
        course_id,
        ctx.user_id,
        ctx.tenant_id,
        is_org_admin(ctx),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !allowed {
        return Err(ApiError::Forbidden);
    }
    let tenant_id = ctx
        .tenant_id
        .ok_or_else(|| ApiError::BadRequest("no tenant".into()))?;
    let mut tx = db::begin_with_context(pool, ctx.user_id, Some(tenant_id))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let ok = db::enrollments::revoke_invitation(&mut tx, tenant_id, course_id, invitation_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !ok {
        return Err(ApiError::NotFound);
    }
    Ok(axum::http::StatusCode::NO_CONTENT)
}

async fn accept_invitation_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    token: &str,
) -> Result<Json<RedeemedDto>, ApiError> {
    let inv = db::enrollments::lookup_invitation_bypass_rls(pool, token)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::InvitationInvalid)?;

    if !ctx.can_target_tenant(inv.tenant_id) {
        return Err(ApiError::Forbidden);
    }

    if inv.status != "pending" {
        return Err(ApiError::InvitationInvalid);
    }
    if inv.expires_at <= chrono::Utc::now() {
        return Err(ApiError::InvitationInvalid);
    }
    if inv.email.to_lowercase() != ctx.email.to_lowercase() {
        return Err(ApiError::InvitationInvalid);
    }

    let mut tx = db::begin_with_context(pool, ctx.user_id, Some(inv.tenant_id))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let activation =
        db::enrollments::ensure_tenant_membership_student(&mut tx, inv.tenant_id, ctx.user_id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
    if activation == db::seats::MembershipActivationOutcome::SeatLimitReached {
        return Err(ApiError::Conflict(db::seats::SEAT_LIMIT_REACHED.into()));
    }
    if activation == db::seats::MembershipActivationOutcome::Suspended {
        return Err(ApiError::Forbidden);
    }

    sqlx::query(
        "INSERT INTO course_memberships (course_id, user_id, tenant_id, role, status)
         VALUES ($1, $2, $3, $4, 'active')
         ON CONFLICT (course_id, user_id) DO UPDATE
            SET role = EXCLUDED.role, status = 'active'",
    )
    .bind(inv.course_id)
    .bind(ctx.user_id)
    .bind(inv.tenant_id)
    .bind(&inv.role)
    .execute(&mut *tx)
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;

    db::enrollments::mark_invitation_accepted(&mut tx, inv.invitation_id, ctx.user_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    db::audit::emit_audit_event(
        &mut tx,
        inv.tenant_id,
        ctx.user_id,
        "course_invitation.accept",
        "course_invitation",
        inv.invitation_id,
        None,
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;

    let title: String = sqlx::query_scalar("SELECT title FROM courses WHERE id = $1")
        .bind(inv.course_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(RedeemedDto {
        course_id: inv.course_id,
        course_title: title,
    }))
}
