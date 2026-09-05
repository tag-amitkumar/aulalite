# AulaLite Phase 1b-δ — Live Class Recording Design

**Date:** 2026-05-09
**Phase:** 1b-δ (last sub-phase of 1b)
**Predecessor:** 1b-γ (live room UX, complete and merged at `9bb82c9`)
**Successor:** Phase 1c (assignments)

## 1. Goal

Close out 1b. Auto-record live class video to disk via MediaMTX, post-process to MP4, upload to MinIO, expose a playback UI that replays video alongside time-synced chat from `live_room_messages`. Recording is gated by the existing `recording_enabled` flag (set per-session and per-series since Phase 1b-β).

## 2. In scope

- MediaMTX records HLS segments per active publish path during the live session.
- In-process Tokio task in the backend container detects ended sessions with `recording_enabled=true`, ffmpeg-remuxes their segments into a single MP4, uploads to MinIO via `S3Client`, creates a `recordings` row + a `file_assets` row.
- Concurrency cap of 1 simultaneous ffmpeg job (semaphore) to avoid backend CPU spikes.
- 365-day retention janitor (`LIVE_ROOM_RECORDING_RETENTION_DAYS` env, default 365).
- Playback UI: `LiveRoomReplay` component renders `<video>` + time-synced chat side panel.
- Three new routes: `/v1/sessions/:id/recording` (presigned playback URL), `/v1/sessions/:id/recording/chat?from=&to=` (chat replay window), `POST /v1/sessions/:id/recording/retry` (teacher retry on failed jobs).
- New trait `RecorderTool` (mirrors `S3Client` / `MediaMtxClient` / `LiveRoomBroker`) with `RealFfmpegRecorder` (production) and `MockRecorderTool` (tests) impls.
- Course-member-only access (no per-recording visibility flags).
- Cross-tenant isolation via RLS on `recordings`.

## 3. Out of scope

| Feature | Phase / status |
|---|---|
| Live DVR / rewind during live session | Phase 1c+ |
| Screen-share path recording (only main camera+mic recorded) | later |
| Promoted-student audio path recording | later |
| Chapter markers / transcripts / captions | later |
| Recording editing or trim UI | later |
| Per-recording visibility flag (private/unlisted/public) | later |
| Super-admin dashboard for retention controls | separate dashboard phase, ties to subscription pricing |
| CDN fronting for MP4 delivery | scaling phase |
| Multi-track recording (audio-only, video-only variants) | later |
| Ingest from external sources (RTMP/SRT) recording | later |

## 4. Architecture

### 4.1 Code organization

Per Approach A (consistent with 1b-α / 1b-β / 1b-γ): each sub-phase gets its own services module.

**Backend touch points:**
- `crates/backend/src/services/recording.rs` — NEW. `RecorderTool` trait, `RealFfmpegRecorder`, `MockRecorderTool`, path-helpers (`recording_object_key`, `concat_list_for`), job-state machine helpers.
- `crates/backend/src/db/recordings.rs` — NEW. SQL for `recordings` table.
- `crates/backend/src/handlers/live_sessions.rs` — extend with three routes (recording details, chat window, retry).
- `crates/backend/src/lib.rs` — `AppState` gains `recorder: Arc<dyn RecorderTool>`.
- `crates/backend/src/main.rs` — spawn 60s recording-sweep task + 24h retention-janitor task.
- `crates/backend/Dockerfile` — install `ffmpeg`.

**Frontend touch points (in `features-courses`):**
- `live_room_replay.rs` — NEW. Playback view with `<video>` + chat sidebar.
- `live_room_shell.rs` — modify: `Branch::Ended` dispatches to `LiveRoomReplay` if recording exists.

**Infrastructure:**
- `ops/mediamtx/mediamtx.yml` — add recording config.
- `docker-compose.yml` — mount `mediamtx_recordings` volume to BOTH mediamtx (`/recordings`) AND backend (`/recordings:ro`).
- `migrations/20260509000014_recordings.sql` — new `recordings` table + RLS.
- `.env.example` — `LIVE_ROOM_RECORDING_RETENTION_DAYS=365`.

### 4.2 Storage layout

