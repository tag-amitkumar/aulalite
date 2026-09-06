// crates/backend/src/db/analytics.rs
//! Read-only analytics aggregations. Each public function opens its own
//! transaction and sets the `app.tenant_id` GUC so the per-table RLS policies
//! apply when the backend connects as the non-superuser `aulalite_app` role
//! (see `migrations/20260517000020_app_role.sql`). All queries are scoped to a
//! single tenant; the caller is responsible for authorization before calling.
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

/// Tenant-wide counts surfaced on the org-admin analytics overview.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct OverviewRow {
    pub courses_total: i64,
    pub courses_published: i64,
    pub members_students: i64,
    pub members_teachers: i64,
    pub members_tas: i64,
    pub members_parents: i64,
    /// live_sessions whose `starts_at` falls within the last 30 days.
    pub sessions_last_30d: i64,
    /// live_sessions with status = 'ended'.
    pub sessions_ended_total: i64,
    /// recordings with processing_status = 'available'.
    pub recordings_available: i64,
    pub assignments_total: i64,
    pub submissions_total: i64,
    /// submissions that have been graded (status = 'graded' / graded_at set).
    pub submissions_graded: i64,
}

/// Compute the tenant-wide overview. One transaction, one query per metric kept
/// small and independent so each COUNT is easy to read and verify. RLS already
/// constrains every table to the active tenant, but we also filter on
/// `tenant_id = $1` defensively.
pub async fn tenant_overview(pool: &PgPool, tenant_id: Uuid) -> sqlx::Result<OverviewRow> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut *tx)
        .await?;

    // A single query with scalar subqueries: keeps the round-trips down while
    // remaining straightforward to read (one labelled subquery per metric).
    let row = sqlx::query_as::<_, OverviewRow>(
        "SELECT
            (SELECT count(*) FROM courses
              WHERE tenant_id = $1) AS courses_total,
            (SELECT count(*) FROM courses
              WHERE tenant_id = $1 AND status = 'published') AS courses_published,
            (SELECT count(*) FROM tenant_memberships
              WHERE tenant_id = $1 AND status = 'active' AND role = 'student') AS members_students,
            (SELECT count(*) FROM tenant_memberships
              WHERE tenant_id = $1 AND status = 'active' AND role = 'teacher') AS members_teachers,
            (SELECT count(*) FROM tenant_memberships
              WHERE tenant_id = $1 AND status = 'active' AND role = 'ta') AS members_tas,
            (SELECT count(*) FROM tenant_memberships
              WHERE tenant_id = $1 AND status = 'active' AND role = 'parent') AS members_parents,
            (SELECT count(*) FROM live_sessions
              WHERE tenant_id = $1 AND starts_at >= now() - interval '30 days') AS sessions_last_30d,
            (SELECT count(*) FROM live_sessions
              WHERE tenant_id = $1 AND status = 'ended') AS sessions_ended_total,
            (SELECT count(*) FROM recordings
              WHERE tenant_id = $1 AND processing_status = 'available') AS recordings_available,
            (SELECT count(*) FROM assignments
              WHERE tenant_id = $1) AS assignments_total,
            (SELECT count(*) FROM submissions
              WHERE tenant_id = $1) AS submissions_total,
            (SELECT count(*) FROM submissions
              WHERE tenant_id = $1 AND status = 'graded') AS submissions_graded",
    )
    .bind(tenant_id)
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(row)
}

/// Course-level rollup metrics.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct CourseAnalyticsRow {
    pub course_id: Uuid,
    /// active course_memberships with role = 'student'.
    pub enrolled_students: i64,
    pub sessions_total: i64,
    pub sessions_ended: i64,
    /// distinct attendance.user_id across the course's sessions.
    pub unique_attendees: i64,
    /// AVG(attendance.total_seconds) across the course's sessions, 0 when none.
    pub avg_attendance_seconds: f64,
}

