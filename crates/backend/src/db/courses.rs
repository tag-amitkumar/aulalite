// crates/backend/src/db/courses.rs
use serde::Serialize;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct CourseRow {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub slug: String,
    pub title: String,
    pub description: Option<String>,
    pub status: String,
    pub visibility: String,
    pub cover_asset_id: Option<Uuid>,
    pub owner_user_id: Uuid,
    /// Optional, student-readable markdown syllabus (migration 046).
    pub syllabus_md: Option<String>,
    /// Optional, student-readable markdown grading policy (migration 046).
    pub grading_policy_md: Option<String>,
    /// When true, any active tenant member may self-enroll from the catalog
    /// (migration 20260823000085). Seat caps still gate the actual enrollment.
    pub self_enrollment_enabled: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, sqlx::FromRow)]
pub struct CourseAccessRow {
    #[sqlx(flatten)]
    pub course: CourseRow,
    pub caller_course_role: Option<String>,
}

/// Shared column list so every `courses` read returns the same shape. Use
/// anywhere we'd otherwise hand-write `*`.
const COURSE_COLS: &str = "id, tenant_id, slug, title, description, status, visibility, \
     cover_asset_id, owner_user_id, syllabus_md, grading_policy_md, self_enrollment_enabled, \
     created_at, updated_at";

pub async fn insert_course(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    slug: &str,
    title: &str,
    description: Option<&str>,
    owner_user_id: Uuid,
) -> sqlx::Result<CourseRow> {
    sqlx::query_as::<_, CourseRow>(sqlx::AssertSqlSafe(format!(
        "INSERT INTO courses
            (tenant_id, slug, title, description, owner_user_id)
         VALUES ($1, $2, $3, $4, $5)
         RETURNING {COURSE_COLS}"
    )))
    .bind(tenant_id)
    .bind(slug)
    .bind(title)
    .bind(description)
    .bind(owner_user_id)
    .fetch_one(&mut **tx)
    .await
}

pub async fn insert_owner_membership(
    tx: &mut Transaction<'_, Postgres>,
    course_id: Uuid,
    user_id: Uuid,
    tenant_id: Uuid,
) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO course_memberships (course_id, user_id, tenant_id, role)
         VALUES ($1, $2, $3, 'teacher')",
    )
    .bind(course_id)
    .bind(user_id)
    .bind(tenant_id)
    .execute(&mut **tx)
    .await
    .map(|_| ())
}

pub async fn fetch_course<'e, E>(executor: E, id: Uuid) -> sqlx::Result<Option<CourseRow>>
where
    E: sqlx::PgExecutor<'e>,
{
    sqlx::query_as::<_, CourseRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {COURSE_COLS} FROM courses WHERE id = $1"
    )))
    .bind(id)
    .fetch_optional(executor)
    .await
}

pub async fn list_for_caller(
    pool: &PgPool,
    user_id: Uuid,
    selected_tenant_id: Option<Uuid>,
    is_org_admin: bool,
) -> sqlx::Result<Vec<CourseAccessRow>> {
    // Runs the membership lookup + main query inside a single tx so the
    // RLS GUCs we set apply to both. Required when the backend connects as
    // a non-superuser role (see migration 20260517000020).
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.user_id', $1, true)")
        .bind(user_id.to_string())
        .execute(&mut *tx)
        .await?;
    let Some(tenant_id) = selected_tenant_for_user_tx(&mut tx, user_id, selected_tenant_id).await?
    else {
        return Ok(Vec::new());
    };
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut *tx)
        .await?;

    let rows = if is_org_admin {
        sqlx::query_as::<_, CourseAccessRow>(sqlx::AssertSqlSafe(format!(
            "SELECT {COURSE_COLS}, NULL::text AS caller_course_role
               FROM courses
              WHERE tenant_id = $1
              ORDER BY created_at DESC"
        )))
        .bind(tenant_id)
        .fetch_all(&mut *tx)
        .await?
    } else {
        // Same column set, but qualified with the `c.` alias for the join.
        let cols = COURSE_COLS
            .split(", ")
            .map(|c| format!("c.{c}"))
            .collect::<Vec<_>>()
            .join(", ");
        sqlx::query_as::<_, CourseAccessRow>(sqlx::AssertSqlSafe(format!(
            "SELECT {cols},
                    COALESCE(cm.role, CASE WHEN c.owner_user_id = $1 THEN 'teacher' END)
                        AS caller_course_role
               FROM courses c
               LEFT JOIN course_memberships cm
                ON cm.course_id = c.id
                AND cm.user_id = $1
                AND cm.status = 'active'
              WHERE c.tenant_id = $2
                AND (c.owner_user_id = $1 OR cm.user_id IS NOT NULL)
              ORDER BY c.created_at DESC"
        )))
        .bind(user_id)
        .bind(tenant_id)
        .fetch_all(&mut *tx)
        .await?
    };
    Ok(rows)
}