**Tier 1 — MediaMTX local volume `mediamtx_recordings:` (transient).**
- Mounted at `/recordings` in the mediamtx container (writeable).
- Mounted at `/recordings` in the backend container (read-only).
- Holds HLS fmp4 segments organized by `%path` (i.e., `aula/<tenant>/<course>/<session>/...`).
- MediaMTX auto-deletes after 24 hours via `recordDeleteAfter: 24h` — gives the sidecar a generous window to remux + upload.
- Sized for 24h × concurrent sessions × ~500 MB/hr at 1080p.

**Tier 2 — MinIO bucket `aulalite` (permanent).**
- Same bucket as 1b-α file uploads.
- Object key: `<tenant_simple>/<year>/<month>/<file_asset_id_simple>/recording.mp4`.
- Same path convention as cover images / lesson videos / attachments — keeps the bucket layout consistent.
- Reuses the existing `MinIoClient` impl of `S3Client`. The trait gains a new method `put_object(key, body, content_type) -> Result<()>` for server-to-server uploads (the existing presigned-URL flow is client-facing).
- Object lifetime: 365 days (default), then janitor deletes.

**Why not GCS / S3 / R2 directly:** the `S3Client` trait is the abstraction. Today: MinIO. Tomorrow: swap the trait impl with no other changes.

### 4.3 RecorderTool trait

```rust
#[async_trait]
pub trait RecorderTool: Send + Sync {
    /// Concatenate fmp4 segments via ffmpeg `-c copy` (no re-encode).
    /// Returns the path to the produced MP4.
    async fn remux_to_mp4(
        &self,
        segments: &[std::path::PathBuf],
        output: &std::path::Path,
    ) -> Result<(), RecorderError>;
    /// Probe duration in seconds via ffprobe.
    async fn probe_duration_seconds(
        &self,
        path: &std::path::Path,
    ) -> Result<i32, RecorderError>;
}

#[derive(Debug, thiserror::Error)]
pub enum RecorderError {
    #[error("ffmpeg failed: {0}")]
    FfmpegFailed(String),
    #[error("ffprobe failed: {0}")]
    FfprobeFailed(String),
    #[error("io: {0}")]
    Io(String),
}
```

`RealFfmpegRecorder` shells out via `tokio::process::Command`. `MockRecorderTool` writes a tiny stub MP4 (≤1 KB, valid header) and returns a configured duration. Tests use the mock — no ffmpeg dep at test time.

## 5. Schema changes

Migration `20260509000014_recordings.sql`:

```sql
CREATE TABLE recordings (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    session_id UUID NOT NULL REFERENCES live_sessions(id) ON DELETE CASCADE,
    file_asset_id UUID REFERENCES file_assets(id),
    started_at TIMESTAMPTZ NOT NULL,
    ended_at TIMESTAMPTZ NOT NULL,
    duration_seconds INTEGER NOT NULL CHECK (duration_seconds > 0),
    processing_status TEXT NOT NULL DEFAULT 'pending'
        CHECK (processing_status IN ('pending','remuxing','uploading','available','failed')),
    processing_error TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (session_id)
);
CREATE INDEX recordings_pending_idx
    ON recordings (processing_status, created_at)
    WHERE processing_status IN ('pending','remuxing','uploading');

ALTER TABLE recordings ENABLE ROW LEVEL SECURITY;
ALTER TABLE recordings FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON recordings
    USING (tenant_id::text = current_setting('app.tenant_id', true));
```

`file_asset_id` is nullable initially because the row is created before the upload finishes; populated when `processing_status` flips to `available`.

`UNIQUE (session_id)` prevents duplicate recordings per session and serves as the dedup point for racing sweep ticks.

## 6. MediaMTX recording config

`ops/mediamtx/mediamtx.yml` additions:

```yaml
record: yes
recordPath: /recordings/%path/%Y-%m-%d_%H-%M-%S-%f
recordFormat: fmp4
recordPartDuration: 100ms
recordSegmentDuration: 30m
recordDeleteAfter: 24h
```

`%path` produces a directory structure mirroring the MediaMTX path (e.g., `aula/<tenant>/<course>/<session>`). Each session's segments live under that prefix.

`docker-compose.yml` updates:

```yaml
mediamtx:
  volumes:
    - ./ops/mediamtx/mediamtx.yml:/mediamtx.yml:ro
    - mediamtx_recordings:/recordings  # NEW

backend:
  volumes:
    - mediamtx_recordings:/recordings:ro  # NEW
```

The backend's read-only mount is exactly what the in-process sweep needs to enumerate segments + run ffmpeg.

## 7. API surface