/// Per-assignment submission/grading progress for a course.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct AssignmentProgressRow {
    pub assignment_id: Uuid,
    pub title: String,
    pub status: String,
    /// submissions in a non-draft state ('submitted','returned','graded').
    pub submitted_count: i64,
    pub graded_count: i64,
    /// AVG(numeric_grade) over graded submissions, NULL when none graded.
    pub avg_numeric_grade: Option<f64>,
}

/// Compute the course-level rollup. Single tx + GUC; `course_id` is bound and
/// RLS already restricts every joined table to the active tenant.
pub async fn course_analytics(
    pool: &PgPool,
    tenant_id: Uuid,
    course_id: Uuid,
) -> sqlx::Result<CourseAnalyticsRow> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut *tx)
        .await?;

    let row = sqlx::query_as::<_, CourseAnalyticsRow>(
        "SELECT
            $1::uuid AS course_id,
            (SELECT count(*) FROM course_memberships
              WHERE course_id = $1 AND status = 'active' AND role = 'student')
                AS enrolled_students,
            (SELECT count(*) FROM live_sessions
              WHERE course_id = $1) AS sessions_total,
            (SELECT count(*) FROM live_sessions
              WHERE course_id = $1 AND status = 'ended') AS sessions_ended,
            (SELECT count(DISTINCT a.user_id)
               FROM attendance a
               JOIN live_sessions s ON s.id = a.session_id
              WHERE s.course_id = $1) AS unique_attendees,
            COALESCE(
                (SELECT AVG(a.total_seconds)::float8
                   FROM attendance a
                   JOIN live_sessions s ON s.id = a.session_id
                  WHERE s.course_id = $1),
                0::float8
            ) AS avg_attendance_seconds",
    )
    .bind(course_id)
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(row)
}

/// One day of tenant-wide activity for the overview charts.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct DailyActivityRow {
    pub day: chrono::NaiveDate,
    pub sessions: i64,
    pub attendance_joins: i64,
    pub submissions: i64,
    pub lessons_completed: i64,
}

/// Daily activity counts for the last `days` days (inclusive of today), one
/// row per day, zero-filled via generate_series. Each metric is a small
/// correlated subquery — at 30-90 rows this stays trivially cheap.
pub async fn daily_activity(
    pool: &PgPool,
    tenant_id: Uuid,
    days: i32,
) -> sqlx::Result<Vec<DailyActivityRow>> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut *tx)
        .await?;

    let rows = sqlx::query_as::<_, DailyActivityRow>(
        // Half-open ranges over the STORED timestamp, not `<col>::date = d::date`.
        //
        // The cast form is not sargable: it applies a function to the indexed
        // column, so every day scanned every tenant row of all four tables and
        // cost grew as days x tenant_rows x 4. It also cannot be fixed with a
        // matching expression index -- these columns are timestamptz and
        // `timestamptz -> date` is STABLE, which Postgres refuses in an index
        // expression ("functions in index expression must be marked IMMUTABLE").
        //
        // `<col> >= D AND <col> < D+1` selects exactly the same rows: the cast
        // equality holds precisely when the value lies in [midnight D, midnight
        // D+1), and `d::date::timestamptz` resolves that midnight in the same
        // session time zone the cast would have used, so the day bucketing is
        // unchanged. Being a range over the raw column, it seeks the
        // (tenant_id, <col>) indexes from
        // 20260906000087_analytics_daily_activity_indexes.
        "SELECT d::date AS day,
            (SELECT count(*) FROM live_sessions s
              WHERE s.tenant_id = $1
                AND s.starts_at >= d::date::timestamptz
                AND s.starts_at <  (d::date + 1)::timestamptz) AS sessions,
            (SELECT count(*) FROM attendance a
              WHERE a.tenant_id = $1
                AND a.first_joined_at >= d::date::timestamptz
                AND a.first_joined_at <  (d::date + 1)::timestamptz)
                AS attendance_joins,
            (SELECT count(*) FROM submissions sub
              WHERE sub.tenant_id = $1
                AND sub.submitted_at >= d::date::timestamptz
                AND sub.submitted_at <  (d::date + 1)::timestamptz)
                AS submissions,
            (SELECT count(*) FROM lesson_completions lc
              WHERE lc.tenant_id = $1
                AND lc.completed_at >= d::date::timestamptz
                AND lc.completed_at <  (d::date + 1)::timestamptz)
                AS lessons_completed
         FROM generate_series(
            CURRENT_DATE - ($2::int - 1) * interval '1 day',
            CURRENT_DATE,
            interval '1 day') d
         ORDER BY day ASC",
    )
    .bind(tenant_id)
    .bind(days)
    .fetch_all(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(rows)
}

