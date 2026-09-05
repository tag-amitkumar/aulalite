// crates/backend/src/db/enrollments.rs
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

const CODE_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

pub fn generate_code() -> String {
    use rand::prelude::IndexedRandom;
    let mut rng = rand::rng();
    (0..8)
        .map(|_| *CODE_ALPHABET.choose(&mut rng).unwrap() as char)
        .collect()
}

pub async fn insert_code(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    course_id: Uuid,
    code: &str,
    max_uses: Option<i32>,
    expires_at: Option<chrono::DateTime<chrono::Utc>>,
    created_by: Uuid,
) -> sqlx::Result<Uuid> {
    sqlx::query_scalar(
        "INSERT INTO enrollment_codes
            (tenant_id, course_id, code, max_uses, expires_at, created_by)
         VALUES ($1,$2,$3,$4,$5,$6) RETURNING id",
    )
    .bind(tenant_id)
    .bind(course_id)
    .bind(code)
    .bind(max_uses)
    .bind(expires_at)
    .bind(created_by)
    .fetch_one(&mut **tx)
    .await
}

#[derive(Debug)]
pub struct LookedUpCode {
    pub code_id: Uuid,
    pub tenant_id: Uuid,
    pub course_id: Uuid,
    pub max_uses: Option<i32>,
    pub uses: i32,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

pub async fn lookup_code_bypass_rls(
    pool: &PgPool,
    code: &str,
) -> sqlx::Result<Option<LookedUpCode>> {
    let row: Option<(
        Uuid,
        Uuid,
        Uuid,
        Option<i32>,
        i32,
        Option<chrono::DateTime<chrono::Utc>>,
    )> = sqlx::query_as("SELECT * FROM lookup_enrollment_code($1)")
        .bind(code)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(
        |(code_id, tenant_id, course_id, max_uses, uses, expires_at)| LookedUpCode {
            code_id,
            tenant_id,
            course_id,
            max_uses,
            uses,
            expires_at,
        },
    ))
}

pub async fn list_codes_for_course<'e, E>(
    executor: E,
    course_id: Uuid,
) -> sqlx::Result<
    Vec<(
        Uuid,
        String,
        Option<i32>,
        i32,
        Option<chrono::DateTime<chrono::Utc>>,
    )>,
>
where
    E: sqlx::PgExecutor<'e>,
{
    sqlx::query_as(
        "SELECT id, code, max_uses, uses, expires_at
           FROM enrollment_codes
          WHERE course_id = $1
          ORDER BY created_at DESC",
    )
    .bind(course_id)
    .fetch_all(executor)
    .await
}

pub async fn revoke_code(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    course_id: Uuid,
    code_id: Uuid,
) -> sqlx::Result<bool> {
    Ok(sqlx::query(
        "UPDATE enrollment_codes SET expires_at = now()
          WHERE tenant_id = $1 AND course_id = $2 AND id = $3",
    )
    .bind(tenant_id)
    .bind(course_id)
    .bind(code_id)
    .execute(&mut **tx)
    .await?
    .rows_affected()
        > 0)
}

pub async fn lock_code_for_redeem(
    tx: &mut Transaction<'_, Postgres>,
    code_id: Uuid,
) -> sqlx::Result<Option<(Option<i32>, i32, Option<chrono::DateTime<chrono::Utc>>)>> {
    sqlx::query_as(
        "SELECT max_uses, uses, expires_at FROM enrollment_codes
          WHERE id = $1 FOR UPDATE",
    )
    .bind(code_id)
    .fetch_optional(&mut **tx)
    .await
}