| Method | Path | Caller | Description |
|---|---|---|---|
| `GET` | `/v1/sessions/:id/recording` | enrolled course member | Returns `RecordingDto`. 404 if no recording row. |
| `GET` | `/v1/sessions/:id/recording/chat?from_seconds=N&to_seconds=N` | enrolled course member | Returns chat messages where `created_at - recordings.started_at` ∈ [from_seconds, to_seconds]. ASC. Used by the chat-replay sidebar. Soft-deleted messages omitted for students; included with marker for teachers. |
| `POST` | `/v1/sessions/:id/recording/retry` | course teacher / org_admin | If `processing_status='failed'`, flip to `pending`. Next sweep retries. Idempotent for non-failed states (returns current status). |

### 7.1 DTOs

```json
// RecordingDto
{
  "session_id": "...",
  "processing_status": "pending|remuxing|uploading|available|failed",
  "processing_error": "..." | null,
  "duration_seconds": 3600 | null,
  "started_at": "2026-05-09T18:00:00Z" | null,
  "playback_url": "https://...minio.../?presigned=..." | null,  // 15-min TTL, only on 'available'
  "course_title": "...",
  "instructor_user_id": "..." | null
}
```

```json
// RecordingChatWindowDto
{
  "messages": [
    {
      "id": "...",
      "sender_user_id": "...",
      "sender_display_name": "...",
      "body": "...",
      "video_offset_seconds": 12.4,
      "deleted": false
    }
  ]
}
```

`video_offset_seconds` is `(message.created_at - recording.started_at).total_seconds()`. Frontend uses it directly to know "this message belongs at video time T".

## 8. Recording sweep + remux + upload pipeline

A new 60s Tokio interval task in `main.rs`. State machine:

```
[no row]
   │
   ▼ sweep finds: status='ended', recording_enabled=true,
                  actual_started_at IS NOT NULL,
                  actual_ended_at - actual_started_at >= 5s,
                  no row in recordings
   │
INSERT recordings (..., processing_status='pending')
   │
   ▼ sweep acquires global semaphore (capacity=1)
   │
   ▼ UPDATE → 'remuxing'
   │
ffmpeg remux: enumerate /recordings/aula/<t>/<c>/<s>/, sort by mtime ASC,
              concat via -f concat -c copy → /tmp/recording-<session>.mp4
   │
   ▼ probe duration
   │
   ▼ UPDATE → 'uploading'
   │
S3Client.put_object(<object_key>, mp4_bytes, "video/mp4")
   │
INSERT file_assets (purpose='session_recording',
                    linked_entity_type='session_recording',
                    linked_entity_id=recording_id,
                    status='available')
   │
UPDATE recordings SET file_asset_id=$, duration_seconds=$, started_at=$,
                       ended_at=$, processing_status='available'
   │
emit_audit_event 'recording.completed'
   │
cleanup /tmp/recording-<session>.mp4 (MediaMTX cleans the source segments after 24h)
```

### 8.1 Failure paths

| Symptom | Handling |
|---|---|
| ffmpeg non-zero exit | `processing_status='failed'`, `processing_error=<stderr tail truncated to 1000 chars>`. No automatic retry. |
| S3 put_object fails | Same — `failed` + error captured. |
| Backend crash mid-job | Sweep query also picks up `(processing_status IN ('remuxing','uploading') AND created_at < now() - interval '30 minutes')` — orphaned jobs reset to `pending`. |
| Concurrent sweep ticks racing | `UNIQUE (session_id)` + `INSERT ... ON CONFLICT DO NOTHING`. |
| Session never went live (`actual_started_at IS NULL`) | Sweep skips. |
| Session too short (<5s) | Sweep skips — no recording row created. |

### 8.2 Retry endpoint

`POST /v1/sessions/:id/recording/retry`:
- Caller must be teacher/admin for the session's course.
- If `processing_status='failed'`, sets it to `pending` and clears `processing_error`. Returns updated DTO.
- Else: returns current DTO unchanged (idempotent).

## 9. Retention janitor

A 5th Tokio interval in `main.rs` (24h cadence). Restart-safe per-row pattern (NOT a single bulk DELETE):

1. `SELECT id, file_asset_id FROM recordings WHERE created_at < now() - (<retention_days>::int || ' days')::interval LIMIT 100`.
2. For each returned `(id, file_asset_id)`:
   1. `SELECT object_key FROM file_assets WHERE id=$1`.
   2. `S3Client.delete_object(object_key)` — best effort; failures logged, NOT aborting the loop (the next janitor pass picks up still-orphaned objects).
   3. `UPDATE file_assets SET status='pruned' WHERE id=$1`.
   4. `DELETE FROM recordings WHERE id=$1`.

