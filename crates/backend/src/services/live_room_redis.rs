// crates/backend/src/services/live_room_redis.rs
//! Production LiveRoomBroker using `fred` against the self-hosted Redis.
//!
//! API adaptation notes (fred 10.1.0):
//! - Client type is `fred::clients::Client` (not `RedisClient`)
//! - Config type is `fred::types::config::Config` (not `RedisConfig`)
//! - Builder is in `fred::types::Builder` (re-exported in prelude)
//! - `client.publish(channel, payload)` returns `FredResult<i64>` — annotated
//! - `SubscriberClient::message_rx()` returns `tokio::sync::broadcast::Receiver<Message>`
//! - `zadd` takes `(key, options, ordering, changed, incr, values)` where values
//!   implements `TryInto<MultipleZaddValues>` — we pass `(score_f64, member_string)` tuple
//! - `client.set(key, value, expiration, options, get)` with `Expiration::EX(secs)`
//! - `client.exists(keys)` returns `FredResult<i64>` (count of existing keys)

use super::live_room::{
    BreakoutState, BrokerError, BrokerEvent, BrokerSubscription, HandRaiseEntry, LiveRoomBroker,
    PollState, PollVoteOutcome, PresenceEntry, WhiteboardStroke,
};
use async_trait::async_trait;
use fred::clients::SubscriberClient;
use fred::interfaces::EventInterface;
use fred::prelude::{
    Builder, Client, ClientLike, Expiration, KeysInterface, ListInterface, LuaInterface,
    PubsubInterface, SetOptions, SortedSetsInterface,
};
use fred::types::config::Config;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use uuid::Uuid;

// ── key helpers ────────────────────────────────────────────────────────────────

fn simple(id: Uuid) -> String {
    id.simple().to_string()
}

fn events_chan(session_id: Uuid) -> String {
    format!("aulalite:room:{}:events", simple(session_id))
}

fn legacy_presence_key(session_id: Uuid) -> String {
    format!("aulalite:room:{}:presence", simple(session_id))
}

/// Presence v2 stores stable user ids in the sorted set. The original format
/// stored the full JSON entry (including a changing heartbeat timestamp) as the
/// member, so every heartbeat inserted a duplicate and made later operations
/// O(room_size × heartbeats). A versioned key avoids misreading pre-deploy data;
/// active sockets reconnect after a backend rollout and populate v2.
fn presence_key(session_id: Uuid) -> String {
    format!("aulalite:room:{}:presence:v2", simple(session_id))
}

fn presence_details_key(session_id: Uuid) -> String {
    format!("aulalite:room:{}:presence:v2:details", simple(session_id))
}

fn queue_key(session_id: Uuid) -> String {
    format!("aulalite:room:{}:hand_queue", simple(session_id))
}

fn whiteboard_key(session_id: Uuid) -> String {
    format!("aulalite:room:{}:whiteboard", simple(session_id))
}

fn draw_open_key(session_id: Uuid) -> String {
    format!("aulalite:room:{}:draw_open", simple(session_id))
}

fn kicks_key(session_id: Uuid, user_id: Uuid) -> String {
    format!(
        "aulalite:room:{}:kicks:{}",
        simple(session_id),
        simple(user_id)
    )
}

/// JSON-serialized `PollState` for the session's currently-active poll.
fn poll_state_key(session_id: Uuid) -> String {
    format!("aulalite:room:{}:poll", simple(session_id))
}

/// SET of voter UUIDs for a specific poll (keyed by poll id so a replaced
/// poll's voters can't leak into a new poll).
fn poll_voters_key(session_id: Uuid, poll_id: Uuid) -> String {
    format!(
        "aulalite:room:{}:poll:{}:voters",
        simple(session_id),
        simple(poll_id)
    )
}

/// TTL guard so an abandoned poll (teacher closes the tab without ending it)
/// eventually evicts itself from Redis rather than lingering for the key's life.
const POLL_TTL_SECS: i64 = 6 * 3600;

