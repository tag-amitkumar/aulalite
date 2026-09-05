use sqlx::{Postgres, Transaction};
use uuid::Uuid;

#[derive(Debug, sqlx::FromRow)]
pub struct OwnershipTransferRow {
    pub previous_owner_user_id: Uuid,
    pub new_owner_user_id: Uuid,
    pub transferred_at: chrono::DateTime<chrono::Utc>,
}

/// Invoke the actor-validating, audited ownership transfer boundary. The
/// caller must have set `app.user_id` and `app.tenant_id` on this transaction;
/// the SQL function derives and verifies the owner from that context.
pub async fn transfer_ownership(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    target_user_id: Uuid,
) -> sqlx::Result<OwnershipTransferRow> {
    sqlx::query_as("SELECT * FROM transfer_tenant_ownership($1, $2)")
        .bind(tenant_id)
        .bind(target_user_id)
        .fetch_one(&mut **tx)
        .await
}
