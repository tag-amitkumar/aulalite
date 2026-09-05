// crates/backend/src/handlers/courses.rs
use axum::extract::{Extension, Path, State};
use axum::{routing, Json, Router};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::context::RequestContext;
use crate::db;
use crate::error::ApiError;
use crate::services::slugger;
use crate::AppState;

#[derive(Deserialize)]
pub struct CreateCourse {
    pub title: String,
    pub description: Option<String>,
}

#[derive(Serialize)]
pub struct CourseDto {
    pub id: Uuid,
    pub slug: String,
    pub title: String,
    pub description: Option<String>,
    pub status: String,
    pub cover_asset_id: Option<Uuid>,
    pub owner_user_id: Uuid,
    /// Active role on this concrete course for the caller. Workspace admins
    /// receive `None` because their organization capability is sufficient.
    pub caller_course_role: Option<String>,
    pub syllabus_md: Option<String>,
    pub grading_policy_md: Option<String>,
    /// Whether any active tenant member can self-enroll from the catalog.
    pub self_enrollment_enabled: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl From<db::courses::CourseRow> for CourseDto {
    fn from(r: db::courses::CourseRow) -> Self {
        Self {
            id: r.id,
            slug: r.slug,
            title: r.title,
            description: r.description,
            status: r.status,
            cover_asset_id: r.cover_asset_id,
            owner_user_id: r.owner_user_id,
            caller_course_role: None,
            syllabus_md: r.syllabus_md,
            grading_policy_md: r.grading_policy_md,
            self_enrollment_enabled: r.self_enrollment_enabled,
            created_at: r.created_at,
        }
    }
}

/// Public-ish syllabus payload: the two markdown fields plus title/slug for
/// context. Returned by `GET /v1/courses/{id}/syllabus` for anyone who can read
/// the course.
#[derive(Serialize)]
pub struct CourseSyllabusDto {
    pub course_id: Uuid,
    pub title: String,
    pub syllabus_md: Option<String>,
    pub grading_policy_md: Option<String>,
}

/// Response of a successful duplicate: enough for the client to navigate to
/// the freshly-created draft.
#[derive(Serialize)]
pub struct DuplicateCourseDto {
    pub id: Uuid,
    pub slug: String,
    pub title: String,
}

#[derive(Serialize)]
pub struct LessonDetailDto {
    pub id: Uuid,
    pub course_id: Uuid,
    pub module_id: Uuid,
    #[serde(rename = "type")]
    pub type_: String,
    pub title: String,
    pub body_md: Option<String>,
    pub video_asset_id: Option<Uuid>,
    pub live_session_id: Option<Uuid>,
    pub sort_order: i32,
}

impl From<db::course_detail::LessonRow> for LessonDetailDto {
    fn from(r: db::course_detail::LessonRow) -> Self {
        Self {
            id: r.id,
            course_id: r.course_id,
            module_id: r.module_id,
            type_: r.r#type,
            title: r.title,
            body_md: r.body_md,
            video_asset_id: r.video_asset_id,
            live_session_id: r.live_session_id,
            sort_order: r.sort_order,
        }
    }
}

#[derive(Serialize)]
pub struct ModuleWithLessonsDto {
    pub id: Uuid,
    pub course_id: Uuid,
    pub title: String,
    pub sort_order: i32,
    pub lessons: Vec<LessonDetailDto>,
}

impl From<db::course_detail::ModuleWithLessonsRow> for ModuleWithLessonsDto {
    fn from(r: db::course_detail::ModuleWithLessonsRow) -> Self {
        Self {
            id: r.id,
            course_id: r.course_id,
            title: r.title,
            sort_order: r.sort_order,
            lessons: r.lessons.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Serialize)]
pub struct CourseMemberDto {
    pub user_id: Uuid,
    pub display_name: Option<String>,
    pub email: String,
    pub role: String,
    pub status: String,
}

impl From<db::course_detail::MemberRow> for CourseMemberDto {
    fn from(r: db::course_detail::MemberRow) -> Self {
        Self {
            user_id: r.user_id,
            display_name: r.display_name,
            email: r.email,
            role: r.role,
            status: r.status,
        }
    }
}

#[derive(Serialize)]
pub struct CourseSessionDto {
    pub session_id: Uuid,
    pub course_id: Uuid,
    pub course_title: String,
    pub course_slug: String,
    pub title: String,
    pub starts_at: chrono::DateTime<chrono::Utc>,
    pub duration_minutes: i32,
    pub status: String,
    pub diverged: bool,
}

impl From<db::course_detail::SessionRow> for CourseSessionDto {
    fn from(r: db::course_detail::SessionRow) -> Self {
        Self {
            session_id: r.session_id,
            course_id: r.course_id,
            course_title: r.course_title,
            course_slug: r.course_slug,
            title: r.title,
            starts_at: r.starts_at,
            duration_minutes: r.duration_minutes,
            status: r.status,
            diverged: r.diverged,
        }
    }
}

#[derive(Deserialize, Default)]
pub struct PatchCourse {
    pub title: Option<String>,
    pub description: Option<String>,
    pub status: Option<String>,
    #[serde(default, with = "::serde_with::rust::double_option")]
    pub cover_asset_id: Option<Option<Uuid>>,
    // Double-Option: omitted = leave as-is, null = clear, value = set.
    #[serde(default, with = "::serde_with::rust::double_option")]
    pub syllabus_md: Option<Option<String>>,
    #[serde(default, with = "::serde_with::rust::double_option")]
    pub grading_policy_md: Option<Option<String>>,
    /// Catalog self-enrollment toggle (teacher/admin only; default off).
    pub self_enrollment_enabled: Option<bool>,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/courses", routing::post(create).get(list))
        .route(
            "/v1/courses/{cid}/modules-with-lessons",
            routing::get(modules_with_lessons),
        )
        .route("/v1/courses/{cid}/sessions", routing::get(sessions))
        .route("/v1/courses/{cid}/members", routing::get(members))
        .route("/v1/courses/{cid}/duplicate", routing::post(duplicate))
        .route("/v1/courses/{cid}/syllabus", routing::get(syllabus))
        .route(
            "/v1/courses/{cid}",
            routing::get(get_one).patch(patch).delete(delete_one),
        )
}

#[doc(hidden)]
pub fn router_for_tests(pool: PgPool) -> Router {
    Router::new()
        .route("/v1/courses", routing::post(create_t).get(list_t))
        .route(
            "/v1/courses/{cid}/modules-with-lessons",
            routing::get(modules_with_lessons_t),
        )
        .route("/v1/courses/{cid}/sessions", routing::get(sessions_t))
        .route("/v1/courses/{cid}/members", routing::get(members_t))
        .route("/v1/courses/{cid}/duplicate", routing::post(duplicate_t))
        .route("/v1/courses/{cid}/syllabus", routing::get(syllabus_t))
        .route(
            "/v1/courses/{cid}",
            routing::get(get_one_t).patch(patch_t).delete(delete_one_t),
        )
        .with_state(TestState { pool })
}

#[derive(Clone)]
struct TestState {
    pool: PgPool,
}

// Production handlers
async fn create(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Json(body): Json<CreateCourse>,
) -> Result<Json<CourseDto>, ApiError> {
    create_inner(&state.pool, &ctx, body).await
}
async fn list(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<Vec<CourseDto>>, ApiError> {
    list_inner(&state.pool, &ctx).await
}
async fn get_one(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<CourseDto>, ApiError> {
    get_one_inner(&state.pool, &ctx, id).await
}
async fn modules_with_lessons(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<ModuleWithLessonsDto>>, ApiError> {
    modules_with_lessons_inner(&state.pool, &ctx, id).await
}
async fn sessions(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<CourseSessionDto>>, ApiError> {
    sessions_inner(&state.pool, &ctx, id).await
}
async fn members(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<CourseMemberDto>>, ApiError> {
    members_inner(&state.pool, &ctx, id).await
}
async fn patch(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    Json(body): Json<PatchCourse>,
) -> Result<Json<CourseDto>, ApiError> {
    patch_inner(&state.pool, &ctx, id, body).await
}
async fn delete_one(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<axum::http::StatusCode, ApiError> {
    delete_inner(&state.pool, &ctx, id).await
}
async fn duplicate(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<DuplicateCourseDto>, ApiError> {
    duplicate_inner(&state.pool, &ctx, id).await
}
async fn syllabus(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<CourseSyllabusDto>, ApiError> {
    syllabus_inner(&state.pool, &ctx, id).await
}

// Test mirrors
async fn create_t(
    State(state): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Json(body): Json<CreateCourse>,
) -> Result<Json<CourseDto>, ApiError> {
    create_inner(&state.pool, &ctx, body).await
}
async fn list_t(
    State(state): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<Vec<CourseDto>>, ApiError> {
    list_inner(&state.pool, &ctx).await
}
async fn get_one_t(
    State(state): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<CourseDto>, ApiError> {
    get_one_inner(&state.pool, &ctx, id).await
}
async fn modules_with_lessons_t(
    State(state): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<ModuleWithLessonsDto>>, ApiError> {
    modules_with_lessons_inner(&state.pool, &ctx, id).await
}
async fn sessions_t(
    State(state): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<CourseSessionDto>>, ApiError> {
    sessions_inner(&state.pool, &ctx, id).await
}
async fn members_t(
    State(state): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<CourseMemberDto>>, ApiError> {
    members_inner(&state.pool, &ctx, id).await
}
async fn patch_t(
    State(state): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    Json(body): Json<PatchCourse>,
) -> Result<Json<CourseDto>, ApiError> {
    patch_inner(&state.pool, &ctx, id, body).await
}
async fn delete_one_t(
    State(state): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<axum::http::StatusCode, ApiError> {
    delete_inner(&state.pool, &ctx, id).await
}
async fn duplicate_t(
    State(state): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<DuplicateCourseDto>, ApiError> {
    duplicate_inner(&state.pool, &ctx, id).await
}
async fn syllabus_t(
    State(state): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<CourseSyllabusDto>, ApiError> {
    syllabus_inner(&state.pool, &ctx, id).await
}

// Inner logic (state-agnostic)
fn require_teacher_or_admin(ctx: &RequestContext) -> Result<(), ApiError> {
    ctx.can_teach().then_some(()).ok_or(ApiError::Forbidden)
}

fn is_org_admin(ctx: &RequestContext) -> bool {
    ctx.can_manage_organization()
}

fn sanitize_optional_markdown_patch(value: Option<&Option<String>>) -> Option<Option<String>> {
    value.map(|maybe_raw| {
        maybe_raw.as_ref().and_then(|raw| {
            let cleaned = crate::services::sanitize::clean_markdown(
                raw,
                crate::services::validate::MAX_BODY_LEN,
            );
            (!cleaned.is_empty()).then_some(cleaned)
        })
    })
}

async fn require_can_read_course(
    pool: &PgPool,
    ctx: &RequestContext,
    id: Uuid,
) -> Result<(), ApiError> {
    let allowed = db::courses::caller_can_read_course(
        pool,
        id,
        ctx.user_id,
        ctx.tenant_id,
        is_org_admin(ctx),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !allowed {
        return Err(ApiError::CourseNotFound);
    }
    Ok(())
}

async fn require_course_in_active_tenant(
    pool: &PgPool,
    ctx: &RequestContext,
    id: Uuid,
) -> Result<db::courses::CourseRow, ApiError> {
    let tenant_id = ctx.tenant_id.ok_or(ApiError::CourseNotFound)?;
    let mut tx = db::begin_with_context(pool, ctx.user_id, ctx.tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let row = db::courses::fetch_course(&mut *tx, id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::CourseNotFound)?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if row.tenant_id != tenant_id {
        return Err(ApiError::CourseNotFound);
    }
    Ok(row)
}

async fn require_can_read_course_detail(
    pool: &PgPool,
    ctx: &RequestContext,
    id: Uuid,
) -> Result<(), ApiError> {
    require_course_in_active_tenant(pool, ctx, id).await?;
    require_can_read_course(pool, ctx, id).await
}

async fn require_can_admin_course_detail(
    pool: &PgPool,
    ctx: &RequestContext,
    id: Uuid,
) -> Result<(), ApiError> {
    require_course_in_active_tenant(pool, ctx, id).await?;
    let allowed = db::courses::caller_can_admin_course(
        pool,
        id,
        ctx.user_id,
        ctx.tenant_id,
        is_org_admin(ctx),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !allowed {
        return Err(ApiError::Forbidden);
    }
    Ok(())
}

async fn create_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    body: CreateCourse,
) -> Result<Json<CourseDto>, ApiError> {
    require_teacher_or_admin(ctx)?;
    let tenant_id = ctx
        .tenant_id
        .ok_or_else(|| ApiError::BadRequest("user has no active tenant".into()))?;

    // Sanitize the title before it is slugged + stored (defense-in-depth).
    let clean_title = crate::services::sanitize::clean_text(
        &body.title,
        crate::services::validate::MAX_TITLE_LEN,
    );
    let base = slugger::slugify(&clean_title);
    if base.is_empty() {
        return Err(ApiError::BadRequest("title produces empty slug".into()));
    }
    let mut tx = db::begin_with_context(pool, ctx.user_id, Some(tenant_id))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let existing: Vec<String> =
        sqlx::query_scalar("SELECT slug FROM courses WHERE tenant_id = $1 AND slug LIKE $2")
            .bind(tenant_id)
            .bind(format!("{base}%"))
            .fetch_all(&mut *tx)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
    let slug = slugger::dedup(&base, &existing);

    let row = db::courses::insert_course(
        &mut tx,
        tenant_id,
        &slug,
        &clean_title,
        body.description.as_deref(),
        ctx.user_id,
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;

    db::courses::insert_owner_membership(&mut tx, row.id, ctx.user_id, tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    db::audit::emit_audit_event(
        &mut tx,
        tenant_id,
        ctx.user_id,
        "course.create",
        "course",
        row.id,
        None,
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;

    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(row.into()))
}

async fn list_inner(pool: &PgPool, ctx: &RequestContext) -> Result<Json<Vec<CourseDto>>, ApiError> {
    let rows = db::courses::list_for_caller(pool, ctx.user_id, ctx.tenant_id, is_org_admin(ctx))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(
        rows.into_iter()
            .map(|row| {
                let mut dto = CourseDto::from(row.course);
                dto.caller_course_role = row.caller_course_role;
                dto
            })
            .collect(),
    ))
}

async fn get_one_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    id: Uuid,
) -> Result<Json<CourseDto>, ApiError> {
    let mut tx = db::begin_with_context(pool, ctx.user_id, ctx.tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let row = db::courses::fetch_course(&mut *tx, id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::CourseNotFound)?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    require_can_read_course(pool, ctx, id).await?;
    Ok(Json(row.into()))
}

async fn modules_with_lessons_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    id: Uuid,
) -> Result<Json<Vec<ModuleWithLessonsDto>>, ApiError> {
    require_can_read_course_detail(pool, ctx, id).await?;
    let mut tx = db::begin_with_context(pool, ctx.user_id, ctx.tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let mut rows = db::course_detail::outline(&mut tx, id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    // Server-side enforcement of drip/prerequisite gating. The lock-state
    // endpoint drives the UI's 🔒 rendering, but a student hitting the API
    // directly must not receive locked lesson CONTENT either. Staff keep full
    // visibility; learners get titles/order (so outlines and lock reasons
    // still render) with bodies and media stripped.
    let is_staff = db::courses::caller_can_staff_course(
        &pool,
        id,
        ctx.user_id,
        ctx.tenant_id,
        is_org_admin(ctx),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !is_staff {
        let locks = db::lessons::course_lock_state_for_user(&mut tx, id, ctx.user_id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        let locked: std::collections::HashSet<Uuid> = locks
            .into_iter()
            .filter(|s| s.locked)
            .map(|s| s.lesson_id)
            .collect();
        if !locked.is_empty() {
            for module in rows.iter_mut() {
                for lesson in module.lessons.iter_mut() {
                    if locked.contains(&lesson.id) {
                        lesson.body_md = None;
                        lesson.video_asset_id = None;
                        lesson.live_session_id = None;
                    }
                }
            }
        }
    }

    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(rows.into_iter().map(Into::into).collect()))
}

async fn sessions_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    id: Uuid,
) -> Result<Json<Vec<CourseSessionDto>>, ApiError> {
    require_can_read_course_detail(pool, ctx, id).await?;
    let mut tx = db::begin_with_context(pool, ctx.user_id, ctx.tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let rows = db::course_detail::sessions(&mut *tx, id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(rows.into_iter().map(Into::into).collect()))
}

async fn members_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    id: Uuid,
) -> Result<Json<Vec<CourseMemberDto>>, ApiError> {
    require_can_admin_course_detail(pool, ctx, id).await?;
    let mut tx = db::begin_with_context(pool, ctx.user_id, ctx.tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let rows = db::course_detail::members(&mut *tx, id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(rows.into_iter().map(Into::into).collect()))
}

async fn patch_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    id: Uuid,
    body: PatchCourse,
) -> Result<Json<CourseDto>, ApiError> {
    let allowed = db::courses::caller_can_admin_course(
        pool,
        id,
        ctx.user_id,
        ctx.tenant_id,
        is_org_admin(ctx),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !allowed {
        return Err(ApiError::Forbidden);
    }

    if let Some(new_status) = &body.status {
        let mut tx = db::begin_with_context(pool, ctx.user_id, ctx.tenant_id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        let row = db::courses::fetch_course(&mut *tx, id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?
            .ok_or(ApiError::CourseNotFound)?;
        tx.commit()
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        if !db::courses::is_valid_status_transition(&row.status, new_status) {
            return Err(ApiError::BadRequest(format!(
                "invalid status transition {} -> {}",
                row.status, new_status
            )));
        }
    }

    let tenant_id = ctx
        .tenant_id
        .ok_or_else(|| ApiError::BadRequest("user has no active tenant".into()))?;

    if let Some(Some(asset_id)) = body.cover_asset_id {
        let mut asset_tx = db::begin_with_context(pool, ctx.user_id, Some(tenant_id))
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        let asset = db::file_assets::fetch(&mut *asset_tx, asset_id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?
            .ok_or(ApiError::FileAssetNotFound)?;
        asset_tx
            .commit()
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        if asset.tenant_id != tenant_id {
            return Err(ApiError::FileAssetNotFound);
        }
        if asset.status != "available" {
            return Err(ApiError::BadRequest(format!(
                "asset is in status '{}'; must be 'available'",
                asset.status
            )));
        }
        let valid_link = asset.linked_entity_type.as_deref() == Some("course")
            && asset.linked_entity_id == Some(id);
        if !valid_link {
            return Err(ApiError::BadRequest(
                "asset is not linked to this course".into(),
            ));
        }
    }

    let syllabus_md = sanitize_optional_markdown_patch(body.syllabus_md.as_ref());
    let grading_policy_md = sanitize_optional_markdown_patch(body.grading_policy_md.as_ref());

    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    db::set_request_guc(&mut tx, ctx.user_id, Some(tenant_id))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let updated = db::courses::update_course(
        &mut tx,
        id,
        db::courses::UpdateCourse {
            title: body.title.as_deref(),
            description: body.description.as_deref(),
            status: body.status.as_deref(),
            cover_asset_id: body.cover_asset_id,
            // Map Option<Option<String>> -> Option<Option<&str>> per element.
            syllabus_md: syllabus_md.as_ref().map(|o| o.as_deref()),
            grading_policy_md: grading_policy_md.as_ref().map(|o| o.as_deref()),
            self_enrollment_enabled: body.self_enrollment_enabled,
        },
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?
    .ok_or(ApiError::CourseNotFound)?;

    db::audit::emit_audit_event(
        &mut tx,
        tenant_id,
        ctx.user_id,
        "course.update",
        "course",
        id,
        None,
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;

    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(updated.into()))
}

async fn delete_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    id: Uuid,
) -> Result<axum::http::StatusCode, ApiError> {
    let allowed = db::courses::caller_can_admin_course(
        pool,
        id,
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
        .ok_or_else(|| ApiError::BadRequest("user has no active tenant".into()))?;

    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    db::set_request_guc(&mut tx, ctx.user_id, Some(tenant_id))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let deleted = db::courses::delete_course(&mut tx, id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !deleted {
        return Err(ApiError::CourseNotFound);
    }
    db::audit::emit_audit_event(
        &mut tx,
        tenant_id,
        ctx.user_id,
        "course.delete",
        "course",
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

/// Deep-copy a course into a new draft. Staff/org-admin only. The new course
/// is owned by the caller; its title is "<original> (copy)" with a deduped
/// slug. Modules, lessons, assignments, quizzes (+ questions) carry over;
/// enrollments/submissions/grades/attempts are reset (none copied).
async fn duplicate_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    id: Uuid,
) -> Result<Json<DuplicateCourseDto>, ApiError> {
    // Staff (owner/teacher/TA) or org-admin on THIS course may duplicate it.
    let allowed = db::courses::caller_can_staff_course(
        pool,
        id,
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
        .ok_or_else(|| ApiError::BadRequest("user has no active tenant".into()))?;

    let mut tx = db::begin_with_context(pool, ctx.user_id, Some(tenant_id))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    // Re-read the source under the tx so we have the original title (and to
    // confirm it lives in the active tenant under RLS).
    let source = db::courses::fetch_course(&mut *tx, id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::CourseNotFound)?;
    if source.tenant_id != tenant_id {
        return Err(ApiError::CourseNotFound);
    }

    let new_title = format!("{} (copy)", source.title);
    let base = slugger::slugify(&new_title);
    let base = if base.is_empty() {
        slugger::slugify(&format!("{} copy", source.slug))
    } else {
        base
    };
    let existing: Vec<String> =
        sqlx::query_scalar("SELECT slug FROM courses WHERE tenant_id = $1 AND slug LIKE $2")
            .bind(tenant_id)
            .bind(format!("{base}%"))
            .fetch_all(&mut *tx)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
    let new_slug = slugger::dedup(&base, &existing);

    let new_course =
        db::courses::duplicate_course(&mut tx, tenant_id, id, &new_slug, &new_title, ctx.user_id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;

    // The caller becomes the teacher on the copy so it shows up in their list.
    db::courses::insert_owner_membership(&mut tx, new_course.id, ctx.user_id, tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    db::audit::emit_audit_event(
        &mut tx,
        tenant_id,
        ctx.user_id,
        "course.duplicate",
        "course",
        new_course.id,
        Some(serde_json::json!({ "source_course_id": id })),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;

    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(DuplicateCourseDto {
        id: new_course.id,
        slug: new_course.slug,
        title: new_course.title,
    }))
}

/// Course syllabus (syllabus_md + grading_policy_md). Readable by anyone who
/// can read the course (students included), so they can see the policy without
/// staff access.
async fn syllabus_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    id: Uuid,
) -> Result<Json<CourseSyllabusDto>, ApiError> {
    require_can_read_course_detail(pool, ctx, id).await?;
    let mut tx = db::begin_with_context(pool, ctx.user_id, ctx.tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let row = db::courses::fetch_course(&mut *tx, id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::CourseNotFound)?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(CourseSyllabusDto {
        course_id: row.id,
        title: row.title,
        syllabus_md: row.syllabus_md,
        grading_policy_md: row.grading_policy_md,
    }))
}
