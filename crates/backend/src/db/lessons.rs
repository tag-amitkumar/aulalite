// crates/backend/src/db/lessons.rs
use serde::Serialize;
use sqlx::{PgConnection, Postgres, Transaction};
use uuid::Uuid;

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct LessonRow {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub course_id: Uuid,
    pub module_id: Uuid,
    pub r#type: String,
    pub title: String,
    pub body_md: Option<String>,
    pub video_asset_id: Option<Uuid>,
    pub live_session_id: Option<Uuid>,
    pub sort_order: i32,
    /// Drip/time-gate: when non-null and in the future, the lesson is locked
    /// for students until this instant. NULL means no time-gate.
    pub release_at: Option<chrono::DateTime<chrono::Utc>>,
}

const LESSON_RETURNING: &str = "id, tenant_id, course_id, module_id, type, title, \
     body_md, video_asset_id, live_session_id, sort_order, release_at";

pub async fn next_sort_order(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    course_id: Uuid,
    module_id: Uuid,
) -> sqlx::Result<Option<i32>> {
    sqlx::query_scalar(
        "SELECT COALESCE(MAX(l.sort_order), 0)::int + 10
           FROM modules m
           LEFT JOIN lessons l
             ON l.module_id = m.id
            AND l.course_id = m.course_id
            AND l.tenant_id = m.tenant_id
          WHERE m.tenant_id = $1 AND m.course_id = $2 AND m.id = $3
          GROUP BY m.id",
    )
    .bind(tenant_id)
    .bind(course_id)
    .bind(module_id)
    .fetch_optional(&mut **tx)
    .await
}

pub async fn insert_lesson(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    course_id: Uuid,
    module_id: Uuid,
    type_: &str,
    title: &str,
    body_md: Option<&str>,
    live_session_id: Option<Uuid>,
    sort_order: i32,
) -> sqlx::Result<Option<LessonRow>> {
    sqlx::query_as::<_, LessonRow>(sqlx::AssertSqlSafe(format!(
        "INSERT INTO lessons
            (tenant_id, course_id, module_id, type, title, body_md,
             live_session_id, sort_order)
         SELECT m.tenant_id, m.course_id, m.id, $4, $5, $6, $7, $8
           FROM modules m
          WHERE m.tenant_id = $1 AND m.course_id = $2 AND m.id = $3
         RETURNING {LESSON_RETURNING}"
    )))
    .bind(tenant_id)
    .bind(course_id)
    .bind(module_id)
    .bind(type_)
    .bind(title)
    .bind(body_md)
    .bind(live_session_id)
    .bind(sort_order)
    .fetch_optional(&mut **tx)
    .await
}

pub async fn reorder(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    course_id: Uuid,
    module_id: Uuid,
    ordered: &[Uuid],
) -> sqlx::Result<bool> {
    let module_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(
             SELECT 1 FROM modules
              WHERE tenant_id = $1 AND course_id = $2 AND id = $3
         )",
    )
    .bind(tenant_id)
    .bind(course_id)
    .bind(module_id)
    .fetch_one(&mut **tx)
    .await?;
    if !module_exists {
        return Ok(false);
    }
    for (idx, lesson_id) in ordered.iter().enumerate() {
        let new_order = ((idx as i32) + 1) * 10;
        let updated = sqlx::query(
            "UPDATE lessons SET sort_order = $1, updated_at = now()
              WHERE id = $2 AND module_id = $3
                AND course_id = $4 AND tenant_id = $5",
        )
        .bind(new_order)
        .bind(lesson_id)
        .bind(module_id)
        .bind(course_id)
        .bind(tenant_id)
        .execute(&mut **tx)
        .await?
        .rows_affected();
        if updated == 0 {
            return Ok(false);
        }
    }
    Ok(true)
}

#[allow(clippy::too_many_arguments)]
pub async fn update_lesson(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    course_id: Uuid,
    module_id: Uuid,
    id: Uuid,
    title: Option<&str>,
    body_md: Option<&str>,
    video_asset_id: Option<Option<Uuid>>,
    live_session_id: Option<Option<Uuid>>,
    // Double-option drip setter: `None` = leave alone, `Some(None)` = clear the
    // gate, `Some(Some(ts))` = gate until `ts`.
    release_at: Option<Option<chrono::DateTime<chrono::Utc>>>,
) -> sqlx::Result<Option<LessonRow>> {
    // $1 = id, $2 = title, $3 = body_md are always bound; optional columns get
    // appended positionally so we never reference an unbound placeholder.
    let mut sets: Vec<String> = vec![
        "title   = COALESCE($2, title)".into(),
        "body_md = COALESCE($3, body_md)".into(),
        "updated_at = now()".into(),
    ];
    let mut next = 7;
    let video_value = match video_asset_id {
        Some(v) => {
            sets.push(format!("video_asset_id = ${next}"));
            next += 1;
            Some(v)
        }
        None => None,
    };
    let live_value = match live_session_id {
        Some(v) => {
            sets.push(format!("live_session_id = ${next}"));
            next += 1;
            Some(v)
        }
        None => None,
    };
    let release_value = match release_at {
        Some(v) => {
            sets.push(format!("release_at = ${next}"));
            Some(v)
        }
        None => None,
    };

    let sql = format!(
        "UPDATE lessons SET {}
          WHERE id = $1 AND tenant_id = $4 AND course_id = $5 AND module_id = $6
          RETURNING {LESSON_RETURNING}",
        sets.join(", ")
    );

    let mut q = sqlx::query_as::<_, LessonRow>(sqlx::AssertSqlSafe(sql.as_str()))
        .bind(id)
        .bind(title)
        .bind(body_md)
        .bind(tenant_id)
        .bind(course_id)
        .bind(module_id);
    if let Some(v) = video_value {
        q = q.bind(v);
    }
    if let Some(v) = live_value {
        q = q.bind(v);
    }
    if let Some(v) = release_value {
        q = q.bind(v);
    }
    q.fetch_optional(&mut **tx).await
}

