// crates/backend/src/db/modules.rs
use serde::Serialize;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct ModuleRow {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub course_id: Uuid,
    pub title: String,
    pub sort_order: i32,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

pub async fn next_sort_order(
    tx: &mut Transaction<'_, Postgres>,
    course_id: Uuid,
) -> sqlx::Result<i32> {
    let max: Option<i32> =
        sqlx::query_scalar("SELECT MAX(sort_order) FROM modules WHERE course_id = $1")
            .bind(course_id)
            .fetch_one(&mut **tx)
            .await?;
    Ok(max.unwrap_or(0) + 10)
}

pub async fn insert_module(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    course_id: Uuid,
    title: &str,
    sort_order: i32,
) -> sqlx::Result<ModuleRow> {
    sqlx::query_as::<_, ModuleRow>(
        "INSERT INTO modules (tenant_id, course_id, title, sort_order)
         VALUES ($1, $2, $3, $4)
         RETURNING id, tenant_id, course_id, title, sort_order, created_at",
    )
    .bind(tenant_id)
    .bind(course_id)
    .bind(title)
    .bind(sort_order)
    .fetch_one(&mut **tx)
    .await
}

pub async fn reorder(
    tx: &mut Transaction<'_, Postgres>,
    course_id: Uuid,
    ordered: &[Uuid],
) -> sqlx::Result<()> {
    for (idx, mod_id) in ordered.iter().enumerate() {
        let new_order = ((idx as i32) + 1) * 10;
        sqlx::query(
            "UPDATE modules SET sort_order = $1, updated_at = now()
              WHERE id = $2 AND course_id = $3",
        )
        .bind(new_order)
        .bind(mod_id)
        .bind(course_id)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

pub async fn update_title(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    course_id: Uuid,
    id: Uuid,
    new_title: &str,
) -> sqlx::Result<Option<ModuleRow>> {
    sqlx::query_as::<_, ModuleRow>(
        "UPDATE modules SET title = $4, updated_at = now()
          WHERE id = $3 AND course_id = $2 AND tenant_id = $1
        RETURNING id, tenant_id, course_id, title, sort_order, created_at",
    )
    .bind(tenant_id)
    .bind(course_id)
    .bind(id)
    .bind(new_title)
    .fetch_optional(&mut **tx)
    .await
}

pub async fn delete_module(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    course_id: Uuid,
    id: Uuid,
) -> sqlx::Result<bool> {
    Ok(
        sqlx::query("DELETE FROM modules WHERE id = $3 AND course_id = $2 AND tenant_id = $1")
            .bind(tenant_id)
            .bind(course_id)
            .bind(id)
            .execute(&mut **tx)
            .await?
            .rows_affected()
            > 0,
    )
}