/// Fields a course PATCH may touch. The nullable-clearing columns
/// (`cover_asset_id`, `syllabus_md`, `grading_policy_md`) use the double-Option
/// convention: `None` = leave as-is, `Some(None)` = set NULL, `Some(Some(v))` =
/// set value. We pass a "do we touch it?" boolean alongside each, so a single
/// SQL statement with `CASE WHEN $flag THEN $val ELSE col END` handles all
/// combinations without the previous match-arm blowup.
#[derive(Default)]
pub struct UpdateCourse<'a> {
    pub title: Option<&'a str>,
    pub description: Option<&'a str>,
    pub status: Option<&'a str>,
    pub cover_asset_id: Option<Option<Uuid>>,
    pub syllabus_md: Option<Option<&'a str>>,
    pub grading_policy_md: Option<Option<&'a str>>,
    pub self_enrollment_enabled: Option<bool>,
}

pub async fn update_course(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    u: UpdateCourse<'_>,
) -> sqlx::Result<Option<CourseRow>> {
    sqlx::query_as::<_, CourseRow>(sqlx::AssertSqlSafe(format!(
        "UPDATE courses
            SET title             = COALESCE($2, title),
                description       = COALESCE($3, description),
                status            = COALESCE($4, status),
                cover_asset_id    = CASE WHEN $5 THEN $6 ELSE cover_asset_id END,
                syllabus_md       = CASE WHEN $7 THEN $8 ELSE syllabus_md END,
                grading_policy_md = CASE WHEN $9 THEN $10 ELSE grading_policy_md END,
                self_enrollment_enabled = COALESCE($11, self_enrollment_enabled),
                updated_at        = now()
          WHERE id = $1
        RETURNING {COURSE_COLS}"
    )))
    .bind(id)
    .bind(u.title)
    .bind(u.description)
    .bind(u.status)
    .bind(u.cover_asset_id.is_some())
    .bind(u.cover_asset_id.flatten())
    .bind(u.syllabus_md.is_some())
    .bind(u.syllabus_md.flatten())
    .bind(u.grading_policy_md.is_some())
    .bind(u.grading_policy_md.flatten())
    .bind(u.self_enrollment_enabled)
    .fetch_optional(&mut **tx)
    .await
}

pub async fn delete_course(tx: &mut Transaction<'_, Postgres>, id: Uuid) -> sqlx::Result<bool> {
    let res = sqlx::query("DELETE FROM courses WHERE id = $1")
        .bind(id)
        .execute(&mut **tx)
        .await?;
    Ok(res.rows_affected() > 0)
}

// ---------------------------------------------------------------------------
// Catalog / self-enrollment (migration 20260823000085)
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct CatalogCourseRow {
    pub id: Uuid,
    pub slug: String,
    pub title: String,
    pub description: Option<String>,
    pub cover_asset_id: Option<Uuid>,
    pub owner_name: Option<String>,
    /// True when the caller already holds an active membership — the UI renders
    /// an "enrolled" state instead of a join button.
    pub enrolled: bool,
}

