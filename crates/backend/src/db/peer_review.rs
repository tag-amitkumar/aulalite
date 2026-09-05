// crates/backend/src/db/peer_review.rs
//! Assignment peer-review data layer.
//!
//! Three tables, all scoped to a `(tenant_id)` and linked to an assignment:
//!   * `peer_review_configs`     — one row per assignment (UNIQUE assignment_id):
//!                                  reviews_per_student, optional rubric_id reuse,
//!                                  anonymous flag, optional due_at.
//!   * `peer_review_allocations` — one row per (assignment, reviewer, submission):
//!                                  the work a given reviewer is assigned to review.
//!   * `peer_reviews`            — one row per allocation (UNIQUE allocation_id):
//!                                  the submitted scores_json + comment_md.
//!
//! Staff configure + allocate (handler gates on
//! `db::courses::caller_can_staff_course`); students see their own allocations
//! and submit reviews; authors see received reviews; staff see the aggregate.
//!
//! TENANT-SCOPED under RLS: every read/write runs inside a tx with the
//! `app.tenant_id` GUC set, mirroring `db::announcements` / `db::rubrics`, so the
//! strict `tenant_isolation` policy applies under the non-bypass `aulalite_app`
//! role (see `migrations/20260517000020_app_role.sql`). Uses ONLY runtime sqlx
//! (no compile-time macros — there is no DATABASE_URL at build).
use serde::Serialize;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

/// Allocation lifecycle. `pending` = assigned, not yet reviewed; `submitted` =
/// the reviewer has filed their review. Stored as TEXT (no custom enum) to keep
/// the migration self-contained.
pub const STATUS_PENDING: &str = "pending";
pub const STATUS_SUBMITTED: &str = "submitted";

/// The peer-review configuration attached 1:1 to an assignment.
#[derive(Debug, Serialize, sqlx::FromRow, Clone)]
pub struct PeerReviewConfigRow {
    pub assignment_id: Uuid,
    pub course_id: Uuid,
    pub reviews_per_student: i32,
    pub rubric_id: Option<Uuid>,
    pub anonymous: bool,
    pub due_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

const CONFIG_COLS: &str = "assignment_id, course_id, reviews_per_student, rubric_id, \
     anonymous, due_at, created_at, updated_at";

/// One allocation joined with the reviewed submission's owner + the reviewer's
/// label. `review_*` columns are populated when the reviewer has submitted.
#[derive(Debug, Serialize, sqlx::FromRow, Clone)]
pub struct AllocationRow {
    pub id: Uuid,
    pub assignment_id: Uuid,
    pub reviewer_user_id: Uuid,
    pub submission_id: Uuid,
    pub status: String,
    /// The author of the submission being reviewed (the reviewee).
    pub author_user_id: Uuid,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub submitted_at: Option<chrono::DateTime<chrono::Utc>>,
    /// The persisted scores as a JSON string. JSONB in the DB; read as `::text`
    /// so we don't need the sqlx `json` decode feature. The handler parses it.
    pub scores_json: Option<String>,
    pub comment_md: Option<String>,
}

const ALLOCATION_COLS: &str = "a.id,
        a.assignment_id,
        a.reviewer_user_id,
        a.submission_id,
        a.status,
        s.student_user_id AS author_user_id,
        a.created_at,
        r.submitted_at,
        r.scores_json::text AS scores_json,
        r.comment_md";

const ALLOCATION_JOINS: &str = "peer_review_allocations a
        JOIN submissions s ON s.id = a.submission_id
        LEFT JOIN peer_reviews r ON r.allocation_id = a.id";

/// Set the tenant GUC the RLS policies depend on, inside `tx`.
async fn set_tenant(tx: &mut Transaction<'_, Postgres>, tenant_id: Uuid) -> sqlx::Result<()> {
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut **tx)
        .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

/// Create-or-replace the peer-review config for `assignment_id`. UPSERT on the
/// UNIQUE(assignment_id) key. Tenant-scoped under RLS.
pub async fn upsert_config(
    pool: &PgPool,
    tenant_id: Uuid,
    course_id: Uuid,
    assignment_id: Uuid,
    reviews_per_student: i32,
    rubric_id: Option<Uuid>,
    anonymous: bool,
    due_at: Option<chrono::DateTime<chrono::Utc>>,
) -> sqlx::Result<PeerReviewConfigRow> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    let row = sqlx::query_as::<_, PeerReviewConfigRow>(sqlx::AssertSqlSafe(format!(
        "INSERT INTO peer_review_configs
             (tenant_id, course_id, assignment_id, reviews_per_student, rubric_id, anonymous, due_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         ON CONFLICT (assignment_id) DO UPDATE SET
             reviews_per_student = EXCLUDED.reviews_per_student,
             rubric_id = EXCLUDED.rubric_id,
             anonymous = EXCLUDED.anonymous,
             due_at = EXCLUDED.due_at,
             updated_at = now()
         RETURNING {CONFIG_COLS}"
    )))
    .bind(tenant_id)
    .bind(course_id)
    .bind(assignment_id)
    .bind(reviews_per_student)
    .bind(rubric_id)
    .bind(anonymous)
    .bind(due_at)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(row)
}

/// Fetch the config for an assignment, if any. Tenant-scoped under RLS.
pub async fn fetch_config(
    pool: &PgPool,
    tenant_id: Uuid,
    assignment_id: Uuid,
) -> sqlx::Result<Option<PeerReviewConfigRow>> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    let row = sqlx::query_as::<_, PeerReviewConfigRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {CONFIG_COLS} FROM peer_review_configs WHERE assignment_id = $1"
    )))
    .bind(assignment_id)
    .fetch_optional(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(row)
}