Each iteration is small + atomic per recording: even if the backend crashes mid-loop, partially-cleaned recordings either have their `file_assets.status='pruned'` (then next pass skips re-S3-deleting and removes the recordings row) or they don't (next pass cleans them up). No orphaned MinIO objects accumulate without the corresponding `file_assets` row to identify them.

**Future super-admin dashboard hook:** the `LIVE_ROOM_RECORDING_RETENTION_DAYS` env var (and `LIVE_ROOM_CHAT_RETENTION_DAYS` from 1b-γ) eventually move to a `tenants.retention_overrides JSONB` column managed via the dashboard, tied to subscription pricing. 1b-δ ships the env-var version; the dashboard consolidates them in a separate phase.

## 10. Frontend playback design

### 10.1 LiveRoomReplay component

```rust
#[derive(Props, Clone, PartialEq)]
pub struct LiveRoomReplayProps {
    pub session_id: String,
    pub course_title: String,
    pub instructor_name: Option<String>,
}

pub fn LiveRoomReplay(props: LiveRoomReplayProps) -> Element { ... }
```

On mount, fetch `GET /v1/sessions/:id/recording`. Branch on `processing_status`:

| Status | Render |
|---|---|
| `available` | `<video controls src={playback_url}>` + chat side panel + course title + instructor |
| `pending` / `remuxing` / `uploading` | Spinner + "Recording is being processed (usually 5-15 min after class ends). Refresh to check." |
| `failed` | "Recording failed to process." For teachers/admins, show "Retry" button → `POST /v1/sessions/:id/recording/retry`. |
| 404 (no row) | Existing "Class has ended" fallback (delegates to `LiveRoomShell` parent). |

### 10.2 Chat side panel — time-synced

Two signals:
- `video_seconds: Signal<f64>` — updated by `<video>`'s `ontimeupdate` event.
- `visible_messages: Signal<Vec<ChatMessage>>` — accumulator.

Two effects:
1. **Window fetch on time advance.** Whenever `video_seconds` crosses a 30-second boundary OR seeking fires, fetch `/recording/chat?from_seconds=0&to_seconds={video_seconds + 5}` (the +5 is lookahead for paused→played snappiness). Replace `visible_messages` with the response.
2. **Auto-scroll to bottom.** On `visible_messages` size change, scroll the chat list element to bottom via `scrollTop = scrollHeight` JS-call.

Reuses `LiveRoomChat` component from 1b-γ in read-only mode (`is_teacher: false`, no input, no delete affordance).

### 10.3 LiveRoomShell wiring

`LiveRoomShell::route_for(role, status)` already returns `Branch::Ended`. The `Ended` branch becomes:

```rust
// Pseudocode in shell:
Branch::Ended => {
    if has_recording_in_join_response {
        rsx! { LiveRoomReplay { session_id, course_title, instructor_name } }
    } else {
        rsx! { div { class: "live-room-ended", "Class has ended." } }
    }
}
```

The `/v1/sessions/:id/join` response (existing from 1b-β) needs one additional field: `has_recording: bool`. Backend sets this by checking `EXISTS (SELECT 1 FROM recordings WHERE session_id = $1 AND processing_status IN ('pending','remuxing','uploading','available'))` — i.e., any recording other than `failed`.

## 11. Authorization rules

- **Recording fetch (`/v1/sessions/:id/recording`)**: caller passes `caller_can_read_course` for the session's course. Cross-tenant masked via 404. If course-member but not yet enrolled at recording time? V1 ignores enrollment-at-recording-time and grants based on current membership only — simpler and matches "recordings persist past my class membership ends" generally being undesirable but acceptable for v1.
- **Chat window (`/recording/chat`)**: same gate.
- **Retry (`POST /recording/retry`)**: caller passes `caller_can_admin_course`.
- **Cross-tenant probe** (RLS): a user in tenant B fetching tenant A's recording_id → 404 (RLS masks the row).

## 12. Testing strategy

### 12.1 Backend unit (pure)

- `services::recording::recording_object_key(tenant, asset_id) -> String` — pattern formatting.
- `services::recording::concat_list_for(segments: &[PathBuf]) -> String` — generates ffmpeg concat manifest content; verifies escaping of paths with quotes.
- `services::recording::video_offset_seconds(message_created_at, recording_started_at) -> f64` — pure timestamp arithmetic.

