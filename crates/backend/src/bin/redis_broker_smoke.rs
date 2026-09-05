// crates/backend/src/bin/redis_broker_smoke.rs
//! One-shot smoke test for the Redis live-room broker. Closes out the
//! Phase 1b-γ §4b manual verification.
//!
//! Usage:
//!   REDIS_URL=redis://localhost:56379 cargo run -p backend --bin redis_broker_smoke
//!
//! Exit code 0 on success; non-zero with an error message on failure.

use std::time::Duration;

use backend::services::live_room::{BrokerEvent, LiveRoomBroker};
use backend::services::live_room_redis::RedisLiveRoomBroker;

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://localhost:56379".to_string());
    eprintln!("connecting to {url}…");

    let broker = RedisLiveRoomBroker::connect(&url).await?;
    let session_id = uuid::Uuid::new_v4();
    eprintln!("session_id: {session_id}");

    let mut subscription = broker.subscribe(session_id).await?;

    let chat_id = uuid::Uuid::new_v4();
    let event = BrokerEvent::Chat {
        id: chat_id,
        sender_user_id: uuid::Uuid::new_v4(),
        sender_display_name: "smoke-test".to_string(),
        body: "hello from smoke test".to_string(),
        created_at: chrono::Utc::now(),
    };
    broker.publish(session_id, event).await?;
    eprintln!("published");

    let received = tokio::time::timeout(Duration::from_secs(5), subscription.rx.recv())
        .await
        .map_err(|_| anyhow::anyhow!("timed out waiting for round-trip"))?
        .ok_or_else(|| anyhow::anyhow!("subscriber closed before message arrived"))?;

    match received {
        BrokerEvent::Chat { body, id, .. } => {
            if body != "hello from smoke test" || id != chat_id {
                anyhow::bail!("payload mismatch: got body={body:?} id={id}");
            }
        }
        other => anyhow::bail!("expected Chat event, got {other:?}"),
    }

    eprintln!("OK — round-trip successful");
    Ok(())
}
