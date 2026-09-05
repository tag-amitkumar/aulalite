use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::auth::jit_provision::ensure_user;
use crate::auth::local_login::LocalLoginConfig;
use crate::error::ApiError;

const TENANT_SLUG: &str = "local-audit";
const COURSE_SLUG: &str = "audit-course";
const ENROLLMENT_CODE: &str = "AUDIT123";

#[derive(Clone)]
pub struct DevSeedState {
    pub pool: PgPool,
    pub config: LocalLoginConfig,
    pub enabled: bool,
}

#[derive(Serialize)]
struct DevSeedResponse {
    tenant_slug: &'static str,
    course_slug: &'static str,
    teacher_email: String,
    student_email: String,
    enrollment_code: &'static str,
}

pub fn routes(state: DevSeedState) -> Router {
    Router::new()
        .route("/v1/dev/audit-seed", post(seed))
        .with_state(state)
}

async fn seed(State(state): State<DevSeedState>) -> Result<Json<DevSeedResponse>, ApiError> {
    if !state.enabled || !state.config.is_enabled() {
        return Err(ApiError::Forbidden);
    }

    let teacher = state
        .config
        .profile("teacher")
        .ok_or(ApiError::Forbidden)?
        .clone();
    let student = state
        .config
        .profile("student")
        .ok_or(ApiError::Forbidden)?
        .clone();

    let teacher_user = ensure_user(&state.pool, &teacher.claims())
        .await
        .map_err(|err| ApiError::Internal(format!("audit teacher provisioning failed: {err}")))?;
    let student_user = ensure_user(&state.pool, &student.claims())
        .await
        .map_err(|err| ApiError::Internal(format!("audit student provisioning failed: {err}")))?;

    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(|err| ApiError::Internal(format!("transaction begin failed: {err}")))?;

    let tenant_id = get_or_create_audit_tenant(
        &mut tx,
        teacher_user.user_id,
        teacher_user.display_name.as_deref(),
    )
    .await?;
    set_tenant(&mut tx, tenant_id).await?;
    configure_audit_tenant(&mut tx, tenant_id).await?;
    let course_id = upsert_course(&mut tx, tenant_id, teacher_user.user_id).await?;
    upsert_tenant_membership(&mut tx, tenant_id, student_user.user_id, "student").await?;
    upsert_course_membership(
        &mut tx,
        tenant_id,
        course_id,
        teacher_user.user_id,
        "teacher",
    )
    .await?;
    upsert_course_membership(
        &mut tx,
        tenant_id,
        course_id,
        student_user.user_id,
        "student",
    )
    .await?;

    let module_id = get_or_create_module(&mut tx, tenant_id, course_id).await?;
    let live_starts_at = Utc::now() + Duration::days(1);
    let series_id = get_or_create_live_series(
        &mut tx,
        tenant_id,
        course_id,
        teacher_user.user_id,
        live_starts_at,
    )
    .await?;
    let session_id = get_or_create_live_session(
        &mut tx,
        tenant_id,
        course_id,
        series_id,
        teacher_user.user_id,
        live_starts_at,
    )
    .await?;
    get_or_create_lesson(&mut tx, tenant_id, course_id, module_id, session_id).await?;
    get_or_create_assignment(&mut tx, tenant_id, course_id, teacher_user.user_id).await?;
    upsert_enrollment_code(&mut tx, tenant_id, course_id, teacher_user.user_id).await?;

    tx.commit()
        .await
        .map_err(|err| ApiError::Internal(format!("transaction commit failed: {err}")))?;

    Ok(Json(DevSeedResponse {
        tenant_slug: TENANT_SLUG,
        course_slug: COURSE_SLUG,
        teacher_email: teacher.email,
        student_email: student.email,
        enrollment_code: ENROLLMENT_CODE,
    }))
}

async fn get_or_create_audit_tenant(
    tx: &mut Transaction<'_, Postgres>,
    teacher_user_id: Uuid,
    teacher_name: Option<&str>,
) -> Result<Uuid, ApiError> {
    sqlx::query("SELECT set_config('app.user_id', $1, true)")
        .bind(teacher_user_id.to_string())
        .execute(&mut **tx)
        .await
        .map_err(db_error)?;
    let existing: Option<Uuid> =
        sqlx::query_scalar("SELECT tenant_id FROM resolve_active_workspace($1, NULL)")
            .bind(teacher_user_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(db_error)?;
    if let Some(tenant_id) = existing {
        return Ok(tenant_id);
    }

    let name = teacher_name
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(|name| format!("{name}'s academy"))
        .unwrap_or_else(|| "Local Audit".into());
    sqlx::query_scalar::<_, Option<Uuid>>("SELECT provision_personal_workspace_if_needed($1, $2)")
        .bind(teacher_user_id)
        .bind(name)
        .fetch_one(&mut **tx)
        .await
        .map_err(db_error)?
        .ok_or_else(|| ApiError::Conflict("audit_workspace_unavailable".into()))
}

async fn configure_audit_tenant(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
) -> Result<(), ApiError> {
    sqlx::query(
        "UPDATE tenants
            SET slug = $2, name = 'Local Audit', status = 'active', updated_at = now()
          WHERE id = $1",
    )
    .bind(tenant_id)
    .bind(TENANT_SLUG)
    .execute(&mut **tx)
    .await
    .map(|_| ())
    .map_err(db_error)
}

async fn set_tenant(tx: &mut Transaction<'_, Postgres>, tenant_id: Uuid) -> Result<(), ApiError> {
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut **tx)
        .await
        .map(|_| ())
        .map_err(db_error)
}