/// Published, self-enrollment-open courses in `tenant_id`, annotated with the
/// caller's membership status. Runs inside a tx that already carries the RLS
/// GUCs; only rows visible under those policies are returned.
pub async fn list_catalog(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<Vec<CatalogCourseRow>> {
    sqlx::query_as(
        "SELECT c.id, c.slug, c.title, c.description, c.cover_asset_id,
                u.display_name AS owner_name,
                (cm.status = 'active') AS enrolled
           FROM courses c
           LEFT JOIN users u ON u.id = c.owner_user_id
           LEFT JOIN course_memberships cm
                  ON cm.course_id = c.id
                 AND cm.user_id = $2
          WHERE c.tenant_id = $1
            AND c.status = 'published'
            AND c.self_enrollment_enabled = true
          ORDER BY c.title, c.id",
    )
    .bind(tenant_id)
    .bind(user_id)
    .fetch_all(&mut **tx)
    .await
}

/// Load the catalog-relevant fields for one course (self-enroll guard reads).
/// Returns None when the course does not exist in the tenant.
pub async fn catalog_course(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    course_id: Uuid,
) -> sqlx::Result<Option<(String, bool)>> {
    sqlx::query_as(
        "SELECT status, self_enrollment_enabled
           FROM courses
          WHERE id = $1
            AND tenant_id = $2",
    )
    .bind(course_id)
    .bind(tenant_id)
    .fetch_optional(&mut **tx)
    .await
}

pub async fn caller_can_admin_course(
    pool: &PgPool,
    course_id: Uuid,
    user_id: Uuid,
    selected_tenant_id: Option<Uuid>,
    is_org_admin: bool,
) -> sqlx::Result<bool> {
    // Single tx so RLS GUCs apply to both lookups.
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.user_id', $1, true)")
        .bind(user_id.to_string())
        .execute(&mut *tx)
        .await?;
    let Some(active_tenant_id) =
        selected_tenant_for_user_tx(&mut tx, user_id, selected_tenant_id).await?
    else {
        return Ok(false);
    };
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(active_tenant_id.to_string())
        .execute(&mut *tx)
        .await?;
    let course: Option<(Uuid, Uuid)> =
        sqlx::query_as("SELECT tenant_id, owner_user_id FROM courses WHERE id = $1")
            .bind(course_id)
            .fetch_optional(&mut *tx)
            .await?;
    let Some((course_tenant_id, owner_user_id)) = course else {
        return Ok(false);
    };
    if course_tenant_id != active_tenant_id {
        return Ok(false);
    }
    if is_org_admin {
        return Ok(true);
    }
    // Ownership and assignment are both subordinate to the current tenant
    // role. A demoted/suspended historical owner must immediately lose author
    // access, while an active teacher explicitly assigned to the course must
    // be able to operate it even when another teacher originally created it.
    let can_teach: bool = sqlx::query_scalar(
        "SELECT EXISTS (
            SELECT 1 FROM tenant_memberships
             WHERE tenant_id = $1
               AND user_id = $2
               AND status = 'active'
               AND role = 'teacher'
         )",
    )
    .bind(active_tenant_id)
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await?;
    if !can_teach {
        return Ok(false);
    }
    if owner_user_id == user_id {
        return Ok(true);
    }

    let assigned_teacher: bool = sqlx::query_scalar(
        "SELECT EXISTS (
            SELECT 1 FROM course_memberships
             WHERE course_id = $1
               AND tenant_id = $2
               AND user_id = $3
               AND status = 'active'
               AND role = 'teacher'
         )",
    )
    .bind(course_id)
    .bind(active_tenant_id)
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await?;
    Ok(assigned_teacher)
}

