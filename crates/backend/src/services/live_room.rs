// crates/backend/src/services/live_room.rs
//! Live-room real-time primitives: broker types, wildcard path matcher,
//! token-bucket rate limiter. Pure helpers in this file; trait + Mock in
//! Task 4; Redis impl in Task 5.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use uuid::Uuid;

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct PresenceEntry {
    pub user_id: Uuid,
    pub display_name: String,
    pub role: String,
    pub last_seen_ms: i64,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct HandRaiseEntry {
    pub user_id: Uuid,
    pub display_name: String,
    pub raised_at_ms: i64,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum WhiteboardTool {
    Pen,
    Eraser,
}

/// Geometric vocabulary for a whiteboard element. `Freehand` is the original
/// pen/eraser polyline; the shape kinds anchor on the first and last point of
/// `points`; `Text` carries a `text` body; `Image` references an uploaded file
/// asset (`asset_id`) and anchors on two points like a rect. `#[serde(default)]`
/// on the owning struct's `kind` field keeps older pen-only clients (no `kind`)
/// parsing as `Freehand`.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum WhiteboardKind {
    #[default]
    Freehand,
    Line,
    Rect,
    Ellipse,
    Arrow,
    Text,
    Image,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct WhiteboardPoint {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct WhiteboardStroke {
    pub id: String,
    pub points: Vec<WhiteboardPoint>,
    pub color: String,
    pub width: f32,
    pub tool: WhiteboardTool,
    /// Geometric kind. Omitted on the wire by older pen-only clients, which
    /// `#[serde(default)]` parses as `WhiteboardKind::Freehand`.
    #[serde(default)]
    pub kind: WhiteboardKind,
    /// Body for `WhiteboardKind::Text` elements; omitted for every other kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// File-asset id for `WhiteboardKind::Image` elements; the uploaded image is
    /// referenced by id (not embedded as a data-URL) to keep the socket payload
    /// small. Omitted for every other kind. `#[serde(default)]` keeps older
    /// streams parsing as `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_id: Option<String>,
    /// User who authored this stroke. Added additively for per-author
    /// undo/redo; `#[serde(default)]` keeps older strokes (no `author`)
    /// parsing as `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<Uuid>,
}

/// Maximum length (in chars) of a whiteboard text-box body.
pub const MAX_WHITEBOARD_TEXT_LEN: usize = 280;

/// Maximum length (in chars) of a whiteboard image element's `asset_id`. File
/// asset ids are UUIDs (36 chars); the cap is a generous backstop against a
/// hostile client stuffing the field.
pub const MAX_WHITEBOARD_ASSET_ID_LEN: usize = 96;

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BrokerEvent {
    Chat {
        id: Uuid,
        sender_user_id: Uuid,
        sender_display_name: String,
        body: String,
        created_at: chrono::DateTime<chrono::Utc>,
    },
    ChatDeleted {
        id: Uuid,
    },
    HandRaiseChanged {
        user_id: Uuid,
        raised: bool,
        #[serde(default)]
        display_name: String,
        queue_position: Option<u32>,
    },
    PresenceCount {
        count: u32,
    },
    PresenceList {
        participants: Vec<PresenceEntry>,
    },
    Promoted {
        user_id: Uuid,
        publish_url: String,
        publish_password: String,
    },
    Demoted {
        user_id: Uuid,
        #[serde(default)]
        display_name: String,
    },
    StudentPublishing {
        user_id: Uuid,
        path: String,
        /// Full WHEP URL the other participants use to subscribe to this
        /// promoted student's feed, built from the public WebRTC base (mirrors
        /// `Promoted.publish_url`). Older servers omit it, so clients fall back
        /// to deriving the URL from `path` when this is empty.
        #[serde(default)]
        whep_url: String,
        #[serde(default)]
        display_name: String,
    },
    WhiteboardStroke {
        stroke: WhiteboardStroke,
    },
    /// Full board state, sent point-to-point to a socket right after it
    /// subscribes so late joiners and reconnects see existing strokes.
    WhiteboardSnapshot {
        strokes: Vec<WhiteboardStroke>,
    },
    /// A single stroke was removed (teacher undo).
    WhiteboardStrokeRemoved {
        stroke_id: String,
    },
    WhiteboardClear,
    /// Ephemeral per-user cursor over the whiteboard. NOT persisted: it never
    /// touches the board history and is dropped from snapshot hydration. `x`/`y`
    /// are normalized board coordinates in `0.0..=1.0`.
    WhiteboardCursor {
        user_id: Uuid,
        #[serde(default)]
        display_name: String,
        x: f32,
        y: f32,
    },
    /// The teacher toggled whether non-teachers may draw on the board.
    DrawPermissionChanged {
        open: bool,
    },
    /// An ephemeral emoji reaction from a participant, broadcast to the room and
    /// rendered as a brief floating burst. Not persisted.
    Reaction {
        user_id: Uuid,
        emoji: String,
        #[serde(default)]
        display_name: String,
    },
    Kicked {
        user_id: Uuid,
    },
    SessionEnded,
    RateLimited {
        retry_after_ms: u64,
    },
    Error {
        code: String,
        message: String,
    },
    /// A client-issued command failed server-side. The client should surface
    /// `reason` (already user-facing) and not retry automatically.
    CommandFailed {
        command: String,
        reason: String,
    },
    /// A teacher started an in-class poll. Ephemeral (not persisted in the
    /// board snapshot); broadcast once to the whole room.
    PollStarted {
        poll_id: Uuid,
        question: String,
        options: Vec<String>,
    },
    /// Live tally for the active poll: `counts[i]` is the vote count for the
    /// i-th option. Broadcast after each accepted vote.
    PollResults {
        poll_id: Uuid,
        counts: Vec<u32>,
    },
    /// The teacher ended the poll; `counts` carries the final tally.
    PollEnded {
        poll_id: Uuid,
        counts: Vec<u32>,
    },
    /// The teacher opened breakout rooms. Carries the full current layout so
    /// every client (teacher dashboard + students) renders the same set of
    /// rooms and assignments. Ephemeral — not snapshot-hydrated beyond the
    /// `BreakoutSnapshot` sent point-to-point on connect.
    BreakoutOpened {
        rooms: Vec<BreakoutRoom>,
    },
    /// The breakout layout changed while open (rooms created, renamed, or
    /// participants re-assigned). Same full-layout payload as `BreakoutOpened`.
    BreakoutUpdated {
        rooms: Vec<BreakoutRoom>,
    },
    /// The teacher closed all breakout rooms; everyone returns to the main room.
    BreakoutClosed,
    /// Full breakout state, sent point-to-point to a socket right after it
    /// subscribes so a (re)joining client hydrates open breakouts. `open` is
    /// false (and `rooms` empty) when no breakouts are running.
    BreakoutSnapshot {
        open: bool,
        rooms: Vec<BreakoutRoom>,
    },
    /// Targeted at a single participant: their breakout assignment changed. The
    /// client (re)subscribes its main WHEP viewer to `room_key`'s WHEP path
    /// while assigned, and falls back to the main room when `room_id` is `None`.
    /// `whep_url` is the full URL the assigned student subscribes to (empty when
    /// returning to the main room).
    BreakoutAssignment {
        user_id: Uuid,
        room_id: Option<Uuid>,
        #[serde(default)]
        room_key: String,
        #[serde(default)]
        whep_url: String,
        #[serde(default)]
        room_name: String,
    },
}

/// Bounds on a breakout layout. A session may have at most `BREAKOUT_MAX_ROOMS`
/// concurrent breakout rooms; each room name is capped at `BREAKOUT_NAME_MAX_LEN`.
pub const BREAKOUT_MAX_ROOMS: usize = 20;
pub const BREAKOUT_NAME_MAX_LEN: usize = 60;

/// One breakout room within a session. `id` keys the MediaMTX sub-room path
/// (`<main_path>/breakout/<id_simple>`); `members` are the user ids assigned to
/// it. Ephemeral — held in the broker only while breakouts are open.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct BreakoutRoom {
    pub id: Uuid,
    pub name: String,
    pub members: Vec<Uuid>,
}

/// Authoritative ephemeral breakout state for a session: whether breakouts are
/// open plus the current room layout.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq, Default)]
pub struct BreakoutState {
    pub open: bool,
    pub rooms: Vec<BreakoutRoom>,
}

impl BreakoutState {
    /// The breakout room (if any) a user is currently assigned to.
    pub fn room_for(&self, user_id: Uuid) -> Option<&BreakoutRoom> {
        if !self.open {
            return None;
        }
        self.rooms.iter().find(|r| r.members.contains(&user_id))
    }
}

/// Bounds on a poll's option list. Mirrors `core_types::live_room`.
pub const POLL_MIN_OPTIONS: usize = 2;
pub const POLL_MAX_OPTIONS: usize = 6;
/// Max length (chars) of a poll question or any single option label.
pub const POLL_QUESTION_MAX_LEN: usize = 240;
pub const POLL_OPTION_MAX_LEN: usize = 120;

/// Authoritative ephemeral state for the session's currently-active poll,
/// held in the broker. A vote is recorded only once per `(poll, user)`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PollState {
    pub poll_id: Uuid,
    pub num_options: usize,
    pub counts: Vec<u32>,
}

/// Outcome of `poll_vote`: whether the vote was newly recorded (so the caller
/// knows to broadcast) plus the resulting tally.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PollVoteOutcome {
    /// `true` if this vote was counted; `false` if the user already voted on
    /// this poll (deduped) — the counts are still returned unchanged.
    pub counted: bool,
    pub counts: Vec<u32>,
}

