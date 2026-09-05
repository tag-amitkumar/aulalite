// crates/backend/src/db/search.rs
//! Structured search (Postgres pg_trgm) backing the `/v1/search` endpoint.
//!
//! Visibility is the load-bearing concern here: search MUST NOT leak content
//! the caller cannot already see via the existing list endpoints. We reuse the
//! exact scoping semantics defined elsewhere in the codebase:
//!
//!  * Course visibility mirrors `db::courses::list_for_caller`:
//!    an org_admin sees every course in the active tenant; everyone else sees
//!    only courses they own OR have an ACTIVE row in `course_memberships` for.
//!  * Assignment draft/published gating mirrors the assignments list handler
//!    (`handlers::assignments::list_course_inner` / `db::assignments::list_by_course`):
//!    course staff (org_admin, owner, or active teacher/ta member) may see
//!    drafts; everyone else sees only `published` assignments. Assignments are
//!    further constrained to the set of courses the caller can read.
//!  * Lesson search (`search_lessons`) matches lesson title + body_md, scoped to
//!    the same course-read visibility as `search_courses`. Lessons carry no
//!    independent publish gate today (the lesson list handler exposes every
//!    lesson of a readable course), so course-read visibility is the only gate.
//!
//! Search uses the existing pg_trgm approach consistently across all three hit
//! types: an ILIKE substring match (term treated as data, not wildcards) OR a
//! trigram (`%`) match, ranked by `similarity()` on the title. Lessons and
//! assignments rely on the per-column GIN trigram indexes added in the
//! `lesson_assignment_search_trgm` migration so these substring/`%` predicates
//! stay index-backed.
//!
//! All queries run inside a single transaction with the RLS GUCs
//! (`app.user_id`, `app.tenant_id`) set, so tenant isolation is enforced both
//! by the explicit `tenant_id = $` predicate and by the RLS policies on
//! `courses` / `assignments`.

use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct CourseHitRow {
    pub id: Uuid,
    pub slug: String,
    pub title: String,
    pub status: String,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct AssignmentHitRow {
    pub id: Uuid,
    pub course_id: Uuid,
    pub course_slug: String,
    pub title: String,
    pub status: String,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct LessonHitRow {
    pub id: Uuid,
    pub course_id: Uuid,
    pub course_slug: String,
    pub module_id: Uuid,
    pub title: String,
    /// A short plain-ish snippet of the lesson body for display. NULL when the
    /// lesson has no body (e.g. video/live lessons).
    pub snippet: Option<String>,
}

/// Search courses the caller may read, ranked by trigram similarity on the
/// title. Matches on title OR description (ILIKE substring OR `% q` trigram).
///
/// Visibility reuses `db::courses::list_for_caller`: org_admin → all courses in
/// tenant; otherwise owner OR active course membership.
pub async fn search_courses(
    pool: &PgPool,
    tenant_id: Uuid,
    user_id: Uuid,
    is_org_admin: bool,
    q: &str,
    limit: i64,
) -> sqlx::Result<Vec<CourseHitRow>> {
    let mut tx = pool.begin().await?;
    super::set_request_guc(&mut tx, user_id, Some(tenant_id)).await?;

    // `$3` is the raw query string; we build the ILIKE pattern in SQL so the
    // literal `%`/`_` in the user's input are treated as data (escaped) rather
    // than wildcards — '%' || $3 || '%' concatenates around the verbatim term.
    let visibility = if is_org_admin {
        // org_admin: every course in the active tenant.
        "TRUE"
    } else {
        // owner OR active course membership.
        "(c.owner_user_id = $2 OR EXISTS (
             SELECT 1 FROM course_memberships cm
              WHERE cm.course_id = c.id
                AND cm.user_id = $2
                AND cm.tenant_id = $1
                AND cm.status = 'active'))"
    };

    let sql = format!(
        "SELECT c.id, c.slug, c.title, c.status
           FROM courses c
          WHERE c.tenant_id = $1
            AND {visibility}
            AND (
                  c.title ILIKE '%' || $3 || '%'
               OR c.description ILIKE '%' || $3 || '%'
               OR c.title % $3
               OR c.description % $3
            )
          ORDER BY similarity(c.title, $3) DESC, c.title ASC
          LIMIT $4"
    );

    let rows = sqlx::query_as::<_, CourseHitRow>(sqlx::AssertSqlSafe(sql.as_str()))
        .bind(tenant_id)
        .bind(user_id)
        .bind(q)
        .bind(limit)
        .fetch_all(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(rows)
}

/// Search assignments within courses the caller may read, ranked by trigram
/// similarity on the title. Draft/published gating mirrors the assignments
/// list: course staff (org_admin / owner / active teacher|ta) may see drafts;
/// everyone else only sees `published` assignments. Joins `courses` to expose
/// `course_slug` and to apply the same course visibility as `search_courses`.
pub async fn search_assignments(
    pool: &PgPool,
    tenant_id: Uuid,
    user_id: Uuid,
    is_org_admin: bool,
    q: &str,
    limit: i64,
) -> sqlx::Result<Vec<AssignmentHitRow>> {
    let mut tx = pool.begin().await?;
    super::set_request_guc(&mut tx, user_id, Some(tenant_id)).await?;

    // Course-read visibility (same expression as search_courses).
    let course_visible = if is_org_admin {
        "TRUE"
    } else {
        "(c.owner_user_id = $2 OR EXISTS (
             SELECT 1 FROM course_memberships cm
              WHERE cm.course_id = c.id
                AND cm.user_id = $2
                AND cm.tenant_id = $1
                AND cm.status = 'active'))"
    };

    // Draft gating: org_admin sees drafts everywhere; otherwise the caller must
    // be course staff (owner OR active teacher/ta member) for THAT course to see
    // its drafts. Published assignments are visible to any reader.
    let draft_ok = if is_org_admin {
        "TRUE"
    } else {
        "EXISTS (
             SELECT 1 FROM tenant_memberships tm
              WHERE tm.tenant_id = $1
                AND tm.user_id = $2
                AND tm.status = 'active'
                AND (
                    (tm.role = 'teacher' AND (
                        c.owner_user_id = $2 OR EXISTS (
                            SELECT 1 FROM course_memberships sm
                             WHERE sm.course_id = c.id
                               AND sm.user_id = $2
                               AND sm.tenant_id = $1
                               AND sm.status = 'active'
                               AND sm.role IN ('teacher','ta')
                        )
                    ))
                    OR (tm.role = 'ta' AND EXISTS (
                        SELECT 1 FROM course_memberships sm
                         WHERE sm.course_id = c.id
                           AND sm.user_id = $2
                           AND sm.tenant_id = $1
                           AND sm.status = 'active'
                           AND sm.role = 'ta'
                    ))
                )
        )"
    };

    let sql = format!(
        "SELECT a.id, a.course_id, c.slug AS course_slug,
                a.title, a.status::text AS status
           FROM assignments a
           JOIN courses c ON c.id = a.course_id
          WHERE a.tenant_id = $1
            AND c.tenant_id = $1
            AND {course_visible}
            AND (a.status = 'published' OR {draft_ok})
            AND (
                  a.title ILIKE '%' || $3 || '%'
               OR a.instructions_md ILIKE '%' || $3 || '%'
               OR a.title % $3
               OR a.instructions_md % $3
            )
          ORDER BY similarity(a.title, $3) DESC, a.title ASC
          LIMIT $4"
    );

    let rows = sqlx::query_as::<_, AssignmentHitRow>(sqlx::AssertSqlSafe(sql.as_str()))
        .bind(tenant_id)
        .bind(user_id)
        .bind(q)
        .bind(limit)
        .fetch_all(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(rows)
}