/// Lesson-progress funnel for a course: how many enrolled students have
/// started (≥1 lesson), reached half, and completed all lessons.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct ProgressFunnelRow {
    pub enrolled: i64,
    pub started: i64,
    pub half: i64,
    pub completed: i64,
}

pub async fn progress_funnel(
    pool: &PgPool,
    tenant_id: Uuid,
    course_id: Uuid,
) -> sqlx::Result<ProgressFunnelRow> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut *tx)
        .await?;

    let row = sqlx::query_as::<_, ProgressFunnelRow>(
        "WITH totals AS (
            SELECT count(*) AS total_lessons FROM lessons WHERE course_id = $1
         ),
         per_student AS (
            SELECT cm.user_id, count(lc.lesson_id) AS done
              FROM course_memberships cm
              LEFT JOIN lesson_completions lc
                ON lc.user_id = cm.user_id AND lc.course_id = cm.course_id
             WHERE cm.course_id = $1 AND cm.role = 'student' AND cm.status = 'active'
             GROUP BY cm.user_id
         )
         SELECT count(*) AS enrolled,
                count(*) FILTER (WHERE done > 0) AS started,
                count(*) FILTER (
                    WHERE total_lessons > 0 AND done * 2 >= total_lessons) AS half,
                count(*) FILTER (
                    WHERE total_lessons > 0 AND done >= total_lessons) AS completed
           FROM per_student, totals",
    )
    .bind(course_id)
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(row)
}

/// Graded-quiz score distribution for a course, bucketed by percentage.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct QuizScoreDistributionRow {
    /// Submitted graded attempts scoring under 50%.
    pub under_50: i64,
    pub from_50_to_69: i64,
    pub from_70_to_89: i64,
    pub from_90_up: i64,
}

pub async fn quiz_score_distribution(
    pool: &PgPool,
    tenant_id: Uuid,
    course_id: Uuid,
) -> sqlx::Result<QuizScoreDistributionRow> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut *tx)
        .await?;

    let row = sqlx::query_as::<_, QuizScoreDistributionRow>(
        "SELECT
            count(*) FILTER (WHERE pct < 0.5) AS under_50,
            count(*) FILTER (WHERE pct >= 0.5 AND pct < 0.7) AS from_50_to_69,
            count(*) FILTER (WHERE pct >= 0.7 AND pct < 0.9) AS from_70_to_89,
            count(*) FILTER (WHERE pct >= 0.9) AS from_90_up
         FROM (
            SELECT qa.score_points::float8 / NULLIF(qa.max_points, 0) AS pct
              FROM quiz_attempts qa
              JOIN quizzes q ON q.id = qa.quiz_id
             WHERE q.course_id = $1 AND q.mode = 'graded'
               AND qa.submitted_at IS NOT NULL
               AND qa.score_points IS NOT NULL AND qa.max_points > 0
         ) scored",
    )
    .bind(course_id)
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(row)
}