/// JSON-serialized `BreakoutState` for the session's current breakout layout.
fn breakout_key(session_id: Uuid) -> String {
    format!("aulalite:room:{}:breakout", simple(session_id))
}

/// TTL guard so an abandoned breakout layout evicts itself if the teacher never
/// closes it (matches the poll TTL convention).
const BREAKOUT_TTL_SECS: i64 = 6 * 3600;

/// Longer than the maximum class duration, but finite so a process crash cannot
/// strand room presence forever when the normal end-of-class cleanup is missed.
const PRESENCE_TTL_SECS: i64 = 24 * 3600;

const PRESENCE_JOIN_SCRIPT: &str = r#"
redis.call('HSET', KEYS[2], ARGV[1], ARGV[3])
redis.call('ZADD', KEYS[1], ARGV[2], ARGV[1])
redis.call('EXPIRE', KEYS[1], ARGV[4])
redis.call('EXPIRE', KEYS[2], ARGV[4])
return 1
"#;

const PRESENCE_LEAVE_SCRIPT: &str = r#"
local removed = redis.call('ZREM', KEYS[1], ARGV[1])
redis.call('HDEL', KEYS[2], ARGV[1])
return removed
"#;

const PRESENCE_LIST_SCRIPT: &str = r#"
local members = redis.call('ZRANGE', KEYS[1], 0, -1, 'WITHSCORES')
local result = {}
for i = 1, #members, 2 do
    local detail = redis.call('HGET', KEYS[2], members[i])
    if detail then
        result[#result + 1] = detail
        result[#result + 1] = members[i + 1]
    end
end
return result
"#;

const PRESENCE_EVICT_SCRIPT: &str = r#"
local members = redis.call('ZRANGEBYSCORE', KEYS[1], '-inf', ARGV[1])
if #members == 0 then
    return 0
end
redis.call('ZREMRANGEBYSCORE', KEYS[1], '-inf', ARGV[1])
redis.call('HDEL', KEYS[2], unpack(members))
return #members
"#;

const WHITEBOARD_APPEND_SCRIPT: &str = r#"
redis.call('RPUSH', KEYS[1], ARGV[1])
redis.call('LTRIM', KEYS[1], -tonumber(ARGV[2]), -1)
return 1
"#;

/// Validate, deduplicate, and tally a poll vote in one Redis operation. The
/// former GET/SADD/EXPIRE/SET sequence could lose increments when many students
/// voted at once because each writer replaced the whole JSON count vector.
const POLL_VOTE_SCRIPT: &str = r#"
local raw = redis.call('GET', KEYS[1])
if not raw then
    return {-1}
end

local decoded, state = pcall(cjson.decode, raw)
local option = tonumber(ARGV[3])
if not decoded
    or state.poll_id ~= ARGV[1]
    or not option
    or option < 1
    or option > tonumber(state.num_options) then
    return {-1}
end

local added = redis.call('SADD', KEYS[2], ARGV[2])
redis.call('EXPIRE', KEYS[2], ARGV[4])
if added == 1 then
    state.counts[option] = (tonumber(state.counts[option]) or 0) + 1
    redis.call('SET', KEYS[1], cjson.encode(state), 'EX', ARGV[4])
end

return {added, cjson.encode(state.counts)}
"#;

fn decode_presence(detail: &str, last_seen_ms: i64) -> Result<PresenceEntry, BrokerError> {
    let mut entry: PresenceEntry = serde_json::from_str(detail)
        .map_err(|e| BrokerError::Payload(format!("presence decode: {e}")))?;
    // The sorted-set score is authoritative and changes on every heartbeat;
    // keeping the hash payload stable makes heartbeat a single O(log N) ZADD.
    entry.last_seen_ms = last_seen_ms;
    Ok(entry)
}

// ── broker struct ──────────────────────────────────────────────────────────────

/// Production broker backed by a self-hosted Redis instance.
///
/// Clone is cheap — the inner `fred::clients::Client` is an `Arc`-wrapped handle.
#[derive(Clone)]
pub struct RedisLiveRoomBroker {
    client: Arc<Client>,
    url: Arc<String>,
}

impl RedisLiveRoomBroker {
    /// Connect to Redis at `url` (e.g. `"redis://127.0.0.1:6379"`).
    pub async fn connect(url: impl Into<String>) -> Result<Self, BrokerError> {
        let url = url.into();
        let config = Config::from_url(&url)
            .map_err(|e| BrokerError::Transport(format!("redis url parse: {e}")))?;
        let client = Builder::from_config(config)
            .build()
            .map_err(|e| BrokerError::Transport(format!("redis build: {e}")))?;
        client
            .init()
            .await
            .map_err(|e| BrokerError::Transport(format!("redis connect: {e}")))?;
        Ok(Self {
            client: Arc::new(client),
            url: Arc::new(url),
        })
    }

    /// Build a fresh `SubscriberClient` from the stored URL (needed per-subscription
    /// because a `SubscriberClient` blocks its connection for pubsub).
    fn new_subscriber(&self) -> Result<SubscriberClient, BrokerError> {
        let config = Config::from_url(&self.url)
            .map_err(|e| BrokerError::Transport(format!("redis url parse: {e}")))?;
        Builder::from_config(config)
            .build_subscriber_client()
            .map_err(|e| BrokerError::Transport(format!("subscriber build: {e}")))
    }

    /// Read the session's active poll state, or `None` when there is no poll.
    async fn read_poll_state(&self, session_id: Uuid) -> Result<Option<PollState>, BrokerError> {
        let raw: Option<fred::prelude::Value> =
            self.client
                .get(poll_state_key(session_id))
                .await
                .map_err(|e| BrokerError::Transport(e.to_string()))?;
        let Some(s) = raw.and_then(|v| v.into_string()) else {
            return Ok(None);
        };
        let state: PollState = serde_json::from_str(&s)
            .map_err(|e| BrokerError::Payload(format!("poll decode: {e}")))?;
        Ok(Some(state))
    }
}

// ── trait impl ─────────────────────────────────────────────────────────────────

#[async_trait]
impl LiveRoomBroker for RedisLiveRoomBroker {
    async fn healthz(&self) -> Result<(), BrokerError> {
        // A read of a dedicated nonexistent key is side-effect free while still
        // exercising the multiplexed Redis connection end to end.
        let _: Option<fred::prelude::Value> = self
            .client
            .get("aulalite:healthz")
            .await
            .map_err(|e| BrokerError::Transport(e.to_string()))?;
        Ok(())
    }

    // ── publish / subscribe ──────────────────────────────────────────────────

    async fn publish(&self, session_id: Uuid, event: BrokerEvent) -> Result<(), BrokerError> {
        let payload =
            serde_json::to_string(&event).map_err(|e| BrokerError::Payload(e.to_string()))?;
        let chan = events_chan(session_id);
        // publish returns i64 (number of subscribers that received it); we discard
        let _: i64 = self
            .client
            .publish(chan, payload)
            .await
            .map_err(|e| BrokerError::Transport(e.to_string()))?;
        Ok(())
    }

    async fn subscribe(&self, session_id: Uuid) -> Result<BrokerSubscription, BrokerError> {
        let sub = self.new_subscriber()?;
        drop(
            sub.init()
                .await
                .map_err(|e| BrokerError::Transport(format!("subscriber init: {e}")))?,
        );
        let chan = events_chan(session_id);
        let _: () = sub
            .subscribe(chan.clone())
            .await
            .map_err(|e| BrokerError::Transport(format!("subscribe: {e}")))?;

        // `message_rx()` returns `tokio::sync::broadcast::Receiver<fred::protocol::types::Message>`
        let mut msg_rx = sub.message_rx();
        let (tx, rx) = mpsc::channel::<BrokerEvent>(64);

        tokio::spawn(async move {
            // Keep the subscriber alive as long as the pump task runs
            let _sub = sub;
            while let Ok(msg) = msg_rx.recv().await {
                let s: String = match msg.value.into_string() {
                    Some(s) => s,
                    None => continue,
                };
                match serde_json::from_str::<BrokerEvent>(&s) {
                    Ok(evt) => {
                        if tx.send(evt).await.is_err() {
                            break; // receiver dropped
                        }
                    }
                    Err(_) => continue,
                }
            }
        });

        Ok(BrokerSubscription { rx })
    }

    // ── presence ─────────────────────────────────────────────────────────────

    async fn presence_join(
        &self,
        session_id: Uuid,
        entry: PresenceEntry,
    ) -> Result<(), BrokerError> {
        let user_id = simple(entry.user_id);
        let score = entry.last_seen_ms.to_string();
        let member =
            serde_json::to_string(&entry).map_err(|e| BrokerError::Payload(e.to_string()))?;
        let _: i64 = self
            .client
            .eval(
                PRESENCE_JOIN_SCRIPT,
                vec![presence_key(session_id), presence_details_key(session_id)],
                vec![user_id, score, member, PRESENCE_TTL_SECS.to_string()],
            )
            .await
            .map_err(|e| BrokerError::Transport(e.to_string()))?;
        Ok(())
    }

    async fn presence_heartbeat(&self, session_id: Uuid, user_id: Uuid) -> Result<(), BrokerError> {
        // Stable user-id members let a heartbeat update one score in O(log N),
        // instead of reading/deserializing the full room and appending a new
        // timestamped JSON member on every heartbeat.
        let _: i64 = self
            .client
            .zadd(
                presence_key(session_id),
                Some(SetOptions::XX),
                None,
                false,
                false,
                (
                    chrono::Utc::now().timestamp_millis() as f64,
                    simple(user_id),
                ),
            )
            .await
            .map_err(|e| BrokerError::Transport(e.to_string()))?;
        Ok(())
    }

    async fn presence_leave(&self, session_id: Uuid, user_id: Uuid) -> Result<(), BrokerError> {
        let _: i64 = self
            .client
            .eval(
                PRESENCE_LEAVE_SCRIPT,
                vec![presence_key(session_id), presence_details_key(session_id)],
                vec![simple(user_id)],
            )
            .await
            .map_err(|e| BrokerError::Transport(e.to_string()))?;
        Ok(())
    }

    async fn presence_count(&self, session_id: Uuid) -> Result<u32, BrokerError> {
        let key = presence_key(session_id);
        let count: i64 = self
            .client
            .zcard(key)
            .await
            .map_err(|e| BrokerError::Transport(e.to_string()))?;
        Ok(count as u32)
    }

    async fn presence_list(&self, session_id: Uuid) -> Result<Vec<PresenceEntry>, BrokerError> {
        // Fetch the consistent sorted-set/hash snapshot in one round trip.
        let raw: Vec<fred::prelude::Value> = self
            .client
            .eval(
                PRESENCE_LIST_SCRIPT,
                vec![presence_key(session_id), presence_details_key(session_id)],
                Vec::<String>::new(),
            )
            .await
            .map_err(|e| BrokerError::Transport(e.to_string()))?;
        if !raw.len().is_multiple_of(2) {
            return Err(BrokerError::Payload(
                "odd presence snapshot response".into(),
            ));
        }
        let mut entries = Vec::with_capacity(raw.len() / 2);
        let mut values = raw.into_iter();
        while let Some(detail) = values.next() {
            let detail = detail
                .into_string()
                .ok_or_else(|| BrokerError::Payload("non-string presence detail".into()))?;
            let score = values
                .next()
                .and_then(|value| value.as_i64())
                .ok_or_else(|| BrokerError::Payload("invalid presence score".into()))?;
            entries.push(decode_presence(&detail, score)?);
        }
        Ok(entries)
    }

    async fn presence_evict_stale(
        &self,
        session_id: Uuid,
        older_than_ms: i64,
    ) -> Result<u32, BrokerError> {
        let count: i64 = self
            .client
            .eval(
                PRESENCE_EVICT_SCRIPT,
                vec![presence_key(session_id), presence_details_key(session_id)],
                vec![older_than_ms.to_string()],
            )
            .await
            .map_err(|e| BrokerError::Transport(e.to_string()))?;
        Ok(count as u32)
    }

    // ── hand queue ────────────────────────────────────────────────────────────

    async fn hand_raise(&self, session_id: Uuid, user_id: Uuid) -> Result<u32, BrokerError> {
        // Idempotent: only push if not already queued
        let existing = self.hand_queue(session_id).await?;
        if existing.iter().any(|e| e.user_id == user_id) {
            return Ok(existing.len() as u32);
        }
        let entry = HandRaiseEntry {
            user_id,
            display_name: String::new(),
            raised_at_ms: chrono::Utc::now().timestamp_millis(),
        };
        let payload =
            serde_json::to_string(&entry).map_err(|e| BrokerError::Payload(e.to_string()))?;
        let key = queue_key(session_id);
        let new_len: i64 = self
            .client
            .rpush(key, payload)
            .await
            .map_err(|e| BrokerError::Transport(e.to_string()))?;
        Ok(new_len as u32)
    }

    async fn hand_lower(&self, session_id: Uuid, user_id: Uuid) -> Result<(), BrokerError> {
        let entries = self.hand_queue(session_id).await?;
        let key = queue_key(session_id);
        for entry in entries.into_iter().filter(|e| e.user_id == user_id) {
            let payload =
                serde_json::to_string(&entry).map_err(|e| BrokerError::Payload(e.to_string()))?;
            // lrem(key, count=1, element) — removes first occurrence
            let _: i64 = self
                .client
                .lrem(key.clone(), 1, payload)
                .await
                .map_err(|e| BrokerError::Transport(e.to_string()))?;
        }
        Ok(())
    }

    async fn hand_queue(&self, session_id: Uuid) -> Result<Vec<HandRaiseEntry>, BrokerError> {
        let key = queue_key(session_id);
        let raw: Vec<fred::prelude::Value> = self
            .client
            .lrange(key, 0, -1)
            .await
            .map_err(|e| BrokerError::Transport(e.to_string()))?;
        let mut entries = Vec::with_capacity(raw.len());
        for v in raw {
            let s = v
                .into_string()
                .ok_or_else(|| BrokerError::Payload("non-string lrange element".into()))?;
            let entry: HandRaiseEntry = serde_json::from_str(&s)
                .map_err(|e| BrokerError::Payload(format!("hand_queue decode: {e}")))?;
            entries.push(entry);
        }
        Ok(entries)
    }

    // ── kick ──────────────────────────────────────────────────────────────────

    async fn kick_set(
        &self,
        session_id: Uuid,
        user_id: Uuid,
        ttl: Duration,
    ) -> Result<(), BrokerError> {
        let key = kicks_key(session_id, user_id);
        let secs = ttl.as_secs().max(1) as i64;
        // set(key, value, expiration, options, get)
        let _: fred::prelude::Value = self
            .client
            .set(key, "1", Some(Expiration::EX(secs)), None, false)
            .await
            .map_err(|e| BrokerError::Transport(e.to_string()))?;
        Ok(())
    }

    async fn is_kicked(&self, session_id: Uuid, user_id: Uuid) -> Result<bool, BrokerError> {
        let key = kicks_key(session_id, user_id);
        let count: i64 = self
            .client
            .exists(key)
            .await
            .map_err(|e| BrokerError::Transport(e.to_string()))?;
        Ok(count > 0)
    }

    // ── whiteboard history ───────────────────────────────────────────────────

    async fn whiteboard_append(
        &self,
        session_id: Uuid,
        stroke: WhiteboardStroke,
    ) -> Result<(), BrokerError> {
        let key = whiteboard_key(session_id);
        let payload =
            serde_json::to_string(&stroke).map_err(|e| BrokerError::Payload(e.to_string()))?;
        let _: i64 = self
            .client
            .eval(
                WHITEBOARD_APPEND_SCRIPT,
                vec![key],
                vec![
                    payload,
                    crate::services::live_room::MAX_WHITEBOARD_STROKES.to_string(),
                ],
            )
            .await
            .map_err(|e| BrokerError::Transport(e.to_string()))?;
        Ok(())
    }

    async fn whiteboard_remove_last(
        &self,
        session_id: Uuid,
    ) -> Result<Option<String>, BrokerError> {
        let key = whiteboard_key(session_id);
        let raw: Option<fred::prelude::Value> = self
            .client
            .rpop(key, None)
            .await
            .map_err(|e| BrokerError::Transport(e.to_string()))?;
        let Some(raw) = raw else { return Ok(None) };
        let s = raw
            .into_string()
            .ok_or_else(|| BrokerError::Payload("non-string rpop element".into()))?;
        let stroke: WhiteboardStroke = serde_json::from_str(&s)
            .map_err(|e| BrokerError::Payload(format!("whiteboard decode: {e}")))?;
        Ok(Some(stroke.id))
    }

    async fn whiteboard_remove_last_by_author(
        &self,
        session_id: Uuid,
        author: Uuid,
    ) -> Result<Option<crate::services::live_room::WhiteboardStroke>, BrokerError> {
        // Strokes are stored as serialized JSON list elements, oldest first.
        // Re-read the list, find the LAST element authored by `author`, then
        // LREM that exact serialized value with count = -1 (remove the last
        // occurrence, walking from the tail) so the most-recent stroke is the
        // one dropped even when an identical serialization appears earlier.
        let key = whiteboard_key(session_id);
        let raw: Vec<fred::prelude::Value> = self
            .client
            .lrange(key.clone(), 0, -1)
            .await
            .map_err(|e| BrokerError::Transport(e.to_string()))?;
        let mut found: Option<(String, crate::services::live_room::WhiteboardStroke)> = None;
        for v in raw {
            let Some(s) = v.into_string() else {
                continue;
            };
            let stroke: crate::services::live_room::WhiteboardStroke =
                match serde_json::from_str(&s) {
                    Ok(stroke) => stroke,
                    Err(_) => continue,
                };
            if stroke.author == Some(author) {
                // Keep overwriting so the final hit is the most-recent stroke.
                found = Some((s, stroke));
            }
        }
        let Some((serialized, stroke)) = found else {
            return Ok(None);
        };
        let removed: i64 = self
            .client
            .lrem(key, -1, serialized)
            .await
            .map_err(|e| BrokerError::Transport(e.to_string()))?;
        if removed > 0 {
            Ok(Some(stroke))
        } else {
            Ok(None)
        }
    }

    async fn whiteboard_remove_by_id(
        &self,
        session_id: Uuid,
        stroke_id: &str,
    ) -> Result<bool, BrokerError> {
        // Strokes are stored as serialized JSON list elements. Re-read the
        // list, find the matching element by its decoded id, then LREM that
        // exact serialized value (count = 0 removes all occurrences, of which
        // there is at most one since ids are unique).
        let key = whiteboard_key(session_id);
        let raw: Vec<fred::prelude::Value> = self
            .client
            .lrange(key.clone(), 0, -1)
            .await
            .map_err(|e| BrokerError::Transport(e.to_string()))?;
        for v in raw {
            let Some(s) = v.into_string() else {
                continue;
            };
            let stroke: WhiteboardStroke = match serde_json::from_str(&s) {
                Ok(stroke) => stroke,
                Err(_) => continue,
            };
            if stroke.id == stroke_id {
                let removed: i64 = self
                    .client
                    .lrem(key, 0, s)
                    .await
                    .map_err(|e| BrokerError::Transport(e.to_string()))?;
                return Ok(removed > 0);
            }
        }
        Ok(false)
    }

    async fn whiteboard_strokes(
        &self,
        session_id: Uuid,
    ) -> Result<Vec<WhiteboardStroke>, BrokerError> {
        let key = whiteboard_key(session_id);
        let raw: Vec<fred::prelude::Value> = self
            .client
            .lrange(key, 0, -1)
            .await
            .map_err(|e| BrokerError::Transport(e.to_string()))?;
        let mut strokes = Vec::with_capacity(raw.len());
        for v in raw {
            let s = v
                .into_string()
                .ok_or_else(|| BrokerError::Payload("non-string lrange element".into()))?;
            let stroke: WhiteboardStroke = serde_json::from_str(&s)
                .map_err(|e| BrokerError::Payload(format!("whiteboard decode: {e}")))?;
            strokes.push(stroke);
        }
        Ok(strokes)
    }

    async fn whiteboard_clear(&self, session_id: Uuid) -> Result<(), BrokerError> {
        let _: i64 = self
            .client
            .del(whiteboard_key(session_id))
            .await
            .map_err(|e| BrokerError::Transport(e.to_string()))?;
        Ok(())
    }

    // ── draw permission flag ───────────────────────────────────────────────

    async fn set_draw_open(&self, session_id: Uuid, open: bool) -> Result<(), BrokerError> {
        let key = draw_open_key(session_id);
        // Store "1"/"0" with no expiry; cleared by clear_room_state on end.
        let _: fred::prelude::Value = self
            .client
            .set(key, if open { "1" } else { "0" }, None, None, false)
            .await
            .map_err(|e| BrokerError::Transport(e.to_string()))?;
        Ok(())
    }

    async fn draw_open(&self, session_id: Uuid) -> Result<bool, BrokerError> {
        let key = draw_open_key(session_id);
        let raw: Option<fred::prelude::Value> = self
            .client
            .get(key)
            .await
            .map_err(|e| BrokerError::Transport(e.to_string()))?;
        // Absent flag → teacher-only (false). Any value other than "1" is false.
        Ok(raw
            .and_then(|v| v.into_string())
            .map(|s| s == "1")
            .unwrap_or(false))
    }

    async fn clear_room_state(&self, session_id: Uuid) -> Result<(), BrokerError> {
        // Drop the active poll first (its voter set is keyed by poll id, so we
        // read the state to learn the id before deleting). Best-effort: a read
        // failure must not block the rest of the room teardown.
        if let Ok(Some(state)) = self.read_poll_state(session_id).await {
            let _: Result<i64, _> = self
                .client
                .del(poll_voters_key(session_id, state.poll_id))
                .await;
        }
        let keys = vec![
            whiteboard_key(session_id),
            presence_key(session_id),
            presence_details_key(session_id),
            legacy_presence_key(session_id),
            queue_key(session_id),
            draw_open_key(session_id),
            poll_state_key(session_id),
            breakout_key(session_id),
        ];
        let _: i64 = self
            .client
            .del(keys)
            .await
            .map_err(|e| BrokerError::Transport(e.to_string()))?;
        Ok(())
    }

    // ── in-class polls (ephemeral) ──────────────────────────────────────────

    async fn poll_start(
        &self,
        session_id: Uuid,
        poll_id: Uuid,
        num_options: usize,
    ) -> Result<(), BrokerError> {
        // Replace any prior poll: drop the previous voter set so a new poll
        // starts clean, then write the fresh state with a TTL guard.
        if let Ok(Some(prev)) = self.read_poll_state(session_id).await {
            let _: Result<i64, _> = self
                .client
                .del(poll_voters_key(session_id, prev.poll_id))
                .await;
        }
        let state = PollState {
            poll_id,
            num_options,
            counts: vec![0; num_options],
        };
        let payload =
            serde_json::to_string(&state).map_err(|e| BrokerError::Payload(e.to_string()))?;
        let _: fred::prelude::Value = self
            .client
            .set(
                poll_state_key(session_id),
                payload,
                Some(Expiration::EX(POLL_TTL_SECS)),
                None,
                false,
            )
            .await
            .map_err(|e| BrokerError::Transport(e.to_string()))?;
        Ok(())
    }

    async fn poll_vote(
        &self,
        session_id: Uuid,
        poll_id: Uuid,
        user_id: Uuid,
        option_index: usize,
    ) -> Result<Option<PollVoteOutcome>, BrokerError> {
        let raw: Vec<fred::prelude::Value> = self
            .client
            .eval(
                POLL_VOTE_SCRIPT,
                vec![
                    poll_state_key(session_id),
                    poll_voters_key(session_id, poll_id),
                ],
                vec![
                    poll_id.to_string(),
                    simple(user_id),
                    (option_index + 1).to_string(),
                    POLL_TTL_SECS.to_string(),
                ],
            )
            .await
            .map_err(|e| BrokerError::Transport(e.to_string()))?;
        let status = raw.first().and_then(|value| value.as_i64()).unwrap_or(-1);
        if status < 0 {
            return Ok(None);
        }
        let counts_json = raw
            .get(1)
            .and_then(|value| value.clone().into_string())
            .ok_or_else(|| BrokerError::Payload("poll vote missing tally".into()))?;
        let counts: Vec<u32> = serde_json::from_str(&counts_json)
            .map_err(|e| BrokerError::Payload(format!("poll tally decode: {e}")))?;
        Ok(Some(PollVoteOutcome {
            counted: status == 1,
            counts,
        }))
    }

    async fn poll_end(
        &self,
        session_id: Uuid,
        poll_id: Uuid,
    ) -> Result<Option<Vec<u32>>, BrokerError> {
        let Some(state) = self.read_poll_state(session_id).await? else {
            return Ok(None);
        };
        if state.poll_id != poll_id {
            return Ok(None);
        }
        // Drop both the state and the voter set.
        let _: i64 = self
            .client
            .del(vec![
                poll_state_key(session_id),
                poll_voters_key(session_id, poll_id),
            ])
            .await
            .map_err(|e| BrokerError::Transport(e.to_string()))?;
        Ok(Some(state.counts))
    }

    // ── breakout rooms (ephemeral) ──────────────────────────────────────────

    async fn breakout_get(&self, session_id: Uuid) -> Result<BreakoutState, BrokerError> {
        let raw: Option<fred::prelude::Value> = self
            .client
            .get(breakout_key(session_id))
            .await
            .map_err(|e| BrokerError::Transport(e.to_string()))?;
        let Some(s) = raw.and_then(|v| v.into_string()) else {
            return Ok(BreakoutState::default());
        };
        let state: BreakoutState = serde_json::from_str(&s)
            .map_err(|e| BrokerError::Payload(format!("breakout decode: {e}")))?;
        Ok(state)
    }

    async fn breakout_set(
        &self,
        session_id: Uuid,
        state: BreakoutState,
    ) -> Result<(), BrokerError> {
        let payload =
            serde_json::to_string(&state).map_err(|e| BrokerError::Payload(e.to_string()))?;
        let _: fred::prelude::Value = self
            .client
            .set(
                breakout_key(session_id),
                payload,
                Some(Expiration::EX(BREAKOUT_TTL_SECS)),
                None,
                false,
            )
            .await
            .map_err(|e| BrokerError::Transport(e.to_string()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presence_keys_are_versioned_away_from_legacy_json_members() {
        let session = Uuid::nil();
        assert_ne!(presence_key(session), legacy_presence_key(session));
        assert!(presence_key(session).ends_with(":presence:v2"));
        assert!(presence_details_key(session).ends_with(":presence:v2:details"));
    }

    #[test]
    fn sorted_set_score_overrides_stale_presence_payload_timestamp() {
        let user_id = Uuid::new_v4();
        let detail = serde_json::to_string(&PresenceEntry {
            user_id,
            display_name: "Ada".into(),
            role: "teacher".into(),
            last_seen_ms: 10,
        })
        .unwrap();

        let decoded = decode_presence(&detail, 42).unwrap();
        assert_eq!(decoded.user_id, user_id);
        assert_eq!(decoded.display_name, "Ada");
        assert_eq!(decoded.last_seen_ms, 42);
    }
}
