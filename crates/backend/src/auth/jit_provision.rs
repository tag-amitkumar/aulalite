use sqlx::{Acquire, PgPool};
use uuid::Uuid;

use crate::auth::identity::EnterpriseProvider;
use crate::auth::verify::FirebaseClaims;

#[derive(Debug, thiserror::Error)]
pub enum ProvisionError {
    #[error("db: {0}")]
    Db(#[from] sqlx::Error),
    #[error("missing email in claims")]
    MissingEmail,
    #[error("verified email required")]
    EmailNotVerified,
    #[error("seat_limit_reached")]
    SeatLimitReached,
    #[error("account_disabled")]
    AccountDisabled,
    #[error("identity_scope_conflict")]
    IdentityScopeConflict,
}

#[derive(Debug, Clone)]
pub struct ProvisionedUser {
    pub user_id: Uuid,
    pub firebase_uid: String,
    pub email: String,
    pub display_name: Option<String>,
}

pub async fn ensure_user(
    pool: &PgPool,
    claims: &FirebaseClaims,
) -> Result<ProvisionedUser, ProvisionError> {
    ensure_user_with_self_service(pool, claims, false).await
}

/// Provision an authenticated identity and, when explicitly enabled by the
/// Firebase authentication path, create its first academy workspace.
///
/// Keep self-service opt-in at the call site: SSO, LTI, local-login fixtures,
/// and other synthetic identities also use [`ensure_user`] but must never
/// become academy owners merely because they lack a membership.
pub async fn ensure_user_with_self_service(
    pool: &PgPool,
    claims: &FirebaseClaims,
    self_service_enabled: bool,
) -> Result<ProvisionedUser, ProvisionError> {
    // Email is an authorization boundary, not merely profile data: the JIT flow
    // below accepts pending tenant/parent invitations by matching this value.
    // Requiring an explicit `true` prevents a valid token for an unverified (or
    // provider-omitted) address from claiming invitations sent to someone else.
    // Keep this check before opening the transaction so a rejected identity
    // cannot create/update a user or mutate any invitation state.
    let email = verified_email(claims)?.to_string();

    // The user upsert and the parent-invitation acceptance share one tx so the
    // acceptance sees the just-created user. `email` here comes from the
    // verified token claims (see the `ok_or(MissingEmail)` above).
    let mut tx = pool.begin().await?;
    let row: Option<(Uuid, String, String, Option<String>)> = sqlx::query_as(
        r#"
        INSERT INTO users (firebase_uid, email, display_name, last_seen_at)
        VALUES ($1, $2, $3, now())
        ON CONFLICT (firebase_uid) DO UPDATE
            SET last_seen_at = EXCLUDED.last_seen_at,
                email = EXCLUDED.email,
                display_name = COALESCE(EXCLUDED.display_name, users.display_name)
            WHERE users.deleted_at IS NULL
              AND users.identity_kind = 'global'
              AND users.identity_tenant_id IS NULL
        RETURNING id, firebase_uid, email::text, display_name
        "#,
    )
    .bind(&claims.sub)
    .bind(&email)
    .bind(claims.name.as_deref())
    .fetch_optional(&mut *tx)
    .await?;

    let Some(row) = row else {
        return Err(classify_identity_conflict(&mut tx, &claims.sub).await?);
    };

    let user_id = row.0;
    // Bind every SECURITY DEFINER bootstrap action to the authenticated user
    // represented by this transaction. The personal-workspace function checks
    // this GUC against its UUID argument instead of trusting a caller-supplied
    // identity on its own.
    sqlx::query("SELECT set_config('app.user_id', $1, true)")
        .bind(user_id.to_string())
        .execute(&mut *tx)
        .await?;

    // Accept any pending tenant member invitations for this email
    // (across all tenants), (re)activating the tenant_memberships seat with the
    // invited role. The savepoint preserves the historical best-effort path
    // for non-self-service identities. Self-service fails closed on an error,
    // because continuing could misclassify an invited user as a new owner.
    // Seat-cap enforcement happened at invite CREATION time, so acceptance
    // here does no seat math.
    let invitation_acceptance = match tx.begin().await {
        Ok(mut sp) => {
            match crate::db::member_invitations::accept_pending_for_email(&mut sp, user_id, &email)
                .await
            {
                Ok(_) => sp.commit().await,
                Err(err) => {
                    // sp is dropped without commit -> SAVEPOINT rolled back.
                    Err(err)
                }
            }
        }
        Err(err) => Err(err),
    };

    if let Err(err) = invitation_acceptance {
        if self_service_enabled {
            // Invitation acceptance is an authorization decision for
            // self-service: failing open here could create an owner workspace
            // for a user who was actually invited into an existing academy.
            return Err(ProvisionError::Db(err));
        } else {
            tracing::warn!(
                user_id = %user_id,
                error = %err,
                "tenant member-invitation acceptance failed during JIT provisioning; continuing"
            );
        }
    }

    // Link pending parent invitations only after tenant-member invitations.
    // Both invitation types reserve a seat by email; accepting the member
    // invitation first turns that reservation into an active membership so a
    // same-email parent link cannot incorrectly block itself at the seat cap.
    // The SECURITY DEFINER function performs the cross-tenant mutations and
    // enforces caps under stable per-tenant locks.
    match tx.begin().await {
        // `tx.begin()` here is `Acquire::begin`, issuing a SAVEPOINT.
        Ok(mut sp) => {
            match crate::db::parent::accept_pending_for_email(&mut sp, user_id, &email).await {
                Ok(outcome) if outcome.blocked_tenants > 0 => {
                    tracing::warn!(
                        user_id = %user_id,
                        blocked_tenants = outcome.blocked_tenants,
                        "parent-invitation acceptance blocked by tenant seat cap"
                    );
                    return Err(ProvisionError::SeatLimitReached);
                }
                Ok(_) => {
                    if let Err(err) = sp.commit().await {
                        tracing::warn!(
                            user_id = %user_id,
                            error = %err,
                            "parent-invitation acceptance savepoint commit failed; continuing"
                        );
                    }
                }
                Err(err) => {
                    tracing::warn!(
                        user_id = %user_id,
                        error = %err,
                        "parent-invitation acceptance failed during JIT provisioning; continuing"
                    );
                    // sp is dropped without commit -> SAVEPOINT rolled back.
                }
            }
        }
        Err(err) => {
            tracing::warn!(
                user_id = %user_id,
                error = %err,
                "could not open savepoint for parent-invitation acceptance; continuing"
            );
        }
    }

    if self_service_enabled {
        let workspace_name = workspace_name(claims, &email);
        let _: Option<Uuid> =
            sqlx::query_scalar("SELECT provision_personal_workspace_if_needed($1, $2)")
                .bind(user_id)
                .bind(workspace_name)
                .fetch_one(&mut *tx)
                .await?;
    }

    tx.commit().await?;

    Ok(ProvisionedUser {
        user_id,
        firebase_uid: row.1,
        email: row.2,
        display_name: row.3,
    })
}

/// Provision an identity asserted by an organization-controlled OIDC or LTI
/// provider. It deliberately does *not* consume email invitations, parent
/// links, or self-service workspace creation: the asserted email is trusted
/// only inside `tenant_id`, never as proof of a platform-global identity.
pub async fn ensure_enterprise_user(
    pool: &PgPool,
    claims: &FirebaseClaims,
    tenant_id: Uuid,
    provider: EnterpriseProvider,
) -> Result<ProvisionedUser, ProvisionError> {
    let email = verified_email(claims)?.to_string();
    let identity_kind = provider.as_db_str();

    let mut tx = pool.begin().await?;
    let row: Option<(Uuid, String, String, Option<String>)> = sqlx::query_as(
        r#"
        INSERT INTO users (
            firebase_uid, email, display_name, last_seen_at,
            identity_kind, identity_tenant_id
        )
        VALUES ($1, $2, $3, now(), $4, $5)
        ON CONFLICT (firebase_uid) DO UPDATE
            SET last_seen_at = EXCLUDED.last_seen_at,
                email = EXCLUDED.email,
                display_name = COALESCE(EXCLUDED.display_name, users.display_name)
            WHERE users.deleted_at IS NULL
              AND users.identity_kind = EXCLUDED.identity_kind
              AND users.identity_tenant_id = EXCLUDED.identity_tenant_id
        RETURNING id, firebase_uid, email::text, display_name
        "#,
    )
    .bind(&claims.sub)
    .bind(&email)
    .bind(claims.name.as_deref())
    .bind(identity_kind)
    .bind(tenant_id)
    .fetch_optional(&mut *tx)
    .await?;

    let Some(row) = row else {
        return Err(classify_identity_conflict(&mut tx, &claims.sub).await?);
    };
    tx.commit().await?;

    Ok(ProvisionedUser {
        user_id: row.0,
        firebase_uid: row.1,
        email: row.2,
        display_name: row.3,
    })
}

async fn classify_identity_conflict(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    firebase_uid: &str,
) -> Result<ProvisionError, sqlx::Error> {
    let deleted_at: Option<Option<chrono::DateTime<chrono::Utc>>> =
        sqlx::query_scalar("SELECT deleted_at FROM users WHERE firebase_uid = $1")
            .bind(firebase_uid)
            .fetch_optional(&mut **tx)
            .await?;
    Ok(match deleted_at {
        Some(Some(_)) => ProvisionError::AccountDisabled,
        _ => ProvisionError::IdentityScopeConflict,
    })
}

fn workspace_name(claims: &FirebaseClaims, email: &str) -> String {
    let owner = claims
        .name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .or_else(|| email.split('@').next().filter(|name| !name.is_empty()))
        .unwrap_or("My");
    format!("{owner}'s academy")
}

fn verified_email(claims: &FirebaseClaims) -> Result<&str, ProvisionError> {
    let email = claims
        .email
        .as_deref()
        .map(str::trim)
        .filter(|email| !email.is_empty())
        .ok_or(ProvisionError::MissingEmail)?;
    if claims.email_verified != Some(true) {
        return Err(ProvisionError::EmailNotVerified);
    }
    Ok(email)
}

#[cfg(test)]
mod tests {
    use super::{verified_email, workspace_name, ProvisionError};
    use crate::auth::verify::FirebaseClaims;

