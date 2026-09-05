//! Certificates (learning-suite Cycle 5).
//!
//! Lifecycle: course completion inserts an `eligible` row (idempotent, via
//! `sync_eligibility` called from the lesson-completion and quiz-submission
//! paths); a teacher then issues (snapshotting recipient/course names and
//! minting the public credential id) or revokes. Public verification goes
//! through the `verify_certificate` SECURITY DEFINER function so it needs no
//! tenant GUC.

use sqlx::{PgConnection, PgPool, Postgres, Transaction};
use uuid::Uuid;

/// Mint a human-readable credential id (e.g. `AULA-1A2B-3C4D`) from a fresh
/// UUID. 16^8 space is plenty; the column's UNIQUE constraint backstops the
/// (cosmically unlikely) collision, which surfaces as a retryable error.
pub fn mint_credential_id() -> String {
    let hex = Uuid::new_v4().simple().to_string().to_uppercase();
    format!("AULA-{}-{}", &hex[0..4], &hex[4..8])
}

/// True when the student has finished the course: every lesson completed
/// (courses with zero lessons are never complete) and every published graded
/// quiz submitted at least once.
pub async fn is_course_complete(
    conn: &mut PgConnection,
    course_id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<bool> {
    sqlx::query_scalar(
        "WITH lesson_stats AS (
            SELECT (SELECT count(*) FROM lessons WHERE course_id = $1) AS total,
                   (SELECT count(*) FROM lesson_completions lc
                      JOIN lessons l ON l.id = lc.lesson_id
                     WHERE l.course_id = $1 AND lc.user_id = $2) AS done
         ),
         quiz_stats AS (
            SELECT count(*) AS total,
                   count(*) FILTER (WHERE EXISTS (
                       SELECT 1 FROM quiz_attempts qa
                        WHERE qa.quiz_id = q.id AND qa.user_id = $2
                          AND qa.submitted_at IS NOT NULL)) AS done
              FROM quizzes q
             WHERE q.course_id = $1 AND q.status = 'published' AND q.mode = 'graded'
         )
         SELECT (SELECT total FROM lesson_stats) > 0
            AND (SELECT done FROM lesson_stats) >= (SELECT total FROM lesson_stats)
            AND (SELECT done FROM quiz_stats) >= (SELECT total FROM quiz_stats)",
    )
    .bind(course_id)
    .bind(user_id)
    .fetch_one(conn)
    .await
}