// ---------------------------------------------------------------------------
// Allocation
// ---------------------------------------------------------------------------

/// A submitted submission eligible for peer review: its id + the student author.
#[derive(Debug, Clone)]
pub struct EligibleSubmission {
    pub submission_id: Uuid,
    pub author_user_id: Uuid,
}

/// List the SUBMITTED (or graded/returned, i.e. actually turned-in) submissions
/// for an assignment alongside their authors — the universe the round-robin
/// allocator draws from. Drafts are excluded (nothing to review). Tenant-scoped
/// under RLS. Returned in a stable order so allocation is deterministic.
pub async fn list_eligible_submissions(
    pool: &PgPool,
    tenant_id: Uuid,
    assignment_id: Uuid,
) -> sqlx::Result<Vec<EligibleSubmission>> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    let rows: Vec<(Uuid, Uuid)> = sqlx::query_as(
        "SELECT id, student_user_id
           FROM submissions
          WHERE assignment_id = $1
            AND status IN ('submitted','graded','returned')
          ORDER BY student_user_id, id",
    )
    .bind(assignment_id)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(rows
        .into_iter()
        .map(|(submission_id, author_user_id)| EligibleSubmission {
            submission_id,
            author_user_id,
        })
        .collect())
}

/// One (reviewer, submission) pair the allocator decided on.
pub struct NewAllocation {
    pub reviewer_user_id: Uuid,
    pub submission_id: Uuid,
}

/// Replace the full allocation set for an assignment: delete all existing
/// allocations (cascading to any filed reviews) and insert the new ones, in a
/// single tx. Tenant-scoped under RLS. Returns the number of allocations
/// inserted. Re-running is idempotent in effect (always rebuilds from scratch).
pub async fn replace_allocations(
    pool: &PgPool,
    tenant_id: Uuid,
    course_id: Uuid,
    assignment_id: Uuid,
    allocations: &[NewAllocation],
) -> sqlx::Result<i64> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;

    sqlx::query("DELETE FROM peer_review_allocations WHERE assignment_id = $1")
        .bind(assignment_id)
        .execute(&mut *tx)
        .await?;

    for a in allocations {
        sqlx::query(
            "INSERT INTO peer_review_allocations
                 (tenant_id, course_id, assignment_id, reviewer_user_id, submission_id, status)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(tenant_id)
        .bind(course_id)
        .bind(assignment_id)
        .bind(a.reviewer_user_id)
        .bind(a.submission_id)
        .bind(STATUS_PENDING)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(allocations.len() as i64)
}

/// The reviewer queue: every allocation assigned to `reviewer_user_id` for an
/// assignment (with the filed review, if any). Tenant-scoped under RLS.
pub async fn list_for_reviewer(
    pool: &PgPool,
    tenant_id: Uuid,
    assignment_id: Uuid,
    reviewer_user_id: Uuid,
) -> sqlx::Result<Vec<AllocationRow>> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    let rows = sqlx::query_as::<_, AllocationRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {ALLOCATION_COLS}
           FROM {ALLOCATION_JOINS}
          WHERE a.assignment_id = $1 AND a.reviewer_user_id = $2
          ORDER BY a.created_at, a.id"
    )))
    .bind(assignment_id)
    .bind(reviewer_user_id)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(rows)
}

