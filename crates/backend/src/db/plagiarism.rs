// crates/backend/src/db/plagiarism.rs
//! Persisted similarity fingerprints for plagiarism checking.
//!
//! One row per submission (`submission_id` UNIQUE), storing the sorted/deduped
//! shingle fingerprint produced by `services::plagiarism::fingerprint`. The
//! handler reads every fingerprint for an assignment and compares them pairwise
//! in-process — the database only stores and lists them.
//!
//! TENANT-SCOPED under RLS, exactly like `db::announcements`: each read/write
//! runs in a tx with the `app.tenant_id` GUC set so the strict
//! `tenant_isolation` policy applies under the non-bypass `aulalite_app` role.
//! Runtime sqlx only (no compile-time macros).
//!
//! Storage note: the service fingerprint is `Vec<u64>`; Postgres `BIGINT[]` is
//! `i64`. We reinterpret the bits losslessly at the boundary (`u64 as i64` /
//! `i64 as u64`) — the values are only ever compared for equality, never
//! ordered as signed integers, so the bit-reinterpretation is safe.

use sqlx::PgPool;
use uuid::Uuid;

/// A submission's fingerprint as loaded from the database. `shingles` is the
/// sorted/deduped `Vec<u64>` ready to hand to `services::plagiarism`.
#[derive(Debug, Clone)]
pub struct FingerprintRow {
    pub submission_id: Uuid,
    pub student_user_id: Uuid,
    pub shingles: Vec<u64>,
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

fn to_i64(shingles: &[u64]) -> Vec<i64> {
    shingles.iter().map(|&h| h as i64).collect()
}

fn from_i64(raw: Vec<i64>) -> Vec<u64> {
    raw.into_iter().map(|h| h as u64).collect()
}

/// Upsert the fingerprint for a single submission. Idempotent on
/// `submission_id`: re-running replaces the stored shingles + bumps
/// `created_at`. Tenant-scoped under RLS.
pub async fn upsert_fingerprint(
    pool: &PgPool,
    tenant_id: Uuid,
    submission_id: Uuid,
    assignment_id: Uuid,
    student_user_id: Uuid,
    shingles: &[u64],
) -> sqlx::Result<()> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    upsert_fingerprint_tx(
        &mut tx,
        tenant_id,
        submission_id,
        assignment_id,
        student_user_id,
        shingles,
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Upsert variant that runs inside a caller-provided tx (the tenant GUC must
/// already be set on it). Used by the recompute path, which fingerprints every
/// submission in one transaction.
pub async fn upsert_fingerprint_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: Uuid,
    submission_id: Uuid,
    assignment_id: Uuid,
    student_user_id: Uuid,
    shingles: &[u64],
) -> sqlx::Result<()> {
    let raw = to_i64(shingles);
    sqlx::query(
        "INSERT INTO submission_fingerprints
             (tenant_id, submission_id, assignment_id, student_user_id, shingles)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (submission_id) DO UPDATE SET
             shingles = EXCLUDED.shingles,
             assignment_id = EXCLUDED.assignment_id,
             student_user_id = EXCLUDED.student_user_id,
             created_at = now()",
    )
    .bind(tenant_id)
    .bind(submission_id)
    .bind(assignment_id)
    .bind(student_user_id)
    .bind(&raw)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// List every stored fingerprint for an assignment. Tenant-scoped under RLS.
/// The handler compares these pairwise in-process.
pub async fn list_fingerprints_for_assignment(
    pool: &PgPool,
    tenant_id: Uuid,
    assignment_id: Uuid,
) -> sqlx::Result<Vec<FingerprintRow>> {
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant_id).await?;
    let rows: Vec<(Uuid, Uuid, Vec<i64>)> = sqlx::query_as(
        "SELECT submission_id, student_user_id, shingles
           FROM submission_fingerprints
          WHERE assignment_id = $1
          ORDER BY created_at",
    )
    .bind(assignment_id)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(rows
        .into_iter()
        .map(|(submission_id, student_user_id, raw)| FingerprintRow {
            submission_id,
            student_user_id,
            shingles: from_i64(raw),
        })
        .collect())
}