#[derive(Debug, thiserror::Error)]
pub enum BrokerError {
    #[error("broker transport: {0}")]
    Transport(String),
    #[error("broker payload: {0}")]
    Payload(String),
}

/// Returns `true` if `requested_path` matches `claim_path`. Supports a
/// trailing `/*` single-segment wildcard.
pub fn matches_wildcard_path(claim_path: &str, requested_path: &str) -> bool {
    if let Some(prefix) = claim_path.strip_suffix("/*") {
        if let Some(remainder) = requested_path.strip_prefix(prefix) {
            if let Some(seg) = remainder.strip_prefix('/') {
                return !seg.is_empty() && !seg.contains('/');
            }
        }
        false
    } else {
        claim_path == requested_path
    }
}

pub fn validate_whiteboard_stroke(stroke: &WhiteboardStroke) -> Result<(), &'static str> {
    if stroke.id.trim().is_empty() || stroke.id.len() > 96 {
        return Err("stroke id is invalid");
    }
    // Point-count rules vary by kind: text anchors on a single point; shapes
    // (line/rect/ellipse/arrow) anchor on exactly two; freehand pen/eraser
    // polylines need at least two and are capped.
    match stroke.kind {
        WhiteboardKind::Text => {
            if stroke.points.is_empty() {
                return Err("stroke must contain at least one point");
            }
            match &stroke.text {
                Some(text) => {
                    let trimmed = text.trim();
                    if trimmed.is_empty() {
                        return Err("text is empty");
                    }
                    if text.chars().count() > MAX_WHITEBOARD_TEXT_LEN {
                        return Err("text is too long");
                    }
                }
                None => return Err("text element requires a body"),
            }
        }
        WhiteboardKind::Line
        | WhiteboardKind::Rect
        | WhiteboardKind::Ellipse
        | WhiteboardKind::Arrow => {
            if stroke.points.len() != 2 {
                return Err("shape must contain exactly two points");
            }
        }
        WhiteboardKind::Image => {
            // Image elements anchor on two points (top-left + bottom-right
            // bounds) and reference an uploaded asset by id.
            if stroke.points.len() != 2 {
                return Err("image must contain exactly two points");
            }
            match &stroke.asset_id {
                Some(id) => {
                    if id.trim().is_empty() || id.len() > MAX_WHITEBOARD_ASSET_ID_LEN {
                        return Err("image asset id is invalid");
                    }
                }
                None => return Err("image element requires an asset id"),
            }
        }
        WhiteboardKind::Freehand => {
            if stroke.points.len() < 2 {
                return Err("stroke must contain at least two points");
            }
        }
    }
    if stroke.points.len() > 512 {
        return Err("stroke contains too many points");
    }
    let color = stroke.color.as_bytes();
    let hex = color.strip_prefix(b"#").unwrap_or_default();
    if !matches!(hex.len(), 3 | 6 | 8) || !hex.iter().all(u8::is_ascii_hexdigit) {
        return Err("stroke color is invalid");
    }
    if !stroke.width.is_finite() || stroke.width < 1.0 || stroke.width > 32.0 {
        return Err("stroke width is invalid");
    }
    for p in &stroke.points {
        if !p.x.is_finite() || !p.y.is_finite() {
            return Err("stroke point is invalid");
        }
        if !(0.0..=1.0).contains(&p.x) || !(0.0..=1.0).contains(&p.y) {
            return Err("stroke point is outside the board");
        }
    }
    Ok(())
}

/// In-memory token bucket. Per-(connection, action) instances live in the
/// WebSocket task. Tests construct directly.
pub struct TokenBucket {
    capacity: u32,
    tokens: u32,
    refill_period: Duration,
    last_refill: Instant,
}

impl TokenBucket {
    pub fn new(capacity: u32, refill_period: Duration) -> Self {
        Self {
            capacity,
            tokens: capacity,
            refill_period,
            last_refill: Instant::now(),
        }
    }

    /// Returns Ok if a token was consumed, Err(retry_after) if not.
    pub fn try_consume(&mut self) -> Result<(), Duration> {
        self.refill_now();
        if self.tokens > 0 {
            self.tokens -= 1;
            Ok(())
        } else {
            Err(self
                .refill_period
                .saturating_sub(self.last_refill.elapsed()))
        }
    }

    fn refill_now(&mut self) {
        let elapsed = self.last_refill.elapsed();
        if elapsed >= self.refill_period {
            let n = (elapsed.as_nanos() / self.refill_period.as_nanos()) as u32;
            self.tokens = self.tokens.saturating_add(n).min(self.capacity);
            self.last_refill = Instant::now();
        }
    }
}

