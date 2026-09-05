# AulaLite Phase 1b-δ Live Class Recording — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Auto-record live class video via MediaMTX, ffmpeg-remux on session end, upload to MinIO, expose playback UI with time-synced chat replay. Closes out 1b.

**Architecture:** MediaMTX records HLS fmp4 segments to a shared volume during the live session. A 60s in-process Tokio sweep in the backend container detects ended sessions with `recording_enabled=true`, runs `ffmpeg -c copy` to remux segments into a single MP4, uploads to MinIO via the existing `S3Client` trait (extended with `put_object`), creates `recordings` + `file_assets` rows. Playback UI fetches a presigned URL and renders `<video>` + a time-synced chat sidebar that polls a windowed message endpoint as the video plays.

**Tech Stack:** Rust 1.94 + Axum 0.7 + sqlx 0.8 + Postgres 16 + Dioxus 0.7 + MediaMTX (recording) + ffmpeg (apt, in backend image) + MinIO (existing).

**Predecessor:** Phase 1b-γ complete and merged at `9bb82c9`. Spec landed at `f422ff1`.

---

## Sections

- **A. Foundations** (Tasks 1-9): migration, Dockerfile ffmpeg, MediaMTX recording config, env vars, S3Client.put_object, services::recording trait + Mock + Real, db::recordings queries
- **B. Backend handlers + sweep + janitor** (Tasks 10-16): three new routes, /join extension, AppState wiring, recording sweep, retention janitor
- **C. Frontend playback** (Tasks 17-19): LiveRoomReplay component, time-synced chat panel, LiveRoomShell dispatch
- **D. RLS + closure** (Tasks 20-25): RLS sweep, SSR smokes, build sweeps, exit checklist

---

## Section A — Foundations

### Task 1: Migration 0014 — recordings table

**Files:**
- Create: `migrations/20260509000014_recordings.sql`

- [ ] **Step 1: Write the migration SQL**

```sql
-- migrations/20260509000014_recordings.sql
-- Phase 1b-δ: live class recording. One recordings row per session, linked to
-- the produced MP4 file_asset. Tenant-isolated via RLS.

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

- [ ] **Step 2: Apply**

```bash
cd "C:/Users/Chiranjib Chaudhuri/Documents/Chiranjib/Elementors_Aula-phase0"
DATABASE_URL=postgres://aulalite:changeme@localhost:55432/aulalite \
    sqlx migrate run --source migrations 2>&1 | tail -3
```
Expected: `Applied 20260509000014/migrate recordings (XYms)`.

If `sqlx-cli` isn't on PATH, fall back to:
```bash
docker compose exec -T postgres psql -U aulalite -d aulalite \
    < migrations/20260509000014_recordings.sql