pub async fn increment_uses(tx: &mut Transaction<'_, Postgres>, code_id: Uuid) -> sqlx::Result<()> {
    sqlx::query("UPDATE enrollment_codes SET uses = uses + 1 WHERE id = $1")
        .bind(code_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

pub async fn ensure_tenant_membership_student(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<crate::db::seats::MembershipActivationOutcome> {
    crate::db::seats::ensure_active_tenant_membership(tx, tenant_id, user_id, "student").await
}

/// Ensure an active student course membership. Returns:
///   * `Ok(None)`      — membership was ALREADY active (no-op; callers such as
///                       code redemption must not consume a use),
///   * `Ok(Some(true))`  — fresh enrollment,
///   * `Ok(Some(false))` — existing non-active (removed) membership reactivated.
pub async fn ensure_course_membership_student(
    tx: &mut Transaction<'_, Postgres>,
    course_id: Uuid,
    user_id: Uuid,
    tenant_id: Uuid,
) -> sqlx::Result<Option<bool>> {
    let row: Option<(bool,)> = sqlx::query_as(
        "INSERT INTO course_memberships (course_id, user_id, tenant_id, role, status)
         VALUES ($1, $2, $3, 'student', 'active')
         ON CONFLICT (course_id, user_id) DO UPDATE
            SET status = 'active'
          WHERE course_memberships.status <> 'active'
         RETURNING (xmax = 0) AS inserted",
    )
    .bind(course_id)
    .bind(user_id)
    .bind(tenant_id)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(|(inserted,)| inserted))
}

// ---- Invitations ---------------------------------------------------------

pub fn generate_invitation_token() -> String {
    use base64::Engine;
    let mut bytes = [0u8; 32];
    use rand::RngCore;
    rand::rng().fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

pub async fn lazy_expire_stale_pending(
    tx: &mut Transaction<'_, Postgres>,
    course_id: Uuid,
    email: &str,
) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE course_invitations SET status='expired'
          WHERE course_id = $1
            AND lower(email::text) = lower($2)
            AND status = 'pending'
            AND expires_at < now()",
    )
    .bind(course_id)
    .bind(email)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub async fn insert_invitation(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    course_id: Uuid,
    email: &str,
    role: &str,
    token: &str,
    expires_at: chrono::DateTime<chrono::Utc>,
    created_by: Uuid,
) -> sqlx::Result<Uuid> {
    sqlx::query_scalar(
        "INSERT INTO course_invitations
            (tenant_id, course_id, email, role, token, expires_at, created_by)
         VALUES ($1,$2,$3,$4,$5,$6,$7) RETURNING id",
    )
    .bind(tenant_id)
    .bind(course_id)
    .bind(email)
    .bind(role)
    .bind(token)
    .bind(expires_at)
    .bind(created_by)
    .fetch_one(&mut **tx)
    .await
}

pub async fn list_invitations_for_course<'e, E>(
    executor: E,
    course_id: Uuid,
) -> sqlx::Result<Vec<(Uuid, String, String, String, chrono::DateTime<chrono::Utc>)>>
where
    E: sqlx::PgExecutor<'e>,
{
    sqlx::query_as(
        "SELECT id, email::text, role, status, expires_at
           FROM course_invitations
          WHERE course_id = $1 AND status = 'pending'
          ORDER BY created_at DESC",
    )
    .bind(course_id)
    .fetch_all(executor)
    .await
}

pub async fn revoke_invitation(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    course_id: Uuid,
    invitation_id: Uuid,
) -> sqlx::Result<bool> {
    Ok(sqlx::query(
        "UPDATE course_invitations
            SET status = 'revoked'
          WHERE tenant_id = $1 AND course_id = $2 AND id = $3
            AND status = 'pending'",
    )
    .bind(tenant_id)
    .bind(course_id)
    .bind(invitation_id)
    .execute(&mut **tx)
    .await?
    .rows_affected()
        > 0)
}

#[derive(Debug)]
pub struct LookedUpInvitation {
    pub invitation_id: Uuid,
    pub tenant_id: Uuid,
    pub course_id: Uuid,
    pub email: String,
    pub role: String,
    pub status: String,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

pub async fn lookup_invitation_bypass_rls(
    pool: &PgPool,
    token: &str,
) -> sqlx::Result<Option<LookedUpInvitation>> {
    let row: Option<(
        Uuid,
        Uuid,
        Uuid,
        String,
        String,
        String,
        chrono::DateTime<chrono::Utc>,
    )> = sqlx::query_as(
        "SELECT invitation_id, tenant_id, course_id, email::text, role, status, expires_at
               FROM lookup_invitation_by_token($1)",
    )
    .bind(token)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(
        |(invitation_id, tenant_id, course_id, email, role, status, expires_at)| {
            LookedUpInvitation {
                invitation_id,
                tenant_id,
                course_id,
                email,
                role,
                status,
                expires_at,
            }
        },
    ))
}

pub async fn mark_invitation_accepted(
    tx: &mut Transaction<'_, Postgres>,
    invitation_id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE course_invitations
            SET status = 'accepted', accepted_by = $2, accepted_at = now()
          WHERE id = $1 AND status = 'pending'",
    )
    .bind(invitation_id)
    .bind(user_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}
