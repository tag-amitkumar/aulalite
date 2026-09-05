// crates/backend/src/db/transcripts.rs
//! Student transcript data layer: raw per-course academic records for ONE user
//! across every course they are (or were) enrolled in within the active
//! tenant. Pure reads — the weighted-total math and DTO assembly live in the
//! handler so it can reuse the gradebook's unit-tested weighting function.
//!
//! All queries run inside a caller-provided transaction that already carries
//! the `app.user_id` / `app.tenant_id` GUCs (`db::begin_with_context`), so
//! every row returned has passed the FORCE ROW LEVEL SECURITY policies under
//! the non-bypass runtime role.

use sqlx::types::BigDecimal;
use uuid::Uuid;

/// One enrolled course's transcript skeleton: identity, membership, lesson
/// progress, and any issued certificate. Grades arrive separately so this
/// query stays cheap.
#[derive(Debug, sqlx::FromRow)]
pub struct TranscriptCourseRow {
    pub course_id: Uuid,
    pub slug: String,
    pub title: String,
    pub status: String,
    pub role: String,
    pub membership_status: String,
    pub joined_at: chrono::DateTime<chrono::Utc>,
    pub lessons_total: i64,
    pub lessons_completed: i64,
    /// Public credential id when a certificate was ISSUED or REVOKED for this
    /// (course, user); eligible-but-unissued rows resolve as NULL here because
    /// they carry no public credential yet.
    pub credential_id: Option<String>,
    pub certificate_status: Option<String>,
}

/// Courses the user is (or was) enrolled in within the tenant, with lesson
/// totals/completions and their issued/revoked certificate if one exists.
/// Includes `removed` memberships so a withdrawn course still shows its
/// earned record; the handler flags them.
pub async fn courses_for_user(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<Vec<TranscriptCourseRow>> {
    sqlx::query_as(
        r#"
        SELECT c.id AS course_id,
               c.slug,
               c.title,
               c.status,
               cm.role,
               cm.status AS membership_status,
               cm.joined_at,
               (SELECT count(*) FROM lessons l WHERE l.course_id = c.id) AS lessons_total,
               (SELECT count(*) FROM lesson_completions lc
                 JOIN lessons l2 ON l2.id = lc.lesson_id
                WHERE l2.course_id = c.id AND lc.user_id = $2) AS lessons_completed,
               cert.credential_id,
               cert.status AS certificate_status
          FROM course_memberships cm
          JOIN courses c ON c.id = cm.course_id
          LEFT JOIN LATERAL (
              SELECT cr.credential_id, cr.status
                FROM certificates cr
               WHERE cr.course_id = c.id
                 AND cr.user_id = $2
                 AND cr.status IN ('issued','revoked')
               ORDER BY cr.created_at DESC
               LIMIT 1
          ) cert ON true
         WHERE cm.user_id = $2
           AND cm.tenant_id = $1
         ORDER BY c.title, c.id
        "#,
    )
    .bind(tenant_id)
    .bind(user_id)
    .fetch_all(&mut **tx)
    .await
}

/// One released, numerically-graded assignment for the student — the exact
/// same population the per-course gradebook counts (published assignment,
/// non-null numeric_grade, released). `max_points` may be NULL (unscorable;
/// excluded from weighting exactly like the gradebook).
#[derive(Debug, sqlx::FromRow)]
pub struct TranscriptGradeItemRow {
    pub course_id: Uuid,
    pub numeric_grade: BigDecimal,
    pub max_points: Option<i32>,
    pub category_id: Option<Uuid>,
}

pub async fn grade_items_for_user(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<Vec<TranscriptGradeItemRow>> {
    sqlx::query_as(
        r#"
        SELECT a.course_id,
               s.numeric_grade,
               a.max_points,
               l.category_id
          FROM submissions s
          JOIN assignments a
            ON a.id = s.assignment_id
           AND a.status = 'published'
          LEFT JOIN assignment_category_links l
            ON l.assignment_id = a.id
         WHERE s.student_user_id = $2
           AND s.tenant_id = $1
           AND s.numeric_grade IS NOT NULL
           AND s.released_at IS NOT NULL
        "#,
    )
    .bind(tenant_id)
    .bind(user_id)
    .fetch_all(&mut **tx)
    .await
}

/// Category weights for the user's enrolled courses in this tenant.
#[derive(Debug, sqlx::FromRow)]
pub struct TranscriptCategoryRow {
    pub course_id: Uuid,
    pub category_id: Uuid,
    pub weight_percent: i32,
}

pub async fn category_weights_for_user(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<Vec<TranscriptCategoryRow>> {
    sqlx::query_as(
        r#"
        SELECT ac.course_id,
               ac.id AS category_id,
               ac.weight_percent
          FROM assignment_categories ac
          JOIN course_memberships cm
            ON cm.course_id = ac.course_id
           AND cm.user_id = $2
           AND cm.tenant_id = $1
         WHERE ac.tenant_id = $1
        "#,
    )
    .bind(tenant_id)
    .bind(user_id)
    .fetch_all(&mut **tx)
    .await
}
