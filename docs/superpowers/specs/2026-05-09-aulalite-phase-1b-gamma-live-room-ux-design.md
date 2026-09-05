# AulaLite Phase 1b-γ — Live Room UX Design

**Date:** 2026-05-09
**Phase:** 1b-γ
**Predecessor:** 1b-β (live class core, complete and pushed at `c0cf5c2`)
**Successor:** 1b-δ (recording — replays chat from `live_room_messages` alongside video)

## 1. Goal

Layer real-time interactivity on top of the 1b-β video pipeline: chat with persistence, asymmetric presence (teacher sees full list, students see count), hand-raise with audio promote (teacher accepts → student publishes their own audio track), basic moderation (rate-limit, teacher delete, teacher kick).

## 2. In scope

- WebSocket transport `/v1/sessions/:id/socket` with JSON envelope protocol.
- Persistent chat (`live_room_messages` table) with 90-day janitor.
- Asymmetric presence: count for everyone, full participant list for teachers/admins.
- Hand-raise queue (broker-resident, FIFO) with audio-only promote/demote flow.
- Rate-limiting: 1 chat message / 2s / user; 5 hand-raise toggles / minute / user.
- Teacher moderation: delete any message, kick any student.
- `LiveRoomBroker` trait (`MockLiveRoomBroker` for tests, `RedisLiveRoomBroker` for production using self-hosted Redis).
- Wildcard JWT path `aula/T/C/S/student/*` for unbounded concurrent promoted students.
- Per-student publish nonces stored as JSONB in `live_sessions.student_publish_nonces`.
- `live_room_kicks` table: cross-tenant RLS, blocks rejoin via cached check.
- Frontend: `live_room_socket`, `live_room_chat`, `live_room_presence`, `live_room_hand_raise`, `live_room_audio_publisher` modules.

## 3. Out of scope

| Feature | Phase / status |
|---|---|
| Profanity / word filter, slow mode, pinned messages, message edit, reactions | Phase 1b-γ+ or Phase 2 |
| Direct messages (DMs), private chat | Later |
| Polls, quizzes, whiteboard | Later |
| Video promote (camera, not just audio) | Phase 1b-γ+ |
| Mute-all, push-to-talk for general students | Later (audio is gated by hand-raise promote in 1b-γ) |
| Recording playback of chat | 1b-δ owns playback; 1b-γ provides the persisted source |

## 4. Architecture

### 4.1 Code organization

Pattern matches 1b-β (Approach 3): extend `handlers::live_sessions` with new routes; pull broker logic into a new `services::live_room` module.

**Backend:**
- `crates/backend/src/handlers/live_sessions.rs` — adds `/v1/sessions/:id/socket` (WebSocket upgrade) and `/v1/sessions/:id/messages` (history pagination).
- `crates/backend/src/services/live_room.rs` — NEW. `LiveRoomBroker` trait, broker event types, JSON envelope types, wildcard-match helper, rate-limit token-bucket helper. Includes `MockLiveRoomBroker`.
- `crates/backend/src/services/live_room_redis.rs` — NEW. `RedisLiveRoomBroker` production impl using `fred`.
- `crates/backend/src/db/live_room.rs` — NEW. SQL for `live_room_messages` (insert, fetch_paginated, soft_delete, prune_old) and `live_room_kicks` (insert, exists).
- `crates/backend/src/lib.rs` — `AppState` gains `live_room: Arc<dyn LiveRoomBroker>`.
- `crates/backend/src/main.rs` — daily janitor task for chat retention; spawn presence-eviction task (30s loop).

**Frontend (`crates/features-courses/src/`):**
- `live_room_socket.rs` — NEW. WebSocket client (wasm32-only).
- `live_room_chat.rs` — NEW.
- `live_room_presence.rs` — NEW.
- `live_room_hand_raise.rs` — NEW.
- `live_room_audio_publisher.rs` — NEW.
- `live_room_view.rs` — modify: integrate chat/presence/hand-raise sidebars + dynamic WHEP for promoted students.
- `live_room_broadcast.rs` — modify: integrate teacher sidebars + queue management.

