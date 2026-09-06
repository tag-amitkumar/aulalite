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
    #[error("recording storage limit reached")]
    RecordingStorageLimitReached,
    #[error("recording finalization commit outcome is uncertain: {0}")]
    FinalizationCommitUncertain(String),
}

/// Returns the object-storage key for a recording's MP4.
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
    if secs < 0.0 {
        0.0
    } else {
        secs
    }
}

use crate::db::recordings as db_rec;
use crate::storage::S3Client;
use async_trait::async_trait;
use std::path::Path;
use std::sync::{Arc, Mutex};

#[async_trait]
pub trait RecorderTool: Send + Sync {
    /// Concatenate fmp4 segments via ffmpeg `-c copy` (no re-encode).
    /// Writes the produced MP4 to `output`.
    async fn remux_to_mp4(&self, segments: &[PathBuf], output: &Path) -> Result<(), RecorderError>;

    /// Probe duration in seconds via ffprobe.
    async fn probe_duration_seconds(&self, path: &Path) -> Result<i32, RecorderError>;

    /// Whether the produced MP4 actually carries a video stream.
    ///
    /// It usually does, but not always: `remux_to_mp4` runs `-c copy`, and the
    /// MP4 muxer cannot carry every codec MediaMTX might have recorded. A VP8
    /// publish (what this app produced before it started preferring H.264)
    /// remuxes to an audio-only MP4 -- ffmpeg drops the track and still exits
    /// 0, so nothing upstream notices. Recording the answer lets the UI say
    /// "audio only" instead of showing a black rectangle that plays sound.
    async fn probe_has_video_stream(&self, path: &Path) -> Result<bool, RecorderError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecorderCall {
    Remux {
        segments: Vec<PathBuf>,
        output: PathBuf,
    },
    ProbeDuration {
        path: PathBuf,
    },
    ProbeHasVideo {
        path: PathBuf,
    },
}

/// Test-only stub for `RecorderTool`. `#[doc(hidden)]` so it does not
/// surface in generated rustdoc and so production callers do not stumble
/// onto it via tab-completion.
#[doc(hidden)]
#[derive(Clone, Default)]
pub struct MockRecorderTool {
    pub calls: Arc<Mutex<Vec<RecorderCall>>>,
    pub duration_seconds: Arc<Mutex<i32>>,
    pub has_video: Arc<Mutex<bool>>,
}

impl MockRecorderTool {
    pub fn new() -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
            duration_seconds: Arc::new(Mutex::new(60)),
            has_video: Arc::new(Mutex::new(true)),
        }
    }

    pub fn set_duration_seconds(&self, s: i32) {
        *self.duration_seconds.lock().unwrap() = s;
    }

    pub fn set_has_video(&self, v: bool) {
        *self.has_video.lock().unwrap() = v;
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
    async fn remux_to_mp4(&self, segments: &[PathBuf], output: &Path) -> Result<(), RecorderError> {
        self.record(RecorderCall::Remux {
            segments: segments.to_vec(),
            output: output.to_path_buf(),
        });
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

    async fn probe_has_video_stream(&self, path: &Path) -> Result<bool, RecorderError> {
        self.record(RecorderCall::ProbeHasVideo {
            path: path.to_path_buf(),
        });
        Ok(*self.has_video.lock().unwrap())
    }
}

#[derive(Clone, Default)]
pub struct RealFfmpegRecorder;

impl RealFfmpegRecorder {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl RecorderTool for RealFfmpegRecorder {
    async fn remux_to_mp4(&self, segments: &[PathBuf], output: &Path) -> Result<(), RecorderError> {
        let manifest_text = concat_list_for(segments);
        let manifest_path = std::env::temp_dir().join(format!(
            "aulalite-concat-{}.txt",
            uuid::Uuid::new_v4().simple()
        ));
        tokio::fs::write(&manifest_path, &manifest_text)
            .await
            .map_err(|e| RecorderError::Io(format!("write manifest: {e}")))?;

        let status = tokio::process::Command::new("ffmpeg")
            .arg("-y")
            .arg("-f")
            .arg("concat")
            .arg("-safe")
            .arg("0")
            .arg("-i")
            .arg(&manifest_path)
            .arg("-c")
            .arg("copy")
            .arg(output)
            .stderr(std::process::Stdio::piped())
            .output()
            .await
            .map_err(|e| RecorderError::Io(format!("spawn ffmpeg: {e}")))?;

        let _ = tokio::fs::remove_file(&manifest_path).await;

        if !status.status.success() {
            let stderr = String::from_utf8_lossy(&status.stderr);
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
            .arg("-v")
            .arg("error")
            .arg("-show_entries")
            .arg("format=duration")
            .arg("-of")
            .arg("default=nw=1:nk=1")
            .arg(path)
            .output()
            .await
            .map_err(|e| RecorderError::Io(format!("spawn ffprobe: {e}")))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(RecorderError::FfprobeFailed(stderr.to_string()));
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let secs: f64 = stdout
            .trim()
            .parse()
            .map_err(|e| RecorderError::FfprobeFailed(format!("parse duration: {e}")))?;
        Ok(secs.max(0.0).round() as i32)
    }

    async fn probe_has_video_stream(&self, path: &Path) -> Result<bool, RecorderError> {
        // `-select_streams v` restricts the listing to video streams, so an
        // audio-only file prints nothing at all and a file with video prints
        // one codec name per stream. Checking for non-empty output is therefore
        // the whole test -- no parsing, and no dependence on the codec name.
        let output = tokio::process::Command::new("ffprobe")
            .arg("-v")
            .arg("error")
            .arg("-select_streams")
            .arg("v")
            .arg("-show_entries")
            .arg("stream=codec_name")
            .arg("-of")
            .arg("default=nw=1:nk=1")
            .arg(path)
            .output()
            .await
            .map_err(|e| RecorderError::Io(format!("spawn ffprobe: {e}")))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(RecorderError::FfprobeFailed(stderr.to_string()));
        }
        Ok(!String::from_utf8_lossy(&output.stdout).trim().is_empty())
    }
}

/// Processes one ended session: insert pending → remux → upload → mark available.
/// Returns Ok(Some(recording_id)) on success, Ok(None) if session is too short
/// or another sweep already won the race, Err on processing failure.
#[allow(clippy::too_many_arguments)]
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
    if duration_initial < 5 {
        return Ok(None);
    }

    let tenant_id_str = tenant_id.to_string();

    // Step 1: atomically create/claim pending -> remuxing. This also makes the
    // manual retry endpoint functional: resetting a failed row to pending lets
    // the next sweep claim it without creating a duplicate recording.
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| RecorderError::Io(format!("tx begin: {e}")))?;
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(&tenant_id_str)
        .execute(&mut *tx)
        .await
        .map_err(|e| RecorderError::Io(format!("set_config: {e}")))?;
    let row = db_rec::claim_for_processing(
        &mut tx,
        tenant_id,
        session_id,
        started_at,
        ended_at,
        duration_initial,
    )
    .await
    .map_err(|e| RecorderError::Io(format!("insert_pending: {e}")))?;
    tx.commit()
        .await
        .map_err(|e| RecorderError::Io(format!("tx commit: {e}")))?;
    let row = match row {
        Some(r) => r,
        None => return Ok(None),
    };

    // Refuse expensive remux work when the plan is already exactly full. The
    // exact byte-size reservation is acquired after ffmpeg produces the MP4.
    match crate::db::usage_limits::recording_storage_capacity(pool, tenant_id, 1).await {
        Ok(decision) if decision.is_limit_reached() => {
            mark_failed(pool, row.id, tenant_id, "recording_storage_limit_reached").await;
            return Err(RecorderError::RecordingStorageLimitReached);
        }
        Ok(_) => {}
        Err(e) => {
            mark_failed(
                pool,
                row.id,
                tenant_id,
                &format!("recording quota preflight: {e}"),
            )
            .await;
            return Err(RecorderError::Io(format!("recording quota preflight: {e}")));
        }
    }

    // Step 3: enumerate segments.
    let segments = match enumerate_session_segments(recordings_dir, tenant_id, session_id) {
        Ok(s) if !s.is_empty() => s,
        Ok(_) => {
            mark_failed(pool, row.id, tenant_id, "no segments found on disk").await;
            return Err(RecorderError::Io("no segments".into()));
        }
        Err(e) => {
            mark_failed(pool, row.id, tenant_id, &format!("enumerate: {e}")).await;
            return Err(RecorderError::Io(e.to_string()));
        }
    };

    // Step 4: remux to tmp MP4.
    let tmp_mp4 =
        std::env::temp_dir().join(format!("aulalite-recording-{}.mp4", session_id.simple()));
    if let Err(e) = recorder.remux_to_mp4(&segments, &tmp_mp4).await {
        mark_failed(pool, row.id, tenant_id, &format!("ffmpeg: {e}")).await;
        return Err(e);
    }

    // Step 5: probe duration.
    let duration = match recorder.probe_duration_seconds(&tmp_mp4).await {
        Ok(d) => d.max(1),
        Err(e) => {
            mark_failed(pool, row.id, tenant_id, &format!("ffprobe: {e}")).await;
            return Err(e);
        }
    };

    // Step 5b: does the remuxed file actually have a picture in it?
    //
    // Non-fatal by design. A recording with sound but no video is still worth
    // keeping and is exactly what we want to LABEL rather than reject, and a
    // probe that fails should not throw away a finished upload. `None` means
    // "not determined", which the UI renders as an ordinary recording -- so the
    // worst case of a failed probe is the pre-existing behaviour, never a
    // healthy recording mislabelled as audio-only.
    let has_video = match recorder.probe_has_video_stream(&tmp_mp4).await {
        Ok(v) => Some(v),
        Err(e) => {
            tracing::warn!(?e, ?session_id, "video-stream probe failed; leaving unknown");
            None
        }
    };

    // Step 6: status → uploading. Same rationale as Step 2.
    if let Err(e) = transition_status(pool, &tenant_id_str, row.id, "uploading").await {
        mark_failed(
            pool,
            row.id,
            tenant_id,
            &format!("set status uploading: {e}"),
        )
        .await;
        return Err(RecorderError::Io(e));
    }

    // Step 7: read MP4 + upload.
    let mp4_bytes = match tokio::fs::read(&tmp_mp4).await {
        Ok(b) => b,
        Err(e) => {
            mark_failed(pool, row.id, tenant_id, &format!("read mp4: {e}")).await;
            return Err(RecorderError::Io(e.to_string()));
        }
    };

    let asset_id = uuid::Uuid::new_v4();
    let key = recording_object_key(tenant_id, asset_id, chrono::Utc::now());
    let mp4_size = mp4_bytes.len() as i64;

    // Reserve exact bytes before the object-store write. The reservation is a
    // short DB transaction and prevents concurrent recording workers from both
    // consuming the tenant's final available storage.
    match crate::db::usage_limits::reserve_recording_storage(pool, tenant_id, row.id, mp4_size)
        .await
    {
        Ok(decision) if decision.is_limit_reached() => {
            let _ = tokio::fs::remove_file(&tmp_mp4).await;
            mark_failed(pool, row.id, tenant_id, "recording_storage_limit_reached").await;
            return Err(RecorderError::RecordingStorageLimitReached);
        }
        Ok(_) => {}
        Err(e) => {
            let _ = tokio::fs::remove_file(&tmp_mp4).await;
            mark_failed(
                pool,
                row.id,
                tenant_id,
                &format!("reserve recording quota: {e}"),
            )
            .await;
            return Err(RecorderError::Io(format!("reserve recording quota: {e}")));
        }
    }

    if let Err(e) = storage.put_object(&key, mp4_bytes, "video/mp4").await {
        if let Err(release_error) =
            crate::db::usage_limits::release_recording_reservation(pool, tenant_id, row.id).await
        {
            tracing::warn!(?release_error, recording_id = %row.id, "failed to release recording quota reservation");
        }
        let _ = tokio::fs::remove_file(&tmp_mp4).await;
        mark_failed(pool, row.id, tenant_id, &format!("s3 put: {e}")).await;
        return Err(RecorderError::Io(e.to_string()));
    }

    // Step 8: validate the still-current plan, insert the file_asset, consume
    // the reservation, and mark available in one atomic DB transaction.
    let finalization: Result<(), RecorderError> = async {
        let mut tx = pool
            .begin()
            .await
            .map_err(|e| RecorderError::Io(format!("tx begin: {e}")))?;
        sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
            .bind(&tenant_id_str)
            .execute(&mut *tx)
            .await
            .map_err(|e| RecorderError::Io(format!("set_config: {e}")))?;
        let final_quota = crate::db::usage_limits::validate_recording_finalization(
            &mut tx, tenant_id, row.id, mp4_size,
        )
        .await
        .map_err(|e| RecorderError::Io(format!("validate recording quota: {e}")))?;
        if final_quota.is_limit_reached() {
            return Err(RecorderError::RecordingStorageLimitReached);
        }

        // Resolve the owner inside the tenant-scoped transaction. A bare pool
        // read is filtered by FORCE RLS under the production database role.
        let owner_user_id: uuid::Uuid = sqlx::query_scalar(
            "SELECT primary_teacher_id
               FROM live_sessions
              WHERE id = $1 AND tenant_id = $2",
        )
        .bind(session_id)
        .bind(tenant_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| RecorderError::Io(format!("fetch session owner: {e}")))?;
        sqlx::query(
            "INSERT INTO file_assets (id, tenant_id, owner_user_id, bucket, object_key,
                                      content_type, size_bytes, status, visibility,
                                      linked_entity_type, linked_entity_id)
             VALUES ($1, $2, $3, $4, $5, 'video/mp4', $6, 'available', 'private',
                     'session_recording', $7)",
        )
        .bind(asset_id)
        .bind(tenant_id)
        .bind(owner_user_id)
        .bind(bucket)
        .bind(&key)
        .bind(mp4_size)
        .bind(row.id)
        .execute(&mut *tx)
        .await
        .map_err(|e| RecorderError::Io(format!("insert file_asset: {e}")))?;
        db_rec::mark_available(&mut tx, row.id, asset_id, duration, has_video)
            .await
            .map_err(|e| RecorderError::Io(format!("mark_available: {e}")))?;
        crate::db::usage_limits::consume_recording_reservation(&mut tx, row.id)
            .await
            .map_err(|e| RecorderError::Io(format!("consume recording reservation: {e}")))?;
        tx.commit()
            .await
            .map_err(|e| RecorderError::FinalizationCommitUncertain(e.to_string()))?;
        Ok(())
    }
    .await;

    if let Err(error) = finalization {
        if matches!(&error, RecorderError::FinalizationCommitUncertain(_)) {
            // A lost connection cannot tell us whether COMMIT reached
            // PostgreSQL. Never delete the object or downgrade the row here:
            // if the commit landed, doing either would corrupt an available
            // recording. If it did not, the in-progress sweep reclaims the row
            // and the byte reservation expires automatically.
            tracing::error!(recording_id = %row.id, error = %error, "recording finalization commit outcome uncertain");
            let _ = tokio::fs::remove_file(&tmp_mp4).await;
            return Err(error);
        }
        // The DB mutation did not complete, so the object must not remain as
        // untracked/billable storage. Deletion and reservation release are
        // independently best-effort and loudly observable.
        if let Err(delete_error) = storage.delete_object(&key).await {
            tracing::warn!(?delete_error, %key, "failed to delete unfinalized recording object");
        }
        if let Err(release_error) =
            crate::db::usage_limits::release_recording_reservation(pool, tenant_id, row.id).await
        {
            tracing::warn!(?release_error, recording_id = %row.id, "failed to release recording quota reservation");
        }
        let _ = tokio::fs::remove_file(&tmp_mp4).await;
        mark_failed(pool, row.id, tenant_id, &error.to_string()).await;
        return Err(error);
    }

    // Step 10: best-effort cleanup of the temp MP4.
    let _ = tokio::fs::remove_file(&tmp_mp4).await;

    Ok(Some(row.id))
}