pub async fn delete_lesson(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    course_id: Uuid,
    module_id: Uuid,
    id: Uuid,
) -> sqlx::Result<bool> {
    Ok(sqlx::query(
        "DELETE FROM lessons
          WHERE id = $4 AND tenant_id = $1 AND course_id = $2 AND module_id = $3",
    )
    .bind(tenant_id)
    .bind(course_id)
    .bind(module_id)
    .bind(id)
    .execute(&mut **tx)
    .await?
    .rows_affected()
        > 0)
}

pub fn type_supported_at_1b_alpha(t: &str) -> bool {
    matches!(t, "rich_text" | "live_session" | "video" | "file_bundle")
}

// ─── Prerequisites + drip/time-gating ────────────────────────────────────────
//
// A lesson can require other lessons (`lesson_prerequisites`) and/or carry a
// future `release_at` (drip). For a STUDENT a lesson is LOCKED when its
// `release_at` is still in the future OR any of its required lessons is not yet
// completed. Staff are never locked (gated in the handler).
//
// All callers run inside a tx with `app.tenant_id` set so the tenant_isolation
// RLS policy on `lesson_prerequisites` applies (mirrors db::announcements).

/// One required-lesson edge, joined with the required lesson's title for the
/// staff picker / lock-reason rendering.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct PrerequisiteRow {
    pub required_lesson_id: Uuid,
    pub required_lesson_title: String,
}

/// The required-lesson list for `lesson_id` (titles joined). Tenant-scoped.
pub async fn list_prerequisites(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    course_id: Uuid,
    lesson_id: Uuid,
) -> sqlx::Result<Vec<PrerequisiteRow>> {
    sqlx::query_as::<_, PrerequisiteRow>(
        "SELECT lp.required_lesson_id, l.title AS required_lesson_title
           FROM lesson_prerequisites lp
           JOIN lessons target
             ON target.id = lp.lesson_id
            AND target.tenant_id = $1 AND target.course_id = $2
           JOIN lessons l
             ON l.id = lp.required_lesson_id
            AND l.tenant_id = $1 AND l.course_id = $2
          WHERE lp.tenant_id = $1 AND lp.lesson_id = $3
          ORDER BY l.sort_order, l.title",
    )
    .bind(tenant_id)
    .bind(course_id)
    .bind(lesson_id)
    .fetch_all(conn)
    .await
}

/// Replace the full prerequisite set for `lesson_id` with `required`.
/// Self-references and the lesson's own id are filtered out. Tenant-scoped.
pub async fn set_prerequisites(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    course_id: Uuid,
    lesson_id: Uuid,
    required: &[Uuid],
) -> sqlx::Result<()> {
    sqlx::query(
        "DELETE FROM lesson_prerequisites lp
          USING lessons target
          WHERE lp.lesson_id = target.id
            AND lp.tenant_id = $1
            AND target.tenant_id = $1 AND target.course_id = $2 AND target.id = $3",
    )
    .bind(tenant_id)
    .bind(course_id)
    .bind(lesson_id)
    .execute(&mut **tx)
    .await?;
    for req in required.iter().filter(|r| **r != lesson_id) {
        sqlx::query(
            "INSERT INTO lesson_prerequisites (tenant_id, lesson_id, required_lesson_id)
             SELECT $1, target.id, required.id
               FROM lessons target
               JOIN lessons required
                 ON required.id = $4
                AND required.tenant_id = $1 AND required.course_id = $2
              WHERE target.id = $3
                AND target.tenant_id = $1 AND target.course_id = $2
             ON CONFLICT (lesson_id, required_lesson_id) DO NOTHING",
        )
        .bind(tenant_id)
        .bind(course_id)
        .bind(lesson_id)
        .bind(*req)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

/// Drop every prerequisite for `lesson_id`. Tenant-scoped.
pub async fn clear_prerequisites(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    course_id: Uuid,
    lesson_id: Uuid,
) -> sqlx::Result<bool> {
    let target_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(
             SELECT 1 FROM lessons
              WHERE tenant_id = $1 AND course_id = $2 AND id = $3
         )",
    )
    .bind(tenant_id)
    .bind(course_id)
    .bind(lesson_id)
    .fetch_one(&mut **tx)
    .await?;
    if !target_exists {
        return Ok(false);
    }
    sqlx::query(
        "DELETE FROM lesson_prerequisites
          WHERE tenant_id = $1 AND lesson_id = $3
            AND EXISTS (
                SELECT 1 FROM lessons
                 WHERE tenant_id = $1 AND course_id = $2 AND id = $3
            )",
    )
    .bind(tenant_id)
    .bind(course_id)
    .bind(lesson_id)
    .execute(&mut **tx)
    .await?;
    Ok(true)
}

/// True iff `required_lesson_id` exists in `course_id` — guards the staff
/// setter so prerequisites stay within the same course/tenant.
pub async fn lesson_exists_in_course(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    course_id: Uuid,
    lesson_id: Uuid,
) -> sqlx::Result<bool> {
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM lessons
          WHERE tenant_id = $1 AND course_id = $2 AND id = $3",
    )
    .bind(tenant_id)
    .bind(course_id)
    .bind(lesson_id)
    .fetch_one(conn)
    .await?;
    Ok(n > 0)
}