#[async_trait]
pub trait LiveRoomBroker: Send + Sync {
    /// Lightweight dependency probe used by deployment readiness checks.
    async fn healthz(&self) -> Result<(), BrokerError>;
    async fn publish(&self, session_id: Uuid, event: BrokerEvent) -> Result<(), BrokerError>;
    async fn subscribe(&self, session_id: Uuid) -> Result<BrokerSubscription, BrokerError>;
    async fn presence_join(
        &self,
        session_id: Uuid,
        entry: PresenceEntry,
    ) -> Result<(), BrokerError>;
    async fn presence_heartbeat(&self, session_id: Uuid, user_id: Uuid) -> Result<(), BrokerError>;
    async fn presence_leave(&self, session_id: Uuid, user_id: Uuid) -> Result<(), BrokerError>;
    async fn presence_count(&self, session_id: Uuid) -> Result<u32, BrokerError>;
    async fn presence_list(&self, session_id: Uuid) -> Result<Vec<PresenceEntry>, BrokerError>;
    async fn presence_evict_stale(
        &self,
        session_id: Uuid,
        older_than_ms: i64,
    ) -> Result<u32, BrokerError>;
    async fn hand_raise(&self, session_id: Uuid, user_id: Uuid) -> Result<u32, BrokerError>;
    async fn hand_lower(&self, session_id: Uuid, user_id: Uuid) -> Result<(), BrokerError>;
    async fn hand_queue(&self, session_id: Uuid) -> Result<Vec<HandRaiseEntry>, BrokerError>;
    async fn kick_set(
        &self,
        session_id: Uuid,
        user_id: Uuid,
        ttl: Duration,
    ) -> Result<(), BrokerError>;
    async fn is_kicked(&self, session_id: Uuid, user_id: Uuid) -> Result<bool, BrokerError>;
    /// Append a validated stroke to the session's board history (capped at
    /// `MAX_WHITEBOARD_STROKES`; older strokes are dropped first).
    async fn whiteboard_append(
        &self,
        session_id: Uuid,
        stroke: WhiteboardStroke,
    ) -> Result<(), BrokerError>;
    /// Remove the most recent stroke (teacher undo). Returns its id, or
    /// `None` when the board is empty.
    async fn whiteboard_remove_last(&self, session_id: Uuid)
        -> Result<Option<String>, BrokerError>;
    /// Remove the most recent stroke authored by `author` (per-author undo).
    /// Returns the removed stroke (so the caller can push it onto a redo
    /// stack / re-emit it on redo), or `None` when the author has no strokes.
    async fn whiteboard_remove_last_by_author(
        &self,
        session_id: Uuid,
        author: Uuid,
    ) -> Result<Option<WhiteboardStroke>, BrokerError>;
    /// Remove a specific stroke by id (geometric eraser hit-test). Returns
    /// `true` if a stroke with that id existed and was removed.
    async fn whiteboard_remove_by_id(
        &self,
        session_id: Uuid,
        stroke_id: &str,
    ) -> Result<bool, BrokerError>;
    /// The current board history, oldest stroke first.
    async fn whiteboard_strokes(
        &self,
        session_id: Uuid,
    ) -> Result<Vec<WhiteboardStroke>, BrokerError>;
    /// Drop the board history (teacher clear).
    async fn whiteboard_clear(&self, session_id: Uuid) -> Result<(), BrokerError>;
    /// Set the per-room "students may draw" flag (teacher toggle). Persisted so
    /// late joiners hydrate the current value on connect.
    async fn set_draw_open(&self, session_id: Uuid, open: bool) -> Result<(), BrokerError>;
    /// Read the current per-room "students may draw" flag. Defaults to `false`
    /// (teacher-only) when no flag has been set.
    async fn draw_open(&self, session_id: Uuid) -> Result<bool, BrokerError>;
    /// Drop all per-room state (board history, presence, hand queue) once a
    /// session ends so nothing lingers in the broker store.
    async fn clear_room_state(&self, session_id: Uuid) -> Result<(), BrokerError>;

    // ── in-class polls (ephemeral) ──────────────────────────────────────────

    /// Open a new poll for the session, replacing any currently-active poll.
    /// `num_options` is the validated option count (2..=6); counts start at 0
    /// and the voter set is empty.
    async fn poll_start(
        &self,
        session_id: Uuid,
        poll_id: Uuid,
        num_options: usize,
    ) -> Result<(), BrokerError>;

    /// Record a single vote. Returns `Ok(None)` when there is no active poll
    /// with `poll_id` or `option_index` is out of range. Otherwise returns the
    /// outcome: `counted = false` (with unchanged counts) if the user already
    /// voted on this poll, else `counted = true` with the incremented tally.
    async fn poll_vote(
        &self,
        session_id: Uuid,
        poll_id: Uuid,
        user_id: Uuid,
        option_index: usize,
    ) -> Result<Option<PollVoteOutcome>, BrokerError>;

    /// End the poll and return its final counts. Returns `Ok(None)` if the
    /// active poll does not match `poll_id` (already ended or never started).
    /// Clears the poll + voter state on success.
    async fn poll_end(
        &self,
        session_id: Uuid,
        poll_id: Uuid,
    ) -> Result<Option<Vec<u32>>, BrokerError>;

    // ── breakout rooms (ephemeral) ──────────────────────────────────────────

    /// Read the session's current breakout state. Defaults to a closed,
    /// empty layout when no breakouts have been opened.
    async fn breakout_get(&self, session_id: Uuid) -> Result<BreakoutState, BrokerError>;

    /// Overwrite the session's breakout state wholesale. Used by every
    /// breakout mutation (open/assign/close) — the handler reads, mutates, and
    /// writes back the full layout so the broker stays a simple value store.
    async fn breakout_set(&self, session_id: Uuid, state: BreakoutState)
        -> Result<(), BrokerError>;
}

/// Board history cap, matching the frontend `MAX_STROKES`.
pub const MAX_WHITEBOARD_STROKES: usize = 256;

pub struct BrokerSubscription {
    pub rx: mpsc::Receiver<BrokerEvent>,
}

impl BrokerSubscription {
    pub async fn recv(&mut self) -> Option<BrokerEvent> {
        self.rx.recv().await
    }
}

#[derive(Clone, Default)]
pub struct MockLiveRoomBroker {
    inner: Arc<Mutex<MockBrokerInner>>,
}

#[derive(Default)]
struct MockBrokerInner {
    subs: HashMap<Uuid, Vec<mpsc::Sender<BrokerEvent>>>,
    presence: HashMap<Uuid, HashMap<Uuid, PresenceEntry>>,
    queue: HashMap<Uuid, VecDeque<HandRaiseEntry>>,
    kicks: HashMap<Uuid, HashSet<Uuid>>,
    whiteboard: HashMap<Uuid, Vec<WhiteboardStroke>>,
    draw_open: HashMap<Uuid, bool>,
    /// Active poll per session (at most one) plus the set of users who voted.
    polls: HashMap<Uuid, (PollState, HashSet<Uuid>)>,
    /// Ephemeral breakout layout per session.
    breakout: HashMap<Uuid, BreakoutState>,
}

impl MockLiveRoomBroker {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl LiveRoomBroker for MockLiveRoomBroker {
    async fn healthz(&self) -> Result<(), BrokerError> {
        Ok(())
    }

    async fn publish(&self, session_id: Uuid, event: BrokerEvent) -> Result<(), BrokerError> {
        let senders = {
            let g = self.inner.lock().unwrap();
            g.subs.get(&session_id).cloned().unwrap_or_default()
        };
        for s in senders {
            let _ = s.send(event.clone()).await;
        }
        Ok(())
    }

    async fn subscribe(&self, session_id: Uuid) -> Result<BrokerSubscription, BrokerError> {
        let (tx, rx) = mpsc::channel(64);
        self.inner
            .lock()
            .unwrap()
            .subs
            .entry(session_id)
            .or_default()
            .push(tx);
        Ok(BrokerSubscription { rx })
    }

    async fn presence_join(
        &self,
        session_id: Uuid,
        entry: PresenceEntry,
    ) -> Result<(), BrokerError> {
        self.inner
            .lock()
            .unwrap()
            .presence
            .entry(session_id)
            .or_default()
            .insert(entry.user_id, entry);
        Ok(())
    }

    async fn presence_heartbeat(&self, session_id: Uuid, user_id: Uuid) -> Result<(), BrokerError> {
        let mut g = self.inner.lock().unwrap();
        if let Some(map) = g.presence.get_mut(&session_id) {
            if let Some(entry) = map.get_mut(&user_id) {
                entry.last_seen_ms = chrono::Utc::now().timestamp_millis();
            }
        }
        Ok(())
    }

    async fn presence_leave(&self, session_id: Uuid, user_id: Uuid) -> Result<(), BrokerError> {
        if let Some(map) = self.inner.lock().unwrap().presence.get_mut(&session_id) {
            map.remove(&user_id);
        }
        Ok(())
    }

    async fn presence_count(&self, session_id: Uuid) -> Result<u32, BrokerError> {
        Ok(self
            .inner
            .lock()
            .unwrap()
            .presence
            .get(&session_id)
            .map(|m| m.len() as u32)
            .unwrap_or(0))
    }

    async fn presence_list(&self, session_id: Uuid) -> Result<Vec<PresenceEntry>, BrokerError> {
        Ok(self
            .inner
            .lock()
            .unwrap()
            .presence
            .get(&session_id)
            .map(|m| m.values().cloned().collect())
            .unwrap_or_default())
    }