/// Broader than `caller_can_admin_course` for assistants: returns true if the
/// user is an org-admin, an active teacher owner/assignee, or an active TA
/// assigned to this course. Used for moderation and grading so a tenant-wide
/// teacher/TA cannot act on courses they are neither assigned to nor own.
pub async fn caller_can_staff_course(
    pool: &PgPool,
    course_id: Uuid,
    user_id: Uuid,
    selected_tenant_id: Option<Uuid>,
    is_org_admin: bool,
) -> sqlx::Result<bool> {
    // Single tx so RLS GUCs apply to both lookups.
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.user_id', $1, true)")
        .bind(user_id.to_string())
        .execute(&mut *tx)
        .await?;
    let Some(active_tenant_id) =
        selected_tenant_for_user_tx(&mut tx, user_id, selected_tenant_id).await?
    else {
        return Ok(false);
    };
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(active_tenant_id.to_string())
        .execute(&mut *tx)
        .await?;
    let course: Option<(Uuid, Uuid)> =
        sqlx::query_as("SELECT tenant_id, owner_user_id FROM courses WHERE id = $1")
            .bind(course_id)
            .fetch_optional(&mut *tx)
            .await?;
    let Some((course_tenant_id, owner_user_id)) = course else {
        return Ok(false);
    };
    if course_tenant_id != active_tenant_id {
        return Ok(false);
    }
    if is_org_admin {
        return Ok(true);
    }
    let tenant_role: Option<String> = sqlx::query_scalar(
        "SELECT role FROM tenant_memberships
          WHERE tenant_id = $1 AND user_id = $2 AND status = 'active'",
    )
    .bind(active_tenant_id)
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(tenant_role) = tenant_role else {
        return Ok(false);
    };

    if owner_user_id == user_id {
        return Ok(tenant_role == "teacher");
    }

    let course_role: Option<String> = sqlx::query_scalar(
        "SELECT role FROM course_memberships
          WHERE course_id = $1
            AND user_id = $2
            AND tenant_id = $3
            AND status = 'active'
            AND role IN ('teacher','ta')",
    )
    .bind(course_id)
    .bind(user_id)
    .bind(active_tenant_id)
    .fetch_optional(&mut *tx)
    .await?;

    Ok(matches!(
        (tenant_role.as_str(), course_role.as_deref()),
        ("teacher", Some("teacher" | "ta")) | ("ta", Some("ta"))
    ))
}

pub async fn caller_can_read_course(
    pool: &PgPool,
    course_id: Uuid,
    user_id: Uuid,
    selected_tenant_id: Option<Uuid>,
    is_org_admin: bool,
) -> sqlx::Result<bool> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.user_id', $1, true)")
        .bind(user_id.to_string())
        .execute(&mut *tx)
        .await?;
    let Some(active_tenant_id) =
        selected_tenant_for_user_tx(&mut tx, user_id, selected_tenant_id).await?
    else {
        return Ok(false);
    };
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(active_tenant_id.to_string())
        .execute(&mut *tx)
        .await?;

    let course: Option<(Uuid, Uuid)> =
        sqlx::query_as("SELECT tenant_id, owner_user_id FROM courses WHERE id = $1")
            .bind(course_id)
            .fetch_optional(&mut *tx)
            .await?;
    let Some((course_tenant_id, owner_user_id)) = course else {
        return Ok(false);
    };
    if course_tenant_id != active_tenant_id {
        return Ok(false);
    }
    if is_org_admin {
        return Ok(true);
    }

    if owner_user_id == user_id {
        let is_active_teacher: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                SELECT 1 FROM tenant_memberships
                 WHERE tenant_id = $1
                   AND user_id = $2
                   AND status = 'active'
                   AND role = 'teacher'
             )",
        )
        .bind(active_tenant_id)
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await?;
        if is_active_teacher {
            return Ok(true);
        }
    }

    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM course_memberships
          WHERE course_id = $1
            AND user_id = $2
            AND tenant_id = $3
            AND status = 'active'",
    )
    .bind(course_id)
    .bind(user_id)
    .bind(active_tenant_id)
    .fetch_one(&mut *tx)
    .await?;
    Ok(count > 0)
}