/// Transitions a recording row to `new_status` inside its own short tx, with
/// `app.tenant_id` set first so RLS policies on `recordings` apply. Returns
/// a stringified error on failure so callers can wrap into RecorderError.
async fn transition_status(
    pool: &sqlx::PgPool,
    tenant_id_str: &str,
    id: uuid::Uuid,
    new_status: &str,
) -> Result<(), String> {
    let mut tx = pool.begin().await.map_err(|e| format!("tx begin: {e}"))?;
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id_str)
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("set_config: {e}"))?;
    db_rec::set_status(&mut tx, id, new_status, None)
        .await
        .map_err(|e| format!("set_status({new_status}): {e}"))?;
    tx.commit().await.map_err(|e| format!("tx commit: {e}"))?;
    Ok(())
}

/// Public counterpart of `mark_failed` for use by the background sweep when
/// reclaiming stuck rows. Same semantics — best-effort, logs on error.
pub async fn mark_orphan_failed(
    pool: &sqlx::PgPool,
    id: uuid::Uuid,
    tenant_id: uuid::Uuid,
    reason: &str,
) {
    mark_failed(pool, id, tenant_id, reason).await;
}

async fn mark_failed(pool: &sqlx::PgPool, id: uuid::Uuid, tenant_id: uuid::Uuid, reason: &str) {
    // Truncate at a UTF-8 char boundary so we never panic on non-ASCII paths
    // (ffmpeg can emit them).
    let truncated = if reason.len() > 1000 {
        let mut start = reason.len() - 1000;
        while start < reason.len() && !reason.is_char_boundary(start) {
            start += 1;
        }
        &reason[start..]
    } else {
        reason
    };
    let tx = pool.begin().await;
    match tx {
        Ok(mut tx) => {
            if let Err(e) = sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
                .bind(tenant_id.to_string())
                .execute(&mut *tx)
                .await
            {
                tracing::warn!(?e, ?id, "mark_failed: set_config failed");
                return;
            }
            if let Err(e) = db_rec::set_status(&mut tx, id, "failed", Some(truncated)).await {
                tracing::warn!(?e, ?id, "mark_failed: set_status failed");
                return;
            }
            match tx.commit().await {
                Ok(()) => {
                    // Release in a separate transaction so a cleanup failure
                    // cannot roll back the operationally important `failed`
                    // state. Reservations are lease-bound as a final fallback.
                    if let Err(e) =
                        crate::db::usage_limits::release_recording_reservation(pool, tenant_id, id)
                            .await
                    {
                        tracing::warn!(?e, ?id, "mark_failed: release quota reservation failed");
                    }
                }
                Err(e) => tracing::warn!(?e, ?id, "mark_failed: tx commit failed"),
            }
        }
        Err(e) => {
            tracing::warn!(?e, ?id, "mark_failed: tx begin failed");
        }
    }
}