    async fn presence_evict_stale(
        &self,
        session_id: Uuid,
        older_than_ms: i64,
    ) -> Result<u32, BrokerError> {
        let mut count = 0u32;
        if let Some(map) = self.inner.lock().unwrap().presence.get_mut(&session_id) {
            let stale: Vec<Uuid> = map
                .iter()
                .filter(|(_, e)| e.last_seen_ms < older_than_ms)
                .map(|(k, _)| *k)
                .collect();
            for k in stale {
                map.remove(&k);
                count += 1;
            }
        }
        Ok(count)
    }

    async fn hand_raise(&self, session_id: Uuid, user_id: Uuid) -> Result<u32, BrokerError> {
        let mut g = self.inner.lock().unwrap();
        let q = g.queue.entry(session_id).or_default();
        if !q.iter().any(|e| e.user_id == user_id) {
            q.push_back(HandRaiseEntry {
                user_id,
                display_name: String::new(),
                raised_at_ms: chrono::Utc::now().timestamp_millis(),
            });
        }
        Ok(q.len() as u32)
    }

    async fn hand_lower(&self, session_id: Uuid, user_id: Uuid) -> Result<(), BrokerError> {
        if let Some(q) = self.inner.lock().unwrap().queue.get_mut(&session_id) {
            q.retain(|e| e.user_id != user_id);
        }
        Ok(())
    }

    async fn hand_queue(&self, session_id: Uuid) -> Result<Vec<HandRaiseEntry>, BrokerError> {
        Ok(self
            .inner
            .lock()
            .unwrap()
            .queue
            .get(&session_id)
            .map(|q| q.iter().cloned().collect())
            .unwrap_or_default())
    }

    async fn kick_set(
        &self,
        session_id: Uuid,
        user_id: Uuid,
        _ttl: Duration,
    ) -> Result<(), BrokerError> {
        self.inner
            .lock()
            .unwrap()
            .kicks
            .entry(session_id)
            .or_default()
            .insert(user_id);
        Ok(())
    }

    async fn is_kicked(&self, session_id: Uuid, user_id: Uuid) -> Result<bool, BrokerError> {
        Ok(self
            .inner
            .lock()
            .unwrap()
            .kicks
            .get(&session_id)
            .is_some_and(|s| s.contains(&user_id)))
    }

    async fn whiteboard_append(
        &self,
        session_id: Uuid,
        stroke: WhiteboardStroke,
    ) -> Result<(), BrokerError> {
        let mut g = self.inner.lock().unwrap();
        let board = g.whiteboard.entry(session_id).or_default();
        board.push(stroke);
        let len = board.len();
        if len > MAX_WHITEBOARD_STROKES {
            board.drain(..len - MAX_WHITEBOARD_STROKES);
        }
        Ok(())
    }

    async fn whiteboard_remove_last(
        &self,
        session_id: Uuid,
    ) -> Result<Option<String>, BrokerError> {
        Ok(self
            .inner
            .lock()
            .unwrap()
            .whiteboard
            .get_mut(&session_id)
            .and_then(|b| b.pop())
            .map(|s| s.id))
    }

    async fn whiteboard_remove_last_by_author(
        &self,
        session_id: Uuid,
        author: Uuid,
    ) -> Result<Option<WhiteboardStroke>, BrokerError> {
        let mut g = self.inner.lock().unwrap();
        let Some(board) = g.whiteboard.get_mut(&session_id) else {
            return Ok(None);
        };
        // Most-recent-first: find the last stroke this author owns.
        let idx = board.iter().rposition(|s| s.author == Some(author));
        Ok(idx.map(|i| board.remove(i)))
    }

    async fn whiteboard_remove_by_id(
        &self,
        session_id: Uuid,
        stroke_id: &str,
    ) -> Result<bool, BrokerError> {
        let mut g = self.inner.lock().unwrap();
        let Some(board) = g.whiteboard.get_mut(&session_id) else {
            return Ok(false);
        };
        let before = board.len();
        board.retain(|s| s.id != stroke_id);
        Ok(board.len() != before)
    }

    async fn whiteboard_strokes(
        &self,
        session_id: Uuid,
    ) -> Result<Vec<WhiteboardStroke>, BrokerError> {
        Ok(self
            .inner
            .lock()
            .unwrap()
            .whiteboard
            .get(&session_id)
            .cloned()
            .unwrap_or_default())
    }

    async fn whiteboard_clear(&self, session_id: Uuid) -> Result<(), BrokerError> {
        self.inner.lock().unwrap().whiteboard.remove(&session_id);
        Ok(())
    }

    async fn set_draw_open(&self, session_id: Uuid, open: bool) -> Result<(), BrokerError> {
        self.inner
            .lock()
            .unwrap()
            .draw_open
            .insert(session_id, open);
        Ok(())
    }

    async fn draw_open(&self, session_id: Uuid) -> Result<bool, BrokerError> {
        Ok(self
            .inner
            .lock()
            .unwrap()
            .draw_open
            .get(&session_id)
            .copied()
            .unwrap_or(false))
    }

    async fn clear_room_state(&self, session_id: Uuid) -> Result<(), BrokerError> {
        let mut g = self.inner.lock().unwrap();
        g.whiteboard.remove(&session_id);
        g.presence.remove(&session_id);
        g.queue.remove(&session_id);
        g.draw_open.remove(&session_id);
        g.polls.remove(&session_id);
        g.breakout.remove(&session_id);
        Ok(())
    }

    async fn poll_start(
        &self,
        session_id: Uuid,
        poll_id: Uuid,
        num_options: usize,
    ) -> Result<(), BrokerError> {
        let state = PollState {
            poll_id,
            num_options,
            counts: vec![0; num_options],
        };
        self.inner
            .lock()
            .unwrap()
            .polls
            .insert(session_id, (state, HashSet::new()));
        Ok(())
    }

    async fn poll_vote(
        &self,
        session_id: Uuid,
        poll_id: Uuid,
        user_id: Uuid,
        option_index: usize,
    ) -> Result<Option<PollVoteOutcome>, BrokerError> {
        let mut g = self.inner.lock().unwrap();
        let Some((state, voters)) = g.polls.get_mut(&session_id) else {
            return Ok(None);
        };
        if state.poll_id != poll_id || option_index >= state.num_options {
            return Ok(None);
        }
        if !voters.insert(user_id) {
            // Already voted — return current tally unchanged.
            return Ok(Some(PollVoteOutcome {
                counted: false,
                counts: state.counts.clone(),
            }));
        }
        state.counts[option_index] = state.counts[option_index].saturating_add(1);
        Ok(Some(PollVoteOutcome {
            counted: true,
            counts: state.counts.clone(),
        }))
    }

    async fn poll_end(
        &self,
        session_id: Uuid,
        poll_id: Uuid,
    ) -> Result<Option<Vec<u32>>, BrokerError> {
        let mut g = self.inner.lock().unwrap();
        match g.polls.get(&session_id) {
            Some((state, _)) if state.poll_id == poll_id => {
                let counts = state.counts.clone();
                g.polls.remove(&session_id);
                Ok(Some(counts))
            }
            _ => Ok(None),
        }
    }

    async fn breakout_get(&self, session_id: Uuid) -> Result<BreakoutState, BrokerError> {
        Ok(self
            .inner
            .lock()
            .unwrap()
            .breakout
            .get(&session_id)
            .cloned()
            .unwrap_or_default())
    }