/// Fetch a single allocation by id (joined with reviewee + any filed review).
/// Used to authorize a review submission and resolve its course. Tenant-scoped
/// under RLS. Returns None when it does not exist in this tenant.
pub async fn fetch_allocation(
    pool: &PgPool,
    tenant_id: Uuid,
    id: Uuid,
) -> sqlx::Result<Option<AllocationRow>> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    let row = sqlx::query_as::<_, AllocationRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {ALLOCATION_COLS}
           FROM {ALLOCATION_JOINS}
          WHERE a.id = $1"
    )))
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(row)
}

/// Resolve the course id for an allocation (cheap auth helper). None when the
/// allocation does not exist in this tenant.
pub async fn course_for_allocation(
    pool: &PgPool,
    tenant_id: Uuid,
    id: Uuid,
) -> sqlx::Result<Option<Uuid>> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    let course_id: Option<Uuid> =
        sqlx::query_scalar("SELECT course_id FROM peer_review_allocations WHERE id = $1")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
    tx.commit().await?;
    Ok(course_id)
}

// ---------------------------------------------------------------------------
// Reviews
// ---------------------------------------------------------------------------

/// Upsert the review for an allocation (UNIQUE allocation_id) and flip the
/// allocation's status to `submitted`, in one tx. A reviewer may overwrite their
/// own review until... there is no lock here; the handler enforces the due_at /
/// ownership rules. Tenant-scoped under RLS. Returns the refreshed allocation.
pub async fn upsert_review(
    pool: &PgPool,
    tenant_id: Uuid,
    allocation_id: Uuid,
    // JSON serialized as a string; bound to the JSONB column via `$3::jsonb`.
    scores_json: &str,
    comment_md: &str,
) -> sqlx::Result<AllocationRow> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;

    sqlx::query(
        "INSERT INTO peer_reviews
             (tenant_id, allocation_id, scores_json, comment_md, submitted_at)
         VALUES ($1, $2, $3::jsonb, $4, now())
         ON CONFLICT (allocation_id) DO UPDATE SET
             scores_json = EXCLUDED.scores_json,
             comment_md = EXCLUDED.comment_md,
             submitted_at = now()",
    )
    .bind(tenant_id)
    .bind(allocation_id)
    .bind(scores_json)
    .bind(comment_md)
    .execute(&mut *tx)
    .await?;

    sqlx::query("UPDATE peer_review_allocations SET status = $2 WHERE id = $1")
        .bind(allocation_id)
        .bind(STATUS_SUBMITTED)
        .execute(&mut *tx)
        .await?;

    let row = sqlx::query_as::<_, AllocationRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {ALLOCATION_COLS}
           FROM {ALLOCATION_JOINS}
          WHERE a.id = $1"
    )))
    .bind(allocation_id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(row)
}

/// The reviews RECEIVED by `author_user_id` for an assignment: every SUBMITTED
/// allocation whose reviewed submission belongs to the author. Tenant-scoped
/// under RLS. The handler optionally strips `reviewer_user_id` when anonymous.
pub async fn list_received_for_author(
    pool: &PgPool,
    tenant_id: Uuid,
    assignment_id: Uuid,
    author_user_id: Uuid,
) -> sqlx::Result<Vec<AllocationRow>> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    let rows = sqlx::query_as::<_, AllocationRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {ALLOCATION_COLS}
           FROM {ALLOCATION_JOINS}
          WHERE a.assignment_id = $1
            AND s.student_user_id = $2
            AND a.status = 'submitted'
          ORDER BY r.submitted_at, a.id"
    )))
    .bind(assignment_id)
    .bind(author_user_id)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(rows)
}

/// The full staff aggregate: every allocation for an assignment (filed or not),
/// joined with reviewee + reviewer + any review. Tenant-scoped under RLS.
pub async fn list_all_for_assignment(
    pool: &PgPool,
    tenant_id: Uuid,
    assignment_id: Uuid,
) -> sqlx::Result<Vec<AllocationRow>> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    let rows = sqlx::query_as::<_, AllocationRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {ALLOCATION_COLS}
           FROM {ALLOCATION_JOINS}
          WHERE a.assignment_id = $1
          ORDER BY s.student_user_id, a.reviewer_user_id, a.id"
    )))
    .bind(assignment_id)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(rows)
}
