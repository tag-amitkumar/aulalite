// crates/backend/src/db/mod.rs
pub mod analytics;
pub mod announcements;
pub mod api_keys;
pub mod assignments;
pub mod attendance;
pub mod audit;
pub mod billing;
pub mod branding;
pub mod bulk;
pub mod calendar;
pub mod certificates;
pub mod course_detail;
pub mod courses;
pub mod discussions;
pub mod enrollments;
pub mod file_assets;
pub mod flashcards;
pub mod gamification;
pub mod gradebook;
pub mod lessons;
pub mod live_room;
pub mod live_sessions;
pub mod lti;
pub mod member_invitations;
pub mod mfa;
pub mod modules;
pub mod notes;
pub mod notifications;
pub mod organization;
pub mod parent;
pub mod peer_review;
pub mod plagiarism;
pub mod platform;
pub mod privacy;
pub mod progress;
pub mod quizzes;
pub mod recordings;
pub mod rubrics;
pub mod scorm;
pub mod search;
pub mod seats;
pub mod secret_migration;
pub mod session_feedback;
pub mod sso;
pub mod submissions;
pub mod transcripts;
pub mod usage_limits;
pub mod webhooks;

use sqlx::postgres::{PgPool, PgPoolOptions};
use sqlx::{Postgres, Transaction};
use std::time::Duration;
use uuid::Uuid;

pub async fn pool_from_env() -> anyhow::Result<PgPool> {
    let url = std::env::var("DATABASE_URL")?;
    pool_from_url(&url).await
}

/// Build a Postgres pool for an explicit connection URL.
///
/// Production uses this to keep the owner-level migration connection separate
/// from the least-privileged runtime pool (`DATABASE_URL`).
pub async fn pool_from_url(url: &str) -> anyhow::Result<PgPool> {
    let max_connections = pool_env_u32("DB_MAX_CONNECTIONS", 20, 1, 200);
    let min_connections = pool_env_u32("DB_MIN_CONNECTIONS", 0, 0, max_connections);
    let acquire_timeout = pool_env_u64("DB_ACQUIRE_TIMEOUT_SECS", 10, 1, 120);
    let idle_timeout = pool_env_u64("DB_IDLE_TIMEOUT_SECS", 600, 30, 86_400);
    let max_lifetime = pool_env_u64("DB_MAX_LIFETIME_SECS", 1_800, 60, 86_400);
    let pool = PgPoolOptions::new()
        .max_connections(max_connections)
        .min_connections(min_connections)
        .acquire_timeout(Duration::from_secs(acquire_timeout))
        .idle_timeout(Some(Duration::from_secs(idle_timeout)))
        .max_lifetime(Some(Duration::from_secs(max_lifetime)))
        .test_before_acquire(true)
        .connect(url)
        .await?;
    Ok(pool)
}

fn pool_env_u32(name: &str, default: u32, min: u32, max: u32) -> u32 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<u32>().ok())
        .map(|value| value.clamp(min, max))
        .unwrap_or(default.clamp(min, max))
}

fn pool_env_u64(name: &str, default: u64, min: u64, max: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(|value| value.clamp(min, max))
        .unwrap_or(default.clamp(min, max))
}

pub async fn run_migrations(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::migrate!("../../migrations").run(pool).await?;
    Ok(())
}

/// Set the transaction-local Postgres GUCs that the RLS policies depend on.
/// Call this at the start of any handler tx that reads from or writes to a
/// tenant-scoped table when the backend is connected as a non-superuser
/// role (see `migrations/20260517000020_app_role.sql`).
///
/// `set_config(name, value, true)` only persists for the current
/// transaction, so this helper must be called inside the same tx that
/// runs subsequent queries.
pub async fn set_request_guc(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    tenant_id: Option<Uuid>,
) -> sqlx::Result<()> {
    sqlx::query("SELECT set_config('app.user_id', $1, true)")
        .bind(user_id.to_string())
        .execute(&mut **tx)
        .await?;
    if let Some(tid) = tenant_id {
        sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
            .bind(tid.to_string())
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}

/// Convenience wrapper: open a fresh transaction and immediately set the
/// RLS GUCs (`app.user_id` + `app.tenant_id`). Use this in handlers that
/// only need a tx to scope reads under RLS — drop-in for `pool.begin()`.
pub async fn begin_with_context(
    pool: &PgPool,
    user_id: Uuid,
    tenant_id: Option<Uuid>,
) -> sqlx::Result<Transaction<'_, Postgres>> {
    let mut tx = pool.begin().await?;
    set_request_guc(&mut tx, user_id, tenant_id).await?;
    Ok(tx)
}

/// Open a transaction with the `app.system` GUC set to `'on'`, granting the
/// trusted background workers (recording sweep, retention janitor, auto-end
/// sweep) cross-tenant visibility via the `system_context_*` RLS policies
/// (migration `20260530000030_system_context_rls.sql`).
///
/// Without this, those discovery queries run with no `app.tenant_id` set and
/// the `tenant_isolation` policies filter out every row under the non-bypass
/// `aulalite_app` role — silently disabling the sweeps. `set_config(_, _, true)`
/// is transaction-local, so the elevation never leaks beyond this tx.
///
/// SECURITY: only the in-process background sweeps may use this — NEVER a
/// request handler. It deliberately bypasses per-tenant isolation for reads.
pub async fn begin_system_context(pool: &PgPool) -> sqlx::Result<Transaction<'_, Postgres>> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.system', 'on', true)")
        .execute(&mut *tx)
        .await?;
    Ok(tx)
}

/// At startup, log loudly if the backend is connecting to Postgres as a
/// SUPERUSER or a role with the BYPASSRLS attribute — in either case the
/// FORCE ROW LEVEL SECURITY policies defined in the migrations do nothing
/// in practice. The migration `20260517000020_app_role.sql` creates
/// `aulalite_app` for this purpose; operators must update `DATABASE_URL`
/// to use it before RLS is actually enforced.
///
/// Returns `true` when the connection role bypasses RLS, so the caller can
/// fail closed in production (see `main.rs`).
pub async fn warn_if_bypassing_rls(pool: &PgPool) -> sqlx::Result<bool> {
    let (rolname, is_super, bypassrls): (String, bool, bool) = sqlx::query_as(
        "SELECT rolname, rolsuper, rolbypassrls FROM pg_roles WHERE rolname = current_user",
    )
    .fetch_one(pool)
    .await?;
    let bypasses = is_super || bypassrls;
    if bypasses {
        tracing::warn!(
            role = %rolname,
            is_super,
            bypassrls,
            "Postgres connection role bypasses Row Level Security. RLS policies in migrations are effectively no-ops. To enforce RLS, run migration 20260517000020_app_role.sql and switch DATABASE_URL to use the `aulalite_app` role."
        );
    }
    Ok(bypasses)
}