### 12.2 Backend integration (Postgres + `MockS3Client` + `MockRecorderTool` + `MockMediaMtxClient`)

`tests/recording.rs`:
- `sweep_picks_up_ended_session_with_recording_enabled`
- `sweep_skips_session_with_recording_enabled_false`
- `sweep_skips_session_with_no_actual_started_at`
- `sweep_skips_session_with_duration_under_5s`
- `recording_failed_after_ffmpeg_error_then_retry_resets`
- `recording_succeeds_inserts_file_asset_and_recording_row`
- `concurrent_sweep_ticks_dedupe_via_unique_session_id`
- `orphaned_remuxing_job_older_than_30min_resets_to_pending`
- `recording_route_returns_playback_url_when_available`
- `recording_route_returns_404_when_no_recording`
- `recording_route_returns_processing_state_when_pending`
- `recording_chat_window_returns_messages_in_range`
- `recording_chat_window_excludes_deleted_for_student`
- `recording_chat_window_includes_deleted_marker_for_teacher`
- `retention_janitor_deletes_recordings_older_than_n_days`

`tests/rls_tenant_isolation.rs` — append `cross_tenant_recordings_masked`.

### 12.3 Frontend SSR smoke (in `live_room_smoke.rs`)

- `replay_renders_video_when_available`
- `replay_renders_processing_state_when_pending`
- `replay_renders_failed_state_with_retry_button_for_teacher`
- `replay_renders_failed_state_without_retry_button_for_student`

### 12.4 Manual exit-checklist (deferred per pattern)

Real Chrome: teacher records a 2-min lecture, ends class, waits ~5 min for status to flip to `available`, opens the recording playback URL, confirms video plays + chat side panel scrolls in sync as the video plays.

## 13. Dependencies

**New runtime deps in backend container:**
- `ffmpeg` (apt package) — installed via Dockerfile RUN. ~100MB. Not a Cargo dep; shells out via `tokio::process::Command`.

**New Rust deps:** none. `tokio::process` is already in the workspace.

**New env vars:**
- `LIVE_ROOM_RECORDING_RETENTION_DAYS` (default 365). Daily janitor uses this.

**Existing reuse:**
- `S3Client` trait (extended with `put_object`); `MinIoClient` production impl; `MockS3Client` for tests.
- `db::audit::emit_audit_event` for `recording.completed` / `recording.failed` / `recording.retried`.
- `live_room_messages` table from 1b-γ for chat window queries.

## 14. Open questions / deferred decisions

1. **Super-admin retention dashboard.** All retention windows (chat 90d, recording 365d, future job-result retention) eventually move into a tenant-overridable dashboard, tied to subscription tiers. 1b-δ ships env-var defaults; the dashboard is a separate phase.
2. **CDN / signed URLs / direct MinIO delivery.** v1 uses 15-min presigned MinIO URLs served direct-to-client. For scale, fronting with Cloudflare or S3+CloudFront is a follow-up.
3. **Long recordings (>2hr) memory pressure.** `S3Client.put_object` with the full MP4 in memory works for typical 1hr recordings (~500MB) but risks OOM for 4hr+ lectures. v1 ships in-memory; multipart streaming upload is a follow-up if a class actually runs >2hr.
4. **"Recordings I can watch" listing UI.** Students currently access recordings by navigating to a session URL. A "Past lectures" dashboard tab is a 1c+ feature.
5. **Duplicate ingest** if a session is ended manually and then auto-end re-runs the sweep. Mitigated by `UNIQUE (session_id)` — second insert is a no-op.

## 15. Acceptance criteria (Phase 1b-δ complete when)

- Migration 0014 applied; `recordings` table visible in `\d recordings`.
- `cargo test --workspace -j 2` green.
- Wasm + native builds clean.
- MediaMTX records a sample session to disk; sweep picks it up; ffmpeg remux completes; MP4 lands in MinIO; `recordings.processing_status='available'`.
- Playback URL works in real Chrome — video plays, chat side panel scrolls in sync.
- Cross-tenant probe masks `recordings` rows.
- Retention janitor verified by inserting a 400-day-old row and observing deletion + S3 object cleanup.
- Manual exit-checklist signed off (deferred per established pattern) before tagging `phase-1b-delta-complete`.
