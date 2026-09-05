// crates/backend/src/db/audit.rs
use serde::Serialize;
use serde_json::Value;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

/// Insert an audit_events row in the same transaction as the mutation.
/// `tenant_id` and `actor_user_id` MUST be set by the caller from the
/// authenticated request context.
pub async fn emit_audit_event(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    actor_user_id: Uuid,
    action: &str,
    resource_type: &str,
    resource_id: Uuid,
    metadata: Option<Value>,
) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO audit_events
            (tenant_id, actor_user_id, action, resource_type, resource_id, metadata)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(tenant_id)
    .bind(actor_user_id)
    .bind(action)
    .bind(resource_type)
    .bind(resource_id)
    .bind(metadata)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Row returned by the admin audit-list endpoint. Joins `users` to surface
/// a human-readable actor without forcing the caller into a second lookup.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct AuditEventRow {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub actor_user_id: Uuid,
    pub actor_email: Option<String>,
    pub actor_display_name: Option<String>,
    pub action: String,
    pub resource_type: String,
    pub resource_id: Uuid,
    pub metadata: Option<Value>,
    pub occurred_at: chrono::DateTime<chrono::Utc>,
}

/// List audit events for a tenant, most-recent first. Caller is expected to
/// scope by tenant via the WHERE clause (RLS provides defence in depth).
pub async fn list_for_tenant<'e, E>(
    executor: E,
    tenant_id: Uuid,
    limit: i64,
    before: Option<chrono::DateTime<chrono::Utc>>,
) -> sqlx::Result<Vec<AuditEventRow>>
where
    E: sqlx::PgExecutor<'e>,
{
    sqlx::query_as::<_, AuditEventRow>(
        "SELECT e.id, e.tenant_id, e.actor_user_id,
                u.email AS actor_email,
                u.display_name AS actor_display_name,
                e.action, e.resource_type, e.resource_id, e.metadata,
                e.occurred_at
           FROM audit_events e
           LEFT JOIN users u ON u.id = e.actor_user_id
          WHERE e.tenant_id = $1
            AND ($2::timestamptz IS NULL OR e.occurred_at < $2)
          ORDER BY e.occurred_at DESC
          LIMIT $3",
    )
    .bind(tenant_id)
    .bind(before)
    .bind(limit)
    .fetch_all(executor)
    .await
}
