// crates/backend/src/handlers/lessons.rs
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
pub struct CreateLesson {
    pub r#type: String,
    pub title: String,
    pub body_md: Option<String>,
    pub live_session_id: Option<Uuid>,
}

#[derive(Deserialize, Default)]
pub struct PatchLesson {
    pub title: Option<String>,
    pub body_md: Option<String>,
    #[serde(default, with = "::serde_with::rust::double_option")]
    pub video_asset_id: Option<Option<Uuid>>,
    #[serde(default, with = "::serde_with::rust::double_option")]
    pub live_session_id: Option<Option<Uuid>>,
    /// Drip/time-gate setter. Omitted = leave alone; `null` = clear the gate;
    /// a timestamp = lock for students until then.
    #[serde(default, with = "::serde_with::rust::double_option")]
    pub release_at: Option<Option<chrono::DateTime<chrono::Utc>>>,
}

/// Replace a lesson's prerequisite set (staff). An empty list clears it.
#[derive(Deserialize)]
pub struct SetPrerequisites {
    pub required_lesson_ids: Vec<Uuid>,
}

#[derive(Deserialize)]
pub struct ReorderLessons {
    pub lesson_ids: Vec<Uuid>,
}

#[derive(Serialize)]
pub struct LessonDto {
    pub id: Uuid,
    pub course_id: Uuid,
    pub module_id: Uuid,
    pub r#type: String,
    pub title: String,
    pub body_md: Option<String>,
    pub video_asset_id: Option<Uuid>,
    pub live_session_id: Option<Uuid>,
    pub sort_order: i32,
    pub release_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl From<db::lessons::LessonRow> for LessonDto {
    fn from(r: db::lessons::LessonRow) -> Self {
        Self {
            id: r.id,
            course_id: r.course_id,
            module_id: r.module_id,
            r#type: r.r#type,
            title: r.title,
            body_md: r.body_md,
            video_asset_id: r.video_asset_id,
            live_session_id: r.live_session_id,
            sort_order: r.sort_order,
            release_at: r.release_at,
        }
    }
}

#[derive(Serialize)]
pub struct PrerequisiteDto {
    pub required_lesson_id: Uuid,
    pub required_lesson_title: String,
}

impl From<db::lessons::PrerequisiteRow> for PrerequisiteDto {
    fn from(r: db::lessons::PrerequisiteRow) -> Self {
        Self {
            required_lesson_id: r.required_lesson_id,
            required_lesson_title: r.required_lesson_title,
        }
    }
}

/// Per-lesson lock state for a student. Staff always receive `locked = false`.
#[derive(Serialize)]
pub struct LessonLockDto {
    pub lesson_id: Uuid,
    pub locked: bool,
    pub reason: Option<String>,
}

impl From<db::lessons::LessonLockState> for LessonLockDto {
    fn from(s: db::lessons::LessonLockState) -> Self {
        Self {
            lesson_id: s.lesson_id,
            locked: s.locked,
            reason: s.reason,
        }
    }
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/v1/courses/{cid}/modules/{mid}/lessons",
            routing::post(create),
        )
        .route(
            "/v1/courses/{cid}/modules/{mid}/lessons/reorder",
            routing::post(reorder),
        )
        .route(
            "/v1/courses/{cid}/modules/{mid}/lessons/{lid}",
            routing::patch(patch).delete(delete_one),
        )
        // Prerequisite CRUD (staff): list / replace / clear a lesson's
        // required-lesson set.
        .route(
            "/v1/courses/{cid}/lessons/{lid}/prerequisites",
            routing::get(get_prereqs)
                .put(put_prereqs)
                .delete(delete_prereqs),
        )
        // Per-lesson lock state for the caller across a whole course
        // (student-facing; staff always unlocked).
        .route(
            "/v1/courses/{cid}/lessons/lock-state",
            routing::get(course_lock_state),
        )
}