**Infrastructure:**
- Migration `20260509000013_live_room_chat.sql` — adds `live_room_messages`, `live_room_kicks`, `live_sessions.student_publish_nonces`.
- No MediaMTX config change (Pattern B from 1b-β handles `action=read` already; 1b-γ extends backend-side wildcard matching).
- Redis (already provisioned in `docker-compose.yml`): activated via `fred` workspace dep.
- New env var: `REDIS_URL` (default `redis://redis:6379`).

### 4.2 LiveRoomBroker trait

```rust
#[async_trait]
pub trait LiveRoomBroker: Send + Sync {
    async fn publish(&self, session_id: Uuid, event: BrokerEvent) -> Result<(), BrokerError>;
    async fn subscribe(&self, session_id: Uuid) -> Result<BrokerSubscription, BrokerError>;
    async fn presence_join(&self, session_id: Uuid, entry: PresenceEntry) -> Result<(), BrokerError>;
    async fn presence_heartbeat(&self, session_id: Uuid, user_id: Uuid) -> Result<(), BrokerError>;
    async fn presence_leave(&self, session_id: Uuid, user_id: Uuid) -> Result<(), BrokerError>;
    async fn presence_count(&self, session_id: Uuid) -> Result<u32, BrokerError>;
    async fn presence_list(&self, session_id: Uuid) -> Result<Vec<PresenceEntry>, BrokerError>;
    async fn presence_evict_stale(&self, session_id: Uuid, older_than: Duration) -> Result<u32, BrokerError>;
    async fn hand_raise(&self, session_id: Uuid, user_id: Uuid) -> Result<u32, BrokerError>;
    async fn hand_lower(&self, session_id: Uuid, user_id: Uuid) -> Result<(), BrokerError>;
    async fn hand_queue(&self, session_id: Uuid) -> Result<Vec<HandRaiseEntry>, BrokerError>;
    async fn kick_set(&self, session_id: Uuid, user_id: Uuid, ttl: Duration) -> Result<(), BrokerError>;
    async fn is_kicked(&self, session_id: Uuid, user_id: Uuid) -> Result<bool, BrokerError>;
}

pub struct BrokerSubscription {
    rx: tokio::sync::mpsc::Receiver<BrokerEvent>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct PresenceEntry {
    pub user_id: Uuid,
    pub display_name: String,
    pub role: String,
    pub last_seen_ms: i64,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct HandRaiseEntry {
    pub user_id: Uuid,
    pub display_name: String,
    pub raised_at_ms: i64,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BrokerEvent {
    Chat { id: Uuid, sender_user_id: Uuid, sender_display_name: String, body: String, created_at: chrono::DateTime<chrono::Utc> },
    ChatDeleted { id: Uuid },
    HandRaiseChanged { user_id: Uuid, raised: bool, queue_position: Option<u32> },
    PresenceCount { count: u32 },
    PresenceList { participants: Vec<PresenceEntry> },
    Promoted { user_id: Uuid, publish_url: String, publish_password: String },
    Demoted { user_id: Uuid },
    StudentPublishing { user_id: Uuid, path: String },
    Kicked { user_id: Uuid },
    SessionEnded,
    RateLimited { retry_after_ms: u64 },
    Error { code: String, message: String },
}
```

### 4.3 RedisLiveRoomBroker implementation notes

- **Events**: pub/sub channel `aulalite:room:{session_id}:events`. JSON-encoded `BrokerEvent`. Each subscriber spawns a background task that pumps Redis pub/sub messages into the `mpsc::Receiver` returned by `subscribe`.
- **Presence**: sorted set `aulalite:room:{session_id}:presence`. Score = unix-millis last-seen. Member = JSON `PresenceEntry`. `presence_evict_stale(now - 30s)` removes timed-out members.
- **Hand-raise queue**: list `aulalite:room:{session_id}:hand_queue`. `LPUSH` on raise, `LREM` on lower, `LRANGE` on read.
- **Kicks**: hash `aulalite:room:{session_id}:kicks` with per-key TTL via SETEX-style write. Hash member = `user_id_simple → kicked_at_ms`.
- **Cleanup on session end**: backend's `end_class` flow calls `broker.publish(SessionEnded)` and explicitly DELs all four keys (best-effort; orphans are harmless thanks to TTL on kicks and presence).

### 4.4 MockLiveRoomBroker

