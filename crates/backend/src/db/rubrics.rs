// crates/backend/src/db/rubrics.rs
//! Rubric-based grading data layer.
//!
//! A *rubric* is a single titled grid attached 1:1 to an assignment (linked via
//! `rubrics.assignment_id` — the `assignments` table is never altered). Each
//! rubric owns an ordered set of *criteria* (`rubric_criteria`), each with a
//! `max_points` ceiling. When a teacher grades a submission with a rubric, the
//! per-criterion points they award are persisted in `submission_criterion_scores`
//! and summed into the submission's numeric grade by the handler layer.
//!
//! TENANT-SCOPED under RLS: every read/write runs inside a tx with the
//! `app.tenant_id` GUC set, mirroring `db::announcements` / `db::attendance`, so
//! the strict `tenant_isolation` policy applies under the non-bypass
//! `aulalite_app` role (`migrations/20260517000020_app_role.sql`). Uses ONLY
//! runtime sqlx (no compile-time macros — there is no DATABASE_URL at build).
use serde::Serialize;
use sqlx::types::BigDecimal;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

/// One rubric row (the grid header). One per assignment.
#[derive(Debug, Serialize, sqlx::FromRow, Clone)]
pub struct RubricRow {
    pub id: Uuid,
    pub course_id: Uuid,
    pub assignment_id: Uuid,
    pub title: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// One criterion (a row in the grid). Ordered by `sort_order` then `id`.
#[derive(Debug, Serialize, sqlx::FromRow, Clone)]
pub struct CriterionRow {
    pub id: Uuid,
    pub rubric_id: Uuid,
    pub label: String,
    pub max_points: i32,
    pub sort_order: i32,
}

/// One persisted per-criterion score for a graded submission.
#[derive(Debug, Serialize, sqlx::FromRow, Clone)]
pub struct CriterionScoreRow {
    pub submission_id: Uuid,
    pub criterion_id: Uuid,
    pub points: BigDecimal,
}

/// Set the tenant GUC the RLS policies depend on, inside `tx`.
async fn set_tenant(tx: &mut Transaction<'_, Postgres>, tenant_id: Uuid) -> sqlx::Result<()> {
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Fetch the rubric attached to `assignment_id`, if any. Tenant-scoped under RLS.
pub async fn fetch_for_assignment(
    pool: &PgPool,
    tenant_id: Uuid,
    assignment_id: Uuid,
) -> sqlx::Result<Option<RubricRow>> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    let row = sqlx::query_as::<_, RubricRow>(
        "SELECT id, course_id, assignment_id, title, created_at, updated_at
           FROM rubrics
          WHERE assignment_id = $1",
    )
    .bind(assignment_id)
    .fetch_optional(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(row)
}

/// List a rubric's criteria in display order. Tenant-scoped under RLS.
pub async fn list_criteria(
    pool: &PgPool,
    tenant_id: Uuid,
    rubric_id: Uuid,
) -> sqlx::Result<Vec<CriterionRow>> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    let rows = sqlx::query_as::<_, CriterionRow>(
        "SELECT id, rubric_id, label, max_points, sort_order
           FROM rubric_criteria
          WHERE rubric_id = $1
          ORDER BY sort_order, id",
    )
    .bind(rubric_id)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(rows)
}

/// A criterion to (re)create when replacing a rubric.
pub struct NewCriterion<'a> {
    pub label: &'a str,
    pub max_points: i32,
    pub sort_order: i32,
}

