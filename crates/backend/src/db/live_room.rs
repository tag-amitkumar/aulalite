// crates/backend/src/db/live_room.rs
use serde::Serialize;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct ChatMessageRow {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub session_id: Uuid,
    pub sender_user_id: Uuid,
    pub body: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub deleted_at: Option<chrono::DateTime<chrono::Utc>>,
    pub deleted_by_user_id: Option<Uuid>,
}

pub async fn insert_message(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    session_id: Uuid,
    sender_user_id: Uuid,
    body: &str,
) -> sqlx::Result<ChatMessageRow> {
    sqlx::query_as::<_, ChatMessageRow>(
        "INSERT INTO live_room_messages
            (tenant_id, session_id, sender_user_id, body)
         VALUES ($1, $2, $3, $4)
         RETURNING id, tenant_id, session_id, sender_user_id, body, created_at,
                   deleted_at, deleted_by_user_id",
    )
    .bind(tenant_id)
    .bind(session_id)
    .bind(sender_user_id)
    .bind(body)
    .fetch_one(&mut **tx)
    .await
}

pub async fn fetch_paginated<'e, E>(
    executor: E,
    session_id: Uuid,
    before: Option<Uuid>,
    limit: i64,
) -> sqlx::Result<Vec<ChatMessageRow>>
where
    E: sqlx::PgExecutor<'e>,
{
    let limit = limit.clamp(1, 200);
    if let Some(cursor) = before {
        sqlx::query_as::<_, ChatMessageRow>(
            "SELECT id, tenant_id, session_id, sender_user_id, body, created_at,
                    deleted_at, deleted_by_user_id
               FROM live_room_messages
              WHERE session_id = $1
                AND created_at < (SELECT created_at FROM live_room_messages WHERE id = $2)
              ORDER BY created_at DESC
              LIMIT $3",
        )
        .bind(session_id)
        .bind(cursor)
        .bind(limit)
        .fetch_all(executor)
        .await
    } else {
        sqlx::query_as::<_, ChatMessageRow>(
            "SELECT id, tenant_id, session_id, sender_user_id, body, created_at,
                    deleted_at, deleted_by_user_id
               FROM live_room_messages
              WHERE session_id = $1
              ORDER BY created_at DESC
              LIMIT $2",
        )
        .bind(session_id)
        .bind(limit)
        .fetch_all(executor)
        .await
    }
}

pub async fn soft_delete(
    tx: &mut Transaction<'_, Postgres>,
    message_id: Uuid,
    deleted_by: Uuid,
) -> sqlx::Result<Option<ChatMessageRow>> {
    sqlx::query_as::<_, ChatMessageRow>(
        "UPDATE live_room_messages
            SET deleted_at = now(), deleted_by_user_id = $2
          WHERE id = $1 AND deleted_at IS NULL
        RETURNING id, tenant_id, session_id, sender_user_id, body, created_at,
                  deleted_at, deleted_by_user_id",
    )
    .bind(message_id)
    .bind(deleted_by)
    .fetch_optional(&mut **tx)
    .await
}

pub async fn prune_older_than(pool: &PgPool, days: i64) -> sqlx::Result<u64> {
    // Retention is a cross-tenant background concern. A bare pool is filtered
    // to zero rows by FORCE RLS under the production app role.
    let mut tx = crate::db::begin_system_context(pool).await?;
    let res = sqlx::query(
        "DELETE FROM live_room_messages
          WHERE created_at < now() - ($1::int || ' days')::interval",
    )
    .bind(days as i32)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(res.rows_affected())
}