In-process state via `Arc<Mutex<...>>` maps. `subscribe` returns a `tokio::sync::mpsc::Receiver` connected to a per-session broadcast fan-out. Used in integration tests to assert event ordering and content without a live Redis dep.

### 4.5 Wildcard auth (extending Phase 1b-β)

Phase 1b-β's `mediamtx_auth_publish_inner` checks `claims.mediamtx_permissions` against `body.path` with exact match (`p.path == body.path`). Phase 1b-γ replaces this with a glob match supporting trailing `*`:

```rust
fn matches_wildcard_path(claim_path: &str, requested_path: &str) -> bool {
    if let Some(prefix) = claim_path.strip_suffix("/*") {
        // Single-segment wildcard: requested must start with prefix + "/", and must
        // not contain a "/" after the prefix.
        if let Some(remainder) = requested_path.strip_prefix(prefix) {
            if let Some(seg) = remainder.strip_prefix('/') {
                return !seg.contains('/');
            }
        }
        false
    } else {
        claim_path == requested_path
    }
}
```

This is pure backend logic (Pattern B routes ALL auth through us), so MediaMTX wildcard support doesn't matter — we control the matcher.

## 5. Schema changes

Migration `20260509000013_live_room_chat.sql`:

```sql
CREATE TABLE live_room_messages (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    session_id UUID NOT NULL REFERENCES live_sessions(id) ON DELETE CASCADE,
    sender_user_id UUID NOT NULL REFERENCES users(id),
    body TEXT NOT NULL CHECK (char_length(body) BETWEEN 1 AND 2000),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at TIMESTAMPTZ,
    deleted_by_user_id UUID REFERENCES users(id)
);
CREATE INDEX live_room_messages_session_idx
    ON live_room_messages (session_id, created_at);
CREATE INDEX live_room_messages_prune_idx
    ON live_room_messages (created_at)
    WHERE deleted_at IS NULL;

ALTER TABLE live_room_messages ENABLE ROW LEVEL SECURITY;
ALTER TABLE live_room_messages FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON live_room_messages
    USING (tenant_id::text = current_setting('app.tenant_id', true));

CREATE TABLE live_room_kicks (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    session_id UUID NOT NULL REFERENCES live_sessions(id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id),
    kicked_by_user_id UUID NOT NULL REFERENCES users(id),
    kicked_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (session_id, user_id)
);

ALTER TABLE live_room_kicks ENABLE ROW LEVEL SECURITY;
ALTER TABLE live_room_kicks FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON live_room_kicks
    USING (tenant_id::text = current_setting('app.tenant_id', true));

ALTER TABLE live_sessions
    ADD COLUMN student_publish_nonces JSONB NOT NULL DEFAULT '{}';
```

The kicks table's primary purpose is **audit history**; the broker holds the live "is kicked?" state in Redis (TTL'd). Backend's socket-upgrade gate first checks the broker (fast path); falls back to the kicks table (durable record, e.g., for a kicked user who tries to rejoin after Redis restart).

## 6. API surface

| Method | Path | Caller | Description |
|---|---|---|---|
| `GET` | `/v1/sessions/:id/socket` | enrolled student or teacher | WebSocket upgrade. JSON envelopes per §6.2. |
| `GET` | `/v1/sessions/:id/messages?before=<id>&limit=<N>` | enrolled student or teacher | Paginated chat history (most recent first; `before` cursor for older). Soft-deleted messages omitted from student responses; included for teachers (with marker). |

### 6.1 Chat history DTO

```json
{
  "messages": [
    {
      "id": "...",
      "sender_user_id": "...",
      "sender_display_name": "...",
      "body": "...",
      "created_at": "...",
      "deleted": true | false
    }
  ],
  "next_cursor": "..." | null
}
```

### 6.2 WebSocket envelopes (full set)

**Client → Server:**
| Type | Payload | Caller |
|---|---|---|
| `heartbeat` | — | any |
| `chat` | `body: String` | any |
| `hand_raise` | `raise: bool` | student |
| `delete_message` | `message_id: Uuid` | teacher/admin |
| `kick` | `user_id: Uuid` | teacher/admin |
| `accept_hand` | `user_id: Uuid` | teacher/admin |
| `demote_hand` | `user_id: Uuid` | teacher/admin |