/// Create-or-replace the rubric for `assignment_id`: deletes any existing rubric
/// for the assignment (cascading to its criteria + scores) and inserts a fresh
/// rubric with the provided criteria, all in one tx. Tenant-scoped under RLS.
/// Returns the new rubric row and its persisted criteria (in order).
///
/// Replacing wipes previously recorded per-criterion scores for the assignment's
/// submissions — those criteria no longer exist. The numeric grade already saved
/// on each submission is NOT touched here (it lives on `submissions`).
pub async fn replace_for_assignment(
    pool: &PgPool,
    tenant_id: Uuid,
    course_id: Uuid,
    assignment_id: Uuid,
    title: &str,
    criteria: &[NewCriterion<'_>],
) -> sqlx::Result<(RubricRow, Vec<CriterionRow>)> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;

    // Drop any existing rubric for this assignment. ON DELETE CASCADE removes
    // its criteria and any submission_criterion_scores keyed to them.
    sqlx::query("DELETE FROM rubrics WHERE assignment_id = $1")
        .bind(assignment_id)
        .execute(&mut *tx)
        .await?;

    let rubric = sqlx::query_as::<_, RubricRow>(
        "INSERT INTO rubrics (tenant_id, course_id, assignment_id, title)
         VALUES ($1, $2, $3, $4)
         RETURNING id, course_id, assignment_id, title, created_at, updated_at",
    )
    .bind(tenant_id)
    .bind(course_id)
    .bind(assignment_id)
    .bind(title)
    .fetch_one(&mut *tx)
    .await?;

    let mut out: Vec<CriterionRow> = Vec::with_capacity(criteria.len());
    for c in criteria {
        let row = sqlx::query_as::<_, CriterionRow>(
            "INSERT INTO rubric_criteria (tenant_id, rubric_id, label, max_points, sort_order)
             VALUES ($1, $2, $3, $4, $5)
             RETURNING id, rubric_id, label, max_points, sort_order",
        )
        .bind(tenant_id)
        .bind(rubric.id)
        .bind(c.label)
        .bind(c.max_points)
        .bind(c.sort_order)
        .fetch_one(&mut *tx)
        .await?;
        out.push(row);
    }

    tx.commit().await?;
    Ok((rubric, out))
}

/// Delete the rubric attached to `assignment_id` (cascades to criteria + scores).
/// Tenant-scoped under RLS. Returns true if a rubric was removed.
pub async fn delete_for_assignment(
    pool: &PgPool,
    tenant_id: Uuid,
    assignment_id: Uuid,
) -> sqlx::Result<bool> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    let res = sqlx::query("DELETE FROM rubrics WHERE assignment_id = $1")
        .bind(assignment_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(res.rows_affected() > 0)
}

/// Within an existing grading tx, fetch the criteria belonging to a rubric.
/// Used by the grade handler to validate the supplied per-criterion scores
/// against the rubric's real criterion ids + max_points caps. The caller has
/// already set the tenant GUC on `tx`.
pub async fn criteria_for_rubric_tx(
    tx: &mut Transaction<'_, Postgres>,
    rubric_id: Uuid,
) -> sqlx::Result<Vec<CriterionRow>> {
    sqlx::query_as::<_, CriterionRow>(
        "SELECT id, rubric_id, label, max_points, sort_order
           FROM rubric_criteria
          WHERE rubric_id = $1
          ORDER BY sort_order, id",
    )
    .bind(rubric_id)
    .fetch_all(&mut **tx)
    .await
}

/// Within an existing grading tx, fetch the rubric for an assignment (if any).
/// The caller has already set the tenant GUC on `tx`.
pub async fn rubric_for_assignment_tx(
    tx: &mut Transaction<'_, Postgres>,
    assignment_id: Uuid,
) -> sqlx::Result<Option<RubricRow>> {
    sqlx::query_as::<_, RubricRow>(
        "SELECT id, course_id, assignment_id, title, created_at, updated_at
           FROM rubrics
          WHERE assignment_id = $1",
    )
    .bind(assignment_id)
    .fetch_optional(&mut **tx)
    .await
}

/// One (criterion_id, points) pair to persist for a submission.
pub struct ScoreInput {
    pub criterion_id: Uuid,
    pub points: BigDecimal,
}

/// Within an existing grading tx, replace the per-criterion scores for a
/// submission: clear the previous set, then insert the new ones. Tenant-scoped
/// via the GUC the caller already set on `tx`.
pub async fn replace_scores_tx(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    submission_id: Uuid,
    scores: &[ScoreInput],
) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM submission_criterion_scores WHERE submission_id = $1")
        .bind(submission_id)
        .execute(&mut **tx)
        .await?;
    for s in scores {
        sqlx::query(
            "INSERT INTO submission_criterion_scores
                (tenant_id, submission_id, criterion_id, points)
             VALUES ($1, $2, $3, $4)",
        )
        .bind(tenant_id)
        .bind(submission_id)
        .bind(s.criterion_id)
        .bind(&s.points)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

/// List the persisted per-criterion scores for a submission. Tenant-scoped under
/// RLS. Empty when the submission was graded without a rubric.
pub async fn list_scores_for_submission(
    pool: &PgPool,
    tenant_id: Uuid,
    submission_id: Uuid,
) -> sqlx::Result<Vec<CriterionScoreRow>> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    let rows = sqlx::query_as::<_, CriterionScoreRow>(
        "SELECT submission_id, criterion_id, points
           FROM submission_criterion_scores
          WHERE submission_id = $1",
    )
    .bind(submission_id)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(rows)
}
