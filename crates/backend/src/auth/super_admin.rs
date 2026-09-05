use std::collections::HashSet;
use std::sync::Arc;

use sqlx::PgPool;
use uuid::Uuid;

/// Environment-backed platform super-admin allowlist.
///
/// `AULALITE_SUPER_ADMIN_EMAILS` is a comma-separated list of verified global
/// identity email addresses. The environment can grant platform authority, but
/// deliberately does not demote manually provisioned operators when an address
/// is removed; revocation remains an explicit audited operation.
#[derive(Clone, Debug, Default)]
pub struct SuperAdminConfig {
    emails: Arc<HashSet<String>>,
}

impl SuperAdminConfig {
    pub fn parse(raw: &str) -> Result<Self, String> {
        let mut emails = HashSet::new();
        for candidate in raw.split(',') {
            let email = candidate.trim().to_ascii_lowercase();
            if email.is_empty() {
                continue;
            }
            if !valid_email(&email) {
                return Err(format!(
                    "AULALITE_SUPER_ADMIN_EMAILS contains an invalid email address: {email}"
                ));
            }
            emails.insert(email);
        }
        Ok(Self {
            emails: Arc::new(emails),
        })
    }

    pub fn is_empty(&self) -> bool {
        self.emails.is_empty()
    }

    pub fn len(&self) -> usize {
        self.emails.len()
    }

    pub fn contains(&self, email: &str) -> bool {
        self.emails.contains(&email.trim().to_ascii_lowercase())
    }

    /// Promote configured global identities that already exist at startup.
    pub async fn promote_existing(&self, pool: &PgPool) -> sqlx::Result<u64> {
        if self.emails.is_empty() {
            return Ok(0);
        }
        let emails: Vec<String> = self.emails.iter().cloned().collect();
        let result = sqlx::query(
            "UPDATE users
                SET is_platform_admin = TRUE
              WHERE identity_kind = 'global'
                AND identity_tenant_id IS NULL
                AND deleted_at IS NULL
                AND lower(email::text) = ANY($1)
                AND is_platform_admin = FALSE",
        )
        .bind(&emails)
        .execute(pool)
        .await?;
        Ok(result.rows_affected())
    }

    /// Promote a newly JIT-provisioned configured identity on its first request.
    pub async fn promote_user_if_configured(
        &self,
        pool: &PgPool,
        user_id: Uuid,
        email: &str,
    ) -> sqlx::Result<bool> {
        if !self.contains(email) {
            return Ok(false);
        }
        let result = sqlx::query(
            "UPDATE users
                SET is_platform_admin = TRUE
              WHERE id = $1
                AND identity_kind = 'global'
                AND identity_tenant_id IS NULL
                AND deleted_at IS NULL
                AND is_platform_admin = FALSE",
        )
        .bind(user_id)
        .execute(pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }
}

fn valid_email(email: &str) -> bool {
    if email.len() > 254 || email.chars().any(char::is_whitespace) {
        return false;
    }
    let mut parts = email.split('@');
    matches!(
        (parts.next(), parts.next(), parts.next()),
        (Some(local), Some(domain), None)
            if !local.is_empty()
                && !domain.is_empty()
                && !domain.starts_with('.')
                && !domain.ends_with('.')
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_emails_are_normalized_and_deduplicated() {
        let config = SuperAdminConfig::parse(
            " Owner@Elementors.Guru,ops@elementors.guru,owner@elementors.guru ",
        )
        .unwrap();
        assert_eq!(config.len(), 2);
        assert!(config.contains("owner@elementors.guru"));
        assert!(config.contains(" OPS@ELEMENTORS.GURU "));
    }

    #[test]
    fn malformed_email_fails_closed() {
        assert!(SuperAdminConfig::parse("valid@example.com,not an email").is_err());
        assert!(SuperAdminConfig::parse("admin@@example.com").is_err());
    }
}