/// Enumerates segment files under `<recordings_dir>/aula/<tenant_simple>/*/<session_simple>/`.
/// Walks the course directory dimension because the sweep doesn't know the course_id.
/// Returns segments sorted by mtime ASC.
fn enumerate_session_segments(
    recordings_dir: &std::path::Path,
    tenant_id: uuid::Uuid,
    session_id: uuid::Uuid,
) -> std::io::Result<Vec<std::path::PathBuf>> {
    let session_simple = session_id.simple().to_string();
    let tenant_simple = tenant_id.simple().to_string();
    let mut segments = Vec::new();
    let tenant_dir = recordings_dir.join("aula").join(&tenant_simple);
    if !tenant_dir.exists() {
        return Ok(segments);
    }
    for course_entry in std::fs::read_dir(&tenant_dir)? {
        let course_dir = course_entry?.path();
        if !course_dir.is_dir() {
            continue;
        }
        let session_dir = course_dir.join(&session_simple);
        if !session_dir.is_dir() {
            continue;
        }
        for seg_entry in std::fs::read_dir(&session_dir)? {
            let p = seg_entry?.path();
            if p.is_file() {
                segments.push(p);
            }
        }
    }
    segments.sort_by_key(|p| {
        std::fs::metadata(p)
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
    });
    Ok(segments)
}