#[doc(hidden)]
pub fn router_for_tests(pool: PgPool) -> Router {
    Router::new()
        .route(
            "/v1/courses/{cid}/modules/{mid}/lessons",
            routing::post(create_t),
        )
        .route(
            "/v1/courses/{cid}/modules/{mid}/lessons/reorder",
            routing::post(reorder_t),
        )
        .route(
            "/v1/courses/{cid}/modules/{mid}/lessons/{lid}",
            routing::patch(patch_t).delete(delete_one_t),
        )
        .route(
            "/v1/courses/{cid}/lessons/{lid}/prerequisites",
            routing::get(get_prereqs_t)
                .put(put_prereqs_t)
                .delete(delete_prereqs_t),
        )
        .route(
            "/v1/courses/{cid}/lessons/lock-state",
            routing::get(course_lock_state_t),
        )
        .with_state(TestState { pool })
}

#[derive(Clone)]
struct TestState {
    pool: PgPool,
}

fn is_org_admin(ctx: &RequestContext) -> bool {
    ctx.can_manage_organization()
}

async fn create(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path((cid, mid)): Path<(Uuid, Uuid)>,
    Json(b): Json<CreateLesson>,
) -> Result<Json<LessonDto>, ApiError> {
    create_inner(&s.pool, &ctx, cid, mid, b).await
}
async fn reorder(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path((cid, mid)): Path<(Uuid, Uuid)>,
    Json(b): Json<ReorderLessons>,
) -> Result<axum::http::StatusCode, ApiError> {
    reorder_inner(&s.pool, &ctx, cid, mid, b).await
}
async fn patch(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path((cid, mid, lid)): Path<(Uuid, Uuid, Uuid)>,
    Json(b): Json<PatchLesson>,
) -> Result<Json<LessonDto>, ApiError> {
    patch_inner(&s.pool, &ctx, cid, mid, lid, b).await
}
async fn delete_one(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path((cid, mid, lid)): Path<(Uuid, Uuid, Uuid)>,
) -> Result<axum::http::StatusCode, ApiError> {
    delete_inner(&s.pool, &ctx, cid, mid, lid).await
}

async fn get_prereqs(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path((cid, lid)): Path<(Uuid, Uuid)>,
) -> Result<Json<Vec<PrerequisiteDto>>, ApiError> {
    get_prereqs_inner(&s.pool, &ctx, cid, lid).await
}
async fn put_prereqs(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path((cid, lid)): Path<(Uuid, Uuid)>,
    Json(b): Json<SetPrerequisites>,
) -> Result<Json<Vec<PrerequisiteDto>>, ApiError> {
    put_prereqs_inner(&s.pool, &ctx, cid, lid, b).await
}
async fn delete_prereqs(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path((cid, lid)): Path<(Uuid, Uuid)>,
) -> Result<axum::http::StatusCode, ApiError> {
    delete_prereqs_inner(&s.pool, &ctx, cid, lid).await
}
async fn course_lock_state(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(cid): Path<Uuid>,
) -> Result<Json<Vec<LessonLockDto>>, ApiError> {
    course_lock_state_inner(&s.pool, &ctx, cid).await
}

async fn create_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path((cid, mid)): Path<(Uuid, Uuid)>,
    Json(b): Json<CreateLesson>,
) -> Result<Json<LessonDto>, ApiError> {
    create_inner(&s.pool, &ctx, cid, mid, b).await
}
async fn reorder_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path((cid, mid)): Path<(Uuid, Uuid)>,
    Json(b): Json<ReorderLessons>,
) -> Result<axum::http::StatusCode, ApiError> {
    reorder_inner(&s.pool, &ctx, cid, mid, b).await
}
async fn patch_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path((cid, mid, lid)): Path<(Uuid, Uuid, Uuid)>,
    Json(b): Json<PatchLesson>,
) -> Result<Json<LessonDto>, ApiError> {
    patch_inner(&s.pool, &ctx, cid, mid, lid, b).await
}
async fn delete_one_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path((cid, mid, lid)): Path<(Uuid, Uuid, Uuid)>,
) -> Result<axum::http::StatusCode, ApiError> {
    delete_inner(&s.pool, &ctx, cid, mid, lid).await
}

