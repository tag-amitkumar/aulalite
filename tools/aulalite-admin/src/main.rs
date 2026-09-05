// tools/aulalite-admin/src/main.rs
use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand};
use sqlx::postgres::PgPoolOptions;

#[derive(Parser)]
#[command(name = "aulalite-admin", version)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Create a trial workspace and assign an existing user as organization owner.
    CreateTenant {
        #[arg(long)]
        slug: String,
        #[arg(long)]
        name: String,
        // Keep --admin-email for backwards-compatible operator scripts while
        // exposing the role-correct --owner-email spelling for new usage.
        #[arg(long, alias = "owner-email")]
        admin_email: String,
    },
    /// Promote a user to platform super admin.
    PromotePlatformAdmin {
        #[arg(long)]
        email: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    // This bootstrap utility performs schema-owner-only provisioning. Never
    // point it at the request-traffic role: production deliberately grants
    // `aulalite_app` no direct organization-owner mutation authority.
    let database_url = std::env::var("MIGRATION_DATABASE_URL")
        .context("MIGRATION_DATABASE_URL (the schema-owner connection) must be set")?;
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url)
        .await?;

    match cli.cmd {
        Cmd::CreateTenant {
            slug,
            name,
            admin_email,
        } => {
            let mut tx = pool.begin().await?;

            let user_id: Option<uuid::Uuid> = sqlx::query_scalar(
                "SELECT id FROM users
                      WHERE email = $1::citext
                        AND deleted_at IS NULL
                        AND identity_kind = 'global'
                        AND identity_tenant_id IS NULL",
            )
            .bind(&admin_email)
            .fetch_optional(&mut *tx)
            .await?;

            let user_id = user_id.ok_or_else(|| {
                anyhow!("no user with email {admin_email}; have them sign up first, then re-run")
            })?;

            let tenant_id: uuid::Uuid = sqlx::query_scalar(
                "INSERT INTO tenants (slug, name, status, trial_ends_at)
                 VALUES ($1, $2, 'trialing', now() + interval '14 days')
                 RETURNING id",
            )
            .bind(&slug)
            .bind(&name)
            .fetch_one(&mut *tx)
            .await?;

            sqlx::query(
                "INSERT INTO tenant_memberships (tenant_id, user_id, role, status)
                 VALUES ($1, $2, 'org_owner', 'active')",
            )
            .bind(tenant_id)
            .bind(user_id)
            .execute(&mut *tx)
            .await?;

            sqlx::query(
                "INSERT INTO subscriptions (
                     tenant_id, plan_id, status, trial_ends_at, overage_behavior
                 ) VALUES (
                     $1, 'starter', 'trialing', now() + interval '14 days', 'block'
                 )",
            )
            .bind(tenant_id)
            .execute(&mut *tx)
            .await?;

            tx.commit().await?;
            println!(
                "Created trial tenant {slug} ({tenant_id}) with organization owner {admin_email}"
            );
        }
        Cmd::PromotePlatformAdmin { email } => {
            let mut tx = pool.begin().await?;
            let candidates: Vec<uuid::Uuid> = sqlx::query_scalar(
                "SELECT id
                   FROM users
                  WHERE email = $1::citext
                    AND deleted_at IS NULL
                    AND identity_kind = 'global'
                    AND identity_tenant_id IS NULL
                  FOR UPDATE",
            )
            .bind(&email)
            .fetch_all(&mut *tx)
            .await?;

            let [user_id] = candidates.as_slice() else {
                if candidates.is_empty() {
                    return Err(anyhow!("no live global user with email {email}"));
                }
                return Err(anyhow!(
                    "multiple live global users share {email}; refusing ambiguous platform promotion"
                ));
            };

            let updated = sqlx::query("UPDATE users SET is_platform_admin = TRUE WHERE id = $1")
                .bind(user_id)
                .execute(&mut *tx)
                .await?;
            if updated.rows_affected() != 1 {
                return Err(anyhow!("no user with email {email}"));
            }

            tx.commit().await?;

            println!("Promoted {email} to platform admin");
        }
    }

    Ok(())
}