/// Per-lesson unlock state for one student in one course.
#[derive(Debug, Serialize)]
pub struct LessonLockState {
    pub lesson_id: Uuid,
    pub locked: bool,
    /// Human reason when locked (release date and/or unmet prerequisites),
    /// `None` when unlocked.
    pub reason: Option<String>,
}

/// One raw lock-input row: a lesson's release gate + how many of its
/// prerequisites remain incomplete for this user, plus a sample unmet title.
#[derive(sqlx::FromRow)]
struct LockInputRow {
    lesson_id: Uuid,
    release_at: Option<chrono::DateTime<chrono::Utc>>,
    unmet_count: i64,
    sample_unmet_title: Option<String>,
}

/// Compute the lock state for every lesson in `course_id` from the viewpoint
/// of `user_id` (a student). A lesson is locked when `release_at > now()` OR it
/// has at least one prerequisite the user has not completed. Tenant-scoped;
/// the caller has already authorized course read access.
pub async fn course_lock_state_for_user(
    conn: &mut PgConnection,
    course_id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<Vec<LessonLockState>> {
    // Join each prerequisite to the caller's completion of it (lc IS NULL =>
    // unmet). count(...) FILTER (WHERE lc.lesson_id IS NULL) is the number of
    // unmet prerequisites; min(req.title) FILTER (...) is one unmet title for
    // the human reason. Lessons with no prerequisites yield unmet_count = 0.
    let rows = sqlx::query_as::<_, LockInputRow>(
        "SELECT l.id AS lesson_id,
                l.release_at,
                count(lp.required_lesson_id) FILTER (WHERE lc.lesson_id IS NULL)
                    AS unmet_count,
                min(req.title) FILTER (WHERE lc.lesson_id IS NULL)
                    AS sample_unmet_title
           FROM lessons l
           LEFT JOIN lesson_prerequisites lp ON lp.lesson_id = l.id
           LEFT JOIN lessons req ON req.id = lp.required_lesson_id
           LEFT JOIN lesson_completions lc
                  ON lc.lesson_id = lp.required_lesson_id
                 AND lc.user_id = $2
          WHERE l.course_id = $1
          GROUP BY l.id, l.release_at",
    )
    .bind(course_id)
    .bind(user_id)
    .fetch_all(conn)
    .await?;

    let now = chrono::Utc::now();
    Ok(rows
        .into_iter()
        .map(|r| {
            let mut reasons: Vec<String> = Vec::new();
            if let Some(rel) = r.release_at {
                if rel > now {
                    reasons.push(format!("Opens {}", rel.format("%b %-d, %Y")));
                }
            }
            if r.unmet_count > 0 {
                let msg = match (r.unmet_count, r.sample_unmet_title.as_deref()) {
                    (1, Some(t)) => format!("Finish \"{t}\" first"),
                    (n, Some(t)) => format!("Finish \"{t}\" and {} more first", n - 1),
                    _ => "Finish the required lessons first".to_string(),
                };
                reasons.push(msg);
            }
            let locked = !reasons.is_empty();
            LessonLockState {
                lesson_id: r.lesson_id,
                locked,
                reason: if locked {
                    Some(reasons.join(" · "))
                } else {
                    None
                },
            }
        })
        .collect())
}

/// Single-lesson lock state for `user_id` (for a future single-lesson GET).
/// Returns `None` when the lesson is not in the course. Tenant-scoped.
#[allow(dead_code)]
pub async fn lesson_lock_state_for_user(
    conn: &mut PgConnection,
    course_id: Uuid,
    lesson_id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<Option<LessonLockState>> {
    Ok(course_lock_state_for_user(conn, course_id, user_id)
        .await?
        .into_iter()
        .find(|s| s.lesson_id == lesson_id))
}