/// Per-assignment progress rows for a course. LEFT JOIN so assignments with
/// zero submissions still appear (with zero counts / NULL average).
pub async fn assignment_progress(
    pool: &PgPool,
    tenant_id: Uuid,
    course_id: Uuid,
) -> sqlx::Result<Vec<AssignmentProgressRow>> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut *tx)
        .await?;

    let rows = sqlx::query_as::<_, AssignmentProgressRow>(
        "SELECT
            a.id AS assignment_id,
            a.title,
            a.status::text AS status,
            count(*) FILTER (
                WHERE sub.status IN ('submitted','returned','graded')
            ) AS submitted_count,
            count(*) FILTER (WHERE sub.status = 'graded') AS graded_count,
            AVG(sub.numeric_grade) FILTER (
                WHERE sub.status = 'graded' AND sub.numeric_grade IS NOT NULL
            )::float8 AS avg_numeric_grade
           FROM assignments a
           LEFT JOIN submissions sub ON sub.assignment_id = a.id
          WHERE a.course_id = $1
          GROUP BY a.id, a.title, a.status, a.created_at
          ORDER BY a.created_at ASC",
    )
    .bind(course_id)
    .fetch_all(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(rows)
}

// ---------------------------------------------------------------------------
// Grade analytics (additive): per-assignment grade distribution, the class
// average trend across recently-graded assignments, and an at-risk student
// list. All percentage-based so numeric assignments with different `max_points`
// are comparable. Only `grading_mode = 'numeric'` assignments with a positive
// `max_points` contribute (pass/fail carries no numeric grade).
// ---------------------------------------------------------------------------

/// Per-assignment grade distribution over graded numeric submissions.
/// Percentage = `numeric_grade / max_points`. Buckets mirror the quiz
/// distribution bands; `median_pct` / `stddev_pct` summarise the spread. Only
/// assignments that have at least one graded numeric submission appear.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct GradeDistributionRow {
    pub assignment_id: Uuid,
    pub title: String,
    pub graded_count: i64,
    pub under_50: i64,
    pub from_50_to_69: i64,
    pub from_70_to_89: i64,
    pub from_90_up: i64,
    /// Median percentage (0.0–1.0+) over graded submissions. Always present
    /// here (the inner join guarantees ≥1 graded row per returned assignment);
    /// modelled as Option for forward-compatibility / a safe NULL decode.
    pub median_pct: Option<f64>,
    /// Population stddev of the percentage (0.0 for a single graded submission).
    pub stddev_pct: Option<f64>,
}

pub async fn assignment_grade_distribution(
    pool: &PgPool,
    tenant_id: Uuid,
    course_id: Uuid,
) -> sqlx::Result<Vec<GradeDistributionRow>> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut *tx)
        .await?;

    let rows = sqlx::query_as::<_, GradeDistributionRow>(
        "SELECT
            a.id AS assignment_id,
            a.title,
            count(*) AS graded_count,
            count(*) FILTER (WHERE pct < 0.5) AS under_50,
            count(*) FILTER (WHERE pct >= 0.5 AND pct < 0.7) AS from_50_to_69,
            count(*) FILTER (WHERE pct >= 0.7 AND pct < 0.9) AS from_70_to_89,
            count(*) FILTER (WHERE pct >= 0.9) AS from_90_up,
            percentile_cont(0.5) WITHIN GROUP (ORDER BY pct)::float8 AS median_pct,
            stddev_pop(pct)::float8 AS stddev_pct
           FROM (
                SELECT a.id, a.title, a.created_at,
                       sub.numeric_grade::float8 / NULLIF(a.max_points, 0) AS pct
                  FROM assignments a
                  JOIN submissions sub ON sub.assignment_id = a.id
                 WHERE a.course_id = $1
                   AND a.grading_mode = 'numeric'
                   AND a.max_points IS NOT NULL AND a.max_points > 0
                   AND sub.status = 'graded'
                   AND sub.numeric_grade IS NOT NULL
           ) a
          GROUP BY a.id, a.title, a.created_at
          ORDER BY a.created_at ASC",
    )
    .bind(course_id)
    .fetch_all(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(rows)
}