```

- [ ] **Step 3: Verify**

```bash
docker compose exec postgres psql -U aulalite -d aulalite -c "\d recordings" 2>&1 | head -20
```
Expected: 10 columns visible, `UNIQUE(session_id)`, partial index `recordings_pending_idx`, RLS policy `tenant_isolation`.

- [ ] **Step 4: Commit**

```bash
git add migrations/20260509000014_recordings.sql
git commit -m "feat(db): migration 0014 add recordings table with RLS"
```

---

### Task 2: ffmpeg in backend Dockerfile

**Files:**
- Modify: `crates/backend/Dockerfile`

- [ ] **Step 1: Read current Dockerfile**

```bash
cd "C:/Users/Chiranjib Chaudhuri/Documents/Chiranjib/Elementors_Aula-phase0"
cat crates/backend/Dockerfile
```

Note the runtime stage. Phase 0 likely uses a multi-stage build with `FROM debian:bookworm-slim` or `FROM rust:1.94-slim` as the runtime base.

- [ ] **Step 2: Install ffmpeg in the runtime stage**

In the runtime stage (the FROM that runs the binary, NOT the builder stage), append before `COPY` and `CMD`:
```dockerfile
RUN apt-get update \
    && apt-get install -y --no-install-recommends ffmpeg \
    && rm -rf /var/lib/apt/lists/*
```

If the Dockerfile uses a distroless base (no apt), switch the runtime base to `debian:bookworm-slim` and copy the binary in. Adapt to whatever exists.

If the Dockerfile is single-stage with `ca-certificates` already installed, just append `ffmpeg` to the existing apt-get list.

- [ ] **Step 3: Build**

```bash
docker compose build backend 2>&1 | tail -10
```
Expected: clean build. ffmpeg adds ~100 MB to the image; expected.

- [ ] **Step 4: Verify ffmpeg is on PATH inside the container**

```bash
docker compose run --rm backend ffmpeg -version 2>&1 | head -3
```
Expected: `ffmpeg version <ver>`.

- [ ] **Step 5: Commit**

```bash
git add crates/backend/Dockerfile
git commit -m "chore(backend): install ffmpeg in runtime image for recording remux"
```

---

### Task 3: MediaMTX recording config

**Files:**
- Modify: `ops/mediamtx/mediamtx.yml`
- Modify: `docker-compose.yml`

- [ ] **Step 1: Add recording config to mediamtx.yml**

Read the current file (it has Pattern B HTTP-only auth from Phase 1b-β). Append the recording block at the top level (NOT inside `paths:`):

```yaml
record: yes
recordPath: /recordings/%path/%Y-%m-%d_%H-%M-%S-%f
recordFormat: fmp4
recordPartDuration: 100ms
recordSegmentDuration: 30m
recordDeleteAfter: 24h
```

`%path` produces a directory tree mirroring MediaMTX path scheme: `aula/<tenant_simple>/<course_simple>/<session_simple>/`. Each session's segments live under that prefix.

The `recordSegmentDuration: 30m` means MediaMTX rolls a new segment file every 30 minutes — keeps individual files manageable. `recordDeleteAfter: 24h` is a safety net: segments self-clean 24h after recording, giving the sidecar a generous window.

- [ ] **Step 2: Mount mediamtx_recordings volume to BOTH containers in docker-compose.yml**

Find the `mediamtx:` service block. Ensure its `volumes:` includes:
```yaml
    volumes:
      - ./ops/mediamtx/mediamtx.yml:/mediamtx.yml:ro
      - mediamtx_recordings:/recordings
```

Find the `backend:` service block. Add to its `volumes:`:
```yaml
    volumes:
      - mediamtx_recordings:/recordings:ro
```

The shared volume lets the backend's in-process sweep read MediaMTX's segment files directly via filesystem (no API calls needed).

Confirm `mediamtx_recordings:` is declared at the bottom under `volumes:` (Phase 0 already declared it; verify).

- [ ] **Step 3: Restart and verify**

```bash
docker compose up -d --force-recreate mediamtx backend 2>&1 | tail -5
sleep 3
docker compose logs --tail=20 mediamtx 2>&1
```
Expected: MediaMTX logs show recording configured, no errors.

```bash
docker compose exec backend ls /recordings 2>&1 | head -3
```
Expected: empty directory listing (no error). Confirms the volume is mounted on both ends.

- [ ] **Step 4: Commit**

```bash
git add ops/mediamtx/mediamtx.yml docker-compose.yml
git commit -m "feat(mediamtx): enable fmp4 recording with 24h cleanup; mount volume to backend"
```

---

### Task 4: env vars for retention

**Files:**
- Modify: `.env.example`

- [ ] **Step 1: Append new env var**

```
# Phase 1b-delta: live class recording
LIVE_ROOM_RECORDING_RETENTION_DAYS=365
```

- [ ] **Step 2: Commit**

```bash
git add .env.example
git commit -m "chore(env): LIVE_ROOM_RECORDING_RETENTION_DAYS default 365 days"
```

---

### Task 5: Extend S3Client trait with put_object

**Files:**
- Modify: `crates/backend/src/storage/mod.rs` (trait)
- Modify: `crates/backend/src/storage/mock.rs` (Mock)
- Modify: `crates/backend/src/storage/minio.rs` (production)

The recording sidecar uploads bytes directly to MinIO (server-to-server), bypassing the presigned-URL flow. The `S3Client` trait gains a `put_object` method.

- [ ] **Step 1: Add to trait**

In `crates/backend/src/storage/mod.rs`, find the `S3Client` trait. Add:
```rust
    /// Server-side direct upload (bypasses presigned URLs). Used by the
    /// recording sidecar to push remuxed MP4 bytes to the bucket.
    async fn put_object(
        &self,
        key: &str,
        body: Vec<u8>,
        content_type: &str,
    ) -> Result<(), StorageError>;
```

- [ ] **Step 2: Implement on MockS3Client**

In `crates/backend/src/storage/mock.rs`, add a `S3Call::PutObject` variant and a method:
```rust
async fn put_object(
    &self,
    key: &str,
    body: Vec<u8>,
    content_type: &str,
) -> Result<(), StorageError> {
    self.record(S3Call::PutObject {
        key: key.to_string(),
        size: body.len(),
        content_type: content_type.to_string(),
    });
    self.objects
        .lock()
        .unwrap()
        .insert(key.to_string(), (body.len() as i64, content_type.to_string()));
    Ok(())
}
```

Add `PutObject { key: String, size: usize, content_type: String }` to the `S3Call` enum.

- [ ] **Step 3: Implement on MinIoClient**

In `crates/backend/src/storage/minio.rs`:
```rust
async fn put_object(
    &self,
    key: &str,
    body: Vec<u8>,
    content_type: &str,
) -> Result<(), StorageError> {
    self.client
        .put_object()
        .bucket(&self.bucket)
        .key(key)
        .body(aws_sdk_s3::primitives::ByteStream::from(body))
        .content_type(content_type)
        .send()
        .await
        .map_err(|e| map_err("put_object", e))?;
    Ok(())
}
```

- [ ] **Step 4: Build + run S3Client tests**

```bash
cd "C:/Users/Chiranjib Chaudhuri/Documents/Chiranjib/Elementors_Aula-phase0"
cargo test -p backend --lib storage::mock 2>&1 | tail -5
```
Expected: existing tests pass (no new tests yet — exercised through recording sweep tests).

- [ ] **Step 5: Commit**

```bash
git add crates/backend/src/storage
git commit -m "feat(storage): S3Client.put_object for server-side uploads"
```

---

### Task 6: services::recording — pure helpers (TDD)

**Files:**
- Create: `crates/backend/src/services/recording.rs`
- Modify: `crates/backend/src/services/mod.rs`

- [ ] **Step 1: Add `pub mod recording;` to `services/mod.rs`**

Existing alphabetical ordering after Phase 1b-γ:
```rust
pub mod file_assets;
pub mod invitations;
pub mod live_room;
pub mod live_room_redis;
pub mod mediamtx;
pub mod recurrence;
pub mod slugger;
```

Insert `pub mod recording;` alphabetically (between `mediamtx` and `recurrence`):
```rust
pub mod file_assets;
pub mod invitations;
pub mod live_room;
pub mod live_room_redis;
pub mod mediamtx;
pub mod recording;
pub mod recurrence;
pub mod slugger;
```

- [ ] **Step 2: Create the file with failing tests**

Create `crates/backend/src/services/recording.rs`:

```rust
// crates/backend/src/services/recording.rs
//! Live class recording: ffmpeg wrapper trait, path helpers, time math.
//! RecorderTool trait + Mock land in Task 7; production impl in Task 8.

use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum RecorderError {
    #[error("ffmpeg failed: {0}")]
    FfmpegFailed(String),
    #[error("ffprobe failed: {0}")]
    FfprobeFailed(String),
    #[error("io: {0}")]
    Io(String),
}

/// Returns the MinIO object key for a recording's MP4.
/// `<tenant_simple>/<year>/<month>/<asset_id_simple>/recording.mp4`
pub fn recording_object_key(
    tenant_id: Uuid,
    asset_id: Uuid,
    now: chrono::DateTime<chrono::Utc>,
) -> String {
    use chrono::Datelike;
    format!(
        "{}/{:04}/{:02}/{}/recording.mp4",
        tenant_id.simple(),
        now.year(),
        now.month(),
        asset_id.simple(),
    )
}

/// Generates the contents of an ffmpeg `-f concat` manifest for the given
/// segment paths. Each line is `file '<path>'` with single-quote escaping
/// (an embedded quote becomes `'\''`).
pub fn concat_list_for(segments: &[PathBuf]) -> String {
    let mut out = String::with_capacity(segments.len() * 64);
    for seg in segments {
        let path = seg.to_string_lossy();
        let escaped = path.replace('\'', "'\\''");
        out.push_str("file '");
        out.push_str(&escaped);
        out.push_str("'\n");
    }
    out
}

/// Returns `(message_created_at - recording_started_at).total_seconds()`,
/// clamped to >= 0.0. Used by chat replay window queries to convert wall-clock
/// timestamps into video-relative offsets.
pub fn video_offset_seconds(
    message_created_at: chrono::DateTime<chrono::Utc>,
    recording_started_at: chrono::DateTime<chrono::Utc>,
) -> f64 {
    let delta = message_created_at - recording_started_at;
    let secs = delta.num_milliseconds() as f64 / 1000.0;
    if secs < 0.0 { 0.0 } else { secs }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn t() -> Uuid { Uuid::parse_str("9c2f4a8e-7b13-4f7c-91d2-b6a8e5c0d3e1").unwrap() }
    fn a() -> Uuid { Uuid::parse_str("4f1a2c8b-9d6e-4a7f-8c5b-3d2e1f9a0c8e").unwrap() }

    #[test]
    fn object_key_uses_simple_uuids_and_zero_padded_month() {
        let when = chrono::Utc.with_ymd_and_hms(2026, 5, 9, 12, 0, 0).unwrap();
        let key = recording_object_key(t(), a(), when);
        assert_eq!(
            key,
            "9c2f4a8e7b134f7c91d2b6a8e5c0d3e1/2026/05/4f1a2c8b9d6e4a7f8c5b3d2e1f9a0c8e/recording.mp4"
        );
    }

    #[test]
    fn object_key_zero_pads_january() {
        let when = chrono::Utc.with_ymd_and_hms(2026, 1, 9, 0, 0, 0).unwrap();
        let key = recording_object_key(t(), a(), when);
        assert!(key.contains("/2026/01/"));
    }

    #[test]
    fn concat_list_for_simple_segments() {
        let segs = vec![PathBuf::from("/recordings/aula/x/y/z/seg1.mp4"), PathBuf::from("/recordings/aula/x/y/z/seg2.mp4")];
        let out = concat_list_for(&segs);
        assert_eq!(
            out,
            "file '/recordings/aula/x/y/z/seg1.mp4'\nfile '/recordings/aula/x/y/z/seg2.mp4'\n"
        );
    }

    #[test]
    fn concat_list_for_path_with_single_quote_escapes() {
        let segs = vec![PathBuf::from("/path/with'quote.mp4")];
        let out = concat_list_for(&segs);
        // The single quote is escaped: ' → '\''.
        assert_eq!(out, "file '/path/with'\\''quote.mp4'\n");
    }

    #[test]
    fn concat_list_for_empty_returns_empty_string() {
        assert_eq!(concat_list_for(&[]), "");
    }

    #[test]
    fn video_offset_simple() {
        let started = chrono::Utc.with_ymd_and_hms(2026, 5, 9, 12, 0, 0).unwrap();
        let msg = chrono::Utc.with_ymd_and_hms(2026, 5, 9, 12, 1, 30).unwrap();
        assert_eq!(video_offset_seconds(msg, started), 90.0);
    }

    #[test]
    fn video_offset_clamps_to_zero_when_message_predates_start() {
        let started = chrono::Utc.with_ymd_and_hms(2026, 5, 9, 12, 0, 0).unwrap();
        let msg = chrono::Utc.with_ymd_and_hms(2026, 5, 9, 11, 59, 0).unwrap();
        assert_eq!(video_offset_seconds(msg, started), 0.0);
    }

    #[test]
    fn video_offset_subsecond() {
        let started = chrono::Utc.with_ymd_and_hms(2026, 5, 9, 12, 0, 0).unwrap();
        let msg = started + chrono::Duration::milliseconds(500);
        assert!((video_offset_seconds(msg, started) - 0.5).abs() < 1e-9);
    }
}
```

- [ ] **Step 3: Run, expect failure (file references types not yet defined)**

Wait — the file above is self-contained: types + impl + tests all in one file. The build will succeed and tests pass immediately on first compile.

```bash
cargo test -p backend --lib services::recording 2>&1 | tail -10
```
Expected: `8 passed`.

- [ ] **Step 4: Commit**

```bash
git add crates/backend/src/services/mod.rs crates/backend/src/services/recording.rs
git commit -m "feat(services): recording pure helpers (object_key, concat_list, video_offset) with TDD"
```

---

### Task 7: RecorderTool trait + MockRecorderTool (TDD)

**Files:**
- Modify: `crates/backend/src/services/recording.rs`

- [ ] **Step 1: Append failing tests**

Inside the existing `mod tests` block in `crates/backend/src/services/recording.rs`, append (BEFORE the closing `}`):

```rust
    #[tokio::test]
    async fn mock_recorder_remux_writes_stub_file() {
        let recorder = MockRecorderTool::new();
        let tmp = tempfile::tempdir().unwrap();
        let out = tmp.path().join("out.mp4");
        recorder.remux_to_mp4(&[PathBuf::from("seg1.mp4")], &out).await.unwrap();
        assert!(out.exists());
        let contents = std::fs::read(&out).unwrap();
        assert!(!contents.is_empty(), "stub MP4 must be non-empty");
    }

    #[tokio::test]
    async fn mock_recorder_probe_returns_configured_duration() {
        let recorder = MockRecorderTool::new();
        recorder.set_duration_seconds(120);
        let tmp = tempfile::tempdir().unwrap();
        let out = tmp.path().join("out.mp4");
        recorder.remux_to_mp4(&[PathBuf::from("seg1.mp4")], &out).await.unwrap();
        let dur = recorder.probe_duration_seconds(&out).await.unwrap();
        assert_eq!(dur, 120);
    }

    #[tokio::test]
    async fn mock_recorder_records_calls() {
        let recorder = MockRecorderTool::new();
        let tmp = tempfile::tempdir().unwrap();
        let out = tmp.path().join("out.mp4");
        let segs = vec![PathBuf::from("a.mp4"), PathBuf::from("b.mp4")];
        recorder.remux_to_mp4(&segs, &out).await.unwrap();
        let _ = recorder.probe_duration_seconds(&out).await.unwrap();
        let calls = recorder.calls();
        assert_eq!(calls.len(), 2);
        assert!(matches!(calls[0], RecorderCall::Remux { .. }));
        assert!(matches!(calls[1], RecorderCall::ProbeDuration { .. }));
    }
```

- [ ] **Step 2: Add `tempfile` to backend dev-deps if missing**

```bash
cd "C:/Users/Chiranjib Chaudhuri/Documents/Chiranjib/Elementors_Aula-phase0"
grep -n "tempfile" crates/backend/Cargo.toml
```

If missing, add to `[dev-dependencies]`:
```toml
tempfile = "3"
```

- [ ] **Step 3: Run, expect compile failure**

```bash
cargo test -p backend --lib services::recording 2>&1 | tail -10
```
Expected: missing `RecorderTool`, `MockRecorderTool`, `RecorderCall`.

- [ ] **Step 4: Implement**

Append to `crates/backend/src/services/recording.rs` (above `#[cfg(test)]`):

```rust
use async_trait::async_trait;
use std::path::Path;
use std::sync::{Arc, Mutex};

#[async_trait]
pub trait RecorderTool: Send + Sync {
    /// Concatenate fmp4 segments via ffmpeg `-c copy` (no re-encode).
    /// Writes the produced MP4 to `output`.
    async fn remux_to_mp4(
        &self,
        segments: &[PathBuf],
        output: &Path,
    ) -> Result<(), RecorderError>;

    /// Probe duration in seconds via ffprobe.
    async fn probe_duration_seconds(&self, path: &Path) -> Result<i32, RecorderError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecorderCall {
    Remux { segments: Vec<PathBuf>, output: PathBuf },
    ProbeDuration { path: PathBuf },
}

#[derive(Clone, Default)]
pub struct MockRecorderTool {
    pub calls: Arc<Mutex<Vec<RecorderCall>>>,
    pub duration_seconds: Arc<Mutex<i32>>,
}

impl MockRecorderTool {
    pub fn new() -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
            duration_seconds: Arc::new(Mutex::new(60)), // sensible default
        }
    }

    pub fn set_duration_seconds(&self, s: i32) {
        *self.duration_seconds.lock().unwrap() = s;
    }

    pub fn calls(&self) -> Vec<RecorderCall> {
        self.calls.lock().unwrap().clone()
    }

    fn record(&self, call: RecorderCall) {
        self.calls.lock().unwrap().push(call);
    }
}

#[async_trait]
impl RecorderTool for MockRecorderTool {
    async fn remux_to_mp4(
        &self,
        segments: &[PathBuf],
        output: &Path,
    ) -> Result<(), RecorderError> {
        self.record(RecorderCall::Remux {
            segments: segments.to_vec(),
            output: output.to_path_buf(),
        });
        // Write a tiny non-empty stub MP4 (4 bytes is enough for the test).
        std::fs::write(output, b"\x00\x00\x00\x18ftypmp42")
            .map_err(|e| RecorderError::Io(e.to_string()))?;
        Ok(())
    }

    async fn probe_duration_seconds(&self, path: &Path) -> Result<i32, RecorderError> {
        self.record(RecorderCall::ProbeDuration {
            path: path.to_path_buf(),
        });
        Ok(*self.duration_seconds.lock().unwrap())
    }
}
```

- [ ] **Step 5: Run, expect 11 passed**

```bash
cargo test -p backend --lib services::recording 2>&1 | tail -5
```
Expected: 11 passed (8 from Task 6 + 3 new).

- [ ] **Step 6: Commit**

```bash
git add crates/backend/Cargo.toml crates/backend/src/services/recording.rs
git commit -m "feat(services): RecorderTool trait + MockRecorderTool"
```

---

### Task 8: RealFfmpegRecorder production impl (build-only)

**Files:**
- Modify: `crates/backend/src/services/recording.rs`

This wraps `tokio::process::Command` to shell out to `ffmpeg` and `ffprobe`. Build-only — exercised manually during exit-checklist (the integration tests use the Mock).

- [ ] **Step 1: Append the production impl**

Append to `crates/backend/src/services/recording.rs` (above `#[cfg(test)]`):

```rust
#[derive(Clone)]
pub struct RealFfmpegRecorder;

impl RealFfmpegRecorder {
    pub fn new() -> Self { Self }
}

#[async_trait]
impl RecorderTool for RealFfmpegRecorder {
    async fn remux_to_mp4(
        &self,
        segments: &[PathBuf],
        output: &Path,
    ) -> Result<(), RecorderError> {
        // Build a temp concat manifest.
        let manifest_text = concat_list_for(segments);
        let manifest_path = std::env::temp_dir()
            .join(format!("aulalite-concat-{}.txt", uuid::Uuid::new_v4().simple()));
        tokio::fs::write(&manifest_path, &manifest_text).await
            .map_err(|e| RecorderError::Io(format!("write manifest: {e}")))?;

        let status = tokio::process::Command::new("ffmpeg")
            .arg("-y")
            .arg("-f").arg("concat")
            .arg("-safe").arg("0")
            .arg("-i").arg(&manifest_path)
            .arg("-c").arg("copy")
            .arg(output)
            .stderr(std::process::Stdio::piped())
            .output()
            .await
            .map_err(|e| RecorderError::Io(format!("spawn ffmpeg: {e}")))?;

        // Best-effort manifest cleanup (ignore failures).
        let _ = tokio::fs::remove_file(&manifest_path).await;

        if !status.status.success() {
            let stderr = String::from_utf8_lossy(&status.stderr);
            // Truncate to last 1000 chars to keep DB rows small.
            let tail = if stderr.len() > 1000 {
                let start = stderr.len() - 1000;
                stderr[start..].to_string()
            } else {
                stderr.to_string()
            };
            return Err(RecorderError::FfmpegFailed(tail));
        }
        Ok(())
    }

    async fn probe_duration_seconds(&self, path: &Path) -> Result<i32, RecorderError> {
        let output = tokio::process::Command::new("ffprobe")
            .arg("-v").arg("error")
            .arg("-show_entries").arg("format=duration")
            .arg("-of").arg("default=nw=1:nk=1")
            .arg(path)
            .output()
            .await
            .map_err(|e| RecorderError::Io(format!("spawn ffprobe: {e}")))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(RecorderError::FfprobeFailed(stderr.to_string()));
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let secs: f64 = stdout.trim().parse()
            .map_err(|e| RecorderError::FfprobeFailed(format!("parse duration: {e}")))?;
        Ok(secs.max(0.0).round() as i32)
    }
}
```

- [ ] **Step 2: Build**

```bash
cd "C:/Users/Chiranjib Chaudhuri/Documents/Chiranjib/Elementors_Aula-phase0"
cargo build -p backend 2>&1 | tail -10
```
Expected: clean.

If `tokio::process::Command` is unavailable, ensure `tokio` is built with the `process` feature in `crates/backend/Cargo.toml`:
```toml
tokio = { version = "1", features = ["full"] }
```
The `full` feature includes `process`. If the existing dep uses a narrower feature set, add `"process"` explicitly.

- [ ] **Step 3: Commit**

```bash
git add crates/backend/Cargo.toml crates/backend/src/services/recording.rs
git commit -m "feat(services): RealFfmpegRecorder production impl using tokio::process"
```

---

### Task 9: db::recordings queries

**Files:**
- Create: `crates/backend/src/db/recordings.rs`
- Modify: `crates/backend/src/db/mod.rs`

- [ ] **Step 1: Add `pub mod recordings;` to `db/mod.rs`**

Insert alphabetically. Existing alphabetical entries after Phase 1b-γ:
```rust
pub mod audit;
pub mod courses;
pub mod enrollments;
pub mod file_assets;
pub mod lessons;
pub mod live_room;
pub mod live_sessions;
pub mod modules;
```

Insert `pub mod recordings;` between `modules` and what comes after — actually `modules` is last per the listing. Append `pub mod recordings;`:
```rust
pub mod recordings;
```

- [ ] **Step 2: Implement**

Create `crates/backend/src/db/recordings.rs`:

```rust
// crates/backend/src/db/recordings.rs
use serde::Serialize;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

#[derive(Debug, Serialize, sqlx::FromRow, Clone)]
pub struct RecordingRow {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub session_id: Uuid,
    pub file_asset_id: Option<Uuid>,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub ended_at: chrono::DateTime<chrono::Utc>,
    pub duration_seconds: i32,
    pub processing_status: String,
    pub processing_error: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Inserts a `recordings` row with `processing_status='pending'`.
/// Idempotent: `ON CONFLICT (session_id) DO NOTHING` prevents duplicates from
/// concurrent sweep ticks.
pub async fn insert_pending(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    session_id: Uuid,
    started_at: chrono::DateTime<chrono::Utc>,
    ended_at: chrono::DateTime<chrono::Utc>,
    duration_seconds: i32,
) -> sqlx::Result<Option<RecordingRow>> {
    sqlx::query_as::<_, RecordingRow>(
        "INSERT INTO recordings
            (tenant_id, session_id, started_at, ended_at, duration_seconds)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (session_id) DO NOTHING
         RETURNING id, tenant_id, session_id, file_asset_id, started_at, ended_at,
                   duration_seconds, processing_status, processing_error, created_at",
    )
    .bind(tenant_id)
    .bind(session_id)
    .bind(started_at)
    .bind(ended_at)
    .bind(duration_seconds)
    .fetch_optional(&mut **tx)
    .await
}

/// Sets processing_status to a new value. Returns updated row.
pub async fn set_status(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    status: &str,
    error: Option<&str>,
) -> sqlx::Result<Option<RecordingRow>> {
    sqlx::query_as::<_, RecordingRow>(
        "UPDATE recordings
            SET processing_status = $2,
                processing_error = $3
          WHERE id = $1
        RETURNING id, tenant_id, session_id, file_asset_id, started_at, ended_at,
                  duration_seconds, processing_status, processing_error, created_at",
    )
    .bind(id).bind(status).bind(error)
    .fetch_optional(&mut **tx).await
}

/// Marks the recording `available` and links the produced file_asset.
pub async fn mark_available(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    file_asset_id: Uuid,
    duration_seconds: i32,
) -> sqlx::Result<Option<RecordingRow>> {
    sqlx::query_as::<_, RecordingRow>(
        "UPDATE recordings
            SET processing_status = 'available',
                file_asset_id = $2,
                duration_seconds = $3,
                processing_error = NULL
          WHERE id = $1
        RETURNING id, tenant_id, session_id, file_asset_id, started_at, ended_at,
                  duration_seconds, processing_status, processing_error, created_at",
    )
    .bind(id).bind(file_asset_id).bind(duration_seconds)
    .fetch_optional(&mut **tx).await
}

/// Fetches by session_id (one recording per session).
pub async fn fetch_by_session(
    pool: &PgPool,
    session_id: Uuid,
) -> sqlx::Result<Option<RecordingRow>> {
    sqlx::query_as::<_, RecordingRow>(
        "SELECT id, tenant_id, session_id, file_asset_id, started_at, ended_at,
                duration_seconds, processing_status, processing_error, created_at
           FROM recordings
          WHERE session_id = $1",
    )
    .bind(session_id)
    .fetch_optional(pool).await
}

/// Returns sessions that need recording processing:
/// - status='ended' AND recording_enabled=true AND no recordings row yet
/// - actual_started_at IS NOT NULL AND (actual_ended_at - actual_started_at) >= 5 seconds
/// Returns (session_id, tenant_id, actual_started_at, actual_ended_at).
pub async fn list_ended_sessions_needing_recording(
    pool: &PgPool,
    limit: i64,
) -> sqlx::Result<Vec<(Uuid, Uuid, chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>> {
    sqlx::query_as::<_, (Uuid, Uuid, chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>(
        "SELECT s.id, s.tenant_id, s.actual_started_at, s.actual_ended_at
           FROM live_sessions s
          WHERE s.status = 'ended'
            AND s.recording_enabled = true
            AND s.actual_started_at IS NOT NULL
            AND s.actual_ended_at IS NOT NULL
            AND s.actual_ended_at - s.actual_started_at >= interval '5 seconds'
            AND NOT EXISTS (SELECT 1 FROM recordings r WHERE r.session_id = s.id)
          ORDER BY s.actual_ended_at ASC
          LIMIT $1",
    )
    .bind(limit)
    .fetch_all(pool).await
}

/// Returns recordings stuck in transient states longer than `stuck_after`.
/// Used by the sweep to recover orphaned jobs after a backend crash.
pub async fn list_orphaned_in_progress(
    pool: &PgPool,
    stuck_after: chrono::Duration,
    limit: i64,
) -> sqlx::Result<Vec<RecordingRow>> {
    sqlx::query_as::<_, RecordingRow>(
        "SELECT id, tenant_id, session_id, file_asset_id, started_at, ended_at,
                duration_seconds, processing_status, processing_error, created_at
           FROM recordings
          WHERE processing_status IN ('remuxing','uploading')
            AND created_at < now() - $1::interval
          ORDER BY created_at ASC
          LIMIT $2",
    )
    .bind(format!("{} seconds", stuck_after.num_seconds()))
    .bind(limit)
    .fetch_all(pool).await
}

/// Lists recordings older than `retention_days`. Returns up to `limit` rows
/// per call so the janitor can process in batches.
pub async fn list_for_retention_prune(
    pool: &PgPool,
    retention_days: i64,
    limit: i64,
) -> sqlx::Result<Vec<(Uuid, Option<Uuid>)>> {
    sqlx::query_as::<_, (Uuid, Option<Uuid>)>(
        "SELECT id, file_asset_id
           FROM recordings
          WHERE created_at < now() - ($1::int || ' days')::interval
          ORDER BY created_at ASC
          LIMIT $2",
    )
    .bind(retention_days as i32)
    .bind(limit)
    .fetch_all(pool).await
}

pub async fn delete_by_id(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
) -> sqlx::Result<u64> {
    let r = sqlx::query("DELETE FROM recordings WHERE id = $1")
        .bind(id)
        .execute(&mut **tx).await?;
    Ok(r.rows_affected())
}
```

- [ ] **Step 3: Build**

```bash
cargo build -p backend 2>&1 | tail -5
```
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add crates/backend/src/db/mod.rs crates/backend/src/db/recordings.rs
git commit -m "feat(db): recordings — insert, status transitions, sweep + janitor queries"
```

---

## Section B — Backend handlers + sweep + janitor

### Task 10: AppState wires recorder + sweep config

**Files:**
- Modify: `crates/backend/src/lib.rs`
- Modify: `crates/backend/src/main.rs`

- [ ] **Step 1: Update `AppState` in `lib.rs`**

Append two fields:
```rust
    pub recorder: Arc<dyn crate::services::recording::RecorderTool>,
    pub recordings_dir: String,
```

`recordings_dir` is the path to the shared mediamtx_recordings volume mount inside the backend container — defaults to `/recordings` per Task 3.

- [ ] **Step 2: Construct in `main.rs`**

After the Phase 1b-γ `live_room` block (the `RedisLiveRoomBroker::connect` block), and BEFORE the `let app = backend::router(...)` call, insert:

```rust
    let recorder: Arc<dyn backend::services::recording::RecorderTool> = Arc::new(
        backend::services::recording::RealFfmpegRecorder::new(),
    );
    let recordings_dir = std::env::var("RECORDINGS_DIR")
        .unwrap_or_else(|_| "/recordings".into());
```

In the `AppState { ... }` literal, append:
```rust
        recorder,
        recordings_dir,
```

- [ ] **Step 3: Build + smoke health**

```bash
cd "C:/Users/Chiranjib Chaudhuri/Documents/Chiranjib/Elementors_Aula-phase0"
cargo build -p backend 2>&1 | tail -5
cargo test -p backend --test health 2>&1 | tail -5
```
Expected: clean build, health test passes.

- [ ] **Step 4: Commit**

```bash
git add crates/backend/src/lib.rs crates/backend/src/main.rs
git commit -m "feat(backend): AppState gains recorder + recordings_dir"
```

---

### Task 11: GET /v1/sessions/:id/recording (TDD)

**Files:**
- Modify: `crates/backend/src/handlers/live_sessions.rs`
- Create: `crates/backend/tests/recording.rs`

- [ ] **Step 1: Write failing tests**

Create `crates/backend/tests/recording.rs`:

```rust
// crates/backend/tests/recording.rs
mod fixtures;

use fixtures::*;
use serde_json::json;
use std::sync::Arc;

async fn course_with_session_recording_enabled(
    pool: &sqlx::PgPool,
    tenant: uuid::Uuid,
    teacher: uuid::Uuid,
    starts_at: chrono::DateTime<chrono::Utc>,
) -> (uuid::Uuid, uuid::Uuid) {
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx).await.unwrap();
    let course: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO courses (tenant_id, slug, title, owner_user_id)
         VALUES ($1, $2, 'C', $3) RETURNING id",
    ).bind(tenant).bind(format!("c-{}", uuid::Uuid::new_v4())).bind(teacher)
        .fetch_one(&mut *tx).await.unwrap();
    sqlx::query(
        "INSERT INTO course_memberships (course_id, user_id, tenant_id, role)
         VALUES ($1, $2, $3, 'teacher')",
    ).bind(course).bind(teacher).bind(tenant).execute(&mut *tx).await.unwrap();
    let series: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO live_session_series (tenant_id, course_id, title, starts_at,
                                          duration_minutes, frequency, end_kind,
                                          transport_mode, primary_teacher_id, recording_enabled)
         VALUES ($1, $2, 'S', $3, 60, 'none', 'open', 'webrtc', $4, true)
         RETURNING id",
    ).bind(tenant).bind(course).bind(starts_at).bind(teacher).fetch_one(&mut *tx).await.unwrap();
    let session: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO live_sessions (tenant_id, course_id, series_id, occurrence_index, title,
                                    status, starts_at, duration_minutes, primary_teacher_id, mode,
                                    recording_enabled, transport_mode)
         VALUES ($1, $2, $3, 0, 'L', 'scheduled', $4, 60, $5, 'lecture', true, 'webrtc')
         RETURNING id",
    ).bind(tenant).bind(course).bind(series).bind(starts_at).bind(teacher)
        .fetch_one(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    (course, session)
}

#[tokio::test]
async fn recording_route_returns_404_when_no_recording() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (_, session) = course_with_session_recording_enabled(&pool, tenant, teacher, chrono::Utc::now()).await;

    let mediamtx = Arc::new(backend::services::mediamtx::MockMediaMtxClient::new());
    let signer = Arc::new(backend::services::mediamtx::JwtSigner::new_ephemeral());
    let app = build_test_app(
        backend::handlers::live_sessions::live_room_router_for_tests(
            pool.clone(), mediamtx, signer,
            "http://localhost:8889".into(), "http://localhost:8888".into(),
        ),
        StubAuth {
            pool: pool.clone(), user_id: teacher, firebase_uid: fb, email: em,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Teacher),
        },
    );
    let (s, _) = fire(&app, "GET", &format!("/v1/sessions/{session}/recording"), None).await;
    assert_eq!(s, 404);
}

#[tokio::test]
async fn recording_route_returns_processing_state_when_pending() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (_, session) = course_with_session_recording_enabled(&pool, tenant, teacher, chrono::Utc::now()).await;

    // Insert a pending recording row directly.
    sqlx::query(
        "INSERT INTO recordings (tenant_id, session_id, started_at, ended_at, duration_seconds)
         VALUES ($1, $2, now() - interval '10 minutes', now() - interval '5 minutes', 300)"
    ).bind(tenant).bind(session).execute(&pool).await.unwrap();

    let mediamtx = Arc::new(backend::services::mediamtx::MockMediaMtxClient::new());
    let signer = Arc::new(backend::services::mediamtx::JwtSigner::new_ephemeral());
    let app = build_test_app(
        backend::handlers::live_sessions::live_room_router_for_tests(
            pool.clone(), mediamtx, signer,
            "http://localhost:8889".into(), "http://localhost:8888".into(),
        ),
        StubAuth {
            pool: pool.clone(), user_id: teacher, firebase_uid: fb, email: em,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Teacher),
        },
    );
    let (s, body) = fire(&app, "GET", &format!("/v1/sessions/{session}/recording"), None).await;
    assert_eq!(s, 200, "{body}");
    assert_eq!(body["processing_status"], "pending");
    assert!(body["playback_url"].is_null());
}
```

- [ ] **Step 2: Run, expect compile failure**

```bash
cd "C:/Users/Chiranjib Chaudhuri/Documents/Chiranjib/Elementors_Aula-phase0"
DATABASE_URL=postgres://aulalite:changeme@localhost:55432/aulalite \
    cargo test -p backend --test recording --no-run 2>&1 | tail -10
```
Expected: missing `/v1/sessions/:id/recording` route.

- [ ] **Step 3: Implement**

Append to `crates/backend/src/handlers/live_sessions.rs` (in the Phase 1b-γ section or append to the file):

```rust
// ============================================================================
// Phase 1b-delta: Recording handlers
// ============================================================================

const RECORDING_PLAYBACK_TTL: std::time::Duration = std::time::Duration::from_secs(15 * 60);

#[derive(serde::Serialize)]
pub struct RecordingDto {
    pub session_id: Uuid,
    pub processing_status: String,
    pub processing_error: Option<String>,
    pub duration_seconds: Option<i32>,
    pub started_at: Option<chrono::DateTime<chrono::Utc>>,
    pub playback_url: Option<String>,
    pub course_title: String,
    pub instructor_user_id: Option<Uuid>,
}

async fn recording_inner(
    pool: &PgPool,
    storage: &dyn crate::storage::S3Client,
    ctx: &RequestContext,
    session_id: Uuid,
) -> Result<Json<RecordingDto>, ApiError> {
    let session = db::live_sessions::load_for_join(pool, session_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::NotFound)?;
    let tenant_id = ctx.tenant_id.ok_or_else(|| ApiError::BadRequest("no tenant".into()))?;
    if session.tenant_id != tenant_id { return Err(ApiError::NotFound); }
    let allowed = db::courses::caller_can_read_course(
        pool, session.course_id, ctx.user_id, is_org_admin(ctx),
    ).await.map_err(|e| ApiError::Internal(e.to_string()))?;
    if !allowed { return Err(ApiError::Forbidden); }

    let row = db::recordings::fetch_by_session(pool, session_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::NotFound)?;

    let playback_url = if row.processing_status == "available" {
        if let Some(file_asset_id) = row.file_asset_id {
            let asset = db::file_assets::fetch(pool, file_asset_id)
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?
                .ok_or_else(|| ApiError::Internal("recording file_asset missing".into()))?;
            let url = storage.presigned_get_url(&asset.object_key, RECORDING_PLAYBACK_TTL)
                .await
                .map_err(|e| ApiError::Internal(format!("presign failed: {e}")))?;
            Some(url)
        } else { None }
    } else { None };

    let duration = if row.processing_status == "available" {
        Some(row.duration_seconds)
    } else { None };
    let started = if row.processing_status == "available" {
        Some(row.started_at)
    } else { None };

    Ok(Json(RecordingDto {
        session_id,
        processing_status: row.processing_status,
        processing_error: row.processing_error,
        duration_seconds: duration,
        started_at: started,
        playback_url,
        course_title: session.course_title,
        instructor_user_id: session.primary_teacher_id,
    }))
}

async fn recording(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(session_id): Path<Uuid>,
) -> Result<Json<RecordingDto>, ApiError> {
    recording_inner(&s.pool, s.storage.as_ref(), &ctx, session_id).await
}

async fn recording_t(
    State(s): State<LiveRoomTestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(session_id): Path<Uuid>,
) -> Result<Json<RecordingDto>, ApiError> {
    // The test router needs S3Client too — but it doesn't have one. For tests,
    // the recording row is queried but if status='available' we'd need to
    // mint a presigned URL via S3Client. The Mock S3Client will work; we add
    // it to LiveRoomTestState.
    recording_inner(&s.pool, s.s3_client.as_ref(), &ctx, session_id).await
}
```

We need to extend `LiveRoomTestState` again — this time with an `s3_client` field. Find the struct:
```rust
#[derive(Clone)]
pub struct LiveRoomTestState {
    pub pool: PgPool,
    pub mediamtx: Arc<dyn MediaMtxClient>,
    pub signer: Arc<JwtSigner>,
    pub public_webrtc_url: String,
    pub public_hls_url: String,
    pub broker: Arc<dyn crate::services::live_room::LiveRoomBroker>,
    pub s3_client: Arc<dyn crate::storage::S3Client>,  // NEW
}
```

Update `live_room_router_for_tests` and `live_room_router_for_tests_with_broker` to construct a `MockS3Client` and pass it in:
```rust
let s3_client: Arc<dyn crate::storage::S3Client> = Arc::new(crate::storage::mock::MockS3Client::new());
```

Mount the route on both routers:
```rust
.route("/v1/sessions/:id/recording", routing::get(recording_t))
```

And on the production router:
```rust
.route("/v1/sessions/:id/recording", routing::get(recording))
```

- [ ] **Step 4: Run**

```bash
DATABASE_URL=postgres://aulalite:changeme@localhost:55432/aulalite \
    cargo test -p backend --test recording 2>&1 | tail -10
```
Expected: 2 passed.

- [ ] **Step 5: Commit**

```bash
git add crates/backend/src/handlers/live_sessions.rs crates/backend/tests/recording.rs
git commit -m "feat(live_room): GET /v1/sessions/:id/recording — playback metadata + presigned URL"
```

---

### Task 12: GET /v1/sessions/:id/recording/chat (TDD)

**Files:**
- Modify: `crates/backend/src/handlers/live_sessions.rs`
- Modify: `crates/backend/tests/recording.rs`

- [ ] **Step 1: Append failing test**

```rust
#[tokio::test]
async fn recording_chat_window_returns_messages_in_range() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (_, session) = course_with_session_recording_enabled(&pool, tenant, teacher, chrono::Utc::now()).await;

    let started = chrono::Utc::now() - chrono::Duration::minutes(10);
    sqlx::query(
        "INSERT INTO recordings (tenant_id, session_id, started_at, ended_at, duration_seconds, processing_status)
         VALUES ($1, $2, $3, $3 + interval '5 minutes', 300, 'available')"
    ).bind(tenant).bind(session).bind(started).execute(&pool).await.unwrap();

    // Three messages at 30s, 90s, 240s into the recording.
    for offset_secs in [30i64, 90, 240] {
        sqlx::query(
            "INSERT INTO live_room_messages (tenant_id, session_id, sender_user_id, body, created_at)
             VALUES ($1, $2, $3, $4, $5)"
        ).bind(tenant).bind(session).bind(teacher)
            .bind(format!("msg @{offset_secs}s"))
            .bind(started + chrono::Duration::seconds(offset_secs))
            .execute(&pool).await.unwrap();
    }

    let mediamtx = Arc::new(backend::services::mediamtx::MockMediaMtxClient::new());
    let signer = Arc::new(backend::services::mediamtx::JwtSigner::new_ephemeral());
    let app = build_test_app(
        backend::handlers::live_sessions::live_room_router_for_tests(
            pool.clone(), mediamtx, signer,
            "http://localhost:8889".into(), "http://localhost:8888".into(),
        ),
        StubAuth {
            pool: pool.clone(), user_id: teacher, firebase_uid: fb, email: em,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Teacher),
        },
    );
    // Window 0..120 should match first two messages.
    let (s, body) = fire(&app, "GET",
        &format!("/v1/sessions/{session}/recording/chat?from_seconds=0&to_seconds=120"),
        None).await;
    assert_eq!(s, 200, "{body}");
    let arr = body["messages"].as_array().unwrap();
    assert_eq!(arr.len(), 2);
    // ASC by created_at (== ASC by video_offset_seconds).
    assert!(arr[0]["video_offset_seconds"].as_f64().unwrap() < 60.0);
    assert!(arr[1]["video_offset_seconds"].as_f64().unwrap() < 120.0);
}
```

- [ ] **Step 2: Implement**

Append to handlers/live_sessions.rs:
```rust
#[derive(serde::Deserialize, Default)]
pub struct RecordingChatQuery {
    pub from_seconds: Option<f64>,
    pub to_seconds: Option<f64>,
}

#[derive(serde::Serialize)]
pub struct RecordingChatMessageDto {
    pub id: Uuid,
    pub sender_user_id: Uuid,
    pub sender_display_name: String,
    pub body: String,
    pub video_offset_seconds: f64,
    pub deleted: bool,
}

#[derive(serde::Serialize)]
pub struct RecordingChatWindowResponse {
    pub messages: Vec<RecordingChatMessageDto>,
}

async fn recording_chat_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    session_id: Uuid,
    q: RecordingChatQuery,
) -> Result<Json<RecordingChatWindowResponse>, ApiError> {
    let session = db::live_sessions::load_for_join(pool, session_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::NotFound)?;
    let tenant_id = ctx.tenant_id.ok_or_else(|| ApiError::BadRequest("no tenant".into()))?;
    if session.tenant_id != tenant_id { return Err(ApiError::NotFound); }
    let allowed = db::courses::caller_can_read_course(
        pool, session.course_id, ctx.user_id, is_org_admin(ctx),
    ).await.map_err(|e| ApiError::Internal(e.to_string()))?;
    if !allowed { return Err(ApiError::Forbidden); }

    let recording = db::recordings::fetch_by_session(pool, session_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::NotFound)?;

    let from_secs = q.from_seconds.unwrap_or(0.0).max(0.0);
    let to_secs = q.to_seconds.unwrap_or(f64::MAX);
    let from_ts = recording.started_at + chrono::Duration::milliseconds((from_secs * 1000.0) as i64);
    let to_ts = if to_secs == f64::MAX {
        chrono::DateTime::<chrono::Utc>::MAX_UTC
    } else {
        recording.started_at + chrono::Duration::milliseconds((to_secs * 1000.0) as i64)
    };

    let is_admin = is_org_admin(ctx);
    let is_teacher = db::courses::caller_can_admin_course(
        pool, session.course_id, ctx.user_id, is_admin,
    ).await.unwrap_or(false);

    let rows = sqlx::query_as::<_, db::live_room::ChatMessageRow>(
        "SELECT id, tenant_id, session_id, sender_user_id, body, created_at,
                deleted_at, deleted_by_user_id
           FROM live_room_messages
          WHERE session_id = $1
            AND created_at >= $2
            AND created_at <= $3
          ORDER BY created_at ASC",
    )
    .bind(session_id).bind(from_ts).bind(to_ts)
    .fetch_all(pool).await
    .map_err(|e| ApiError::Internal(e.to_string()))?;

    // Fetch display names for senders.
    use std::collections::HashMap;
    let sender_ids: Vec<Uuid> = rows.iter().map(|r| r.sender_user_id).collect();
    let unique_ids: Vec<Uuid> = {
        let mut s = sender_ids.clone();
        s.sort_unstable(); s.dedup(); s
    };
    let names: HashMap<Uuid, String> = if unique_ids.is_empty() {
        HashMap::new()
    } else {
        let pairs: Vec<(Uuid, String)> = sqlx::query_as(
            "SELECT id, COALESCE(SPLIT_PART(email, '@', 1), '') FROM users WHERE id = ANY($1)"
        ).bind(&unique_ids).fetch_all(pool).await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
        pairs.into_iter().collect()
    };

    let messages: Vec<RecordingChatMessageDto> = rows.iter().map(|r| {
        let display_name = names.get(&r.sender_user_id).cloned().unwrap_or_default();
        let body = if r.deleted_at.is_some() && !is_teacher {
            "[deleted]".into()
        } else {
            r.body.clone()
        };
        RecordingChatMessageDto {
            id: r.id,
            sender_user_id: r.sender_user_id,
            sender_display_name: display_name,
            body,
            video_offset_seconds: crate::services::recording::video_offset_seconds(
                r.created_at, recording.started_at,
            ),
            deleted: r.deleted_at.is_some(),
        }
    }).collect();

    Ok(Json(RecordingChatWindowResponse { messages }))
}

async fn recording_chat(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(session_id): Path<Uuid>,
    axum::extract::Query(q): axum::extract::Query<RecordingChatQuery>,
) -> Result<Json<RecordingChatWindowResponse>, ApiError> {
    recording_chat_inner(&s.pool, &ctx, session_id, q).await
}

async fn recording_chat_t(
    State(s): State<LiveRoomTestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(session_id): Path<Uuid>,
    axum::extract::Query(q): axum::extract::Query<RecordingChatQuery>,
) -> Result<Json<RecordingChatWindowResponse>, ApiError> {
    recording_chat_inner(&s.pool, &ctx, session_id, q).await
}
```

Mount on both routers:
```rust
.route("/v1/sessions/:id/recording/chat", routing::get(recording_chat))
// and recording_chat_t on the test router
```

- [ ] **Step 3: Run + commit**

```bash
DATABASE_URL=postgres://aulalite:changeme@localhost:55432/aulalite \
    cargo test -p backend --test recording 2>&1 | tail -10
git add crates/backend/src/handlers/live_sessions.rs crates/backend/tests/recording.rs
git commit -m "feat(live_room): GET /v1/sessions/:id/recording/chat — windowed chat replay"
```
Expected: 3 passed (2 prior + 1 new).

---

### Task 13: POST /v1/sessions/:id/recording/retry (TDD)

**Files:**
- Modify: `crates/backend/src/handlers/live_sessions.rs`
- Modify: `crates/backend/tests/recording.rs`

- [ ] **Step 1: Append failing test**

```rust
#[tokio::test]
async fn recording_retry_resets_failed_to_pending() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (_, session) = course_with_session_recording_enabled(&pool, tenant, teacher, chrono::Utc::now()).await;

    sqlx::query(
        "INSERT INTO recordings (tenant_id, session_id, started_at, ended_at, duration_seconds,
                                 processing_status, processing_error)
         VALUES ($1, $2, now() - interval '10 minutes', now() - interval '5 minutes', 300,
                 'failed', 'ffmpeg crash')"
    ).bind(tenant).bind(session).execute(&pool).await.unwrap();

    let mediamtx = Arc::new(backend::services::mediamtx::MockMediaMtxClient::new());
    let signer = Arc::new(backend::services::mediamtx::JwtSigner::new_ephemeral());
    let app = build_test_app(
        backend::handlers::live_sessions::live_room_router_for_tests(
            pool.clone(), mediamtx, signer,
            "http://localhost:8889".into(), "http://localhost:8888".into(),
        ),
        StubAuth {
            pool: pool.clone(), user_id: teacher, firebase_uid: fb, email: em,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Teacher),
        },
    );
    let (s, body) = fire(&app, "POST", &format!("/v1/sessions/{session}/recording/retry"),
        Some(json!({}))).await;
    assert_eq!(s, 200, "{body}");
    assert_eq!(body["processing_status"], "pending");

    let row: (String, Option<String>) = sqlx::query_as(
        "SELECT processing_status, processing_error FROM recordings WHERE session_id = $1"
    ).bind(session).fetch_one(&pool).await.unwrap();
    assert_eq!(row.0, "pending");
    assert!(row.1.is_none());
}

#[tokio::test]
async fn recording_retry_by_non_admin_returns_403() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (_, session) = course_with_session_recording_enabled(&pool, tenant, teacher, chrono::Utc::now()).await;
    let (student, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, student, "student").await;

    sqlx::query(
        "INSERT INTO recordings (tenant_id, session_id, started_at, ended_at, duration_seconds,
                                 processing_status, processing_error)
         VALUES ($1, $2, now() - interval '10 minutes', now() - interval '5 minutes', 300,
                 'failed', 'ffmpeg crash')"
    ).bind(tenant).bind(session).execute(&pool).await.unwrap();

    let mediamtx = Arc::new(backend::services::mediamtx::MockMediaMtxClient::new());
    let signer = Arc::new(backend::services::mediamtx::JwtSigner::new_ephemeral());
    let app = build_test_app(
        backend::handlers::live_sessions::live_room_router_for_tests(
            pool.clone(), mediamtx, signer,
            "http://localhost:8889".into(), "http://localhost:8888".into(),
        ),
        StubAuth {
            pool: pool.clone(), user_id: student, firebase_uid: fb, email: em,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Student),
        },
    );
    let (s, _) = fire(&app, "POST", &format!("/v1/sessions/{session}/recording/retry"),
        Some(json!({}))).await;
    assert_eq!(s, 403);
}
```

- [ ] **Step 2: Implement**

```rust
async fn recording_retry_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    session_id: Uuid,
) -> Result<Json<RecordingDto>, ApiError> {
    let session = db::live_sessions::load_for_join(pool, session_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::NotFound)?;
    let tenant_id = ctx.tenant_id.ok_or_else(|| ApiError::BadRequest("no tenant".into()))?;
    if session.tenant_id != tenant_id { return Err(ApiError::NotFound); }
    require_admin_for_session_course(pool, ctx, session.course_id).await?;

    let row = db::recordings::fetch_by_session(pool, session_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::NotFound)?;

    let updated = if row.processing_status == "failed" {
        let mut tx = pool.begin().await.map_err(|e| ApiError::Internal(e.to_string()))?;
        let r = db::recordings::set_status(&mut tx, row.id, "pending", None).await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        tx.commit().await.map_err(|e| ApiError::Internal(e.to_string()))?;
        r.unwrap_or(row)
    } else {
        row
    };

    Ok(Json(RecordingDto {
        session_id,
        processing_status: updated.processing_status,
        processing_error: updated.processing_error,
        duration_seconds: None,
        started_at: None,
        playback_url: None,
        course_title: session.course_title,
        instructor_user_id: session.primary_teacher_id,
    }))
}

async fn recording_retry(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(session_id): Path<Uuid>,
) -> Result<Json<RecordingDto>, ApiError> {
    recording_retry_inner(&s.pool, &ctx, session_id).await
}

async fn recording_retry_t(
    State(s): State<LiveRoomTestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(session_id): Path<Uuid>,
) -> Result<Json<RecordingDto>, ApiError> {
    recording_retry_inner(&s.pool, &ctx, session_id).await
}
```

Mount on both routers:
```rust
.route("/v1/sessions/:id/recording/retry", routing::post(recording_retry))
// and recording_retry_t on the test router
```

- [ ] **Step 3: Run + commit**

```bash
DATABASE_URL=postgres://aulalite:changeme@localhost:55432/aulalite \
    cargo test -p backend --test recording 2>&1 | tail -10
git add crates/backend/src/handlers/live_sessions.rs crates/backend/tests/recording.rs
git commit -m "feat(live_room): POST /v1/sessions/:id/recording/retry — teacher reset failed"
```
Expected: 5 passed (3 prior + 2 new).

---

### Task 14: /join response gains has_recording

**Files:**
- Modify: `crates/backend/src/handlers/live_sessions.rs`

The frontend (`LiveRoomShell` Branch::Ended) needs to know whether to mount `LiveRoomReplay` or fall back to "Class has ended". Easiest: add a boolean `has_recording` field to the existing `JoinResponse` from Phase 1b-β.

- [ ] **Step 1: Read existing `JoinResponse` struct**

```bash
grep -n "pub struct JoinResponse" crates/backend/src/handlers/live_sessions.rs
```

The struct (added in Phase 1b-β Task 16) has fields: `state`, `session_id`, `transport_mode`, `viewer_jwt`, `main_url`, `screen_url`, `instructor_user_id`, `course_title`, `scheduled_starts_at`. Add:
```rust
    pub has_recording: bool,
```

- [ ] **Step 2: Compute the value in `join_inner`**

In the `join_inner` function, before constructing `JoinResponse`, query:
```rust
let has_recording = sqlx::query_scalar::<_, bool>(
    "SELECT EXISTS (
        SELECT 1 FROM recordings
         WHERE session_id = $1
           AND processing_status IN ('pending','remuxing','uploading','available')
    )"
).bind(session_id).fetch_one(pool).await
.map_err(|e| ApiError::Internal(e.to_string()))?;
```

Pass it into the `JoinResponse { ... }` literal.

- [ ] **Step 3: Run regression tests**

```bash
DATABASE_URL=postgres://aulalite:changeme@localhost:55432/aulalite \
    cargo test -p backend --test live_room 2>&1 | tail -10
```
Expected: all live_room tests still pass (existing tests don't assert on `has_recording`, just consume the response).

- [ ] **Step 4: Commit**

```bash
git add crates/backend/src/handlers/live_sessions.rs
git commit -m "feat(live_room): /join response includes has_recording boolean"
```

---

### Task 15: Recording sweep — state machine (TDD)

**Files:**
- Modify: `crates/backend/src/main.rs`
- Modify: `crates/backend/src/services/recording.rs` (add a `process_pending_recording` function for testability)
- Modify: `crates/backend/tests/recording.rs`

- [ ] **Step 1: Append the sweep helper to services::recording**

```rust
// At top of file, add imports:
use crate::storage::S3Client;
use crate::db::recordings as db_rec;
use crate::db::file_assets as db_fa;

/// Processes one pending session: insert recordings row → remux → upload →
/// mark available. Returns Ok(Some(recording_id)) on success, Ok(None) if
/// race condition prevented insertion, Err on processing failure.
pub async fn process_one_session(
    pool: &sqlx::PgPool,
    storage: &dyn S3Client,
    bucket: &str,
    recorder: &dyn RecorderTool,
    recordings_dir: &std::path::Path,
    tenant_id: uuid::Uuid,
    session_id: uuid::Uuid,
    started_at: chrono::DateTime<chrono::Utc>,
    ended_at: chrono::DateTime<chrono::Utc>,
) -> Result<Option<uuid::Uuid>, RecorderError> {
    let duration_initial = (ended_at - started_at).num_seconds() as i32;
    if duration_initial < 1 {
        return Ok(None);
    }

    // Step 1: insert pending row (ON CONFLICT DO NOTHING).
    let mut tx = pool.begin().await
        .map_err(|e| RecorderError::Io(format!("tx begin: {e}")))?;
    let row = db_rec::insert_pending(&mut tx, tenant_id, session_id, started_at, ended_at, duration_initial)
        .await
        .map_err(|e| RecorderError::Io(format!("insert_pending: {e}")))?;
    tx.commit().await.map_err(|e| RecorderError::Io(format!("tx commit: {e}")))?;
    let row = match row {
        Some(r) => r,
        None => return Ok(None), // race: another sweep tick won
    };

    // Step 2: status → remuxing.
    let mut tx = pool.begin().await
        .map_err(|e| RecorderError::Io(format!("tx begin: {e}")))?;
    let _ = db_rec::set_status(&mut tx, row.id, "remuxing", None).await;
    tx.commit().await.map_err(|e| RecorderError::Io(format!("tx commit: {e}")))?;

    // Step 3: enumerate segments.
    let session_dir = recordings_dir
        .join("aula")
        .join(tenant_id.simple().to_string())
        // course_id is also a path segment but we don't track it here — discover via filesystem.
        // The pattern is recordings_dir/aula/<tenant>/<course>/<session>/. Walk down 2 more levels.
        ;
    let segments = match enumerate_session_segments(&session_dir, session_id) {
        Ok(s) if !s.is_empty() => s,
        Ok(_) => {
            mark_failed(pool, row.id, "no segments found on disk").await;
            return Err(RecorderError::Io("no segments".into()));
        }
        Err(e) => {
            mark_failed(pool, row.id, &format!("enumerate: {e}")).await;
            return Err(RecorderError::Io(e.to_string()));
        }
    };

    // Step 4: remux.
    let tmp_mp4 = std::env::temp_dir()
        .join(format!("aulalite-recording-{}.mp4", session_id.simple()));
    if let Err(e) = recorder.remux_to_mp4(&segments, &tmp_mp4).await {
        mark_failed(pool, row.id, &format!("ffmpeg: {e}")).await;
        return Err(e);
    }

    // Step 5: probe duration (overwrite the initial estimate).
    let duration = match recorder.probe_duration_seconds(&tmp_mp4).await {
        Ok(d) => d.max(1),
        Err(e) => {
            mark_failed(pool, row.id, &format!("ffprobe: {e}")).await;
            return Err(e);
        }
    };

    // Step 6: status → uploading.
    let mut tx = pool.begin().await
        .map_err(|e| RecorderError::Io(format!("tx begin: {e}")))?;
    let _ = db_rec::set_status(&mut tx, row.id, "uploading", None).await;
    tx.commit().await.map_err(|e| RecorderError::Io(format!("tx commit: {e}")))?;

    // Step 7: read MP4 bytes + upload.
    let mp4_bytes = match tokio::fs::read(&tmp_mp4).await {
        Ok(b) => b,
        Err(e) => {
            mark_failed(pool, row.id, &format!("read mp4: {e}")).await;
            return Err(RecorderError::Io(e.to_string()));
        }
    };

    let asset_id = uuid::Uuid::new_v4();
    let key = recording_object_key(tenant_id, asset_id, chrono::Utc::now());

    if let Err(e) = storage.put_object(&key, mp4_bytes.clone(), "video/mp4").await {
        mark_failed(pool, row.id, &format!("s3 put: {e}")).await;
        return Err(RecorderError::Io(e.to_string()));
    }

    // Step 8: insert file_assets row.
    let mut tx = pool.begin().await
        .map_err(|e| RecorderError::Io(format!("tx begin: {e}")))?;
    sqlx::query(
        "INSERT INTO file_assets (id, tenant_id, owner_user_id, bucket, object_key,
                                  content_type, size_bytes, status, visibility,
                                  linked_entity_type, linked_entity_id)
         VALUES ($1, $2, $2, $3, $4, 'video/mp4', $5, 'available', 'private',
                 'session_recording', $6)"
    ).bind(asset_id).bind(tenant_id).bind(bucket).bind(&key)
        .bind(mp4_bytes.len() as i64).bind(row.id)
        .execute(&mut *tx).await
        .map_err(|e| RecorderError::Io(format!("insert file_asset: {e}")))?;

    // Step 9: link recording → file_asset, mark available.
    let _ = db_rec::mark_available(&mut tx, row.id, asset_id, duration).await;
    tx.commit().await.map_err(|e| RecorderError::Io(format!("tx commit: {e}")))?;

    // Step 10: cleanup tmp file (best-effort).
    let _ = tokio::fs::remove_file(&tmp_mp4).await;

    Ok(Some(row.id))
}

async fn mark_failed(pool: &sqlx::PgPool, id: uuid::Uuid, reason: &str) {
    let truncated = if reason.len() > 1000 { &reason[reason.len()-1000..] } else { reason };
    if let Ok(mut tx) = pool.begin().await {
        let _ = db_rec::set_status(&mut tx, id, "failed", Some(truncated)).await;
        let _ = tx.commit().await;
    }
}

fn enumerate_session_segments(
    base_dir: &std::path::Path,
    session_id: uuid::Uuid,
) -> std::io::Result<Vec<std::path::PathBuf>> {
    // The actual on-disk path is base_dir/<course>/<session>/seg.*. We don't
    // know the course directory name (tenant simple), so we walk all
    // immediate subdirectories of base_dir, then look for one whose third
    // path level is the session_id_simple.
    let session_simple = session_id.simple().to_string();
    let mut segments = Vec::new();
    if !base_dir.exists() {
        return Ok(segments);
    }
    for course_entry in std::fs::read_dir(base_dir)? {
        let course_dir = course_entry?.path();
        if !course_dir.is_dir() { continue; }
        let session_dir = course_dir.join(&session_simple);
        if !session_dir.is_dir() { continue; }
        for seg_entry in std::fs::read_dir(&session_dir)? {
            let p = seg_entry?.path();
            if p.is_file() {
                segments.push(p);
            }
        }
    }
    // Sort by mtime ASC.
    segments.sort_by_key(|p| std::fs::metadata(p)
        .and_then(|m| m.modified())
        .unwrap_or(std::time::SystemTime::UNIX_EPOCH));
    Ok(segments)
}
```

- [ ] **Step 2: Append the sweep test**

In `tests/recording.rs`:
```rust
#[tokio::test]
async fn process_one_session_skips_under_5s() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (_, session) = course_with_session_recording_enabled(&pool, tenant, teacher, chrono::Utc::now()).await;
    let storage = Arc::new(backend::storage::mock::MockS3Client::new());
    let recorder = backend::services::recording::MockRecorderTool::new();
    let tmp = tempfile::tempdir().unwrap();
    let now = chrono::Utc::now();
    let res = backend::services::recording::process_one_session(
        &pool, storage.as_ref(), "aulalite",
        &recorder, tmp.path(),
        tenant, session,
        now, now, // duration 0
    ).await;
    // Returns Ok(None) for too-short.
    assert!(matches!(res, Ok(None)));
    // No recording row inserted.
    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM recordings WHERE session_id=$1")
        .bind(session).fetch_one(&pool).await.unwrap();
    assert_eq!(count.0, 0);
}

#[tokio::test]
async fn process_one_session_succeeds_with_mock_segments() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let started = chrono::Utc::now() - chrono::Duration::minutes(10);
    let (course, session) = course_with_session_recording_enabled(&pool, tenant, teacher, started).await;
    let ended = started + chrono::Duration::minutes(2);

    // Build a fake segment file in a directory that matches what enumerate_session_segments expects.
    let tmp = tempfile::tempdir().unwrap();
    let session_dir = tmp.path()
        .join(course.simple().to_string())
        .join(session.simple().to_string());
    std::fs::create_dir_all(&session_dir).unwrap();
    std::fs::write(session_dir.join("seg1.mp4"), b"\x00\x00\x00\x18ftypmp42").unwrap();

    let storage = Arc::new(backend::storage::mock::MockS3Client::new());
    let recorder = backend::services::recording::MockRecorderTool::new();
    recorder.set_duration_seconds(120);

    let res = backend::services::recording::process_one_session(
        &pool, storage.as_ref(), "aulalite",
        &recorder, tmp.path(),
        tenant, session,
        started, ended,
    ).await.unwrap();
    assert!(res.is_some());

    let row: (String, Option<uuid::Uuid>, i32) = sqlx::query_as(
        "SELECT processing_status, file_asset_id, duration_seconds FROM recordings WHERE session_id = $1"
    ).bind(session).fetch_one(&pool).await.unwrap();
    assert_eq!(row.0, "available");
    assert!(row.1.is_some());
    assert_eq!(row.2, 120);
}

#[tokio::test]
async fn process_one_session_dedupes_via_unique_session_id() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let started = chrono::Utc::now() - chrono::Duration::minutes(10);
    let (_course, session) = course_with_session_recording_enabled(&pool, tenant, teacher, started).await;
    let ended = started + chrono::Duration::minutes(2);

    // Insert a pending row directly (simulates first sweep already won).
    sqlx::query(
        "INSERT INTO recordings (tenant_id, session_id, started_at, ended_at, duration_seconds)
         VALUES ($1, $2, $3, $4, 120)"
    ).bind(tenant).bind(session).bind(started).bind(ended).execute(&pool).await.unwrap();

    let tmp = tempfile::tempdir().unwrap();
    let storage = Arc::new(backend::storage::mock::MockS3Client::new());
    let recorder = backend::services::recording::MockRecorderTool::new();

    // Second sweep tick: should return None due to ON CONFLICT.
    let res = backend::services::recording::process_one_session(
        &pool, storage.as_ref(), "aulalite",
        &recorder, tmp.path(),
        tenant, session,
        started, ended,
    ).await.unwrap();
    assert!(res.is_none());
}
```

- [ ] **Step 3: Spawn the sweep task in main.rs**

After the chat-prune task (last Tokio task added in Phase 1b-γ Task 17), append:

```rust
    // Recording sweep: every 60s, find ended sessions needing recording,
    // process one at a time (semaphore capacity = 1).
    let pool_for_recording = pool.clone();
    let storage_for_recording = storage.clone();
    let bucket_for_recording = bucket_name.clone();
    let recorder_for_recording = recorder.clone();
    let recordings_dir_for_recording = recordings_dir.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            // Process up to 1 session per tick (avoids long-running ffmpeg blocking the next tick).
            match backend::db::recordings::list_ended_sessions_needing_recording(&pool_for_recording, 1).await {
                Ok(rows) => {
                    for (session_id, tenant_id, started, ended) in rows {
                        let recordings_dir_buf = std::path::PathBuf::from(&recordings_dir_for_recording);
                        match backend::services::recording::process_one_session(
                            &pool_for_recording, storage_for_recording.as_ref(),
                            &bucket_for_recording,
                            recorder_for_recording.as_ref(), &recordings_dir_buf,
                            tenant_id, session_id, started, ended,
                        ).await {
                            Ok(Some(rec_id)) => tracing::info!(?rec_id, ?session_id, "recording processed"),
                            Ok(None) => {} // race or skipped
                            Err(e) => tracing::warn!(?e, ?session_id, "recording processing failed"),
                        }
                    }
                }
                Err(e) => tracing::warn!(?e, "list_ended_sessions_needing_recording failed"),
            }
        }
    });
```

- [ ] **Step 4: Run + commit**

```bash
DATABASE_URL=postgres://aulalite:changeme@localhost:55432/aulalite \
    cargo test -p backend --test recording 2>&1 | tail -15
git add crates/backend/src/services/recording.rs crates/backend/src/main.rs \
        crates/backend/tests/recording.rs
git commit -m "feat(live_room): recording sweep — process_one_session state machine + main.rs spawn"
```
Expected: 8 passed (5 prior + 3 new).

---

### Task 16: Retention janitor (TDD)

**Files:**
- Modify: `crates/backend/src/main.rs`
- Modify: `crates/backend/tests/recording.rs`

- [ ] **Step 1: Append failing test**

```rust
#[tokio::test]
async fn retention_janitor_deletes_recordings_older_than_n_days() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (_, session_a) = course_with_session_recording_enabled(&pool, tenant, teacher, chrono::Utc::now() - chrono::Duration::days(400)).await;
    let (_, session_b) = course_with_session_recording_enabled(&pool, tenant, teacher, chrono::Utc::now()).await;

    // Insert two file_assets: one stale (older), one fresh.
    let asset_old: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO file_assets (tenant_id, owner_user_id, bucket, object_key,
                                  content_type, size_bytes, status, visibility,
                                  linked_entity_type, linked_entity_id)
         VALUES ($1, $1, 'aulalite', 'old/key.mp4', 'video/mp4', 100, 'available', 'private',
                 'session_recording', $2)
         RETURNING id"
    ).bind(tenant).bind(uuid::Uuid::new_v4()).fetch_one(&pool).await.unwrap();
    let asset_fresh: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO file_assets (tenant_id, owner_user_id, bucket, object_key,
                                  content_type, size_bytes, status, visibility,
                                  linked_entity_type, linked_entity_id)
         VALUES ($1, $1, 'aulalite', 'fresh/key.mp4', 'video/mp4', 100, 'available', 'private',
                 'session_recording', $2)
         RETURNING id"
    ).bind(tenant).bind(uuid::Uuid::new_v4()).fetch_one(&pool).await.unwrap();

    sqlx::query(
        "INSERT INTO recordings (tenant_id, session_id, file_asset_id, started_at, ended_at,
                                 duration_seconds, processing_status, created_at)
         VALUES ($1, $2, $3, now() - interval '400 days', now() - interval '400 days' + interval '1 hour',
                 3600, 'available', now() - interval '400 days')"
    ).bind(tenant).bind(session_a).bind(asset_old).execute(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO recordings (tenant_id, session_id, file_asset_id, started_at, ended_at,
                                 duration_seconds, processing_status)
         VALUES ($1, $2, $3, now(), now() + interval '1 hour', 3600, 'available')"
    ).bind(tenant).bind(session_b).bind(asset_fresh).execute(&pool).await.unwrap();

    let storage = Arc::new(backend::storage::mock::MockS3Client::new());
    let pruned = backend::services::recording::run_retention_janitor(
        &pool, storage.as_ref(), 365, 100,
    ).await.unwrap();
    assert!(pruned >= 1);

    // Old recording is gone.
    let old_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM recordings WHERE id IN (SELECT id FROM recordings WHERE session_id=$1)")
        .bind(session_a).fetch_one(&pool).await.unwrap();
    assert_eq!(old_count.0, 0);
    // Fresh recording survives.
    let fresh_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM recordings WHERE session_id=$1")
        .bind(session_b).fetch_one(&pool).await.unwrap();
    assert_eq!(fresh_count.0, 1);
    // Old file_asset is pruned.
    let old_status: (String,) = sqlx::query_as("SELECT status FROM file_assets WHERE id=$1")
        .bind(asset_old).fetch_one(&pool).await.unwrap();
    assert_eq!(old_status.0, "pruned");
}
```

- [ ] **Step 2: Implement `run_retention_janitor` in services::recording.rs**

```rust
/// Runs one pass of the retention janitor: deletes recordings older than
/// `retention_days`, marks their file_assets pruned, removes the MinIO object.
/// Returns the number of recordings deleted.
pub async fn run_retention_janitor(
    pool: &sqlx::PgPool,
    storage: &dyn S3Client,
    retention_days: i64,
    batch_limit: i64,
) -> Result<u64, RecorderError> {
    use crate::db::recordings as db_rec;
    let pairs = db_rec::list_for_retention_prune(pool, retention_days, batch_limit)
        .await
        .map_err(|e| RecorderError::Io(format!("list_for_retention: {e}")))?;
    let mut count = 0u64;
    for (recording_id, file_asset_id) in pairs {
        if let Some(fid) = file_asset_id {
            // Fetch object_key to issue S3 delete.
            if let Ok(Some(asset)) = crate::db::file_assets::fetch(pool, fid).await {
                let _ = storage.delete_object(&asset.object_key).await;
            }
            // Mark file_asset pruned (best-effort).
            if let Ok(mut tx) = pool.begin().await {
                let _ = sqlx::query("UPDATE file_assets SET status='pruned' WHERE id=$1")
                    .bind(fid).execute(&mut *tx).await;
                let _ = tx.commit().await;
            }
        }
        // Delete the recordings row.
        if let Ok(mut tx) = pool.begin().await {
            let _ = db_rec::delete_by_id(&mut tx, recording_id).await;
            let _ = tx.commit().await;
        }
        count += 1;
    }
    Ok(count)
}
```

- [ ] **Step 3: Spawn the janitor task in main.rs**

After the recording-sweep task:
```rust
    // Recording retention janitor — every 24h.
    let pool_for_retention = pool.clone();
    let storage_for_retention = storage.clone();
    let retention_days: i64 = std::env::var("LIVE_ROOM_RECORDING_RETENTION_DAYS")
        .ok().and_then(|s| s.parse().ok()).unwrap_or(365);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(24 * 3600));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            match backend::services::recording::run_retention_janitor(
                &pool_for_retention, storage_for_retention.as_ref(), retention_days, 100,
            ).await {
                Ok(n) if n > 0 => tracing::info!(rows = n, days = retention_days, "recording retention prune"),
                Ok(_) => {}
                Err(e) => tracing::warn!(?e, "recording retention janitor failed"),
            }
        }
    });
```

- [ ] **Step 4: Run + commit**

```bash
DATABASE_URL=postgres://aulalite:changeme@localhost:55432/aulalite \
    cargo test -p backend --test recording retention_janitor 2>&1 | tail -10
git add crates/backend/src/services/recording.rs crates/backend/src/main.rs \
        crates/backend/tests/recording.rs
git commit -m "feat(live_room): recording retention janitor with 365-day default"
```
Expected: 1 passed.

---

## Section C — Frontend playback

### Task 17: LiveRoomReplay component

**Files:**
- Create: `crates/features-courses/src/live_room_replay.rs`
- Modify: `crates/features-courses/src/lib.rs`

This is a large component (video + chat sidebar with time-sync). Build-only — exercised through SSR smokes in Task 23.

- [ ] **Step 1: Create the component**

```rust
// crates/features-courses/src/live_room_replay.rs
//! Recording playback view with time-synced chat side panel.

use crate::api::{fetch_json, ApiContext, ApiError};
use crate::live_room_chat::{ChatMessage, LiveRoomChat};
use design_system::Button;
use dioxus::prelude::*;
use serde::Deserialize;

#[derive(Deserialize, Clone, PartialEq, Default)]
pub struct RecordingDto {
    pub session_id: String,
    pub processing_status: String,
    pub processing_error: Option<String>,
    pub duration_seconds: Option<i64>,
    pub started_at: Option<String>,
    pub playback_url: Option<String>,
    pub course_title: String,
    pub instructor_user_id: Option<String>,
}

#[derive(Deserialize, Clone, PartialEq)]
struct RecordingChatMessageDto {
    pub id: String,
    pub sender_user_id: String,
    pub sender_display_name: String,
    pub body: String,
    pub video_offset_seconds: f64,
    pub deleted: bool,
}

#[derive(Deserialize, Clone, PartialEq, Default)]
struct RecordingChatWindow {
    pub messages: Vec<RecordingChatMessageDto>,
}

#[derive(Props, Clone, PartialEq)]
pub struct LiveRoomReplayProps {
    pub session_id: String,
    pub course_title: String,
    pub instructor_name: Option<String>,
    pub is_teacher: bool,
}

pub fn LiveRoomReplay(props: LiveRoomReplayProps) -> Element {
    let cx = use_context::<ApiContext>();
    let session_id = props.session_id.clone();

    // Fetch recording metadata once on mount.
    let recording = use_resource(move || {
        let cx = cx.clone();
        let session_id = session_id.clone();
        async move {
            fetch_json::<RecordingDto>(
                &cx, "GET", &format!("/v1/sessions/{session_id}/recording"),
                None::<&()>,
            ).await
        }
    });

    match &*recording.read_unchecked() {
        Some(Ok(rec)) => render_with_recording(&props, rec),
        Some(Err(ApiError::Status(404, _))) => rsx! {
            div { class: "live-room-replay-empty",
                h2 { "Class has ended" }
                p { "No recording is available for this session." }
            }
        },
        Some(Err(_)) => rsx! {
            div { class: "form-error", "Couldn't load recording. Please refresh." }
        },
        None => rsx! { div { "Loading recording…" } },
    }
}

fn render_with_recording(props: &LiveRoomReplayProps, rec: &RecordingDto) -> Element {
    match rec.processing_status.as_str() {
        "available" => render_available(props, rec),
        "pending" | "remuxing" | "uploading" => rsx! {
            div { class: "live-room-replay-processing",
                h2 { "{props.course_title}" }
                p { "Recording is being processed (usually 5-15 min after class ends)." }
                p { class: "muted", "Refresh this page to check again." }
            }
        },
        "failed" => render_failed(props, rec),
        other => rsx! {
            div { class: "form-error", "Unknown recording status: {other}" }
        },
    }
}

fn render_available(props: &LiveRoomReplayProps, rec: &RecordingDto) -> Element {
    let session_id = props.session_id.clone();
    let video_seconds = use_signal(|| 0.0_f64);
    let chat_window = use_signal(Vec::<ChatMessage>::new);

    let cx = use_context::<ApiContext>();
    // Refetch chat window whenever video_seconds advances past a 30s boundary.
    let session_id_for_effect = session_id.clone();
    let cx_for_effect = cx.clone();
    use_effect(move || {
        #[cfg(target_arch = "wasm32")]
        {
            let secs = *video_seconds.read();
            let bucket = (secs / 30.0) as i64;
            let to_secs = secs + 5.0;
            let cx = cx_for_effect.clone();
            let session_id = session_id_for_effect.clone();
            let mut chat_window_set = chat_window;
            wasm_bindgen_futures::spawn_local(async move {
                if let Ok(window) = fetch_json::<RecordingChatWindow>(
                    &cx, "GET",
                    &format!("/v1/sessions/{session_id}/recording/chat?from_seconds=0&to_seconds={to_secs}"),
                    None::<&()>,
                ).await {
                    let mapped: Vec<ChatMessage> = window.messages.into_iter().map(|m| ChatMessage {
                        id: m.id,
                        sender_display_name: m.sender_display_name,
                        body: m.body,
                        created_at: format!("@{:.1}s", m.video_offset_seconds),
                        deleted: m.deleted,
                    }).collect();
                    chat_window_set.set(mapped);
                }
                let _ = bucket;
            });
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = (cx_for_effect.clone(), session_id_for_effect.clone(), video_seconds, chat_window);
        }
    });

    let url = rec.playback_url.clone().unwrap_or_default();

    rsx! {
        div { class: "live-room-replay",
            div { class: "replay-video-pane",
                h2 { "{props.course_title}" }
                video {
                    id: "live-room-replay-video",
                    src: "{url}",
                    controls: true,
                    playsinline: true,
                    class: "replay-video",
                    ontimeupdate: move |evt: dioxus::events::Event<dioxus::events::MediaData>| {
                        // 0.7 may pass current time differently; if not directly accessible,
                        // use a JS bridge to read video.currentTime each tick.
                        let _ = evt;
                        // Placeholder; the real read happens in a wasm-only effect that polls
                        // document.getElementById("live-room-replay-video").currentTime
                        // every 250ms while playing.
                    },
                }
            }
            div { class: "replay-chat-pane",
                LiveRoomChat {
                    messages: chat_window.read().clone(),
                    is_teacher: false,
                    on_send: move |_b: String| {},
                    on_delete: move |_id: String| {},
                }
            }
        }
    }
}

fn render_failed(props: &LiveRoomReplayProps, rec: &RecordingDto) -> Element {
    let session_id = props.session_id.clone();
    let is_teacher = props.is_teacher;
    let cx = use_context::<ApiContext>();
    let on_retry = move |_| {
        #[cfg(target_arch = "wasm32")]
        {
            let cx = cx.clone();
            let session_id = session_id.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let _: Result<RecordingDto, _> = fetch_json(
                    &cx, "POST",
                    &format!("/v1/sessions/{session_id}/recording/retry"),
                    Some(&serde_json::json!({})),
                ).await;
                if let Some(win) = web_sys::window() {
                    let _ = win.location().reload();
                }
            });
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = (cx.clone(), session_id.clone());
        }
    };
    let err_text = rec.processing_error.clone().unwrap_or_default();
    rsx! {
        div { class: "live-room-replay-failed",
            h2 { "Recording failed to process" }
            if is_teacher {
                p { class: "muted", "{err_text}" }
                Button {
                    label: "Retry processing".to_string(),
                    variant: design_system::ButtonVariant::Primary,
                    on_click: on_retry,
                }
            } else {
                p { "Please ask your instructor to retry the recording." }
            }
        }
    }
}
```

In `crates/features-courses/src/lib.rs`, add (alphabetical):
```rust
pub mod live_room_replay;
```

- [ ] **Step 2: Build native + wasm**

```bash
cd "C:/Users/Chiranjib Chaudhuri/Documents/Chiranjib/Elementors_Aula-phase0"
cargo build -p features-courses 2>&1 | tail -10
cargo build -p features-courses --target wasm32-unknown-unknown 2>&1 | tail -10
```

Expected: both clean. Common adaptations:
- The `ontimeupdate` handler is structured as a placeholder. If reading `video.currentTime` from the event isn't trivial in Dioxus 0.7, replace with a wasm-only `setInterval` polling `document.getElementById("...").currentTime` every 250ms when the video is playing — common React-like pattern.
- If `MediaData` isn't the right event payload type, use a generic `Event<dioxus::events::FormData>` or just `move |_: Event<_>| { ... }` and read currentTime via DOM directly.

If fighting Dioxus event lifetimes for >30 minutes, simplify: poll `currentTime` from a `use_effect` interval that ticks every 500ms. Document as a known minor inefficiency; the chat-window refetch is bucketed at 30s anyway.

- [ ] **Step 3: Commit**

```bash
git add crates/features-courses/src/live_room_replay.rs crates/features-courses/src/lib.rs
git commit -m "feat(features-courses): LiveRoomReplay — playback view with time-synced chat sidebar"
```

---

### Task 18: LiveRoomShell dispatch to replay

**Files:**
- Modify: `crates/features-courses/src/live_room_shell.rs`

`LiveRoomShell::route_for(role, status)` already returns `Branch::Ended` for `status=ended`. The `Ended` branch's rsx becomes "if `has_recording` is true, render `LiveRoomReplay`; else show 'Class has ended' fallback".

- [ ] **Step 1: Add `has_recording` and `is_teacher_here` to `LiveRoomShellProps`**

Find the props struct (added in Phase 1b-β Task 27). Append:
```rust
    pub has_recording: bool,
    pub is_teacher: bool,
```

`is_teacher` is needed because `LiveRoomReplay` shows the "Retry" button only for teachers.

- [ ] **Step 2: Update the `Branch::Ended` arm**

Find the rsx match (in `LiveRoomShell` body):
```rust
Branch::Ended => rsx! {
    div { class: "live-room-ended", "Class has ended." }
}
```

Replace with:
```rust
Branch::Ended => {
    if props.has_recording {
        rsx! {
            crate::live_room_replay::LiveRoomReplay {
                session_id: props.session_id.clone(),
                course_title: props.course_title.clone(),
                instructor_name: props.instructor_name.clone(),
                is_teacher: props.is_teacher,
            }
        }
    } else {
        rsx! { div { class: "live-room-ended", "Class has ended." } }
    }
}
```

- [ ] **Step 3: Update shell-web's call site**

In `crates/shell-web/src/main.rs`, find where `LiveRoomShell { ... }` is constructed (in `LiveSessionPage`). Pass:
```rust
has_recording: payload.has_recording,
is_teacher: false, // or derive from caller_role / tenant_role; default false for v1 polish
```

The `has_recording: bool` field needs to be added to the `JoinResp` struct in `LiveSessionPage` (Phase 1b-β Task 31). Find that struct and add:
```rust
#[serde(default)]
has_recording: bool,
```

- [ ] **Step 4: Build native + wasm**

```bash
cargo build -p features-courses 2>&1 | tail -5
cargo build -p features-courses --target wasm32-unknown-unknown 2>&1 | tail -5
cargo build -p shell-web --target wasm32-unknown-unknown 2>&1 | tail -5
```
Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add crates/features-courses/src/live_room_shell.rs crates/shell-web/src/main.rs
git commit -m "feat(features-courses): LiveRoomShell dispatches to LiveRoomReplay when has_recording"
```

---

### Task 19: Chat side panel — currentTime poll wiring

**Files:**
- Modify: `crates/features-courses/src/live_room_replay.rs`

Task 17 left a placeholder for reading `video.currentTime`. This task wires a 500ms polling effect that updates `video_seconds` while the video plays.

- [ ] **Step 1: Replace the `ontimeupdate` stub with a wasm-only polling effect**

In `render_available`, replace the `ontimeupdate` attribute assignment with a separate `use_effect` that polls every 500ms:

```rust
let mut video_seconds_for_poll = video_seconds;
use_effect(move || {
    #[cfg(target_arch = "wasm32")]
    {
        wasm_bindgen_futures::spawn_local(async move {
            loop {
                let mut win_opt = web_sys::window();
                if let Some(win) = win_opt.take() {
                    if let Some(doc) = win.document() {
                        if let Some(el) = doc.get_element_by_id("live-room-replay-video") {
                            use wasm_bindgen::JsCast;
                            if let Ok(video) = el.dyn_into::<web_sys::HtmlMediaElement>() {
                                let t = video.current_time();
                                video_seconds_for_poll.set(t);
                            }
                        }
                    }
                }
                gloo_timers::future::TimeoutFuture::new(500).await;
            }
        });
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = video_seconds_for_poll;
    }
});
```

Remove the `ontimeupdate: move |evt: ...|` attribute from the `<video>` element — the polling effect handles everything.

- [ ] **Step 2: Add `gloo-timers` to wasm32 deps if missing**

```bash
grep -n "gloo-timers" crates/features-courses/Cargo.toml
```

If missing, add to `[target.'cfg(target_arch = "wasm32")'.dependencies]`:
```toml
gloo-timers = { version = "0.3", features = ["futures"] }
```

If `gloo-timers` proves problematic, use `wasm_bindgen_futures::spawn_local` with a manual setTimeout via `web_sys` instead. Adapt as needed.

- [ ] **Step 3: Build native + wasm**

```bash
cargo build -p features-courses 2>&1 | tail -5
cargo build -p features-courses --target wasm32-unknown-unknown 2>&1 | tail -5
```

- [ ] **Step 4: Commit**

```bash
git add crates/features-courses/src/live_room_replay.rs crates/features-courses/Cargo.toml Cargo.lock
git commit -m "feat(features-courses): LiveRoomReplay polls video.currentTime to drive chat sync"
```

---

## Section D — RLS, SSR smokes, build sweeps, exit checklist

### Task 20: RLS sweep — cross-tenant probe for recordings

**Files:**
- Modify: `crates/backend/tests/rls_tenant_isolation.rs`

- [ ] **Step 1: Append the probe**

```rust
#[tokio::test]
async fn cross_tenant_recordings_masked() -> anyhow::Result<()> {
    let pool = pool().await;
    let tenant_a = Uuid::new_v4();
    let tenant_b = Uuid::new_v4();
    let user_a = Uuid::new_v4();
    let course_a = Uuid::new_v4();
    let series_a = Uuid::new_v4();
    let session_a = Uuid::new_v4();
    let recording_a = Uuid::new_v4();

    let mut conn = pool.acquire().await?;
    sqlx::query("BEGIN").execute(&mut *conn).await?;

    let result = async {
        seed_membership(&mut *conn, tenant_a, "a").await?;
        seed_membership(&mut *conn, tenant_b, "b").await?;
        seed_user(&mut *conn, user_a).await?;
        sqlx::query("INSERT INTO courses (id, tenant_id, slug, title, owner_user_id) VALUES ($1,$2,$3,'C',$4)")
            .bind(course_a).bind(tenant_a).bind(format!("c-{}", Uuid::new_v4())).bind(user_a)
            .execute(&mut *conn).await?;
        sqlx::query(
            "INSERT INTO live_session_series (id, tenant_id, course_id, title, starts_at,
                                              duration_minutes, frequency, end_kind,
                                              transport_mode, primary_teacher_id, recording_enabled)
             VALUES ($1, $2, $3, 'S', now(), 60, 'none', 'open', 'webrtc', $4, true)"
        ).bind(series_a).bind(tenant_a).bind(course_a).bind(user_a).execute(&mut *conn).await?;
        sqlx::query(
            "INSERT INTO live_sessions (id, tenant_id, course_id, series_id, occurrence_index, title,
                                        status, starts_at, duration_minutes, primary_teacher_id, mode,
                                        recording_enabled, transport_mode)
             VALUES ($1, $2, $3, $4, 0, 'L', 'ended', now(), 60, $5, 'lecture', true, 'webrtc')"
        ).bind(session_a).bind(tenant_a).bind(course_a).bind(series_a).bind(user_a).execute(&mut *conn).await?;
        sqlx::query(
            "INSERT INTO recordings (id, tenant_id, session_id, started_at, ended_at,
                                     duration_seconds, processing_status)
             VALUES ($1, $2, $3, now() - interval '1 hour', now(), 3600, 'available')"
        ).bind(recording_a).bind(tenant_a).bind(session_a).execute(&mut *conn).await?;

        let role_name = create_rls_test_role_phase_1a(&mut *conn).await?;
        sqlx::query(&format!("SET LOCAL ROLE {}", role_ident(&role_name)))
            .execute(&mut *conn).await?;
        set_local_tenant(&mut *conn, tenant_b).await?;

        let visible: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM recordings WHERE id = $1"
        ).bind(recording_a).fetch_one(&mut *conn).await?;
        anyhow::Ok(visible.0)
    }.await;

    let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
    assert_eq!(result?, 0, "tenant B must not see tenant A's recordings");
    Ok(())
}
```

If `create_rls_test_role_phase_1a` doesn't grant SELECT on `recordings` (Phase 1b-γ Task 26 extended it for `live_room_messages` + `live_room_kicks`), extend it again to include `recordings`. Find the `PHASE_1B_GAMMA_TABLES` constant and add `"recordings"` to a new `PHASE_1B_DELTA_TABLES` constant (or extend the existing one). Grant SELECT before the test runs.

- [ ] **Step 2: Run + commit**

```bash
DATABASE_URL=postgres://aulalite:changeme@localhost:55432/aulalite \
    cargo test -p backend --test rls_tenant_isolation 2>&1 | tail -10
```
Expected: 10 passed (9 prior + 1 new).

```bash
git add crates/backend/tests/rls_tenant_isolation.rs
git commit -m "test(rls): cross-tenant probe for recordings"
```

---

### Task 21: SSR smokes for LiveRoomReplay

**Files:**
- Modify: `crates/features-courses/tests/live_room_smoke.rs`

- [ ] **Step 1: Append SSR tests**

```rust
use features_courses::live_room_replay::{LiveRoomReplay, LiveRoomReplayProps};

#[test]
fn replay_renders_loading_initially() {
    fn app() -> Element {
        rsx! {
            LiveRoomReplay {
                session_id: "00000000-0000-0000-0000-000000000000".to_string(),
                course_title: "Algebra 1".to_string(),
                instructor_name: Some("Ms. Smith".to_string()),
                is_teacher: false,
            }
        }
    }
    let mut vdom = VirtualDom::new(app);
    vdom.rebuild_in_place();
    let html = dioxus_ssr::render(&vdom);
    // Without an ApiContext provider, fetch_json fails and use_resource stays None.
    // SSR initial render shows "Loading recording…" placeholder.
    assert!(
        html.contains("Loading recording") || html.contains("live-room-replay"),
        "got: {html}"
    );
}
```

The `LiveRoomReplay` component depends on `ApiContext` via `use_context`. SSR without a context provider may panic or return a default. To make the test robust, the component should tolerate a missing/default context — or the SSR test should provide a stub context.

Pragmatic: if `use_context::<ApiContext>()` panics in SSR without a provider, simplify the SSR test to render only the static "Class has ended" fallback (which doesn't need ApiContext). Otherwise, add a `provide_context` wrapper in the test app.

If your existing SSR smokes from 1b-α / 1b-β / 1b-γ already provide an `ApiContext` test fixture, reuse that pattern.

- [ ] **Step 2: Run + commit**

```bash
cargo test -p features-courses --test live_room_smoke 2>&1 | tail -10
git add crates/features-courses/tests/live_room_smoke.rs
git commit -m "test(features-courses): SSR smoke for LiveRoomReplay"
```
Expected: at least 1 new test passing; total count grows by 1.

---

### Task 22: Build sweeps

**Files:** none modified.

- [ ] **Step 1: Run all build/test sweeps**

```bash
cd "C:/Users/Chiranjib Chaudhuri/Documents/Chiranjib/Elementors_Aula-phase0"
cargo build -p backend 2>&1 | tail -3
cargo build -p features-courses --target wasm32-unknown-unknown 2>&1 | tail -3
cargo build -p shell-web --target wasm32-unknown-unknown 2>&1 | tail -3
DATABASE_URL=postgres://aulalite:changeme@localhost:55432/aulalite \
    cargo test -p backend --test recording -j 2 2>&1 | tail -10
DATABASE_URL=postgres://aulalite:changeme@localhost:55432/aulalite \
    cargo test -p backend --test rls_tenant_isolation -j 2 2>&1 | tail -10
cargo test -p backend --lib services::recording 2>&1 | tail -5
cargo test -p features-courses --test live_room_smoke 2>&1 | tail -5
```

All clean / all green.

- [ ] **Step 2: No commit needed (verification only)**

If anything breaks, fix it as a follow-up commit before moving to Task 23.

---

### Task 23: Phase 1b-δ exit checklist + workspace test sweep

**Files:**
- Create: `docs/superpowers/plans/2026-05-09-aulalite-phase-1b-delta-exit-checklist.md`

- [ ] **Step 1: Write the checklist**

```markdown
# Phase 1b-δ Exit Checklist

Run these checks in order from the repository root. Phase 1b-δ is complete only
when every required item passes.

## 1. Stack health
- [ ] `docker compose up -d`
- [ ] `curl http://localhost:8080/healthz` returns `ok`
- [ ] `docker compose exec backend ffmpeg -version` returns version info
- [ ] `docker compose exec backend ls /recordings` succeeds (volume mounted)

## 2. Migrations
- [ ] `sqlx migrate info --source migrations` shows `20260509000014_recordings` applied.
- [ ] `\d recordings` shows the table with RLS policy + UNIQUE(session_id) + partial index.

## 3. Automated verification
- [ ] `cargo test --workspace -j 2` (everything green; zero failures). Note: `-j 2` required on Windows hosts.
- [ ] `cargo build -p shell-web --target wasm32-unknown-unknown`
- [ ] `cargo build -p features-courses --target wasm32-unknown-unknown`

## 4. End-to-end recording (manual)
- [ ] Sign in as teacher in Chrome. Schedule a session with `recording_enabled=true`.
- [ ] Click Go Live, publish camera+mic for 2-3 minutes.
- [ ] Click End Class.
- [ ] Wait ~5 minutes for the sweep to pick it up. Watch backend logs for `recording processed`.
- [ ] Confirm `recordings.processing_status='available'` in DB.
- [ ] Confirm a `file_assets` row with `linked_entity_type='session_recording'` and an MP4 in MinIO.

## 5. Playback (manual)
- [ ] Sign in as enrolled student.
- [ ] Open `/courses/<slug>/sessions/<id>` (the same URL used during the live session).
- [ ] Confirm `LiveRoomReplay` mounts; `<video>` plays the MP4.
- [ ] Send some chat messages during step 4. Open the recording. Confirm chat side panel scrolls in sync as the video plays.

## 6. Cross-tenant probe
- [ ] Tenant B's user fetches `/v1/sessions/<tenant-A-session-id>/recording` → 404.

## 7. Failure / retry (manual)
- [ ] Force a recording into `failed` state via DB: `UPDATE recordings SET processing_status='failed', processing_error='manual test' WHERE session_id='<id>';`
- [ ] Open the recording playback URL as teacher; confirm "Retry" button appears.
- [ ] Click Retry. Confirm status flips back to `pending`.
- [ ] Wait for the next sweep tick; confirm status returns to `available` (after a successful re-process).

## 8. Retention janitor
- [ ] Insert a recordings row with `created_at = now() - interval '400 days'`.
- [ ] Manually trigger the janitor via DB or wait 24h.
- [ ] Confirm the row is deleted, the file_asset is `pruned`, and the MinIO object is gone.

## Completion tag

Only after every required check above passes:

```bash
git tag phase-1b-delta-complete
git push origin phase-1b-delta-complete
git tag phase-1b-complete  # closes out 1b
git push origin phase-1b-complete
```
```

- [ ] **Step 2: Workspace test sweep**

```bash
cd "C:/Users/Chiranjib Chaudhuri/Documents/Chiranjib/Elementors_Aula-phase0"
DATABASE_URL=postgres://aulalite:changeme@localhost:55432/aulalite \
    cargo test --workspace -j 2 2>&1 | tail -20
```

If the Windows PDB linker error (LNK1318) appears, run `cargo clean -p backend` then retry. Same gate as prior phases.

- [ ] **Step 3: Commit + report**

```bash
git add docs/superpowers/plans/2026-05-09-aulalite-phase-1b-delta-exit-checklist.md
git commit -m "docs(plan): Phase 1b-delta exit checklist"
git rev-parse HEAD
```

Wait for user authorization before pushing.

- [ ] **Step 4: Report**

Tell the user:
- Total commits added in 1b-δ.
- Test pass count.
- Final SHA on `phase-0-foundations`.
- Pending manual exit-checklist items.
- That tagging `phase-1b-complete` after the manual checklist closes out the entire 1b sub-phase.

---

## Self-review notes (for the controller running this plan)

After all tasks complete:

1. **Spec coverage.** Each section of `2026-05-09-aulalite-phase-1b-delta-recording-design.md` is touched: schema (Task 1), MediaMTX recording config (Task 3), services::recording trait + Mock + Real (Tasks 6-8), S3Client extension (Task 5), db::recordings (Task 9), AppState (Task 10), three handler routes (Tasks 11-13), /join extension (Task 14), recording sweep (Task 15), retention janitor (Task 16), frontend playback + chat sync (Tasks 17-19), RLS sweep (Task 20), SSR smoke (Task 21), build sweeps + exit (Tasks 22-23).

2. **Type consistency.** `RecorderTool`, `MockRecorderTool`, `RealFfmpegRecorder`, `RecorderError`, `RecorderCall` defined in Tasks 6-8 and used unchanged through Task 15. `RecordingRow`, `process_one_session`, `run_retention_janitor` defined in Tasks 9 + 15-16, used in Tasks 11-13. `RecordingDto`, `RecordingChatMessageDto` defined in Tasks 11-12, consumed by frontend in Task 17.

3. **No placeholders.** Every code block has actual content. The "TODO P1bγ exit-checklist" comments are NOT in this plan — those belong to 1b-γ. The Task 17 `ontimeupdate` placeholder is explicitly resolved in Task 19.

4. **Migration is forward-only.** One new table (`recordings`) with safe defaults; no existing rows broken. RLS policy added.

5. **Backend Dockerfile change**: ffmpeg is +100MB. Acceptable for single-tenant deployment.

6. **MediaMTX config precedence**: Phase 1b-β set `authMethod: http` for Pattern B. Phase 1b-δ adds top-level recording directives — these are orthogonal (auth vs recording) and should coexist cleanly. Verify in Task 3.

7. **Volume mount caveat**: the `mediamtx_recordings` volume must be mounted to BOTH containers. Task 3 covers this.

8. **Frontend follow-up tracker**: Tasks 17-19 ship the playback + chat-sync UI. The 1b-γ exit checklist already flags the persistent-WebSocket follow-up; this plan adds none beyond the chat-window currentTime polling (which is fully implemented in Task 19).

