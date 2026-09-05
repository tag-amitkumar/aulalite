use axum::extract::{Extension, Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::context::RequestContext;
use crate::db;
use crate::error::ApiError;
use crate::AppState;

#[derive(Serialize)]
pub struct MeResponse {
    pub user_id: uuid::Uuid,
    pub firebase_uid: String,
    pub email: String,
    pub display_name: Option<String>,
    pub tenant_id: Option<uuid::Uuid>,
    pub tenant_role: Option<core_types::TenantRole>,
    pub is_platform_admin: bool,
}

pub async fn me(Extension(ctx): Extension<RequestContext>) -> Result<Json<MeResponse>, ApiError> {
    Ok(Json(MeResponse {
        user_id: ctx.user_id,
        firebase_uid: ctx.firebase_uid,
        email: ctx.email,
        display_name: ctx.display_name,
        tenant_id: ctx.tenant_id,
        tenant_role: ctx.tenant_role,
        is_platform_admin: ctx.is_platform_admin,
    }))
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct MyWorkspace {
    pub tenant_id: Uuid,
    pub name: String,
    pub slug: String,
    pub role: String,
    pub current: bool,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct MyWorkspacesResponse {
    pub current_tenant_id: Option<Uuid>,
    pub workspaces: Vec<MyWorkspace>,
}

/// Returns only ACTIVE tenant memberships for the signed-in user. The narrow
/// DB function can read tenant display names across those memberships without
/// granting a platform admin implicit tenant access.
pub async fn my_workspaces(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<MyWorkspacesResponse>, ApiError> {
    let mut tx = db::begin_with_context(&state.pool, ctx.user_id, ctx.tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let rows: Vec<(Uuid, String, String, String)> =
        sqlx::query_as("SELECT tenant_id, name, slug, role FROM list_my_workspaces()")
            .fetch_all(&mut *tx)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(MyWorkspacesResponse {
        current_tenant_id: ctx.tenant_id,
        workspaces: rows
            .into_iter()
            .map(|(tenant_id, name, slug, role)| MyWorkspace {
                tenant_id,
                name,
                slug,
                role,
                current: Some(tenant_id) == ctx.tenant_id,
            })
            .collect(),
    }))
}

#[derive(Serialize)]
pub struct MyCourse {
    pub course_id: Uuid,
    pub slug: String,
    pub title: String,
    pub status: String,
    pub role: String,
    pub next_session_at: Option<chrono::DateTime<chrono::Utc>>,
}

pub async fn my_courses(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<Vec<MyCourse>>, ApiError> {
    // Wrap reads in a tx with RLS GUCs so this works under the
    // non-superuser `aulalite_app` role (see migration 20260517000020).
    let mut tx = db::begin_with_context(&state.pool, ctx.user_id, ctx.tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let rows: Vec<(
        Uuid,
        String,
        String,
        String,
        String,
        Option<chrono::DateTime<chrono::Utc>>,
    )> = sqlx::query_as(
        r#"
            SELECT
                c.id, c.slug, c.title, c.status,
                cm.role,
                (SELECT MIN(starts_at) FROM live_sessions
                  WHERE course_id = c.id
                    AND starts_at >= now()
                    AND status IN ('scheduled','live')) AS next_session_at
              FROM course_memberships cm
              JOIN courses c ON c.id = cm.course_id
             WHERE cm.user_id = $1 AND cm.status = 'active'
             ORDER BY c.title
            "#,
    )
    .bind(ctx.user_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(
        rows.into_iter()
            .map(
                |(course_id, slug, title, status, role, next_session_at)| MyCourse {
                    course_id,
                    slug,
                    title,
                    status,
                    role,
                    next_session_at,
                },
            )
            .collect(),
    ))
}

#[derive(Serialize)]
pub struct MyActiveSession {
    pub session_id: Uuid,
    pub course_id: Uuid,
    pub title: String,
    pub starts_at: chrono::DateTime<chrono::Utc>,
    pub transport_mode: String,
}

#[derive(Serialize)]
pub struct MyActiveSessionResponse {
    pub active: Option<MyActiveSession>,
}

/// Returns the caller's currently-live session in the active tenant, if
/// any. Lets the UI surface a "you already have a live session in X"
/// banner before the user hits the Start button a second time. Paired
/// with the `live_sessions_one_live_per_user` DB partial unique index
/// for atomic enforcement.
pub async fn my_active_session(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<MyActiveSessionResponse>, ApiError> {
    let row =
        db::live_sessions::find_live_for_user_with_context(&state.pool, ctx.user_id, ctx.tenant_id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(MyActiveSessionResponse {
        active: row.map(|r| MyActiveSession {
            session_id: r.id,
            course_id: r.course_id,
            title: r.title,
            starts_at: r.actual_started_at.unwrap_or(r.starts_at),
            transport_mode: r.transport_mode,
        }),
    }))
}

#[derive(Deserialize)]
pub struct ScheduleQuery {
    pub days: Option<i64>,
}
#[derive(Serialize)]
pub struct ScheduleEntry {
    pub session_id: Uuid,
    pub course_id: Uuid,
    pub course_title: String,
    pub title: String,
    pub starts_at: chrono::DateTime<chrono::Utc>,
    pub duration_minutes: i32,
    pub status: String,
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

pub async fn my_schedule(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Query(q): Query<ScheduleQuery>,
) -> Result<Json<Vec<ScheduleEntry>>, ApiError> {
    let days = normalize_schedule_days(q.days)?;
    let mut tx = db::begin_with_context(&state.pool, ctx.user_id, ctx.tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let rows: Vec<(
        Uuid,
        Uuid,
        String,
        String,
        chrono::DateTime<chrono::Utc>,
        i32,
        String,
    )> = sqlx::query_as(
        r#"
            SELECT ls.id, ls.course_id, c.title, ls.title, ls.starts_at,
                   ls.duration_minutes, ls.status
              FROM live_sessions ls
              JOIN course_memberships cm
                ON cm.course_id = ls.course_id
               AND cm.user_id = $1
               AND cm.status = 'active'
              JOIN courses c ON c.id = ls.course_id
             WHERE (ls.starts_at + make_interval(mins => ls.duration_minutes)) >= now()
               AND ls.starts_at <= now() + ($2::int || ' days')::interval
               AND ls.status IN ('scheduled','live')
             ORDER BY ls.starts_at
            "#,
    )
    .bind(ctx.user_id)
    .bind(days)
    .fetch_all(&mut *tx)
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(
        rows.into_iter()
            .map(
                |(session_id, course_id, course_title, title, starts_at, dur, status)| {
                    ScheduleEntry {
                        session_id,
                        course_id,
                        course_title,
                        title,
                        starts_at,
                        duration_minutes: dur,
                        status,
                    }
                },
            )
            .collect(),
    ))
}

// ---------------------------------------------------------------------------
// GET /v1/me/transcript — consolidated cross-course academic record
// ---------------------------------------------------------------------------

/// One course's line on the transcript.
#[derive(Serialize)]
pub struct TranscriptCourseDto {
    pub course_id: Uuid,
    pub slug: String,
    pub title: String,
    /// Course lifecycle status (`active`/`archived`/…).
    pub status: String,
    pub role: String,
    /// `active` or `removed` (withdrawn — record retained, flagged in UI).
    pub membership_status: String,
    pub joined_at: chrono::DateTime<chrono::Utc>,
    pub lessons_total: i64,
    pub lessons_completed: i64,
    /// Released, numerically-graded assignment count.
    pub graded_released_count: i64,
    /// Weighted course total as 0..=100 using the SAME math as the staff
    /// gradebook. `None` when there is no counted coursework yet.
    pub weighted_total: Option<f64>,
    /// Letter grade derived from `weighted_total` on the standard A–F scale.
    /// `None` when no weighted total exists.
    pub letter_grade: Option<String>,
    /// Present when a certificate was issued (or revoked) for this course.
    pub certificate: Option<TranscriptCertificateDto>,
}

#[derive(Serialize)]
pub struct TranscriptCertificateDto {
    pub credential_id: String,
    pub status: String,
}

#[derive(Serialize)]
pub struct TranscriptDto {
    pub student_user_id: Uuid,
    pub generated_at: chrono::DateTime<chrono::Utc>,
    pub courses: Vec<TranscriptCourseDto>,
}

/// Map a weighted percentage to a letter grade on the standard college scale:
/// A ≥ 90, B ≥ 80, C ≥ 70, D ≥ 60, else F. Pure + unit-tested; the transcript
/// and any future report-card export must agree on this mapping.
fn letter_for_percent(pct: f64) -> &'static str {
    match pct {
        p if p >= 90.0 => "A",
        p if p >= 80.0 => "B",
        p if p >= 70.0 => "C",
        p if p >= 60.0 => "D",
        _ => "F",
    }
}

fn transcript_bd_f64(v: &sqlx::types::BigDecimal) -> f64 {
    v.to_string().parse::<f64>().unwrap_or(0.0)
}

pub async fn my_transcript(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<TranscriptDto>, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    let mut tx = db::begin_with_context(&state.pool, ctx.user_id, Some(tenant))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let courses = db::transcripts::courses_for_user(&mut tx, tenant, ctx.user_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let grade_items = db::transcripts::grade_items_for_user(&mut tx, tenant, ctx.user_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let category_rows = db::transcripts::category_weights_for_user(&mut tx, tenant, ctx.user_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    // Group the flat rows per course for the weighting pass.
    use std::collections::HashMap;
    let mut items_by_course: HashMap<Uuid, Vec<crate::handlers::gradebook::ScoredItem>> =
        HashMap::new();
    let mut graded_count_by_course: HashMap<Uuid, i64> = HashMap::new();
    for it in &grade_items {
        *graded_count_by_course.entry(it.course_id).or_insert(0) += 1;
        items_by_course.entry(it.course_id).or_default().push(
            crate::handlers::gradebook::ScoredItem {
                earned: transcript_bd_f64(&it.numeric_grade),
                max_points: it.max_points.unwrap_or(0) as f64,
                category: it.category_id,
            },
        );
    }
    let mut weights_by_course: HashMap<Uuid, Vec<(Uuid, i32)>> = HashMap::new();
    for c in &category_rows {
        weights_by_course
            .entry(c.course_id)
            .or_default()
            .push((c.category_id, c.weight_percent));
    }

    let out_courses = courses
        .into_iter()
        .map(|c| {
            let items = items_by_course.get(&c.course_id);
            let weights = weights_by_course
                .get(&c.course_id)
                .map(|v| v.as_slice())
                .unwrap_or(&[]);
            let weighted_total = items.and_then(|items| {
                crate::handlers::gradebook::compute_weighted_total(items, weights)
            });
            TranscriptCourseDto {
                course_id: c.course_id,
                slug: c.slug,
                title: c.title,
                status: c.status,
                role: c.role,
                membership_status: c.membership_status,
                joined_at: c.joined_at,
                lessons_total: c.lessons_total,
                lessons_completed: c.lessons_completed,
                graded_released_count: graded_count_by_course
                    .get(&c.course_id)
                    .copied()
                    .unwrap_or(0),
                weighted_total,
                letter_grade: weighted_total.map(|p| letter_for_percent(p).to_string()),
                certificate: c
                    .credential_id
                    .map(|credential_id| TranscriptCertificateDto {
                        credential_id,
                        status: c.certificate_status.unwrap_or_else(|| "issued".into()),
                    }),
            }
        })
        .collect();

    Ok(Json(TranscriptDto {
        student_user_id: ctx.user_id,
        generated_at: chrono::Utc::now(),
        courses: out_courses,
    }))
}

#[derive(Serialize, Deserialize)]
pub struct PreferencesResponse {
    pub theme: String,
    pub density: String,
    pub tour_dismissed: bool,
}

#[derive(Deserialize)]
pub struct PreferencesPatch {
    pub theme: Option<String>,
    pub density: Option<String>,
    pub tour_dismissed: Option<bool>,
}

const THEMES: [&str; 3] = ["system", "light", "dark"];
const DENSITIES: [&str; 3] = ["compact", "comfortable", "spacious"];

fn validate_preference(
    value: &Option<String>,
    allowed: &[&str],
    field: &str,
) -> Result<(), ApiError> {
    match value {
        Some(v) if !allowed.contains(&v.as_str()) => Err(ApiError::BadRequest(format!(
            "invalid {field} preference `{v}`; expected one of {allowed:?}"
        ))),
        _ => Ok(()),
    }
}

pub async fn my_preferences(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<PreferencesResponse>, ApiError> {
    let (theme, density, tour_dismissed): (String, String, bool) = sqlx::query_as(
        "SELECT theme_preference, density_preference, tour_dismissed FROM users WHERE id = $1",
    )
    .bind(ctx.user_id)
    .fetch_one(&state.pool)
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(PreferencesResponse {
        theme,
        density,
        tour_dismissed,
    }))
}

pub async fn patch_my_preferences(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Json(patch): Json<PreferencesPatch>,
) -> Result<Json<PreferencesResponse>, ApiError> {
    validate_preference(&patch.theme, &THEMES, "theme")?;
    validate_preference(&patch.density, &DENSITIES, "density")?;
    let (theme, density, tour_dismissed): (String, String, bool) = sqlx::query_as(
        "UPDATE users SET theme_preference = COALESCE($2, theme_preference), \
         density_preference = COALESCE($3, density_preference), \
         tour_dismissed = COALESCE($4, tour_dismissed) \
         WHERE id = $1 RETURNING theme_preference, density_preference, tour_dismissed",
    )
    .bind(ctx.user_id)
    .bind(patch.theme)
    .bind(patch.density)
    .bind(patch.tour_dismissed)
    .fetch_one(&state.pool)
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(PreferencesResponse {
        theme,
        density,
        tour_dismissed,
    }))
}

#[cfg(test)]
mod tests {
    use super::{
        letter_for_percent, normalize_schedule_days, validate_preference, DENSITIES, THEMES,
    };

    #[test]
    fn transcript_letter_scale_matches_standard_cutoffs() {
        assert_eq!(letter_for_percent(100.0), "A");
        assert_eq!(letter_for_percent(90.0), "A");
        assert_eq!(letter_for_percent(89.999), "B");
        assert_eq!(letter_for_percent(80.0), "B");
        assert_eq!(letter_for_percent(79.5), "C");
        assert_eq!(letter_for_percent(70.0), "C");
        assert_eq!(letter_for_percent(65.0), "D");
        assert_eq!(letter_for_percent(60.0), "D");
        assert_eq!(letter_for_percent(59.9), "F");
        assert_eq!(letter_for_percent(0.0), "F");
    }

    #[test]
    fn preference_validation_accepts_known_values_and_none() {
        for theme in ["system", "light", "dark"] {
            assert!(validate_preference(&Some(theme.into()), &THEMES, "theme").is_ok());
        }
        for density in ["compact", "comfortable", "spacious"] {
            assert!(validate_preference(&Some(density.into()), &DENSITIES, "density").is_ok());
        }
        assert!(validate_preference(&None, &THEMES, "theme").is_ok());
    }

    #[test]
    fn preference_validation_rejects_unknown_values() {
        assert!(validate_preference(&Some("midnight".into()), &THEMES, "theme").is_err());
        assert!(validate_preference(&Some("dense".into()), &DENSITIES, "density").is_err());
    }

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
    fn schedule_days_rejects_zero_negative_and_large_values() {
        assert!(normalize_schedule_days(Some(0)).is_err());
        assert!(normalize_schedule_days(Some(-1)).is_err());
        assert!(normalize_schedule_days(Some(181)).is_err());
    }
}
