//! Lesson completion queries (learning-suite Cycle 2). All callers run
//! inside a `begin_with_context` transaction so the RLS GUCs apply.

use sqlx::{PgConnection, Postgres, Transaction};
use uuid::Uuid;

/// True when the lesson exists and belongs to the course (path sanity check
/// before writing a completion).
pub async fn lesson_in_course(
    conn: &mut PgConnection,
    course_id: Uuid,
    lesson_id: Uuid,
) -> sqlx::Result<bool> {
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM lessons WHERE id = $1 AND course_id = $2")
            .bind(lesson_id)
            .bind(course_id)
            .fetch_one(conn)
            .await?;
    Ok(count > 0)
}

/// Idempotent mark-complete. Returns `true` when a new completion was
/// recorded, `false` when it already existed.
pub async fn mark_complete(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    course_id: Uuid,
    lesson_id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<bool> {
    let res = sqlx::query(
        "INSERT INTO lesson_completions (tenant_id, course_id, lesson_id, user_id) \
         VALUES ($1, $2, $3, $4) ON CONFLICT (lesson_id, user_id) DO NOTHING",
    )
    .bind(tenant_id)
    .bind(course_id)
    .bind(lesson_id)
    .bind(user_id)
    .execute(&mut **tx)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// Undo a completion. Returns `true` when a row was removed.
pub async fn unmark_complete(
    tx: &mut Transaction<'_, Postgres>,
    lesson_id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<bool> {
    let res = sqlx::query("DELETE FROM lesson_completions WHERE lesson_id = $1 AND user_id = $2")
        .bind(lesson_id)
        .bind(user_id)
        .execute(&mut **tx)
        .await?;
    Ok(res.rows_affected() > 0)
}

/// Completed lesson ids for one user in one course.
pub async fn completed_lesson_ids(
    conn: &mut PgConnection,
    course_id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<Vec<Uuid>> {
    sqlx::query_scalar(
        "SELECT lesson_id FROM lesson_completions WHERE course_id = $1 AND user_id = $2",
    )
    .bind(course_id)
    .bind(user_id)
    .fetch_all(conn)
    .await
}

/// Total lesson count for a course.
pub async fn lesson_count(conn: &mut PgConnection, course_id: Uuid) -> sqlx::Result<i64> {
    sqlx::query_scalar("SELECT count(*) FROM lessons WHERE course_id = $1")
        .bind(course_id)
        .fetch_one(conn)
        .await
}

/// The user's resume point: the first lesson (module order, then lesson
/// order) without a completion. `None` when everything is complete or the
/// course has no lessons.
pub async fn first_incomplete_lesson(
    conn: &mut PgConnection,
    course_id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<Option<(Uuid, String)>> {
    sqlx::query_as(
        "SELECT l.id, l.title FROM lessons l \
         JOIN modules m ON m.id = l.module_id \
         WHERE l.course_id = $1 AND NOT EXISTS ( \
             SELECT 1 FROM lesson_completions lc \
             WHERE lc.lesson_id = l.id AND lc.user_id = $2) \
         ORDER BY m.sort_order, l.sort_order LIMIT 1",
    )
    .bind(course_id)
    .bind(user_id)
    .fetch_optional(conn)
    .await
}

/// Per-course progress rows for every published course the user is an
/// active student of: (course_id, slug, title, total, completed,
/// last_activity).
pub async fn my_course_progress(
    conn: &mut PgConnection,
    user_id: Uuid,
    tenant_id: Uuid,
) -> sqlx::Result<
    Vec<(
        Uuid,
        String,
        String,
        i64,
        i64,
        Option<chrono::DateTime<chrono::Utc>>,
    )>,
> {
    sqlx::query_as(
        "SELECT c.id, c.slug, c.title, \
                count(l.id) AS total, \
                count(lc.lesson_id) AS completed, \
                max(lc.completed_at) AS last_activity \
         FROM course_memberships cm \
         JOIN courses c ON c.id = cm.course_id \
         LEFT JOIN lessons l ON l.course_id = c.id \
         LEFT JOIN lesson_completions lc \
             ON lc.lesson_id = l.id AND lc.user_id = cm.user_id \
         WHERE cm.user_id = $1 AND cm.tenant_id = $2 \
           AND cm.status = 'active' AND cm.role = 'student' \
           AND c.status = 'published' \
         GROUP BY c.id, c.slug, c.title \
         ORDER BY max(lc.completed_at) DESC NULLS LAST, c.title",
    )
    .bind(user_id)
    .bind(tenant_id)
    .fetch_all(conn)
    .await
}

/// Teacher view: per-student completion counts for a course:
/// (user_id, display_name, email, completed).
pub async fn course_student_progress(
    conn: &mut PgConnection,
    course_id: Uuid,
) -> sqlx::Result<Vec<(Uuid, Option<String>, String, i64)>> {
    sqlx::query_as(
        "SELECT cm.user_id, u.display_name, u.email::text, count(lc.lesson_id) AS completed \
         FROM course_memberships cm \
         JOIN users u ON u.id = cm.user_id \
         LEFT JOIN lesson_completions lc \
             ON lc.user_id = cm.user_id AND lc.course_id = cm.course_id \
         WHERE cm.course_id = $1 AND cm.role = 'student' AND cm.status = 'active' \
         GROUP BY cm.user_id, u.display_name, u.email \
         ORDER BY u.display_name NULLS LAST, u.email",
    )
    .bind(course_id)
    .fetch_all(conn)
    .await
}