/// One point on the class-average trend: a graded numeric assignment and the
/// mean percentage scored across its graded submissions. Ordered oldest→newest
/// so the caller can plot it directly.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct ClassAverageTrendRow {
    pub assignment_id: Uuid,
    pub title: String,
    pub graded_count: i64,
    /// Mean percentage (0.0–1.0+) over graded submissions for this assignment.
    pub avg_pct: f64,
}

/// Class-average trend across the most recent `last_n` graded numeric
/// assignments (by `created_at`), returned oldest→newest. `last_n` is clamped
/// by the caller; we still bind it so the SQL stays parameterised.
pub async fn class_average_trend(
    pool: &PgPool,
    tenant_id: Uuid,
    course_id: Uuid,
    last_n: i64,
) -> sqlx::Result<Vec<ClassAverageTrendRow>> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut *tx)
        .await?;

    let rows = sqlx::query_as::<_, ClassAverageTrendRow>(
        "SELECT assignment_id, title, graded_count, avg_pct
           FROM (
                SELECT a.id AS assignment_id,
                       a.title,
                       a.created_at,
                       count(*) AS graded_count,
                       AVG(sub.numeric_grade::float8 / NULLIF(a.max_points, 0))::float8 AS avg_pct
                  FROM assignments a
                  JOIN submissions sub ON sub.assignment_id = a.id
                 WHERE a.course_id = $1
                   AND a.grading_mode = 'numeric'
                   AND a.max_points IS NOT NULL AND a.max_points > 0
                   AND sub.status = 'graded'
                   AND sub.numeric_grade IS NOT NULL
                 GROUP BY a.id, a.title, a.created_at
                 ORDER BY a.created_at DESC
                 LIMIT $2
           ) recent
          ORDER BY created_at ASC",
    )
    .bind(course_id)
    .bind(last_n)
    .fetch_all(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(rows)
}

/// A student whose mean percentage across graded numeric work is below the
/// at-risk threshold. `display_name` falls back to the email when unset.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct AtRiskStudentRow {
    pub student_user_id: Uuid,
    pub display_name: String,
    /// Mean percentage (0.0–1.0+) across the student's graded numeric work.
    pub avg_pct: f64,
    /// Number of graded numeric submissions backing the average.
    pub graded_count: i64,
}

/// Active students enrolled in the course whose average percentage across
/// graded numeric work is strictly below `threshold` (e.g. 0.60). Requires at
/// least one graded numeric submission — students with no graded work are not
/// flagged (no signal yet). Worst average first.
pub async fn at_risk_students(
    pool: &PgPool,
    tenant_id: Uuid,
    course_id: Uuid,
    threshold: f64,
) -> sqlx::Result<Vec<AtRiskStudentRow>> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut *tx)
        .await?;

    let rows = sqlx::query_as::<_, AtRiskStudentRow>(
        "SELECT
            cm.user_id AS student_user_id,
            COALESCE(NULLIF(u.display_name, ''), u.email::text) AS display_name,
            AVG(sub.numeric_grade::float8 / NULLIF(a.max_points, 0))::float8 AS avg_pct,
            count(*) AS graded_count
           FROM course_memberships cm
           JOIN users u ON u.id = cm.user_id
           JOIN submissions sub ON sub.student_user_id = cm.user_id
                               AND sub.course_id = cm.course_id
           JOIN assignments a ON a.id = sub.assignment_id
          WHERE cm.course_id = $1
            AND cm.role = 'student' AND cm.status = 'active'
            AND a.grading_mode = 'numeric'
            AND a.max_points IS NOT NULL AND a.max_points > 0
            AND sub.status = 'graded'
            AND sub.numeric_grade IS NOT NULL
          GROUP BY cm.user_id, u.display_name, u.email
         HAVING AVG(sub.numeric_grade::float8 / NULLIF(a.max_points, 0)) < $2
          ORDER BY avg_pct ASC, display_name ASC",
    )
    .bind(course_id)
    .bind(threshold)
    .fetch_all(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(rows)
}
