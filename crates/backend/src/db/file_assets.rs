// crates/backend/src/db/file_assets.rs
use serde::Serialize;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct FileAssetRow {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub owner_user_id: Uuid,
    pub bucket: String,
    pub object_key: String,
    pub content_type: String,
    pub size_bytes: i64,
    pub status: String,
    pub visibility: String,
    pub linked_entity_type: Option<String>,
    pub linked_entity_id: Option<Uuid>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

pub async fn insert_pending(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    owner_user_id: Uuid,
    bucket: &str,
    object_key: &str,
    content_type: &str,
    size_bytes: i64,
    linked_entity_type: Option<&str>,
    linked_entity_id: Option<Uuid>,
) -> sqlx::Result<FileAssetRow> {
    sqlx::query_as::<_, FileAssetRow>(
        "INSERT INTO file_assets
            (tenant_id, owner_user_id, bucket, object_key, content_type,
             size_bytes, status, visibility, linked_entity_type, linked_entity_id)
         VALUES ($1, $2, $3, $4, $5, $6, 'pending', 'private', $7, $8)
         RETURNING id, tenant_id, owner_user_id, bucket, object_key, content_type,
                   size_bytes, status, visibility, linked_entity_type, linked_entity_id,
                   created_at",
    )
    .bind(tenant_id)
    .bind(owner_user_id)
    .bind(bucket)
    .bind(object_key)
    .bind(content_type)
    .bind(size_bytes)
    .bind(linked_entity_type)
    .bind(linked_entity_id)
    .fetch_one(&mut **tx)
    .await
}

pub async fn fetch<'e, E>(executor: E, id: Uuid) -> sqlx::Result<Option<FileAssetRow>>
where
    E: sqlx::PgExecutor<'e>,
{
    sqlx::query_as::<_, FileAssetRow>(
        "SELECT id, tenant_id, owner_user_id, bucket, object_key, content_type,
                size_bytes, status, visibility, linked_entity_type, linked_entity_id,
                created_at
           FROM file_assets WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(executor)
    .await
}

pub async fn mark_available(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
) -> sqlx::Result<Option<FileAssetRow>> {
    sqlx::query_as::<_, FileAssetRow>(
        "UPDATE file_assets SET status = 'available'
          WHERE id = $1
        RETURNING id, tenant_id, owner_user_id, bucket, object_key, content_type,
                  size_bytes, status, visibility, linked_entity_type, linked_entity_id,
                  created_at",
    )
    .bind(id)
    .fetch_optional(&mut **tx)
    .await
}

pub async fn mark_failed(tx: &mut Transaction<'_, Postgres>, id: Uuid) -> sqlx::Result<()> {
    sqlx::query("UPDATE file_assets SET status = 'failed' WHERE id = $1")
        .bind(id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

pub async fn mark_pruned(tx: &mut Transaction<'_, Postgres>, id: Uuid) -> sqlx::Result<bool> {
    Ok(
        sqlx::query(
            "UPDATE file_assets SET status = 'pruned' WHERE id = $1 AND status <> 'pruned'",
        )
        .bind(id)
        .execute(&mut **tx)
        .await?
        .rows_affected()
            > 0,
    )
}

pub async fn list_for_entity<'e, E>(
    executor: E,
    linked_entity_type: &str,
    linked_entity_id: Uuid,
) -> sqlx::Result<Vec<FileAssetRow>>
where
    E: sqlx::PgExecutor<'e>,
{
    sqlx::query_as::<_, FileAssetRow>(
        "SELECT id, tenant_id, owner_user_id, bucket, object_key, content_type,
                size_bytes, status, visibility, linked_entity_type, linked_entity_id,
                created_at
           FROM file_assets
          WHERE linked_entity_type = $1
            AND linked_entity_id = $2
            AND status = 'available'
          ORDER BY created_at ASC",
    )
    .bind(linked_entity_type)
    .bind(linked_entity_id)
    .fetch_all(executor)
    .await
}

pub async fn list_for_tenant<'e, E>(
    executor: E,
    tenant_id: Uuid,
    limit: i64,
) -> sqlx::Result<Vec<FileAssetRow>>
where
    E: sqlx::PgExecutor<'e>,
{
    sqlx::query_as::<_, FileAssetRow>(
        "SELECT id, tenant_id, owner_user_id, bucket, object_key, content_type,
                size_bytes, status, visibility, linked_entity_type, linked_entity_id,
                created_at
           FROM file_assets
          WHERE tenant_id = $1
            AND status = 'available'
          ORDER BY created_at DESC
          LIMIT $2",
    )
    .bind(tenant_id)
    .bind(limit)
    .fetch_all(executor)
    .await
}
