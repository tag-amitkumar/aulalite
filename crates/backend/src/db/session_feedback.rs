// crates/backend/src/db/session_feedback.rs
//! Post-session feedback / ratings data layer.
//!
//! One row per `(session_id, user_id)`, scoped to a tenant. Students submit a
//! 1..5 star rating with an optional comment; the write is an UPSERT keyed on
//! the `UNIQUE(session_id, user_id)` constraint so a student editing their
//! rating updates the same row. Course staff read an aggregate summary
//! (count + average + recent comments).
//!
//! TENANT-SCOPED under RLS: every read/write runs inside a tx with the
//! `app.tenant_id` GUC set, mirroring `db::announcements`, so the strict
//! `tenant_isolation` policy applies under the non-bypass `aulalite_app` role
//! (see `migrations/20260517000020_app_role.sql`). Uses ONLY runtime sqlx (no
//! compile-time macros — there is no DATABASE_URL at build).
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

/// The course a session belongs to, looked up to drive authorization. Returned
/// from `fetch_session_course` so the handler can gate on
/// `caller_can_read_course` / `caller_can_staff_course`.
#[derive(Debug, sqlx::FromRow)]
pub struct SessionCourse {
    pub course_id: Uuid,
}

/// One feedback row as the submitting student sees it back.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct FeedbackRow {
    pub id: Uuid,
    pub session_id: Uuid,
    pub user_id: Uuid,
    pub rating: i32,
    pub comment: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// A recent comment in the staff summary, joined with the rater's display
/// name/email (the rating travels with it so staff see "5★ — great class").
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct FeedbackCommentRow {
    pub rating: i32,
    pub comment: String,
    pub author_display_name: Option<String>,
    pub author_email: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Aggregate feedback for a session: how many ratings, their average, and the
/// most recent non-empty comments. Computed in the handler from the two
/// queries below.
#[derive(Debug, Serialize)]
pub struct FeedbackSummary {
    pub count: i64,
    pub average: Option<f64>,
    pub recent_comments: Vec<FeedbackCommentRow>,
}

/// Set the tenant GUC the RLS policy depends on, inside `tx`.
async fn set_tenant(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: Uuid,
) -> sqlx::Result<()> {
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Resolve the `course_id` for a live session within `tenant_id`. Tenant-scoped
/// under RLS. Returns None when the session does not exist (or is in another
/// tenant). The handler uses this to authorize against the course before
/// touching feedback.
pub async fn fetch_session_course(
    pool: &PgPool,
    tenant_id: Uuid,
    session_id: Uuid,
) -> sqlx::Result<Option<SessionCourse>> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    let row = sqlx::query_as::<_, SessionCourse>(
        "SELECT course_id FROM live_sessions WHERE id = $1 AND tenant_id = $2",
    )
    .bind(session_id)
    .bind(tenant_id)
    .fetch_optional(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(row)
}

/// Upsert one student's rating for a session and return the persisted row.
/// Keyed on `UNIQUE(session_id, user_id)`: re-submitting updates rating +
/// comment in place. Tenant-scoped under RLS.
pub async fn upsert(
    pool: &PgPool,
    tenant_id: Uuid,
    session_id: Uuid,
    user_id: Uuid,
    rating: i32,
    comment: Option<&str>,
) -> sqlx::Result<FeedbackRow> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    let row = sqlx::query_as::<_, FeedbackRow>(
        "INSERT INTO session_feedback
             (tenant_id, session_id, user_id, rating, comment)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (session_id, user_id)
         DO UPDATE SET rating = EXCLUDED.rating, comment = EXCLUDED.comment
         RETURNING id, session_id, user_id, rating, comment, created_at",
    )
    .bind(tenant_id)
    .bind(session_id)
    .bind(user_id)
    .bind(rating)
    .bind(comment)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(row)
}

/// Staff summary for a session: total count, average rating, and up to `limit`
/// most-recent non-empty comments (joined with the rater's user record).
/// Tenant-scoped under RLS. Count + average and the comment list are read in a
/// single tx so they reflect one consistent snapshot.
pub async fn summary(
    pool: &PgPool,
    tenant_id: Uuid,
    session_id: Uuid,
    limit: i64,
) -> sqlx::Result<FeedbackSummary> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;

    // AVG returns NUMERIC; cast to double precision so it decodes as f64. NULL
    // when there are no rows — surfaced as `average: None`.
    let (count, average): (i64, Option<f64>) = sqlx::query_as(
        "SELECT COUNT(*)::bigint, AVG(rating)::float8
           FROM session_feedback
          WHERE session_id = $1",
    )
    .bind(session_id)
    .fetch_one(&mut *tx)
    .await?;

    let recent_comments = sqlx::query_as::<_, FeedbackCommentRow>(
        "SELECT f.rating,
                f.comment,
                u.display_name AS author_display_name,
                u.email::text  AS author_email,
                f.created_at
           FROM session_feedback f
           LEFT JOIN users u ON u.id = f.user_id
          WHERE f.session_id = $1
            AND f.comment IS NOT NULL
            AND length(btrim(f.comment)) > 0
          ORDER BY f.created_at DESC, f.id DESC
          LIMIT $2",
    )
    .bind(session_id)
    .bind(limit)
    .fetch_all(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(FeedbackSummary {
        count,
        average,
        recent_comments,
    })
}