pub async fn insert_kick(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    session_id: Uuid,
    user_id: Uuid,
    kicked_by_user_id: Uuid,
) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO live_room_kicks
            (tenant_id, session_id, user_id, kicked_by_user_id)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (session_id, user_id) DO NOTHING",
    )
    .bind(tenant_id)
    .bind(session_id)
    .bind(user_id)
    .bind(kicked_by_user_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub async fn kick_exists<'e, E>(executor: E, session_id: Uuid, user_id: Uuid) -> sqlx::Result<bool>
where
    E: sqlx::PgExecutor<'e>,
{
    let row: Option<(Uuid,)> =
        sqlx::query_as("SELECT id FROM live_room_kicks WHERE session_id = $1 AND user_id = $2")
            .bind(session_id)
            .bind(user_id)
            .fetch_optional(executor)
            .await?;
    Ok(row.is_some())
}

/// Inserts/updates the per-student publish nonce hash on a live session.
pub async fn set_student_publish_nonce(
    tx: &mut Transaction<'_, Postgres>,
    session_id: Uuid,
    student_user_id: Uuid,
    nonce_hash: &str,
    expires_at: chrono::DateTime<chrono::Utc>,
) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE live_sessions
            SET student_publish_nonces = jsonb_set(
                student_publish_nonces,
                ARRAY[$2::text],
                jsonb_build_object('hash', $3::text, 'expires_at', $4::text)
            )
          WHERE id = $1",
    )
    .bind(session_id)
    .bind(student_user_id.simple().to_string())
    .bind(nonce_hash)
    .bind(expires_at.to_rfc3339())
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub async fn consume_student_publish_nonce(
    pool: &PgPool,
    session_id: Uuid,
    student_user_id: Uuid,
    candidate_hash: &str,
) -> sqlx::Result<bool> {
    // This is invoked by MediaMTX rather than an authenticated tenant request.
    // Keep the read and one-time consume in one system-scoped transaction so
    // production FORCE RLS does not silently turn promotion into a no-op.
    let mut tx = crate::db::begin_system_context(pool).await?;
    let row: Option<(serde_json::Value,)> =
        sqlx::query_as("SELECT student_publish_nonces FROM live_sessions WHERE id = $1")
            .bind(session_id)
            .fetch_optional(&mut *tx)
            .await?;
    let nonces = match row {
        Some((v,)) => v,
        None => return Ok(false),
    };
    let key = student_user_id.simple().to_string();
    let entry = nonces.get(&key);
    let stored_hash = entry
        .and_then(|e| e.get("hash"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let expires_at_str = entry
        .and_then(|e| e.get("expires_at"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if stored_hash != candidate_hash || stored_hash.is_empty() {
        return Ok(false);
    }
    let expires_at: chrono::DateTime<chrono::Utc> = expires_at_str
        .parse()
        .unwrap_or(chrono::DateTime::<chrono::Utc>::MIN_UTC);
    if expires_at < chrono::Utc::now() {
        return Ok(false);
    }
    let res = sqlx::query(
        "UPDATE live_sessions
            SET student_publish_nonces = student_publish_nonces - $2::text
          WHERE id = $1
            AND student_publish_nonces -> $2::text ->> 'hash' = $3::text",
    )
    .bind(session_id)
    .bind(&key)
    .bind(candidate_hash)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(res.rows_affected() > 0)
}

pub async fn clear_student_publish_nonce(
    tx: &mut Transaction<'_, Postgres>,
    session_id: Uuid,
    student_user_id: Uuid,
) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE live_sessions
            SET student_publish_nonces = student_publish_nonces - $2::text
          WHERE id = $1",
    )
    .bind(session_id)
    .bind(student_user_id.simple().to_string())
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Returns the user's display name. Falls back to the email local-part if
/// `users.display_name` is null, and to an empty string if the user does
/// not exist. Never errors on missing rows — callers want a best-effort
/// label for events.
pub async fn lookup_display_name<'e, E>(executor: E, user_id: Uuid) -> sqlx::Result<String>
where
    E: sqlx::PgExecutor<'e>,
{
    let row: Option<(Option<String>, String)> =
        sqlx::query_as("SELECT display_name, email FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_optional(executor)
            .await?;
    Ok(match row {
        Some((Some(name), _)) if !name.trim().is_empty() => name,
        Some((_, email)) => email.split('@').next().unwrap_or(&email).to_string(),
        None => String::new(),
    })
}