/// Search lessons within courses the caller may read, matching lesson title OR
/// body_md (ILIKE substring OR `% q` trigram), ranked by trigram similarity on
/// the title. Joins `courses` to expose `course_slug` and to apply the SAME
/// course-read visibility as `search_courses` (org_admin → all in tenant;
/// otherwise owner OR active membership). A short body snippet is returned when
/// the lesson has a body. The `course_id`/`module_id` let the caller build a
/// deep link to the lesson page.
pub async fn search_lessons(
    pool: &PgPool,
    tenant_id: Uuid,
    user_id: Uuid,
    is_org_admin: bool,
    q: &str,
    limit: i64,
) -> sqlx::Result<Vec<LessonHitRow>> {
    let mut tx = pool.begin().await?;
    super::set_request_guc(&mut tx, user_id, Some(tenant_id)).await?;

    // Course-read visibility (same expression as search_courses / search_assignments).
    let course_visible = if is_org_admin {
        "TRUE"
    } else {
        "(c.owner_user_id = $2 OR EXISTS (
             SELECT 1 FROM course_memberships cm
              WHERE cm.course_id = c.id
                AND cm.user_id = $2
                AND cm.tenant_id = $1
                AND cm.status = 'active'))"
    };

    // The snippet is a left-trimmed 160-char window of body_md; NULL bodies stay
    // NULL. Substring extraction is display-only and never widens visibility.
    let sql = format!(
        "SELECT l.id, l.course_id, c.slug AS course_slug, l.module_id, l.title,
                CASE WHEN l.body_md IS NULL THEN NULL
                     ELSE left(l.body_md, 160) END AS snippet
           FROM lessons l
           JOIN courses c ON c.id = l.course_id
          WHERE l.tenant_id = $1
            AND c.tenant_id = $1
            AND {course_visible}
            AND (
                  l.title ILIKE '%' || $3 || '%'
               OR l.body_md ILIKE '%' || $3 || '%'
               OR l.title % $3
               OR l.body_md % $3
            )
          ORDER BY similarity(l.title, $3) DESC, l.title ASC
          LIMIT $4"
    );

    let rows = sqlx::query_as::<_, LessonHitRow>(sqlx::AssertSqlSafe(sql.as_str()))
        .bind(tenant_id)
        .bind(user_id)
        .bind(q)
        .bind(limit)
        .fetch_all(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(rows)
}