async fn upsert_course(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    owner_user_id: Uuid,
) -> Result<Uuid, ApiError> {
    sqlx::query_scalar(
        "INSERT INTO courses (tenant_id, slug, title, description, status, owner_user_id)
         VALUES ($1, $2, 'Audit Course', 'Local audit checklist course.', 'published', $3)
         ON CONFLICT (tenant_id, slug) DO UPDATE
            SET title = EXCLUDED.title,
                description = EXCLUDED.description,
                status = EXCLUDED.status,
                owner_user_id = EXCLUDED.owner_user_id,
                updated_at = now()
         RETURNING id",
    )
    .bind(tenant_id)
    .bind(COURSE_SLUG)
    .bind(owner_user_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(db_error)
}

async fn upsert_tenant_membership(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    user_id: Uuid,
    role: &str,
) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO tenant_memberships (tenant_id, user_id, role, status)
         VALUES ($1, $2, $3, 'active')
         ON CONFLICT (tenant_id, user_id) DO UPDATE
            SET role = EXCLUDED.role,
                status = EXCLUDED.status,
                updated_at = now()",
    )
    .bind(tenant_id)
    .bind(user_id)
    .bind(role)
    .execute(&mut **tx)
    .await
    .map(|_| ())
    .map_err(db_error)
}

async fn upsert_course_membership(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    course_id: Uuid,
    user_id: Uuid,
    role: &str,
) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO course_memberships (course_id, user_id, tenant_id, role, status)
         VALUES ($1, $2, $3, $4, 'active')
         ON CONFLICT (course_id, user_id) DO UPDATE
            SET role = EXCLUDED.role,
                status = EXCLUDED.status",
    )
    .bind(course_id)
    .bind(user_id)
    .bind(tenant_id)
    .bind(role)
    .execute(&mut **tx)
    .await
    .map(|_| ())
    .map_err(db_error)
}