    fn claims(email: Option<&str>, email_verified: Option<bool>) -> FirebaseClaims {
        FirebaseClaims {
            sub: "uid".into(),
            email: email.map(str::to_string),
            email_verified,
            name: None,
            picture: None,
            aud: "aud".into(),
            iss: "iss".into(),
            exp: i64::MAX,
            iat: 0,
            auth_time: None,
        }
    }

    #[test]
    fn verified_email_requires_explicit_true() {
        assert!(matches!(
            verified_email(&claims(Some("person@example.test"), Some(false))),
            Err(ProvisionError::EmailNotVerified)
        ));
        assert!(matches!(
            verified_email(&claims(Some("person@example.test"), None)),
            Err(ProvisionError::EmailNotVerified)
        ));
    }

    #[test]
    fn verified_email_rejects_missing_or_blank_email() {
        assert!(matches!(
            verified_email(&claims(None, Some(true))),
            Err(ProvisionError::MissingEmail)
        ));
        assert!(matches!(
            verified_email(&claims(Some("  "), Some(true))),
            Err(ProvisionError::MissingEmail)
        ));
    }

    #[test]
    fn verified_email_returns_trimmed_address() {
        assert_eq!(
            verified_email(&claims(Some("  person@example.test  "), Some(true))).unwrap(),
            "person@example.test"
        );
    }

    #[test]
    fn workspace_name_prefers_display_name_then_email() {
        assert_eq!(
            workspace_name(
                &FirebaseClaims {
                    name: Some("  Ada Lovelace  ".into()),
                    ..claims(Some("ada@example.test"), Some(true))
                },
                "ada@example.test"
            ),
            "Ada Lovelace's academy"
        );
        assert_eq!(
            workspace_name(
                &claims(Some("grace@example.test"), Some(true)),
                "grace@example.test"
            ),
            "grace's academy"
        );
    }
}