/// Lock and verify the caller's active tenant + course roles inside an
/// existing request-scoped transaction. Learner writes use this instead of
/// trusting a stale course row or tenant role in isolation.
pub async fn has_active_course_membership_roles(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    course_id: Uuid,
    user_id: Uuid,
    tenant_role: &str,
    course_role: &str,
) -> sqlx::Result<bool> {
    let membership: Option<i32> = sqlx::query_scalar(
        "SELECT 1
           FROM tenant_memberships tm
           JOIN course_memberships cm
             ON cm.tenant_id = tm.tenant_id
            AND cm.user_id = tm.user_id
          WHERE tm.tenant_id = $1
            AND tm.user_id = $2
            AND tm.role = $3
            AND tm.status = 'active'
            AND cm.course_id = $4
            AND cm.role = $5
            AND cm.status = 'active'
          FOR SHARE OF tm, cm",
    )
    .bind(tenant_id)
    .bind(user_id)
    .bind(tenant_role)
    .bind(course_id)
    .bind(course_role)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(membership.is_some())
}

/// Tx-bound variant. Callers running inside a transaction (with
/// `app.user_id` already set on the same connection) reuse the connection
/// so the GUC applies; this is the only variant now that all callers run
/// inside a tx for RLS coherence.
async fn selected_tenant_for_user_tx(
    conn: &mut sqlx::PgConnection,
    user_id: Uuid,
    selected_tenant_id: Option<Uuid>,
) -> sqlx::Result<Option<Uuid>> {
    let Some(selected_tenant_id) = selected_tenant_id else {
        return Ok(None);
    };

    sqlx::query_scalar(
        "SELECT tenant_id
           FROM tenant_memberships
          WHERE user_id = $1
            AND tenant_id = $2
            AND status = 'active'",
    )
    .bind(user_id)
    .bind(selected_tenant_id)
    .fetch_optional(&mut *conn)
    .await
}