/// Insert the `eligible` row when the course just became complete. Idempotent:
/// an existing row (eligible/issued/revoked) is left untouched. Returns true
/// when a new eligibility row was created.
pub async fn sync_eligibility(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    course_id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<bool> {
    if !is_course_complete(&mut *tx, course_id, user_id).await? {
        return Ok(false);
    }
    let inserted = sqlx::query(
        "INSERT INTO certificates (tenant_id, course_id, user_id)
         VALUES ($1, $2, $3) ON CONFLICT (course_id, user_id) DO NOTHING",
    )
    .bind(tenant_id)
    .bind(course_id)
    .bind(user_id)
    .execute(&mut **tx)
    .await?
    .rows_affected()
        > 0;
    Ok(inserted)
}

#[derive(Debug, sqlx::FromRow)]
pub struct CertificateRow {
    pub id: Uuid,
    pub course_id: Uuid,
    pub user_id: Uuid,
    pub credential_id: Option<String>,
    pub status: String,
    pub recipient_name: Option<String>,
    pub course_title: Option<String>,
    pub issued_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Live display name/email of the student (for the teacher's list).
    pub display_name: Option<String>,
    pub email: String,
}

/// All certificate rows for a course (eligible + issued + revoked), newest
/// eligibility first, with live student identity joined for the staff view.
pub async fn list_for_course(
    conn: &mut PgConnection,
    course_id: Uuid,
) -> sqlx::Result<Vec<CertificateRow>> {
    sqlx::query_as(
        "SELECT c.id, c.course_id, c.user_id, c.credential_id, c.status,
                c.recipient_name, c.course_title, c.issued_at, c.created_at,
                u.display_name, u.email::text AS email
           FROM certificates c
           JOIN users u ON u.id = c.user_id
          WHERE c.course_id = $1
          ORDER BY c.created_at DESC",
    )
    .bind(course_id)
    .fetch_all(conn)
    .await
}

/// The caller's own certificates across courses, newest first.
pub async fn list_mine(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<Vec<CertificateRow>> {
    sqlx::query_as(
        "SELECT c.id, c.course_id, c.user_id, c.credential_id, c.status,
                COALESCE(c.course_title, co.title) AS course_title,
                c.recipient_name, c.issued_at, c.created_at,
                u.display_name, u.email::text AS email
           FROM certificates c
           JOIN users u ON u.id = c.user_id
           JOIN courses co ON co.id = c.course_id
          WHERE c.tenant_id = $1 AND c.user_id = $2
          ORDER BY c.created_at DESC",
    )
    .bind(tenant_id)
    .bind(user_id)
    .fetch_all(conn)
    .await
}

/// Issue (or re-issue after revoke). Snapshots the recipient's current
/// display name (falling back to email) and the course title; mints the
/// credential id only on first issue so the public link stays stable.
/// Returns the updated row, or None when the student isn't eligible.
pub async fn issue(
    tx: &mut Transaction<'_, Postgres>,
    course_id: Uuid,
    user_id: Uuid,
    issued_by: Uuid,
) -> sqlx::Result<Option<CertificateRow>> {
    let credential = mint_credential_id();
    sqlx::query_as(
        "UPDATE certificates c SET
            status = 'issued',
            credential_id = COALESCE(c.credential_id, $4),
            issued_by = $3,
            issued_at = now(),
            recipient_name = COALESCE(
                NULLIF((SELECT display_name FROM users WHERE id = $2), ''),
                (SELECT email::text FROM users WHERE id = $2)),
            course_title = (SELECT title FROM courses WHERE id = $1)
          FROM users u
         WHERE c.course_id = $1 AND c.user_id = $2 AND u.id = c.user_id
           AND c.status IN ('eligible','revoked')
         RETURNING c.id, c.course_id, c.user_id, c.credential_id, c.status,
                   c.recipient_name, c.course_title, c.issued_at, c.created_at,
                   u.display_name, u.email::text AS email",
    )
    .bind(course_id)
    .bind(user_id)
    .bind(issued_by)
    .bind(credential)
    .fetch_optional(&mut **tx)
    .await
}

/// Revoke an issued certificate. Returns false when there was nothing issued.
pub async fn revoke(
    tx: &mut Transaction<'_, Postgres>,
    course_id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<bool> {
    let n = sqlx::query(
        "UPDATE certificates SET status = 'revoked'
          WHERE course_id = $1 AND user_id = $2 AND status = 'issued'",
    )
    .bind(course_id)
    .bind(user_id)
    .execute(&mut **tx)
    .await?
    .rows_affected();
    Ok(n > 0)
}

#[derive(Debug, sqlx::FromRow)]
pub struct VerifyRow {
    pub credential_id: String,
    pub status: String,
    pub recipient_name: Option<String>,
    pub course_title: Option<String>,
    pub issued_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Public, unauthenticated verification by credential id. Backed by the
/// SECURITY DEFINER `verify_certificate` function; returns None for unknown
/// ids and rows with status `revoked` for revoked certificates.
pub async fn verify(pool: &PgPool, credential_id: &str) -> sqlx::Result<Option<VerifyRow>> {
    sqlx::query_as("SELECT * FROM verify_certificate($1)")
        .bind(credential_id)
        .fetch_optional(pool)
        .await
}

#[cfg(test)]
mod tests {
    use super::mint_credential_id;

    #[test]
    fn credential_ids_have_the_documented_shape() {
        let id = mint_credential_id();
        assert_eq!(id.len(), "AULA-XXXX-XXXX".len());
        assert!(id.starts_with("AULA-"));
        assert_eq!(id.matches('-').count(), 2);
        assert!(id
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '-'));
        // Two mints are (overwhelmingly) distinct.
        assert_ne!(id, mint_credential_id());
    }
}