/// Runs one pass of the retention janitor. Each recording is pruned once it is
/// older than its tenant's configured `recording_retention_days`;
/// `default_retention_days` is the global SAFETY-CAP fallback applied only when
/// a tenant's value is NULL. For each prunable recording: best-effort S3 delete
/// → mark file_asset pruned → delete recording row. Returns count of recordings
/// deleted.
pub async fn run_retention_janitor(
    pool: &sqlx::PgPool,
    storage: &dyn S3Client,
    default_retention_days: i64,
    batch_limit: i64,
) -> Result<u64, RecorderError> {
    let pairs = db_rec::list_for_retention_prune(pool, default_retention_days, batch_limit)
        .await
        .map_err(|e| RecorderError::Io(format!("list_for_retention: {e}")))?;
    let mut count = 0u64;
    for (recording_id, file_asset_id, tenant_id) in pairs {
        // tenant_id is carried from the discovery query (under system context),
        // so no second per-row lookup is needed — that lookup was both an N+1
        // round-trip and itself RLS-filtered to zero rows under aulalite_app.

        if let Some(fid) = file_asset_id {
            // Fetch file_asset inside a tx with RLS context to get the object_key.
            let asset_opt: Option<crate::db::file_assets::FileAssetRow> = {
                let mut tx = pool
                    .begin()
                    .await
                    .map_err(|e| RecorderError::Io(format!("tx begin: {e}")))?;
                let _ = sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
                    .bind(tenant_id.to_string())
                    .execute(&mut *tx)
                    .await;
                let result = sqlx::query_as::<_, crate::db::file_assets::FileAssetRow>(
                    "SELECT id, tenant_id, owner_user_id, bucket, object_key, content_type, size_bytes,
                            status, visibility, linked_entity_type, linked_entity_id, created_at
                       FROM file_assets WHERE id = $1"
                ).bind(fid).fetch_optional(&mut *tx).await.ok().flatten();
                let _ = tx.commit().await;
                result
            };
            if let Some(asset) = asset_opt {
                let _ = storage.delete_object(&asset.object_key).await;
            }

            // Mark file_asset pruned (separate tx with tenant context).
            if let Ok(mut tx) = pool.begin().await {
                let _ = sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
                    .bind(tenant_id.to_string())
                    .execute(&mut *tx)
                    .await;
                let _ = sqlx::query("UPDATE file_assets SET status='pruned' WHERE id=$1")
                    .bind(fid)
                    .execute(&mut *tx)
                    .await;
                let _ = tx.commit().await;
            }
        }

        // Delete the recordings row (separate tx with tenant context).
        if let Ok(mut tx) = pool.begin().await {
            let _ = sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
                .bind(tenant_id.to_string())
                .execute(&mut *tx)
                .await;
            let _ = db_rec::delete_by_id(&mut tx, recording_id).await;
            let _ = tx.commit().await;
        }
        count += 1;
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn t() -> Uuid {
        Uuid::parse_str("9c2f4a8e-7b13-4f7c-91d2-b6a8e5c0d3e1").unwrap()
    }
    fn a() -> Uuid {
        Uuid::parse_str("4f1a2c8b-9d6e-4a7f-8c5b-3d2e1f9a0c8e").unwrap()
    }

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
        let segs = vec![
            PathBuf::from("/recordings/aula/x/y/z/seg1.mp4"),
            PathBuf::from("/recordings/aula/x/y/z/seg2.mp4"),
        ];
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

    #[tokio::test]
    async fn mock_recorder_remux_writes_stub_file() {
        let recorder = MockRecorderTool::new();
        let tmp = tempfile::tempdir().unwrap();
        let out = tmp.path().join("out.mp4");
        recorder
            .remux_to_mp4(&[PathBuf::from("seg1.mp4")], &out)
            .await
            .unwrap();
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
        recorder
            .remux_to_mp4(&[PathBuf::from("seg1.mp4")], &out)
            .await
            .unwrap();
        let dur = recorder.probe_duration_seconds(&out).await.unwrap();
        assert_eq!(dur, 120);
    }

    #[tokio::test]
    async fn mock_recorder_reports_configured_video_presence() {
        let recorder = MockRecorderTool::new();
        let tmp = tempfile::tempdir().unwrap();
        let out = tmp.path().join("out.mp4");

        // Default is "has video" -- the overwhelmingly common case, and the
        // safe default if a caller forgets to configure it.
        assert!(recorder.probe_has_video_stream(&out).await.unwrap());

        recorder.set_has_video(false);
        assert!(!recorder.probe_has_video_stream(&out).await.unwrap());
        assert!(
            recorder
                .calls()
                .iter()
                .any(|c| matches!(c, RecorderCall::ProbeHasVideo { .. })),
            "the probe must be observable so pipeline tests can assert it ran"
        );
    }

    #[tokio::test]
    async fn real_ffprobe_distinguishes_audio_only_from_video() {
        // Guards the actual contract the UI depends on: `-select_streams v`
        // prints nothing for an audio-only file. Generates both files with
        // ffmpeg so it tests the real tool, not a stubbed answer. Skipped where
        // ffmpeg is unavailable so the suite stays runnable off the container.
        if tokio::process::Command::new("ffprobe")
            .arg("-version")
            .output()
            .await
            .is_err()
        {
            eprintln!("skipping: ffprobe not on PATH");
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let with_video = tmp.path().join("av.mp4");
        let audio_only = tmp.path().join("a.mp4");

        let gen = |args: Vec<String>| async move {
            tokio::process::Command::new("ffmpeg")
                .args(args)
                .output()
                .await
                .expect("ffmpeg run")
        };
        gen(vec![
            "-y", "-f", "lavfi", "-i", "testsrc=size=64x64:rate=10:duration=1",
            "-f", "lavfi", "-i", "sine=frequency=440:duration=1",
            "-c:v", "libx264", "-c:a", "aac", "-shortest",
        ]
        .into_iter()
        .map(String::from)
        .chain(std::iter::once(with_video.to_string_lossy().into_owned()))
        .collect())
        .await;
        gen(vec![
            "-y", "-f", "lavfi", "-i", "sine=frequency=440:duration=1", "-c:a", "aac",
        ]
        .into_iter()
        .map(String::from)
        .chain(std::iter::once(audio_only.to_string_lossy().into_owned()))
        .collect())
        .await;

        let recorder = RealFfmpegRecorder::new();
        if with_video.exists() {
            assert!(
                recorder.probe_has_video_stream(&with_video).await.unwrap(),
                "a file with a video stream must probe true"
            );
        }
        if audio_only.exists() {
            assert!(
                !recorder.probe_has_video_stream(&audio_only).await.unwrap(),
                "an audio-only file must probe false -- this is the exact shape \
                 of the pre-H.264 recordings the UI has to label"
            );
        }
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
}