async fn get_or_create_module(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    course_id: Uuid,
) -> Result<Uuid, ApiError> {
    if let Some(id) = sqlx::query_scalar(
        "SELECT id FROM modules WHERE course_id = $1 AND title = 'Audit Module' LIMIT 1",
    )
    .bind(course_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(db_error)?
    {
        return Ok(id);
    }

    sqlx::query_scalar(
        "INSERT INTO modules (tenant_id, course_id, title, sort_order)
         VALUES ($1, $2, 'Audit Module', 1)
         RETURNING id",
    )
    .bind(tenant_id)
    .bind(course_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(db_error)
}

async fn get_or_create_live_series(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    course_id: Uuid,
    teacher_user_id: Uuid,
    starts_at: DateTime<Utc>,
) -> Result<Uuid, ApiError> {
    if let Some(id) = sqlx::query_scalar(
        "SELECT id FROM live_session_series
         WHERE course_id = $1 AND title = 'Audit Live Class'
         LIMIT 1",
    )
    .bind(course_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(db_error)?
    {
        sqlx::query(
            "UPDATE live_session_series
                SET starts_at = $2,
                    duration_minutes = 45,
                    primary_teacher_id = $3,
                    recording_enabled = true,
                    transport_mode = 'webrtc',
                    updated_at = now()
              WHERE id = $1",
        )
        .bind(id)
        .bind(starts_at)
        .bind(teacher_user_id)
        .execute(&mut **tx)
        .await
        .map_err(db_error)?;
        return Ok(id);
    }

    sqlx::query_scalar(
        "INSERT INTO live_session_series
            (tenant_id, course_id, title, starts_at, duration_minutes, frequency,
             byweekday, end_kind, occurrence_count, primary_teacher_id,
             recording_enabled, transport_mode)
         VALUES ($1, $2, 'Audit Live Class', $3, 45, 'none', NULL, 'count', 1, $4, true, 'webrtc')
         RETURNING id",
    )
    .bind(tenant_id)
    .bind(course_id)
    .bind(starts_at)
    .bind(teacher_user_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(db_error)
}

async fn get_or_create_live_session(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    course_id: Uuid,
    series_id: Uuid,
    teacher_user_id: Uuid,
    starts_at: DateTime<Utc>,
) -> Result<Uuid, ApiError> {
    if let Some(id) = sqlx::query_scalar(
        "SELECT id FROM live_sessions WHERE series_id = $1 AND occurrence_index = 0",
    )
    .bind(series_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(db_error)?
    {
        sqlx::query(
            "UPDATE live_sessions
                SET title = 'Audit Live Class',
                    starts_at = $2,
                    duration_minutes = 45,
                    primary_teacher_id = $3,
                    recording_enabled = true,
                    transport_mode = 'webrtc',
                    status = 'scheduled',
                    actual_started_at = NULL,
                    actual_ended_at = NULL,
                    main_path = NULL,
                    screen_path = NULL,
                    publish_nonce = NULL,
                    publish_nonce_expires_at = NULL,
                    diverged = false,
                    updated_at = now()
              WHERE id = $1",
        )
        .bind(id)
        .bind(starts_at)
        .bind(teacher_user_id)
        .execute(&mut **tx)
        .await
        .map_err(db_error)?;
        return Ok(id);
    }

    sqlx::query_scalar(
        "INSERT INTO live_sessions
            (tenant_id, course_id, series_id, occurrence_index, title, starts_at,
             duration_minutes, primary_teacher_id, recording_enabled, transport_mode)
         VALUES ($1, $2, $3, 0, 'Audit Live Class', $4, 45, $5, true, 'webrtc')
         RETURNING id",
    )
    .bind(tenant_id)
    .bind(course_id)
    .bind(series_id)
    .bind(starts_at)
    .bind(teacher_user_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(db_error)
}

async fn get_or_create_lesson(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    course_id: Uuid,
    module_id: Uuid,
    session_id: Uuid,
) -> Result<Uuid, ApiError> {
    if let Some(id) = sqlx::query_scalar(
        "SELECT id FROM lessons WHERE course_id = $1 AND title = 'Audit Live Lesson' LIMIT 1",
    )
    .bind(course_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(db_error)?
    {
        return Ok(id);
    }

    sqlx::query_scalar(
        "INSERT INTO lessons
            (tenant_id, course_id, module_id, type, title, body_md, live_session_id,
             sort_order, published_at)
         VALUES ($1, $2, $3, 'live_session', 'Audit Live Lesson',
                 'Join the audit live class and verify the learner experience.',
                 $4, 1, now())
         RETURNING id",
    )
    .bind(tenant_id)
    .bind(course_id)
    .bind(module_id)
    .bind(session_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(db_error)
}

async fn get_or_create_assignment(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    course_id: Uuid,
    teacher_user_id: Uuid,
) -> Result<Uuid, ApiError> {
    if let Some(id) = sqlx::query_scalar(
        "SELECT id FROM assignments WHERE course_id = $1 AND title = 'Audit Assignment' LIMIT 1",
    )
    .bind(course_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(db_error)?
    {
        sqlx::query(
            "UPDATE assignments SET accepts_files = true, updated_at = now() WHERE id = $1",
        )
        .bind(id)
        .execute(&mut **tx)
        .await
        .map_err(db_error)?;
        return Ok(id);
    }

    sqlx::query_scalar(
        "INSERT INTO assignments
            (tenant_id, course_id, title, instructions_md, grading_mode, max_points,
             allow_late, lock_on_submit, accepts_text, accepts_files, release_mode,
             status, published_at, created_by)
         VALUES ($1, $2, 'Audit Assignment',
                 'Submit a short response during the local audit.',
                 'numeric'::assignment_grading_mode, 10, true, false, true, true,
                 'instant'::assignment_release_mode, 'published'::assignment_status, now(), $3)
         RETURNING id",
    )
    .bind(tenant_id)
    .bind(course_id)
    .bind(teacher_user_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(db_error)
}

async fn upsert_enrollment_code(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    course_id: Uuid,
    teacher_user_id: Uuid,
) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO enrollment_codes (tenant_id, course_id, code, max_uses, created_by)
         VALUES ($1, $2, $3, NULL, $4)
         ON CONFLICT (code) DO UPDATE
            SET tenant_id = EXCLUDED.tenant_id,
                course_id = EXCLUDED.course_id,
                max_uses = EXCLUDED.max_uses,
                expires_at = NULL,
                created_by = EXCLUDED.created_by",
    )
    .bind(tenant_id)
    .bind(course_id)
    .bind(ENROLLMENT_CODE)
    .bind(teacher_user_id)
    .execute(&mut **tx)
    .await
    .map(|_| ())
    .map_err(db_error)
}

fn db_error(err: sqlx::Error) -> ApiError {
    ApiError::Internal(format!("audit seed failed: {err}"))
}