async fn get_prereqs_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path((cid, lid)): Path<(Uuid, Uuid)>,
) -> Result<Json<Vec<PrerequisiteDto>>, ApiError> {
    get_prereqs_inner(&s.pool, &ctx, cid, lid).await
}
async fn put_prereqs_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path((cid, lid)): Path<(Uuid, Uuid)>,
    Json(b): Json<SetPrerequisites>,
) -> Result<Json<Vec<PrerequisiteDto>>, ApiError> {
    put_prereqs_inner(&s.pool, &ctx, cid, lid, b).await
}
async fn delete_prereqs_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path((cid, lid)): Path<(Uuid, Uuid)>,
) -> Result<axum::http::StatusCode, ApiError> {
    delete_prereqs_inner(&s.pool, &ctx, cid, lid).await
}
async fn course_lock_state_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(cid): Path<Uuid>,
) -> Result<Json<Vec<LessonLockDto>>, ApiError> {
    course_lock_state_inner(&s.pool, &ctx, cid).await
}

async fn create_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
    module_id: Uuid,
    b: CreateLesson,
) -> Result<Json<LessonDto>, ApiError> {
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
    if !db::lessons::type_supported_at_1b_alpha(&b.r#type) {
        return Err(ApiError::LessonTypeNotSupported(b.r#type));
    }
    let tenant_id = ctx
        .tenant_id
        .ok_or_else(|| ApiError::BadRequest("no tenant".into()))?;

    if b.r#type == "live_session" && b.live_session_id.is_none() {
        return Err(ApiError::BadRequest(
            "live_session lesson requires live_session_id".into(),
        ));
    }
    // video lesson may be created without video_asset_id; teacher uploads later.
    // file_bundle lesson has no payload at create-time; attachments come via /v1/uploads/begin.

    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    db::set_request_guc(&mut tx, ctx.user_id, Some(tenant_id))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let so = db::lessons::next_sort_order(&mut tx, tenant_id, course_id, module_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::NotFound)?;
    // Sanitize the lesson title + body before persistence (defense-in-depth).
    let lesson_title =
        crate::services::sanitize::clean_text(&b.title, crate::services::validate::MAX_TITLE_LEN);
    let lesson_body = b.body_md.as_deref().map(|m| {
        crate::services::sanitize::clean_markdown(m, crate::services::validate::MAX_BODY_LEN)
    });
    let row = db::lessons::insert_lesson(
        &mut tx,
        tenant_id,
        course_id,
        module_id,
        &b.r#type,
        &lesson_title,
        lesson_body.as_deref(),
        b.live_session_id,
        so,
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?
    .ok_or(ApiError::NotFound)?;
    db::audit::emit_audit_event(
        &mut tx,
        tenant_id,
        ctx.user_id,
        "lesson.create",
        "lesson",
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

async fn reorder_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
    module_id: Uuid,
    b: ReorderLessons,
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
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    db::set_request_guc(&mut tx, ctx.user_id, Some(tenant_id))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let module_exists =
        db::lessons::reorder(&mut tx, tenant_id, course_id, module_id, &b.lesson_ids)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !module_exists {
        return Err(ApiError::NotFound);
    }
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(axum::http::StatusCode::OK)
}

async fn patch_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
    module_id: Uuid,
    lesson_id: Uuid,
    b: PatchLesson,
) -> Result<Json<LessonDto>, ApiError> {
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

    if let Some(Some(asset_id)) = b.video_asset_id {
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
        let valid_link = asset.linked_entity_type.as_deref() == Some("lesson")
            && asset.linked_entity_id == Some(lesson_id);
        if !valid_link {
            return Err(ApiError::BadRequest(
                "asset is not linked to this lesson".into(),
            ));
        }
    }

    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    db::set_request_guc(&mut tx, ctx.user_id, Some(tenant_id))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let row = db::lessons::update_lesson(
        &mut tx,
        tenant_id,
        course_id,
        module_id,
        lesson_id,
        b.title.as_deref(),
        b.body_md.as_deref(),
        b.video_asset_id,
        b.live_session_id,
        b.release_at,
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?
    .ok_or(ApiError::NotFound)?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(row.into()))
}

async fn delete_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
    module_id: Uuid,
    lesson_id: Uuid,
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
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    db::set_request_guc(&mut tx, ctx.user_id, Some(tenant_id))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let deleted = db::lessons::delete_lesson(&mut tx, tenant_id, course_id, module_id, lesson_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !deleted {
        return Err(ApiError::NotFound);
    }
    Ok(axum::http::StatusCode::NO_CONTENT)
}

// ─── Prerequisites + lock-state ──────────────────────────────────────────────

async fn get_prereqs_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
    lesson_id: Uuid,
) -> Result<Json<Vec<PrerequisiteDto>>, ApiError> {
    // Staff-only: the picker lives on the lesson editor.
    let allowed = db::courses::caller_can_staff_course(
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
    if !db::lessons::lesson_exists_in_course(&mut tx, tenant_id, course_id, lesson_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
    {
        return Err(ApiError::NotFound);
    }
    let rows = db::lessons::list_prerequisites(&mut tx, tenant_id, course_id, lesson_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(rows.into_iter().map(Into::into).collect()))
}

async fn put_prereqs_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
    lesson_id: Uuid,
    b: SetPrerequisites,
) -> Result<Json<Vec<PrerequisiteDto>>, ApiError> {
    // Prerequisites change learner-visible course structure, so Assist/Grade
    // alone is insufficient even when the TA is assigned to this course.
    if !ctx.can_teach() {
        return Err(ApiError::Forbidden);
    }
    let allowed = db::courses::caller_can_staff_course(
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

    // The lesson and every required lesson must live in this course (which
    // also pins them to this tenant under RLS) — reject cross-course edges.
    if !db::lessons::lesson_exists_in_course(&mut tx, tenant_id, course_id, lesson_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
    {
        return Err(ApiError::NotFound);
    }
    for req in &b.required_lesson_ids {
        if *req == lesson_id {
            return Err(ApiError::BadRequest(
                "a lesson cannot require itself".into(),
            ));
        }
        if !db::lessons::lesson_exists_in_course(&mut tx, tenant_id, course_id, *req)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?
        {
            return Err(ApiError::BadRequest(
                "prerequisite lesson is not in this course".into(),
            ));
        }
    }

    db::lessons::set_prerequisites(
        &mut tx,
        tenant_id,
        course_id,
        lesson_id,
        &b.required_lesson_ids,
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    db::audit::emit_audit_event(
        &mut tx,
        tenant_id,
        ctx.user_id,
        "lesson.prerequisites.set",
        "lesson",
        lesson_id,
        None,
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    let rows = db::lessons::list_prerequisites(&mut tx, tenant_id, course_id, lesson_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(rows.into_iter().map(Into::into).collect()))
}

async fn delete_prereqs_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
    lesson_id: Uuid,
) -> Result<axum::http::StatusCode, ApiError> {
    if !ctx.can_teach() {
        return Err(ApiError::Forbidden);
    }
    let allowed = db::courses::caller_can_staff_course(
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
    let target_exists = db::lessons::clear_prerequisites(&mut tx, tenant_id, course_id, lesson_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !target_exists {
        return Err(ApiError::NotFound);
    }
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

async fn course_lock_state_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
) -> Result<Json<Vec<LessonLockDto>>, ApiError> {
    // Anyone who can read the course can ask for lock state.
    if !db::courses::caller_can_read_course(
        pool,
        course_id,
        ctx.user_id,
        ctx.tenant_id,
        is_org_admin(ctx),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?
    {
        return Err(ApiError::Forbidden);
    }
    // Staff are never gated: short-circuit to all-unlocked so the editor and
    // teacher preview always see every lesson.
    let is_staff = db::courses::caller_can_staff_course(
        pool,
        course_id,
        ctx.user_id,
        ctx.tenant_id,
        is_org_admin(ctx),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;

    let mut tx = db::begin_with_context(pool, ctx.user_id, ctx.tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let states = if is_staff {
        Vec::new()
    } else {
        db::lessons::course_lock_state_for_user(&mut tx, course_id, ctx.user_id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?
    };
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(states.into_iter().map(Into::into).collect()))
}