**Server → Client:** all `BrokerEvent` variants from §4.2 (serialized as the same JSON shape).

### 6.3 Direct vs broadcast events

Most server events are broadcast to all session subscribers. Three are **direct** (pushed only to one specific client):
- `Promoted` — only to the promoted student (it carries the publish nonce).
- `RateLimited` — only to the offender.
- `Error` — only to the relevant client.

`PresenceList` is **routed by role at the WebSocket task layer**, not by the broker. The broker emits a single `PresenceList` event; each per-connection task checks the connection's stored role and forwards `PresenceList` only if teacher/admin, otherwise it forwards (or generates from the `PresenceCount` event) `PresenceCount`. Students never see other students' identities. The broker remains role-agnostic; routing happens at the WebSocket fan-out stage.

## 7. Authorization rules

- **Socket upgrade**: caller must (a) pass `caller_can_read_course` for the session's course AND (b) NOT be in `live_room_kicks` for this session AND (c) session.status ∈ {scheduled, live}.
- **Chat send**: caller must NOT be kicked. Subject to rate-limit. Body must be 1-2000 chars.
- **Chat delete**: caller must be teacher/admin for the course (`caller_can_admin_course`).
- **Kick**: caller must be teacher/admin. Cannot kick self. Cannot kick another teacher/admin.
- **Hand-raise**: any non-kicked student. Toggle is rate-limited.
- **Accept/demote hand**: caller must be teacher/admin.

## 8. Lifecycle interactions with 1b-β

- **Go-live (existing)**: no change. Sockets can be opened any time during scheduled lobby + live windows.
- **End-class (existing)**: extended to also push `SessionEnded` over the broker and DEL all Redis keys for the session.
- **Auto-end sweep (existing 60s task)**: same — when a session auto-ends, the existing end-class flow runs (which now includes the broker cleanup).
- **Per-occurrence cancel (Phase 1a)**: same — calls end-class flow if status was live.

## 9. Rate-limiting

In-process token-bucket per (user_id, session_id, action_type). State held in the WebSocket task (one bucket per connection). Buckets:
- Chat: capacity 1, refill 1 per 2s.
- Hand-raise toggle: capacity 5, refill 5 per 60s.

On rate-limit hit, server emits `RateLimited` direct event with `retry_after_ms`. Message dropped silently (not persisted). Token-bucket state is per-connection, so reconnecting resets it (acceptable trade-off for a small implementation; revisit if abuse appears).

## 10. Failure modes

| Scenario | Handling |
|---|---|
| WebSocket disconnects (network blip) | Client reconnects with exponential backoff (1s → 2s → 4s → 8s → 30s max). After 5 attempts, banner: "disconnected — please refresh". |
| Server restart | All sockets close. Clients reconnect. Chat history reloads from DB. Presence + hand-raise queue rebuild organically. Active student WHIP publishers survive (independent of backend). |
| Redis down | Broker calls return errors. Backend sends `Error{code: "broker_unavailable"}` and disables chat input client-side. Video stream unaffected. |
| Promoted student denies mic permission | `getUserMedia` rejects → audio publisher emits error → UI prompts re-enable. Server still considers them promoted until teacher demotes or session ends. |
| Promoted student loses connection | Their WHIP closes; MediaMTX path goes idle. Other viewers see audio stop but their WHEP stays connected (silent). On reconnect, student's UI sees they're still flagged promoted (via fresh socket connect + state replay) and re-publishes. |
| Teacher kicks student mid-publish | Server emits `Demoted` to that student → their audio publisher closes. Server emits `Kicked` → socket closes. Subsequent reconnect gates on `is_kicked` and 410's. |

## 11. Testing strategy

### 11.1 Backend unit (pure)

In `services::live_room`:
- `matches_wildcard_path` — happy paths + edge cases (trailing slash, double wildcard, multi-segment).
- `TokenBucket` — fill, drain, refill timing.
- `BrokerEvent` JSON serde round-trip for each variant.

### 11.2 Backend integration (Postgres + `MockLiveRoomBroker`)