/// Deep-copy a course into a brand-new DRAFT course owned by `owner_user_id`.
///
/// Copies (with fresh UUIDs + remapped FKs): the course row, its modules, its
/// lessons (including video/file-bundle asset references and lesson body), its
/// assignments (course- and lesson-attached), and its quizzes + quiz_questions.
/// RESETS everything learner-scoped: NO enrollments/memberships (only the new
/// owner is added by the caller), NO submissions, NO grades, NO quiz attempts,
/// NO lesson progress, NO certificates. The new course is always `draft` and
/// every assignment/quiz is reset to `draft` (unpublished) so the teacher can
/// re-publish deliberately.
///
/// Runs entirely inside the caller's tenant-scoped tx; the caller must have set
/// `app.tenant_id`/`app.user_id` first (RLS). Returns the new `CourseRow`.
pub async fn duplicate_course(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    source_course_id: Uuid,
    new_slug: &str,
    new_title: &str,
    owner_user_id: Uuid,
) -> sqlx::Result<CourseRow> {
    // 1) The new course row (draft, fresh slug/title, owned by the duplicator).
    //    description + syllabus + grading policy + cover carry over.
    let new_course = sqlx::query_as::<_, CourseRow>(sqlx::AssertSqlSafe(format!(
        "INSERT INTO courses
            (tenant_id, slug, title, description, status, visibility,
             cover_asset_id, owner_user_id, syllabus_md, grading_policy_md)
         SELECT $1, $2, $3, description, 'draft', visibility,
                cover_asset_id, $4, syllabus_md, grading_policy_md
           FROM courses
          WHERE id = $5
         RETURNING {COURSE_COLS}"
    )))
    .bind(tenant_id)
    .bind(new_slug)
    .bind(new_title)
    .bind(owner_user_id)
    .bind(source_course_id)
    .fetch_one(&mut **tx)
    .await?;
    let new_course_id = new_course.id;

    // 2) Modules — copy per-row so we can build a (old_module_id ->
    //    new_module_id) remap for lessons, lesson-attached assignments, and
    //    module-attached quizzes. A set-based INSERT...SELECT can't return the
    //    source id alongside the new one, so we iterate.
    let source_modules: Vec<(Uuid, String, i32)> = sqlx::query_as(
        "SELECT id, title, sort_order FROM modules WHERE course_id = $1 ORDER BY sort_order",
    )
    .bind(source_course_id)
    .fetch_all(&mut **tx)
    .await?;

    let mut module_remap: std::collections::HashMap<Uuid, Uuid> =
        std::collections::HashMap::with_capacity(source_modules.len());
    for (old_id, title, sort_order) in &source_modules {
        let new_id: Uuid = sqlx::query_scalar(
            "INSERT INTO modules (tenant_id, course_id, title, sort_order)
             VALUES ($1, $2, $3, $4) RETURNING id",
        )
        .bind(tenant_id)
        .bind(new_course_id)
        .bind(title)
        .bind(sort_order)
        .fetch_one(&mut **tx)
        .await?;
        module_remap.insert(*old_id, new_id);
    }

    // 3) Lessons — remap module_id; carry body, asset refs, sort, type, and
    //    published_at. live_session_id is intentionally DROPPED (live sessions
    //    belong to the original course's schedule, not the copy).
    #[allow(clippy::type_complexity)]
    let source_lessons: Vec<(
        Uuid,
        Uuid,
        String,
        String,
        Option<String>,
        Option<Uuid>,
        i32,
        Option<chrono::DateTime<chrono::Utc>>,
    )> = sqlx::query_as(
        "SELECT id, module_id, type, title, body_md, video_asset_id, sort_order, published_at
           FROM lessons WHERE course_id = $1 ORDER BY module_id, sort_order",
    )
    .bind(source_course_id)
    .fetch_all(&mut **tx)
    .await?;

    let mut lesson_remap: std::collections::HashMap<Uuid, Uuid> =
        std::collections::HashMap::with_capacity(source_lessons.len());
    for (old_id, old_module_id, type_, title, body_md, video_asset_id, sort_order, published_at) in
        &source_lessons
    {
        // A lesson's module must have been copied above; skip orphans defensively.
        let Some(&new_module_id) = module_remap.get(old_module_id) else {
            continue;
        };
        let new_id: Uuid = sqlx::query_scalar(
            "INSERT INTO lessons
                (tenant_id, course_id, module_id, type, title, body_md,
                 video_asset_id, sort_order, published_at)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9) RETURNING id",
        )
        .bind(tenant_id)
        .bind(new_course_id)
        .bind(new_module_id)
        .bind(type_)
        .bind(title)
        .bind(body_md)
        .bind(video_asset_id)
        .bind(sort_order)
        .bind(published_at)
        .fetch_one(&mut **tx)
        .await?;
        lesson_remap.insert(*old_id, new_id);
    }

    // 4) Assignments — remap optional lesson_id; reset status to draft
    //    (published_at NULL). Carries instructions, grading config, attachments.
    #[allow(clippy::type_complexity)]
    let source_assignments: Vec<(
        Uuid,
        Option<Uuid>,
        String,
        String,
        String,
        Option<i32>,
        bool,
        bool,
        bool,
        bool,
        String,
        i32,
        i32,
        Vec<Uuid>,
        Option<chrono::DateTime<chrono::Utc>>,
    )> = sqlx::query_as(
        "SELECT id, lesson_id, title, instructions_md, grading_mode::text, max_points,
                allow_late, lock_on_submit, accepts_text, accepts_files,
                release_mode::text, late_penalty_percent, max_resubmissions,
                attachment_asset_ids, due_at
           FROM assignments WHERE course_id = $1",
    )
    .bind(source_course_id)
    .fetch_all(&mut **tx)
    .await?;

    for (
        _old_id,
        lesson_id,
        title,
        instructions_md,
        grading_mode,
        max_points,
        allow_late,
        lock_on_submit,
        accepts_text,
        accepts_files,
        release_mode,
        late_penalty_percent,
        max_resubmissions,
        attachment_asset_ids,
        due_at,
    ) in &source_assignments
    {
        let new_lesson_id = lesson_id.and_then(|lid| lesson_remap.get(&lid).copied());
        sqlx::query(
            "INSERT INTO assignments
                (tenant_id, course_id, lesson_id, title, instructions_md,
                 grading_mode, max_points, allow_late, lock_on_submit,
                 accepts_text, accepts_files, release_mode, attachment_asset_ids,
                 due_at, status, created_by, late_penalty_percent, max_resubmissions)
             VALUES ($1,$2,$3,$4,$5,$6::assignment_grading_mode,$7,$8,$9,$10,$11,
                     $12::assignment_release_mode,$13,$14,'draft',$15,$16,$17)",
        )
        .bind(tenant_id)
        .bind(new_course_id)
        .bind(new_lesson_id)
        .bind(title)
        .bind(instructions_md)
        .bind(grading_mode)
        .bind(max_points)
        .bind(allow_late)
        .bind(lock_on_submit)
        .bind(accepts_text)
        .bind(accepts_files)
        .bind(release_mode)
        .bind(attachment_asset_ids)
        .bind(due_at)
        .bind(owner_user_id)
        .bind(late_penalty_percent)
        .bind(max_resubmissions)
        .execute(&mut **tx)
        .await?;
    }

    // 5) Quizzes — remap optional module_id; reset to draft; created_by becomes
    //    the duplicator. Then copy each quiz's questions verbatim.
    #[allow(clippy::type_complexity)]
    let source_quizzes: Vec<(
        Uuid,
        Option<Uuid>,
        String,
        Option<String>,
        String,
        Option<i32>,
        Option<i32>,
        i32,
    )> = sqlx::query_as(
        "SELECT id, module_id, title, description, mode, time_limit_seconds, max_attempts, sort_order
           FROM quizzes WHERE course_id = $1",
    )
    .bind(source_course_id)
    .fetch_all(&mut **tx)
    .await?;

    for (
        old_quiz_id,
        module_id,
        title,
        description,
        mode,
        time_limit_seconds,
        max_attempts,
        sort_order,
    ) in &source_quizzes
    {
        let new_module_id = module_id.and_then(|mid| module_remap.get(&mid).copied());
        let new_quiz_id: Uuid = sqlx::query_scalar(
            "INSERT INTO quizzes
                (tenant_id, course_id, module_id, title, description, mode,
                 time_limit_seconds, max_attempts, status, sort_order, created_by)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,'draft',$9,$10) RETURNING id",
        )
        .bind(tenant_id)
        .bind(new_course_id)
        .bind(new_module_id)
        .bind(title)
        .bind(description)
        .bind(mode)
        .bind(time_limit_seconds)
        .bind(max_attempts)
        .bind(sort_order)
        .bind(owner_user_id)
        .fetch_one(&mut **tx)
        .await?;

        // Questions copy with the answer key intact (set-based; positions and
        // payload are preserved exactly).
        sqlx::query(
            "INSERT INTO quiz_questions
                (tenant_id, quiz_id, position, prompt_text, prompt, explanation, points)
             SELECT $1, $2, position, prompt_text, prompt, explanation, points
               FROM quiz_questions WHERE quiz_id = $3",
        )
        .bind(tenant_id)
        .bind(new_quiz_id)
        .bind(old_quiz_id)
        .execute(&mut **tx)
        .await?;
    }

    Ok(new_course)
}

/// Allowed status transitions: draft → published → archived. Never backward.
pub fn is_valid_status_transition(from: &str, to: &str) -> bool {
    matches!(
        (from, to),
        ("draft", "draft")
            | ("published", "published")
            | ("archived", "archived")
            | ("draft", "published")
            | ("published", "archived")
            | ("draft", "archived")
    )
}