    async fn breakout_set(
        &self,
        session_id: Uuid,
        state: BreakoutState,
    ) -> Result<(), BrokerError> {
        self.inner
            .lock()
            .unwrap()
            .breakout
            .insert(session_id, state);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stroke(id: &str) -> WhiteboardStroke {
        WhiteboardStroke {
            id: id.to_string(),
            points: vec![
                WhiteboardPoint { x: 0.1, y: 0.1 },
                WhiteboardPoint { x: 0.2, y: 0.2 },
            ],
            color: "#111827".into(),
            width: 4.0,
            tool: WhiteboardTool::Pen,
            kind: WhiteboardKind::Freehand,
            text: None,
            asset_id: None,
            author: None,
        }
    }

    fn stroke_by(id: &str, author: Uuid) -> WhiteboardStroke {
        WhiteboardStroke {
            author: Some(author),
            ..stroke(id)
        }
    }

    #[tokio::test]
    async fn mock_broker_whiteboard_history_appends_caps_and_removes() {
        let broker = MockLiveRoomBroker::new();
        let session = Uuid::new_v4();

        // Appends keep insertion order and cap at MAX_WHITEBOARD_STROKES
        // (oldest dropped first).
        for i in 0..MAX_WHITEBOARD_STROKES + 5 {
            broker
                .whiteboard_append(session, stroke(&format!("s{i}")))
                .await
                .unwrap();
        }
        let strokes = broker.whiteboard_strokes(session).await.unwrap();
        assert_eq!(strokes.len(), MAX_WHITEBOARD_STROKES);
        assert_eq!(strokes.first().unwrap().id, "s5");

        // Undo pops the most recent stroke and returns its id.
        let removed = broker.whiteboard_remove_last(session).await.unwrap();
        assert_eq!(removed.as_deref(), Some("s260"));

        // Clear empties the board; remove on empty returns None.
        broker.whiteboard_clear(session).await.unwrap();
        assert!(broker.whiteboard_strokes(session).await.unwrap().is_empty());
        assert_eq!(broker.whiteboard_remove_last(session).await.unwrap(), None);
    }

    #[tokio::test]
    async fn mock_broker_clear_room_state_drops_all_room_keys() {
        let broker = MockLiveRoomBroker::new();
        let session = Uuid::new_v4();
        let user = Uuid::new_v4();
        broker
            .whiteboard_append(session, stroke("a"))
            .await
            .unwrap();
        broker
            .presence_join(
                session,
                PresenceEntry {
                    user_id: user,
                    display_name: "T".into(),
                    role: "teacher".into(),
                    last_seen_ms: 0,
                },
            )
            .await
            .unwrap();
        broker.hand_raise(session, user).await.unwrap();

        broker.clear_room_state(session).await.unwrap();

        assert!(broker.whiteboard_strokes(session).await.unwrap().is_empty());
        assert_eq!(broker.presence_count(session).await.unwrap(), 0);
        assert!(broker.hand_queue(session).await.unwrap().is_empty());
    }

    #[test]
    fn wildcard_matches_single_segment() {
        assert!(matches_wildcard_path(
            "aula/x/y/z/student/*",
            "aula/x/y/z/student/abc"
        ));
        assert!(matches_wildcard_path(
            "aula/x/y/z/student/*",
            "aula/x/y/z/student/123"
        ));
    }

    #[test]
    fn wildcard_rejects_multi_segment() {
        assert!(!matches_wildcard_path(
            "aula/x/y/z/student/*",
            "aula/x/y/z/student/abc/screen"
        ));
    }

    #[test]
    fn wildcard_rejects_wrong_prefix() {
        assert!(!matches_wildcard_path(
            "aula/x/y/z/student/*",
            "aula/X/y/z/student/abc"
        ));
    }

    #[test]
    fn wildcard_rejects_empty_segment() {
        assert!(!matches_wildcard_path(
            "aula/x/y/z/student/*",
            "aula/x/y/z/student/"
        ));
    }

    #[test]
    fn exact_match_without_wildcard() {
        assert!(matches_wildcard_path("aula/x/y/z", "aula/x/y/z"));
        assert!(!matches_wildcard_path("aula/x/y/z", "aula/x/y/z/extra"));
    }

    #[test]
    fn token_bucket_initial_full_capacity() {
        let mut tb = TokenBucket::new(3, Duration::from_secs(60));
        assert!(tb.try_consume().is_ok());
        assert!(tb.try_consume().is_ok());
        assert!(tb.try_consume().is_ok());
    }

    #[test]
    fn token_bucket_drains_then_blocks() {
        let mut tb = TokenBucket::new(1, Duration::from_secs(60));
        assert!(tb.try_consume().is_ok());
        assert!(tb.try_consume().is_err());
    }

    #[test]
    fn token_bucket_refills_after_period() {
        let mut tb = TokenBucket::new(1, Duration::from_millis(50));
        assert!(tb.try_consume().is_ok());
        assert!(tb.try_consume().is_err());
        std::thread::sleep(Duration::from_millis(60));
        assert!(tb.try_consume().is_ok());
    }

    #[test]
    fn broker_event_chat_round_trips_json() {
        let evt = BrokerEvent::Chat {
            id: Uuid::nil(),
            sender_user_id: Uuid::nil(),
            sender_display_name: "Alice".into(),
            body: "hello".into(),
            created_at: chrono::Utc::now(),
        };
        let s = serde_json::to_string(&evt).unwrap();
        let d: BrokerEvent = serde_json::from_str(&s).unwrap();
        match d {
            BrokerEvent::Chat {
                body,
                sender_display_name,
                ..
            } => {
                assert_eq!(body, "hello");
                assert_eq!(sender_display_name, "Alice");
            }
            _ => panic!("wrong variant after round-trip"),
        }
    }

    #[test]
    fn broker_event_session_ended_round_trips_json() {
        let evt = BrokerEvent::SessionEnded;
        let s = serde_json::to_string(&evt).unwrap();
        let d: BrokerEvent = serde_json::from_str(&s).unwrap();
        assert!(matches!(d, BrokerEvent::SessionEnded));
    }

    #[tokio::test]
    async fn mock_broker_publish_received_by_subscriber() {
        let session_id = Uuid::new_v4();
        let broker = MockLiveRoomBroker::new();
        let mut sub = broker.subscribe(session_id).await.unwrap();
        broker
            .publish(session_id, BrokerEvent::SessionEnded)
            .await
            .unwrap();
        let evt = sub.recv().await.unwrap();
        assert!(matches!(evt, BrokerEvent::SessionEnded));
    }

    #[tokio::test]
    async fn mock_broker_isolated_per_session() {
        let s1 = Uuid::new_v4();
        let s2 = Uuid::new_v4();
        let broker = MockLiveRoomBroker::new();
        let mut sub = broker.subscribe(s1).await.unwrap();
        broker.publish(s2, BrokerEvent::SessionEnded).await.unwrap();
        let timeout = tokio::time::timeout(Duration::from_millis(100), sub.recv()).await;
        assert!(
            timeout.is_err(),
            "subscriber on s1 must not receive s2's events"
        );
    }

    #[tokio::test]
    async fn mock_broker_presence_join_increments_count() {
        let session_id = Uuid::new_v4();
        let broker = MockLiveRoomBroker::new();
        broker
            .presence_join(
                session_id,
                PresenceEntry {
                    user_id: Uuid::new_v4(),
                    display_name: "A".into(),
                    role: "student".into(),
                    last_seen_ms: 0,
                },
            )
            .await
            .unwrap();
        assert_eq!(broker.presence_count(session_id).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn mock_broker_presence_evict_stale() {
        let session_id = Uuid::new_v4();
        let broker = MockLiveRoomBroker::new();
        broker
            .presence_join(
                session_id,
                PresenceEntry {
                    user_id: Uuid::new_v4(),
                    display_name: "old".into(),
                    role: "student".into(),
                    last_seen_ms: 1_000,
                },
            )
            .await
            .unwrap();
        broker
            .presence_join(
                session_id,
                PresenceEntry {
                    user_id: Uuid::new_v4(),
                    display_name: "fresh".into(),
                    role: "student".into(),
                    last_seen_ms: 9_999_999_999_999,
                },
            )
            .await
            .unwrap();
        let evicted = broker
            .presence_evict_stale(session_id, 5_000_000_000)
            .await
            .unwrap();
        assert_eq!(evicted, 1);
        assert_eq!(broker.presence_count(session_id).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn mock_broker_hand_raise_appends_and_returns_position() {
        let session_id = Uuid::new_v4();
        let u1 = Uuid::new_v4();
        let u2 = Uuid::new_v4();
        let broker = MockLiveRoomBroker::new();
        let p1 = broker.hand_raise(session_id, u1).await.unwrap();
        let p2 = broker.hand_raise(session_id, u2).await.unwrap();
        assert_eq!(p1, 1);
        assert_eq!(p2, 2);
    }

    #[tokio::test]
    async fn mock_broker_kick_then_is_kicked() {
        let session_id = Uuid::new_v4();
        let user_id = Uuid::new_v4();
        let broker = MockLiveRoomBroker::new();
        assert!(!broker.is_kicked(session_id, user_id).await.unwrap());
        broker
            .kick_set(session_id, user_id, Duration::from_secs(3600))
            .await
            .unwrap();
        assert!(broker.is_kicked(session_id, user_id).await.unwrap());
    }

    fn sample_stroke() -> WhiteboardStroke {
        WhiteboardStroke {
            id: "stroke-1".into(),
            points: vec![
                WhiteboardPoint { x: 0.1, y: 0.2 },
                WhiteboardPoint { x: 0.3, y: 0.4 },
            ],
            color: "#111827".into(),
            width: 4.0,
            tool: WhiteboardTool::Pen,
            kind: WhiteboardKind::Freehand,
            text: None,
            asset_id: None,
            author: None,
        }
    }

    #[test]
    fn broker_event_whiteboard_stroke_round_trips_json() {
        let evt = BrokerEvent::WhiteboardStroke {
            stroke: sample_stroke(),
        };
        let s = serde_json::to_string(&evt).unwrap();
        assert!(s.contains("\"type\":\"whiteboard_stroke\""));
        let d: BrokerEvent = serde_json::from_str(&s).unwrap();
        match d {
            BrokerEvent::WhiteboardStroke { stroke } => {
                assert_eq!(stroke.id, "stroke-1");
                assert_eq!(stroke.points.len(), 2);
            }
            other => panic!("wrong event: {other:?}"),
        }
    }

    #[test]
    fn broker_event_whiteboard_clear_round_trips_json() {
        let evt = BrokerEvent::WhiteboardClear;
        let s = serde_json::to_string(&evt).unwrap();
        assert_eq!(s, "{\"type\":\"whiteboard_clear\"}");
        let d: BrokerEvent = serde_json::from_str(&s).unwrap();
        assert!(matches!(d, BrokerEvent::WhiteboardClear));
    }

    #[test]
    fn validate_whiteboard_stroke_accepts_normal_stroke() {
        assert_eq!(validate_whiteboard_stroke(&sample_stroke()), Ok(()));
    }

    #[test]
    fn validate_whiteboard_stroke_rejects_oversized_points() {
        let mut stroke = sample_stroke();
        stroke.points = (0..513)
            .map(|i| WhiteboardPoint {
                x: (i as f32 % 100.0) / 100.0,
                y: 0.5,
            })
            .collect();
        assert_eq!(
            validate_whiteboard_stroke(&stroke),
            Err("stroke contains too many points")
        );
    }

    #[test]
    fn validate_whiteboard_stroke_rejects_empty_id() {
        let mut stroke = sample_stroke();
        stroke.id = "   ".into();
        assert_eq!(
            validate_whiteboard_stroke(&stroke),
            Err("stroke id is invalid")
        );
    }

    #[test]
    fn validate_whiteboard_stroke_rejects_too_few_points() {
        let mut stroke = sample_stroke();
        stroke.points = vec![WhiteboardPoint { x: 0.1, y: 0.2 }];
        assert_eq!(
            validate_whiteboard_stroke(&stroke),
            Err("stroke must contain at least two points")
        );
    }

    #[test]
    fn validate_whiteboard_stroke_rejects_non_hex_color() {
        let mut stroke = sample_stroke();
        stroke.color = "#zzzzzz".into();
        assert_eq!(
            validate_whiteboard_stroke(&stroke),
            Err("stroke color is invalid")
        );
    }

    #[test]
    fn validate_whiteboard_stroke_rejects_invalid_width() {
        let mut stroke = sample_stroke();
        stroke.width = 0.5;
        assert_eq!(
            validate_whiteboard_stroke(&stroke),
            Err("stroke width is invalid")
        );
    }

    #[test]
    fn validate_whiteboard_stroke_rejects_out_of_range_point() {
        let mut stroke = sample_stroke();
        stroke.points[0].x = 1.1;
        assert_eq!(
            validate_whiteboard_stroke(&stroke),
            Err("stroke point is outside the board")
        );
    }

    #[test]
    fn whiteboard_stroke_without_kind_defaults_to_freehand() {
        // Backward-compat: pen-only client sends no `kind`/`text`. The
        // additive fields default rather than fail to parse.
        let raw = r##"{"id":"s1","points":[{"x":0.1,"y":0.2},{"x":0.3,"y":0.4}],"color":"#111827","width":4.0,"tool":"pen"}"##;
        let stroke: WhiteboardStroke = serde_json::from_str(raw).unwrap();
        assert_eq!(stroke.kind, WhiteboardKind::Freehand);
        assert_eq!(stroke.text, None);
        assert_eq!(validate_whiteboard_stroke(&stroke), Ok(()));
    }

    #[test]
    fn validate_whiteboard_accepts_two_point_shapes() {
        for kind in [
            WhiteboardKind::Line,
            WhiteboardKind::Rect,
            WhiteboardKind::Ellipse,
            WhiteboardKind::Arrow,
        ] {
            let mut stroke = sample_stroke();
            stroke.kind = kind.clone();
            assert_eq!(validate_whiteboard_stroke(&stroke), Ok(()), "kind {kind:?}");
        }
    }

    #[test]
    fn validate_whiteboard_rejects_shape_without_two_points() {
        let mut stroke = sample_stroke();
        stroke.kind = WhiteboardKind::Rect;
        stroke.points = vec![WhiteboardPoint { x: 0.1, y: 0.2 }];
        assert_eq!(
            validate_whiteboard_stroke(&stroke),
            Err("shape must contain exactly two points")
        );
    }

    #[test]
    fn validate_whiteboard_text_requires_body_and_length() {
        let mut stroke = sample_stroke();
        stroke.kind = WhiteboardKind::Text;
        stroke.points = vec![WhiteboardPoint { x: 0.1, y: 0.2 }];

        // Missing body.
        stroke.text = None;
        assert_eq!(
            validate_whiteboard_stroke(&stroke),
            Err("text element requires a body")
        );

        // Empty / whitespace-only body.
        stroke.text = Some("   ".into());
        assert_eq!(validate_whiteboard_stroke(&stroke), Err("text is empty"));

        // Over the cap.
        stroke.text = Some("x".repeat(MAX_WHITEBOARD_TEXT_LEN + 1));
        assert_eq!(validate_whiteboard_stroke(&stroke), Err("text is too long"));

        // Valid.
        stroke.text = Some("Pythagoras".into());
        assert_eq!(validate_whiteboard_stroke(&stroke), Ok(()));
    }

    #[test]
    fn validate_whiteboard_image_requires_two_points_and_asset_id() {
        let mut stroke = sample_stroke();
        stroke.kind = WhiteboardKind::Image;

        // Two points but no asset id → rejected.
        stroke.asset_id = None;
        assert_eq!(
            validate_whiteboard_stroke(&stroke),
            Err("image element requires an asset id")
        );

        // Asset id present but wrong point count → rejected.
        stroke.asset_id = Some("asset-1".into());
        stroke.points = vec![WhiteboardPoint { x: 0.1, y: 0.2 }];
        assert_eq!(
            validate_whiteboard_stroke(&stroke),
            Err("image must contain exactly two points")
        );

        // Blank asset id → rejected.
        stroke.points = vec![
            WhiteboardPoint { x: 0.1, y: 0.2 },
            WhiteboardPoint { x: 0.4, y: 0.5 },
        ];
        stroke.asset_id = Some("  ".into());
        assert_eq!(
            validate_whiteboard_stroke(&stroke),
            Err("image asset id is invalid")
        );

        // Valid image element.
        stroke.asset_id = Some("00000000-0000-0000-0000-000000000000".into());
        assert_eq!(validate_whiteboard_stroke(&stroke), Ok(()));
    }

    #[test]
    fn broker_event_whiteboard_image_round_trips_json() {
        let mut stroke = sample_stroke();
        stroke.kind = WhiteboardKind::Image;
        stroke.asset_id = Some("asset-xyz".into());
        let evt = BrokerEvent::WhiteboardStroke { stroke };
        let s = serde_json::to_string(&evt).unwrap();
        assert!(s.contains("\"kind\":\"image\""));
        assert!(s.contains("\"asset_id\":\"asset-xyz\""));
        let d: BrokerEvent = serde_json::from_str(&s).unwrap();
        match d {
            BrokerEvent::WhiteboardStroke { stroke } => {
                assert_eq!(stroke.kind, WhiteboardKind::Image);
                assert_eq!(stroke.asset_id.as_deref(), Some("asset-xyz"));
            }
            other => panic!("wrong event: {other:?}"),
        }
    }

    #[test]
    fn broker_event_whiteboard_shape_round_trips_json() {
        let mut stroke = sample_stroke();
        stroke.kind = WhiteboardKind::Arrow;
        let evt = BrokerEvent::WhiteboardStroke { stroke };
        let s = serde_json::to_string(&evt).unwrap();
        assert!(s.contains("\"kind\":\"arrow\""));
        let d: BrokerEvent = serde_json::from_str(&s).unwrap();
        match d {
            BrokerEvent::WhiteboardStroke { stroke } => {
                assert_eq!(stroke.kind, WhiteboardKind::Arrow);
            }
            other => panic!("wrong event: {other:?}"),
        }
    }

    #[tokio::test]
    async fn mock_broker_whiteboard_remove_by_id() {
        let broker = MockLiveRoomBroker::new();
        let session = Uuid::new_v4();
        broker
            .whiteboard_append(session, stroke("a"))
            .await
            .unwrap();
        broker
            .whiteboard_append(session, stroke("b"))
            .await
            .unwrap();
        broker
            .whiteboard_append(session, stroke("c"))
            .await
            .unwrap();

        // Removing a middle element returns true and drops only that one.
        assert!(broker.whiteboard_remove_by_id(session, "b").await.unwrap());
        let ids: Vec<String> = broker
            .whiteboard_strokes(session)
            .await
            .unwrap()
            .into_iter()
            .map(|s| s.id)
            .collect();
        assert_eq!(ids, vec!["a".to_string(), "c".to_string()]);

        // Removing an unknown id returns false and leaves the board intact.
        assert!(!broker
            .whiteboard_remove_by_id(session, "missing")
            .await
            .unwrap());
        assert_eq!(broker.whiteboard_strokes(session).await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn mock_broker_whiteboard_remove_last_by_author_is_per_author() {
        let broker = MockLiveRoomBroker::new();
        let session = Uuid::new_v4();
        let alice = Uuid::new_v4();
        let bob = Uuid::new_v4();

        broker
            .whiteboard_append(session, stroke_by("a1", alice))
            .await
            .unwrap();
        broker
            .whiteboard_append(session, stroke_by("b1", bob))
            .await
            .unwrap();
        broker
            .whiteboard_append(session, stroke_by("a2", alice))
            .await
            .unwrap();

        // Undo for Alice removes her most recent stroke ("a2"), not Bob's.
        let removed = broker
            .whiteboard_remove_last_by_author(session, alice)
            .await
            .unwrap()
            .expect("alice has strokes");
        assert_eq!(removed.id, "a2");
        let ids: Vec<String> = broker
            .whiteboard_strokes(session)
            .await
            .unwrap()
            .into_iter()
            .map(|s| s.id)
            .collect();
        assert_eq!(ids, vec!["a1".to_string(), "b1".to_string()]);

        // A second undo for Alice removes "a1"; a third returns None.
        assert_eq!(
            broker
                .whiteboard_remove_last_by_author(session, alice)
                .await
                .unwrap()
                .map(|s| s.id),
            Some("a1".to_string())
        );
        assert!(broker
            .whiteboard_remove_last_by_author(session, alice)
            .await
            .unwrap()
            .is_none());
        // Bob's stroke is untouched.
        assert_eq!(broker.whiteboard_strokes(session).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn mock_broker_draw_open_defaults_false_and_toggles() {
        let broker = MockLiveRoomBroker::new();
        let session = Uuid::new_v4();
        // Default is teacher-only.
        assert!(!broker.draw_open(session).await.unwrap());
        broker.set_draw_open(session, true).await.unwrap();
        assert!(broker.draw_open(session).await.unwrap());
        broker.set_draw_open(session, false).await.unwrap();
        assert!(!broker.draw_open(session).await.unwrap());
        // clear_room_state drops the flag back to the default.
        broker.set_draw_open(session, true).await.unwrap();
        broker.clear_room_state(session).await.unwrap();
        assert!(!broker.draw_open(session).await.unwrap());
    }

    #[test]
    fn broker_event_whiteboard_cursor_round_trips_json() {
        let evt = BrokerEvent::WhiteboardCursor {
            user_id: Uuid::nil(),
            display_name: "Ada".into(),
            x: 0.25,
            y: 0.5,
        };
        let s = serde_json::to_string(&evt).unwrap();
        assert!(s.contains("\"type\":\"whiteboard_cursor\""));
        let d: BrokerEvent = serde_json::from_str(&s).unwrap();
        match d {
            BrokerEvent::WhiteboardCursor { x, y, .. } => {
                assert!((x - 0.25).abs() < 1e-6);
                assert!((y - 0.5).abs() < 1e-6);
            }
            other => panic!("wrong event: {other:?}"),
        }
    }

    #[test]
    fn broker_event_draw_permission_changed_round_trips_json() {
        let evt = BrokerEvent::DrawPermissionChanged { open: true };
        let s = serde_json::to_string(&evt).unwrap();
        assert!(s.contains("\"type\":\"draw_permission_changed\""));
        let d: BrokerEvent = serde_json::from_str(&s).unwrap();
        assert!(matches!(
            d,
            BrokerEvent::DrawPermissionChanged { open: true }
        ));
    }

    #[test]
    fn whiteboard_stroke_legacy_json_defaults_author_none() {
        // A stroke serialized before the `author` field existed must still
        // parse, defaulting `author` to None.
        let raw = r##"{"id":"s1","points":[{"x":0.1,"y":0.2},{"x":0.3,"y":0.4}],"color":"#111827","width":4.0,"tool":"pen"}"##;
        let stroke: WhiteboardStroke = serde_json::from_str(raw).unwrap();
        assert_eq!(stroke.author, None);
        assert_eq!(validate_whiteboard_stroke(&stroke), Ok(()));
    }

    #[tokio::test]
    async fn mock_broker_poll_lifecycle_dedup_and_tally() {
        let broker = MockLiveRoomBroker::new();
        let session = Uuid::new_v4();
        let poll = Uuid::new_v4();
        let alice = Uuid::new_v4();
        let bob = Uuid::new_v4();

        broker.poll_start(session, poll, 3).await.unwrap();

        // First vote counts; second vote by the same user is deduped.
        let out = broker
            .poll_vote(session, poll, alice, 0)
            .await
            .unwrap()
            .expect("active poll");
        assert!(out.counted);
        assert_eq!(out.counts, vec![1, 0, 0]);
        let dup = broker
            .poll_vote(session, poll, alice, 2)
            .await
            .unwrap()
            .expect("active poll");
        assert!(!dup.counted, "second vote by same user must not count");
        assert_eq!(dup.counts, vec![1, 0, 0]);

        // A different user voting a different option increments that option.
        let out = broker
            .poll_vote(session, poll, bob, 1)
            .await
            .unwrap()
            .expect("active poll");
        assert!(out.counted);
        assert_eq!(out.counts, vec![1, 1, 0]);

        // Out-of-range option is rejected (None).
        assert!(broker
            .poll_vote(session, poll, Uuid::new_v4(), 9)
            .await
            .unwrap()
            .is_none());

        // Ending returns the final tally and clears state.
        let final_counts = broker.poll_end(session, poll).await.unwrap().expect("poll");
        assert_eq!(final_counts, vec![1, 1, 0]);
        // A vote after end finds no active poll.
        assert!(broker
            .poll_vote(session, poll, Uuid::new_v4(), 0)
            .await
            .unwrap()
            .is_none());
        // Ending again returns None.
        assert!(broker.poll_end(session, poll).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn mock_broker_poll_vote_rejects_wrong_poll_id() {
        let broker = MockLiveRoomBroker::new();
        let session = Uuid::new_v4();
        let poll = Uuid::new_v4();
        broker.poll_start(session, poll, 2).await.unwrap();
        // A vote for a stale/unknown poll id is ignored.
        assert!(broker
            .poll_vote(session, Uuid::new_v4(), Uuid::new_v4(), 0)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn mock_broker_poll_start_replaces_previous_and_clear_room_drops_it() {
        let broker = MockLiveRoomBroker::new();
        let session = Uuid::new_v4();
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        broker.poll_start(session, first, 2).await.unwrap();
        broker
            .poll_vote(session, first, Uuid::new_v4(), 0)
            .await
            .unwrap();
        // Starting a new poll replaces the old one; old poll id no longer votes.
        broker.poll_start(session, second, 3).await.unwrap();
        assert!(broker
            .poll_vote(session, first, Uuid::new_v4(), 0)
            .await
            .unwrap()
            .is_none());
        let out = broker
            .poll_vote(session, second, Uuid::new_v4(), 2)
            .await
            .unwrap()
            .expect("new poll active");
        assert_eq!(out.counts, vec![0, 0, 1]);
        // Ending the room drops the active poll.
        broker.clear_room_state(session).await.unwrap();
        assert!(broker.poll_end(session, second).await.unwrap().is_none());
    }

    #[test]
    fn broker_event_poll_started_round_trips_json() {
        let evt = BrokerEvent::PollStarted {
            poll_id: Uuid::nil(),
            question: "Q?".into(),
            options: vec!["A".into(), "B".into()],
        };
        let s = serde_json::to_string(&evt).unwrap();
        assert!(s.contains("\"type\":\"poll_started\""));
        let d: BrokerEvent = serde_json::from_str(&s).unwrap();
        match d {
            BrokerEvent::PollStarted {
                question, options, ..
            } => {
                assert_eq!(question, "Q?");
                assert_eq!(options.len(), 2);
            }
            other => panic!("wrong event: {other:?}"),
        }
    }

    #[test]
    fn broker_event_poll_results_and_ended_round_trip_json() {
        for evt in [
            BrokerEvent::PollResults {
                poll_id: Uuid::nil(),
                counts: vec![1, 2, 3],
            },
            BrokerEvent::PollEnded {
                poll_id: Uuid::nil(),
                counts: vec![1, 2, 3],
            },
        ] {
            let s = serde_json::to_string(&evt).unwrap();
            let d: BrokerEvent = serde_json::from_str(&s).unwrap();
            match d {
                BrokerEvent::PollResults { counts, .. } | BrokerEvent::PollEnded { counts, .. } => {
                    assert_eq!(counts, vec![1, 2, 3])
                }
                other => panic!("wrong event: {other:?}"),
            }
        }
    }

    #[test]
    fn broker_event_student_publishing_carries_whep_url() {
        // Regression lock for the promoted-student-audio fix: the event the
        // publish-auth callback now emits must carry the full WHEP URL so every
        // participant can subscribe to the student's feed. Older clients that
        // don't read `whep_url` keep parsing (additive, non-deny_unknown).
        let evt = BrokerEvent::StudentPublishing {
            user_id: Uuid::nil(),
            path: "aula/t/c/s/student/abc".into(),
            whep_url: "http://webrtc.example/aula/t/c/s/student/abc/whep".into(),
            display_name: "Ada".into(),
        };
        let s = serde_json::to_string(&evt).unwrap();
        assert!(s.contains("\"type\":\"student_publishing\""));
        assert!(
            s.contains("\"whep_url\":\"http://webrtc.example/aula/t/c/s/student/abc/whep\""),
            "whep_url must serialize: {s}"
        );
        let d: BrokerEvent = serde_json::from_str(&s).unwrap();
        match d {
            BrokerEvent::StudentPublishing { whep_url, .. } => {
                assert_eq!(
                    whep_url,
                    "http://webrtc.example/aula/t/c/s/student/abc/whep"
                );
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[tokio::test]
    async fn mock_broker_breakout_get_defaults_closed_then_persists_and_clears() {
        let broker = MockLiveRoomBroker::new();
        let session = Uuid::new_v4();
        let alice = Uuid::new_v4();

        // Default is a closed, empty layout.
        let initial = broker.breakout_get(session).await.unwrap();
        assert!(!initial.open);
        assert!(initial.rooms.is_empty());
        assert!(initial.room_for(alice).is_none());

        // Open one room with alice assigned.
        let room_id = Uuid::new_v4();
        let state = BreakoutState {
            open: true,
            rooms: vec![BreakoutRoom {
                id: room_id,
                name: "Group A".into(),
                members: vec![alice],
            }],
        };
        broker.breakout_set(session, state).await.unwrap();
        let read = broker.breakout_get(session).await.unwrap();
        assert!(read.open);
        assert_eq!(read.room_for(alice).map(|r| r.id), Some(room_id));

        // clear_room_state drops the breakout layout back to the default.
        broker.clear_room_state(session).await.unwrap();
        assert!(!broker.breakout_get(session).await.unwrap().open);
    }

    #[test]
    fn breakout_state_room_for_ignores_when_closed() {
        let alice = Uuid::new_v4();
        let mut state = BreakoutState {
            open: false,
            rooms: vec![BreakoutRoom {
                id: Uuid::new_v4(),
                name: "G".into(),
                members: vec![alice],
            }],
        };
        // Closed → no assignment surfaced even though membership exists.
        assert!(state.room_for(alice).is_none());
        state.open = true;
        assert!(state.room_for(alice).is_some());
    }

    #[test]
    fn broker_event_breakout_opened_round_trips_json() {
        let evt = BrokerEvent::BreakoutOpened {
            rooms: vec![BreakoutRoom {
                id: Uuid::nil(),
                name: "Group 1".into(),
                members: vec![Uuid::nil()],
            }],
        };
        let s = serde_json::to_string(&evt).unwrap();
        assert!(s.contains("\"type\":\"breakout_opened\""));
        let d: BrokerEvent = serde_json::from_str(&s).unwrap();
        match d {
            BrokerEvent::BreakoutOpened { rooms } => {
                assert_eq!(rooms.len(), 1);
                assert_eq!(rooms[0].name, "Group 1");
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn broker_event_breakout_assignment_round_trips_json() {
        let evt = BrokerEvent::BreakoutAssignment {
            user_id: Uuid::nil(),
            room_id: Some(Uuid::nil()),
            room_key: "aula/t/c/s/breakout/r1".into(),
            whep_url: "http://webrtc.example/aula/t/c/s/breakout/r1/whep".into(),
            room_name: "Group 1".into(),
        };
        let s = serde_json::to_string(&evt).unwrap();
        assert!(s.contains("\"type\":\"breakout_assignment\""));
        let d: BrokerEvent = serde_json::from_str(&s).unwrap();
        match d {
            BrokerEvent::BreakoutAssignment {
                room_id, whep_url, ..
            } => {
                assert!(room_id.is_some());
                assert!(whep_url.contains("/whep"));
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn broker_event_breakout_closed_round_trips_json() {
        let evt = BrokerEvent::BreakoutClosed;
        let s = serde_json::to_string(&evt).unwrap();
        assert_eq!(s, "{\"type\":\"breakout_closed\"}");
        let d: BrokerEvent = serde_json::from_str(&s).unwrap();
        assert!(matches!(d, BrokerEvent::BreakoutClosed));
    }
}