`tests/live_room_chat.rs`:
- `chat_send_persists_and_broadcasts`
- `chat_rate_limit_drops_excess`
- `chat_delete_by_teacher_succeeds`
- `chat_delete_by_student_returns_403`
- `chat_history_pagination_returns_recent_first`
- `messages_route_excludes_deleted_when_caller_is_student`

`tests/live_room_presence.rs`:
- `presence_join_increments_count`
- `presence_list_visible_to_teacher_only`
- `presence_stale_eviction_decrements_count`

`tests/live_room_hand_raise.rs`:
- `hand_raise_appends_to_queue`
- `hand_raise_rate_limit`
- `accept_hand_promotes_with_publish_credentials`
- `mediamtx_auth_publish_accepts_student_nonce_for_per_student_path`
- `mediamtx_auth_publish_accepts_wildcard_read_for_student_path`
- `demote_hand_clears_nonce_and_broadcasts`

`tests/live_room_kick.rs`:
- `kick_blocks_subsequent_socket_upgrade`
- `kick_by_non_teacher_returns_403`
- `kick_persists_to_db_for_audit`

`tests/rls_tenant_isolation.rs` — append:
- `cross_tenant_live_room_messages_masked`
- `cross_tenant_live_room_kicks_masked`

### 11.3 Frontend

- `live_room_socket::parse_event` — JSON envelope round-trip per type.
- `live_room_socket::backoff_schedule` — exponential sequence.
- SSR smokes for `LiveRoomChat`, `LiveRoomPresence`, `LiveRoomHandRaise` (empty + populated states + role-asymmetric rendering).

### 11.4 Manual exit-checklist (deferred per pattern)

Real Chrome teacher + 2 student profiles: chat back-and-forth, hand-raise from student → accept → audio audible to all → demote → audio stops, kick a student → socket closes + reconnect blocked, presence count tracks across opens/closes.

## 12. Dependencies

**New Rust workspace deps:**
- `fred = "10"` — async-Tokio Redis client. Better fit than `bb8-redis` for pub/sub (built-in support for subscriber connections).

**Existing reuse:**
- `axum::extract::ws` (already in axum; no new dep).
- `tokio::sync::mpsc`, `tokio::sync::broadcast` (already in workspace).
- `jsonwebtoken` (1b-β) for verifying viewer JWTs in the auth callback (now with wildcard support in our matcher).

**Env vars:**
- `REDIS_URL` (default `redis://redis:6379`). Backend wires `RedisLiveRoomBroker` from this.

## 13. Open questions / deferred decisions

1. **Cross-instance presence consistency.** Single backend instance is safe; multi-instance is fine for chat/events (Redis pub/sub) but presence eviction races between instances. Mitigation: only the instance that handled the original `presence_join` evicts that user's stale entry. Phase 2 problem.
2. **Chat history page size limit.** 50 messages per page is the default. No per-tenant override yet.
3. **Hand-raise queue cap.** Unbounded in 1b-γ. If a session has 200 students and 50 raise hands, the teacher's queue UI scrolls. Future polish: cap at, say, 20 or paginate.
4. **Audio mixing on the receiver side.** When 3 students are simultaneously promoted, the viewer hears all three concurrently (browser default audio mixing). For now, that's acceptable. A future enhancement: client-side spatial audio or "active speaker" detection. Out of scope.
5. **Recording chat playback.** 1b-δ reads `live_room_messages` ordered by `created_at` and replays alongside video timecode. 1b-γ doesn't write any video-correlation timestamps yet — `created_at` is wall-clock. Sufficient for naive replay.

## 14. Acceptance criteria (Phase 1b-γ complete when)

- Migration 0013 applied; new tables visible in `\d`; new column visible on `live_sessions`.
- `cargo test --workspace -j 2` green.
- Wasm + native builds clean.
- WebSocket upgrade route works against the running stack: real Chrome teacher + 2 students can chat, raise hands, hear each other on accept, see kicks take effect.
- Redis usage observed (`KEYS aulalite:room:*` shows live keys during a session, drained after end-class).
- Cross-tenant probes mask `live_room_messages` and `live_room_kicks` rows.
- Rate-limit experimentally verified (sending 10 chats in 1s drops 9 of them).
- Manual exit-checklist (deferred per established pattern) signed off before tagging `phase-1b-gamma-complete`.
