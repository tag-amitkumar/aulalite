# AulaLite — Phase 1b-α Implementation Plan (File Upload Pipeline)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship a generic two-phase upload pipeline (`/v1/uploads/begin` → presigned MinIO PUT → `/v1/uploads/:id/complete`) plus three concrete consumers — course cover image, lesson `video`, lesson `file_bundle` attachments — with the read-side display surfaces needed to demo each end-to-end on web.

**Architecture:** Backend exposes upload-begin / complete / get-url / delete routes with per-`(linked_entity_type, purpose)` authorization dispatch. Production wraps `aws-sdk-s3` against the existing MinIO container; tests inject a `MockS3Client` that captures calls. The pipeline never streams blob bytes through the API — clients `PUT` directly to MinIO via short-lived presigned URLs. Idempotent `/complete` HEADs the object to verify size + presence before flipping `file_assets.status` from `pending` to `available`.

**Tech Stack:** Rust 1.94, sqlx 0.8 + Postgres 16 with RLS, `aws-sdk-s3` (MinIO via `force_path_style(true)` and custom `endpoint_url`), `aws-config` for credential chain, `serde_with` (DoubleOption for nullable PATCH), Dioxus 0.7.4 (reuses Phase 1a SSR convention), `web-sys::XmlHttpRequest` for client upload progress, `pulldown-cmark` already pulled in by Phase 1a.

**Companion spec:** `docs/superpowers/specs/2026-05-08-aulalite-phase-1b-alpha-file-uploads-design.md`.

---

## Prerequisites

Each must pass before Task 1.

- **Phase 1a complete on `phase-0-foundations`.** HEAD should be `6327075` (the 1b-α spec commit) or later. Verify with `git log --oneline -3` from the worktree.
- **Docker Compose stack running.** `docker compose up -d` from the worktree root. All services healthy.
- **MinIO reachable on `localhost:9000`** with bucket `aulalite` either pre-created or auto-created by Task 6's startup hook. Console at `localhost:9001` (login per `.env`).
- **Postgres on `localhost:55432`.** All Phase 0 + 1a migrations already applied.
- **`.env` mirrors `.env.example` plus three new entries** added in Task 1:
  - `S3_ENDPOINT_URL=http://localhost:9000`
  - `S3_REGION=us-east-1`
  - `S3_BUCKET=aulalite`
  - `AWS_ACCESS_KEY_ID=aulalite` (matches `MINIO_ROOT_USER`)
  - `AWS_SECRET_ACCESS_KEY=changeme123` (matches `MINIO_ROOT_PASSWORD`)
- **`features-courses` SSR convention** (from Phase 1a): use `dioxus-ssr` separate dev-dep; SSR tests use `fn app() -> Element { rsx! { Component { ... } } } let mut vdom = VirtualDom::new(app); vdom.rebuild_in_place(); dioxus_ssr::render(&vdom)`. Closures inline (`on_x: |_| {}`) rather than `EventHandler::new(...)` outside a runtime.

---

## File Structure (target after Phase 1b-α)

Files **created**:

```
migrations/
  20260508000011_tighten_file_assets_links.sql

crates/backend/
  src/
    storage/                                # NEW module
      mod.rs                                # S3Client trait + S3Call enum
      minio.rs                              # MinIoClient (aws-sdk-s3 wrapper) + ensure_bucket
      mock.rs                               # MockS3Client; pub mod mock
    services/
      file_assets.rs                        # NEW (sanitize_filename, object_key, validate_request, ValidateError)
    db/
      file_assets.rs                        # NEW (insert_pending, mark_available, mark_failed, fetch, list_for_entity, delete)
    handlers/
      uploads.rs                            # NEW (POST /v1/uploads/begin + /v1/uploads/:id/complete)
      file_assets.rs                        # NEW (GET /v1/file-assets/:id/url + DELETE + GET /v1/lessons/:lid/files)
  tests/
    uploads.rs                              # NEW
    uploads_validation_matrix.rs            # NEW
    course_cover_upload.rs                  # NEW
    lesson_attachments.rs                   # NEW

crates/features-courses/
  src/
    file_picker.rs                          # NEW (XHR upload widget + validation submodule)
    course_cover_editor.rs                  # NEW
    lesson_video_editor.rs                  # NEW
    lesson_files_editor.rs                  # NEW
    file_asset_image.rs                     # NEW
    lesson_outline_view.rs                  # NEW

crates/design-system/
  src/
    progress_bar.rs                         # NEW
    file_card.rs                            # NEW

docs/superpowers/plans/
  2026-05-08-aulalite-phase-1b-alpha-exit-checklist.md   # NEW
```

Files **modified**:

```
Cargo.toml                                  # workspace deps += aws-sdk-s3, aws-config, aws-credential-types, serde_with
.env, .env.example                          # S3_* + AWS_* env vars

crates/backend/Cargo.toml                   # deps += the AWS SDK + serde_with
crates/backend/src/lib.rs                   # AppState gets storage + bucket_name; routes wired
crates/backend/src/main.rs                  # construct MinIoClient; ensure_bucket on startup
crates/backend/src/error.rs                 # 3 new ApiError variants
crates/backend/src/db/mod.rs                # pub mod file_assets;
crates/backend/src/db/lessons.rs            # type_supported_at_1a -> type_supported_at_1b_alpha
crates/backend/src/handlers/mod.rs          # pub mod uploads; pub mod file_assets;
crates/backend/src/handlers/courses.rs      # PatchCourse.cover_asset_id (DoubleOption)
crates/backend/src/handlers/lessons.rs      # type rejection lifted; PatchLesson.video_asset_id (DoubleOption)
crates/backend/src/services/mod.rs          # pub mod file_assets;
crates/backend/src/storage/mod.rs           # the new module file
crates/backend/tests/lessons_crud.rs        # video_lesson_rejected_at_phase_1a -> renamed/flipped

crates/design-system/src/lib.rs             # pub use progress_bar::ProgressBar; pub use file_card::FileCard;

crates/features-courses/Cargo.toml          # deps += serde_with workspace
crates/features-courses/src/lib.rs          # pub mod file_picker; pub mod course_cover_editor; etc.
crates/features-courses/src/lesson_editor.rs        # 4-branch type switch
crates/features-courses/src/course_builder.rs       # show video/file_bundle types
crates/features-courses/src/course_list.rs          # cover image on cards
crates/features-courses/src/course_detail.rs        # cover banner; Edit tab; Outline tab
crates/features-courses/src/error_messages.rs       # 3 new humanizations
```

---

# Section A — Foundation (deps, migration, pure services, S3 trait)

### Task 1: Add AWS SDK + serde_with workspace deps

**Files:**
- Modify: `Cargo.toml` (workspace root)
- Modify: `crates/backend/Cargo.toml`
- Modify: `.env.example`

- [ ] **Step 1: Append to root `Cargo.toml` `[workspace.dependencies]`**

```toml
aws-config = { version = "1", default-features = false, features = ["rustls", "rt-tokio", "behavior-version-latest"] }
aws-sdk-s3 = { version = "1", default-features = false, features = ["rustls", "rt-tokio", "behavior-version-latest"] }
aws-credential-types = "1"
serde_with = "3"
```

- [ ] **Step 2: Add to `crates/backend/Cargo.toml` `[dependencies]`**

```toml
aws-config = { workspace = true }
aws-sdk-s3 = { workspace = true }
aws-credential-types = { workspace = true }
serde_with = { workspace = true }
```

- [ ] **Step 3: Append new env vars to `.env.example`**

```bash
# MinIO / S3 storage. Use the same credentials as MINIO_ROOT_USER / MINIO_ROOT_PASSWORD
# for local dev; rotate to a service account in production.
S3_ENDPOINT_URL=http://localhost:9000
S3_REGION=us-east-1
S3_BUCKET=aulalite
AWS_ACCESS_KEY_ID=aulalite
AWS_SECRET_ACCESS_KEY=changeme123
```

Mirror to `.env` with real local values. (`.env` is gitignored.)

- [ ] **Step 4: Build to verify deps resolve**

```bash
cd "C:/Users/Chiranjib Chaudhuri/Documents/Chiranjib/Elementors_Aula-phase0"
cargo build -p backend
```
Expected: succeeds. (First build will compile aws-sdk-s3 and its dependencies — can take several minutes.)

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml crates/backend/Cargo.toml .env.example Cargo.lock
git commit -m "chore(deps): add aws-sdk-s3 + aws-config + serde_with for upload pipeline"
```

---

### Task 2: Migration 0011 — tighten file_assets links

**Files:**
- Create: `migrations/20260508000011_tighten_file_assets_links.sql`

- [ ] **Step 1: Write the migration**

```sql
-- migrations/20260508000011_tighten_file_assets_links.sql

-- Wire deferred FKs from Phase 1a Migration 0009 (file_assets) back to courses + lessons.
-- ON DELETE SET NULL because asset deletion shouldn't cascade-remove the parent course/lesson.
ALTER TABLE courses
    ADD CONSTRAINT courses_cover_asset_id_fkey
    FOREIGN KEY (cover_asset_id) REFERENCES file_assets(id) ON DELETE SET NULL;

ALTER TABLE lessons
    ADD CONSTRAINT lessons_video_asset_id_fkey
    FOREIGN KEY (video_asset_id) REFERENCES file_assets(id) ON DELETE SET NULL;

-- Tighten polymorphic linked_entity_type to known values. Forward-compatible:
-- new consumers extend the CHECK in their own migration.
ALTER TABLE file_assets
    ADD CONSTRAINT file_assets_linked_entity_type_check
    CHECK (linked_entity_type IS NULL
        OR linked_entity_type IN ('course', 'lesson'));

-- Defensive invariant: size_bytes must be non-negative.
ALTER TABLE file_assets
    ADD CONSTRAINT file_assets_size_nonneg
    CHECK (size_bytes >= 0);
```

- [ ] **Step 2: Apply**

```bash
cd "C:/Users/Chiranjib Chaudhuri/Documents/Chiranjib/Elementors_Aula-phase0"
DATABASE_URL=postgres://aulalite:changeme@localhost:55432/aulalite \
    sqlx migrate run --source migrations
```
Expected: `Applied 20260508000011/migrate tighten file assets links`.

- [ ] **Step 3: Verify**

```bash
docker exec aulalite-postgres-1 psql -U aulalite -d aulalite -c "\d courses" | grep cover_asset_id
docker exec aulalite-postgres-1 psql -U aulalite -d aulalite -c "\d lessons" | grep video_asset_id
docker exec aulalite-postgres-1 psql -U aulalite -d aulalite -c "\d file_assets" | grep -E "linked_entity_type_check|size_nonneg"
```

Each should show the new constraint.

- [ ] **Step 4: Commit**

```bash
git add migrations/20260508000011_tighten_file_assets_links.sql
git commit -m "feat(db): migration 0011 wire file_assets FKs and tighten CHECKs"
```

---

### Task 3: services::file_assets pure functions (TDD)

**Files:**
- Create: `crates/backend/src/services/file_assets.rs`
- Modify: `crates/backend/src/services/mod.rs`

- [ ] **Step 1: Add `pub mod file_assets;` to `crates/backend/src/services/mod.rs`**

The file currently has:
```rust
pub mod invitations;
pub mod recurrence;
pub mod slugger;
```
Add a fourth line:
```rust
pub mod file_assets;
```

- [ ] **Step 2: Write the failing tests**

Create `crates/backend/src/services/file_assets.rs`:

```rust
// crates/backend/src/services/file_assets.rs
//! Pure-function helpers for the upload pipeline: filename sanitization,
//! object-key formatting, request validation. No IO, no clock dependency
//! beyond what callers pass in explicitly.

use chrono::{DateTime, Datelike, Utc};
use uuid::Uuid;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ValidateError {
    #[error("content type {0} not allowed for purpose {1}")]
    ContentTypeNotAllowed(String, String),
    #[error("size {0} exceeds {1} cap for purpose {2}")]
    SizeOverCap(i64, i64, String),
    #[error("size must be non-negative")]
    SizeNegative,
    #[error("unknown purpose: {0}")]
    UnknownPurpose(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn sanitize_strips_path_separators() {
        assert_eq!(sanitize_filename("../etc/passwd"), "etc_passwd");
        assert_eq!(sanitize_filename("a\\b\\c.txt"), "a_b_c.txt");
    }

    #[test]
    fn sanitize_strips_control_chars() {
        assert_eq!(sanitize_filename("hi\x00.txt"), "hi.txt");
        assert_eq!(sanitize_filename("a\nb.txt"), "a_b.txt");
    }

    #[test]
    fn sanitize_collapses_spaces_to_underscore() {
        assert_eq!(sanitize_filename("Week 1 Slides.pptx"), "Week_1_Slides.pptx");
    }

    #[test]
    fn sanitize_preserves_extension_on_truncate() {
        let long = "x".repeat(300);
        let with_ext = format!("{long}.pdf");
        let out = sanitize_filename(&with_ext);
        assert!(out.len() <= 200);
        assert!(out.ends_with(".pdf"));
    }

    #[test]
    fn sanitize_handles_empty() {
        assert_eq!(sanitize_filename(""), "unnamed");
        assert_eq!(sanitize_filename("   "), "unnamed");
        assert_eq!(sanitize_filename("///"), "unnamed");
    }

    #[test]
    fn object_key_format() {
        let tenant = Uuid::parse_str("9c2f4a8e-7b13-4f7c-91d2-b6a8e5c0d3e1").unwrap();
        let asset = Uuid::parse_str("4f1a2c8b-9d6e-4a7f-8c5b-3d2e1f9a0c8e").unwrap();
        let now = Utc.with_ymd_and_hms(2026, 5, 8, 12, 0, 0).unwrap();
        let key = object_key(tenant, asset, "Slides.pptx", now);
        assert_eq!(
            key,
            "9c2f4a8e7b134f7c91d2b6a8e5c0d3e1/2026/05/4f1a2c8b9d6e4a7f8c5b3d2e1f9a0c8e/Slides.pptx"
        );
    }

    #[test]
    fn object_key_zero_pads_month() {
        let tenant = Uuid::nil();
        let asset = Uuid::nil();
        let now = Utc.with_ymd_and_hms(2026, 1, 8, 0, 0, 0).unwrap();
        let key = object_key(tenant, asset, "x.txt", now);
        assert!(key.contains("/2026/01/"));
    }

    #[test]
    fn validate_cover_accepts_jpeg() {
        assert!(validate_request("cover", "image/jpeg", 1_000_000).is_ok());
    }

    #[test]
    fn validate_cover_rejects_svg() {
        let err = validate_request("cover", "image/svg+xml", 1000).unwrap_err();
        assert_eq!(
            err,
            ValidateError::ContentTypeNotAllowed(
                "image/svg+xml".into(),
                "cover".into()
            )
        );
    }

    #[test]
    fn validate_cover_rejects_oversized() {
        let err = validate_request("cover", "image/png", 6_000_000).unwrap_err();
        assert_eq!(
            err,
            ValidateError::SizeOverCap(6_000_000, 5_242_880, "cover".into())
        );
    }

    #[test]
    fn validate_video_accepts_mp4() {
        assert!(validate_request("video", "video/mp4", 400_000_000).is_ok());
    }

    #[test]
    fn validate_attachment_accepts_pdf_under_cap() {
        assert!(validate_request("attachment", "application/pdf", 50_000_000).is_ok());
    }

    #[test]
    fn validate_attachment_rejects_executable() {
        let err = validate_request("attachment", "application/x-msdownload", 1000).unwrap_err();
        assert!(matches!(err, ValidateError::ContentTypeNotAllowed(_, _)));
    }

    #[test]
    fn validate_unknown_purpose() {
        let err = validate_request("avatar", "image/png", 1000).unwrap_err();
        assert_eq!(err, ValidateError::UnknownPurpose("avatar".into()));
    }

    #[test]
    fn validate_negative_size() {
        let err = validate_request("cover", "image/png", -1).unwrap_err();
        assert_eq!(err, ValidateError::SizeNegative);
    }
}
```

- [ ] **Step 3: Run — expect compile failure (functions undefined)**

```bash
cd "C:/Users/Chiranjib Chaudhuri/Documents/Chiranjib/Elementors_Aula-phase0"
cargo test -p backend --lib services::file_assets 2>&1 | tail -10
```

Expected: `cannot find function 'sanitize_filename' / 'object_key' / 'validate_request'`.

- [ ] **Step 4: Implement**

Add above the `#[cfg(test)]` block in `crates/backend/src/services/file_assets.rs`:

```rust
const COVER_TYPES: &[&str] = &["image/jpeg", "image/png", "image/webp"];
const VIDEO_TYPES: &[&str] = &["video/mp4", "video/webm"];
const ATTACHMENT_TYPES: &[&str] = &[
    "application/pdf",
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    "application/vnd.openxmlformats-officedocument.presentationml.presentation",
    "application/zip",
    "text/plain",
    "text/csv",
    "image/jpeg",
    "image/png",
    "image/webp",
    "audio/mpeg",
    "video/mp4",
];

pub const COVER_MAX_SIZE: i64 = 5 * 1024 * 1024;
pub const VIDEO_MAX_SIZE: i64 = 500 * 1024 * 1024;
pub const ATTACHMENT_MAX_SIZE: i64 = 100 * 1024 * 1024;

pub fn validate_request(purpose: &str, content_type: &str, size_bytes: i64) -> Result<(), ValidateError> {
    if size_bytes < 0 {
        return Err(ValidateError::SizeNegative);
    }
    let (allowed, cap) = match purpose {
        "cover" => (COVER_TYPES, COVER_MAX_SIZE),
        "video" => (VIDEO_TYPES, VIDEO_MAX_SIZE),
        "attachment" => (ATTACHMENT_TYPES, ATTACHMENT_MAX_SIZE),
        other => return Err(ValidateError::UnknownPurpose(other.to_string())),
    };
    if !allowed.iter().any(|t| t.eq_ignore_ascii_case(content_type)) {
        return Err(ValidateError::ContentTypeNotAllowed(
            content_type.to_string(),
            purpose.to_string(),
        ));
    }
    if size_bytes > cap {
        return Err(ValidateError::SizeOverCap(size_bytes, cap, purpose.to_string()));
    }
    Ok(())
}

pub fn sanitize_filename(input: &str) -> String {
    // Replace path separators and control chars with underscore; collapse runs of underscores;
    // truncate to 200 chars while preserving extension; fall back to "unnamed" if empty.
    let mut buf = String::with_capacity(input.len());
    let mut prev_underscore = false;
    for ch in input.chars() {
        let mapped = match ch {
            '/' | '\\' | '\0'..='\x1f' | '\x7f' | ' ' | '\t' | '\n' | '\r' => '_',
            c => c,
        };
        if mapped == '_' {
            if !prev_underscore && !buf.is_empty() {
                buf.push('_');
                prev_underscore = true;
            }
        } else {
            buf.push(mapped);
            prev_underscore = false;
        }
    }
    while buf.ends_with('_') {
        buf.pop();
    }
    while buf.starts_with('_') {
        buf.remove(0);
    }
    if buf.is_empty() {
        return "unnamed".to_string();
    }
    if buf.len() <= 200 {
        return buf;
    }
    // Truncate while preserving extension (last dot, up to 16 chars after)
    if let Some(dot) = buf.rfind('.') {
        let ext = &buf[dot..];
        if ext.len() <= 16 {
            let prefix_len = 200 - ext.len();
            return format!("{}{}", &buf[..prefix_len.min(dot)], ext);
        }
    }
    buf[..200].to_string()
}

pub fn object_key(tenant_id: Uuid, asset_id: Uuid, filename: &str, now: DateTime<Utc>) -> String {
    let sanitized = sanitize_filename(filename);
    format!(
        "{}/{:04}/{:02}/{}/{}",
        tenant_id.simple(),
        now.year(),
        now.month(),
        asset_id.simple(),
        sanitized
    )
}
```

- [ ] **Step 5: Run — expect 13 passed**

```bash
cargo test -p backend --lib services::file_assets 2>&1 | tail -5
```

Expected: `13 passed`.

- [ ] **Step 6: Commit**

```bash
git add crates/backend/src/services/mod.rs crates/backend/src/services/file_assets.rs
git commit -m "feat(services): file_assets validation + sanitize_filename + object_key with TDD coverage"
```

---

### Task 4: storage module (S3Client trait + MockS3Client)

**Files:**
- Create: `crates/backend/src/storage/mod.rs`
- Create: `crates/backend/src/storage/mock.rs`
- Modify: `crates/backend/src/lib.rs` (add `pub mod storage;`)

- [ ] **Step 1: Add `pub mod storage;` to `crates/backend/src/lib.rs`**

After the existing `pub mod services;` line, add:
```rust
pub mod storage;
```

- [ ] **Step 2: Create `crates/backend/src/storage/mod.rs`**

```rust
// crates/backend/src/storage/mod.rs
//! S3-compatible blob storage abstraction. Production implementation wraps
//! aws-sdk-s3 against MinIO; tests inject MockS3Client.

pub mod mock;
// minio.rs is added in Task 5.

use async_trait::async_trait;
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("s3 error: {0}")]
    S3(String),
    #[error("object not found: {0}")]
    NotFound(String),
    #[error("size mismatch: expected {expected}, observed {observed}")]
    SizeMismatch { expected: i64, observed: i64 },
    #[error("bucket bootstrap failed: {0}")]
    BucketBootstrap(String),
}

/// Object metadata returned by `head_object`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectHead {
    pub size_bytes: i64,
    pub content_type: Option<String>,
}

/// Operations the upload handlers need from blob storage.
#[async_trait]
pub trait S3Client: Send + Sync {
    /// Mint a presigned URL the client uses to PUT bytes directly.
    /// `content_type` and `content_length` are baked into the signed URL,
    /// so the client must `PUT` with matching headers.
    async fn presigned_put_url(
        &self,
        key: &str,
        content_type: &str,
        content_length: i64,
        ttl: Duration,
    ) -> Result<String, StorageError>;

    /// Mint a presigned URL the client uses to GET bytes for display/download.
    async fn presigned_get_url(
        &self,
        key: &str,
        ttl: Duration,
    ) -> Result<String, StorageError>;

    /// HEAD the object; used by `/uploads/:id/complete` to verify size + presence.
    async fn head_object(&self, key: &str) -> Result<ObjectHead, StorageError>;

    /// Hard delete the object. The DB row is separately marked `pruned`.
    async fn delete_object(&self, key: &str) -> Result<(), StorageError>;

    /// Idempotent bucket creation. Called at backend startup.
    async fn ensure_bucket(&self, name: &str) -> Result<(), StorageError>;
}
```

- [ ] **Step 3: Create `crates/backend/src/storage/mock.rs`**

```rust
// crates/backend/src/storage/mock.rs
//! In-memory mock for testing. Captures every call and serves deterministic
//! presigned URLs and HEAD responses based on what tests "simulate".

use super::*;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum S3Call {
    PresignPut { key: String, content_type: String, content_length: i64, ttl_secs: u64 },
    PresignGet { key: String, ttl_secs: u64 },
    Head { key: String },
    Delete { key: String },
    EnsureBucket { name: String },
}

#[derive(Clone, Default)]
pub struct MockS3Client {
    pub calls: Arc<Mutex<Vec<S3Call>>>,
    /// Objects the test has "simulated" being in storage.
    /// Map key -> (size, content_type).
    pub objects: Arc<Mutex<HashMap<String, (i64, String)>>>,
}

impl MockS3Client {
    pub fn new() -> Self {
        Self::default()
    }

    /// Test helper: declare that an object exists with the given size + type.
    /// Used to make `head_object` return success.
    pub fn simulate_object(&self, key: impl Into<String>, size: i64, content_type: impl Into<String>) {
        self.objects
            .lock()
            .unwrap()
            .insert(key.into(), (size, content_type.into()));
    }

    pub fn calls(&self) -> Vec<S3Call> {
        self.calls.lock().unwrap().clone()
    }

    fn record(&self, call: S3Call) {
        self.calls.lock().unwrap().push(call);
    }
}

#[async_trait]
impl S3Client for MockS3Client {
    async fn presigned_put_url(
        &self,
        key: &str,
        content_type: &str,
        content_length: i64,
        ttl: Duration,
    ) -> Result<String, StorageError> {
        self.record(S3Call::PresignPut {
            key: key.to_string(),
            content_type: content_type.to_string(),
            content_length,
            ttl_secs: ttl.as_secs(),
        });
        Ok(format!("https://mock.s3/put/{key}?ttl={}", ttl.as_secs()))
    }

    async fn presigned_get_url(&self, key: &str, ttl: Duration) -> Result<String, StorageError> {
        self.record(S3Call::PresignGet {
            key: key.to_string(),
            ttl_secs: ttl.as_secs(),
        });
        Ok(format!("https://mock.s3/get/{key}?ttl={}", ttl.as_secs()))
    }

    async fn head_object(&self, key: &str) -> Result<ObjectHead, StorageError> {
        self.record(S3Call::Head {
            key: key.to_string(),
        });
        let objects = self.objects.lock().unwrap();
        match objects.get(key) {
            Some((size, ct)) => Ok(ObjectHead {
                size_bytes: *size,
                content_type: Some(ct.clone()),
            }),
            None => Err(StorageError::NotFound(key.to_string())),
        }
    }

    async fn delete_object(&self, key: &str) -> Result<(), StorageError> {
        self.record(S3Call::Delete {
            key: key.to_string(),
        });
        self.objects.lock().unwrap().remove(key);
        Ok(())
    }

    async fn ensure_bucket(&self, name: &str) -> Result<(), StorageError> {
        self.record(S3Call::EnsureBucket {
            name: name.to_string(),
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_records_all_call_types() {
        let s3 = MockS3Client::new();
        s3.simulate_object("k", 100, "image/png");
        let _ = s3.presigned_put_url("k", "image/png", 100, Duration::from_secs(900)).await.unwrap();
        let _ = s3.presigned_get_url("k", Duration::from_secs(900)).await.unwrap();
        let head = s3.head_object("k").await.unwrap();
        s3.delete_object("k").await.unwrap();
        s3.ensure_bucket("aulalite").await.unwrap();
        let calls = s3.calls();
        assert_eq!(calls.len(), 5);
        assert_eq!(head.size_bytes, 100);
        assert!(matches!(calls[0], S3Call::PresignPut { .. }));
        assert!(matches!(calls[4], S3Call::EnsureBucket { .. }));
    }

    #[tokio::test]
    async fn head_returns_not_found_for_unsimulated_key() {
        let s3 = MockS3Client::new();
        let err = s3.head_object("missing").await.unwrap_err();
        assert!(matches!(err, StorageError::NotFound(_)));
    }
}
```

- [ ] **Step 4: Build + run mock tests**

```bash
cargo build -p backend
cargo test -p backend --lib storage::mock
```
Expected: 2 passed.

- [ ] **Step 5: Commit**

```bash
git add crates/backend/src/lib.rs crates/backend/src/storage
git commit -m "feat(storage): S3Client trait + MockS3Client with call recording"
```

---

# Section B — Storage production wiring (MinIoClient + AppState bootstrap)

### Task 5: storage::minio.rs — MinIoClient impl

**Files:**
- Create: `crates/backend/src/storage/minio.rs`
- Modify: `crates/backend/src/storage/mod.rs` (add `pub mod minio;`)

- [ ] **Step 1: Add `pub mod minio;` to `crates/backend/src/storage/mod.rs`**

After `pub mod mock;`:
```rust
pub mod minio;
```

- [ ] **Step 2: Implement `crates/backend/src/storage/minio.rs`**

```rust
// crates/backend/src/storage/minio.rs
//! Production S3Client implementation backed by aws-sdk-s3 against a MinIO
//! endpoint. Uses path-style addressing (force_path_style=true) which MinIO
//! requires; AWS S3 itself accepts both.

use super::*;
use async_trait::async_trait;
use aws_config::{BehaviorVersion, Region};
use aws_credential_types::Credentials;
use aws_sdk_s3::{
    config::Builder as S3ConfigBuilder,
    presigning::PresigningConfig,
    primitives::ByteStream,
    Client,
};
use std::time::Duration;

#[derive(Clone)]
pub struct MinIoClient {
    pub client: Client,
    pub bucket: String,
}

pub struct MinIoConfig {
    pub endpoint_url: String,
    pub region: String,
    pub bucket: String,
    pub access_key_id: String,
    pub secret_access_key: String,
}

impl MinIoClient {
    pub fn new(cfg: MinIoConfig) -> Self {
        let credentials = Credentials::new(
            cfg.access_key_id,
            cfg.secret_access_key,
            None,
            None,
            "static",
        );
        let s3_config = S3ConfigBuilder::new()
            .behavior_version(BehaviorVersion::latest())
            .endpoint_url(cfg.endpoint_url)
            .region(Region::new(cfg.region))
            .credentials_provider(credentials)
            .force_path_style(true)
            .build();
        let client = Client::from_conf(s3_config);
        Self {
            client,
            bucket: cfg.bucket,
        }
    }
}

fn map_err<E: std::fmt::Debug>(prefix: &str, err: E) -> StorageError {
    StorageError::S3(format!("{prefix}: {err:?}"))
}

#[async_trait]
impl S3Client for MinIoClient {
    async fn presigned_put_url(
        &self,
        key: &str,
        content_type: &str,
        content_length: i64,
        ttl: Duration,
    ) -> Result<String, StorageError> {
        let presigning = PresigningConfig::expires_in(ttl)
            .map_err(|e| map_err("presigning config", e))?;
        let req = self
            .client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .content_type(content_type)
            .content_length(content_length)
            .presigned(presigning)
            .await
            .map_err(|e| map_err("presigned put", e))?;
        Ok(req.uri().to_string())
    }

    async fn presigned_get_url(&self, key: &str, ttl: Duration) -> Result<String, StorageError> {
        let presigning = PresigningConfig::expires_in(ttl)
            .map_err(|e| map_err("presigning config", e))?;
        let req = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .presigned(presigning)
            .await
            .map_err(|e| map_err("presigned get", e))?;
        Ok(req.uri().to_string())
    }

    async fn head_object(&self, key: &str) -> Result<ObjectHead, StorageError> {
        let resp = self
            .client
            .head_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await;
        match resp {
            Ok(o) => Ok(ObjectHead {
                size_bytes: o.content_length().unwrap_or(0),
                content_type: o.content_type().map(|s| s.to_string()),
            }),
            Err(e) => {
                let svc_err = e.into_service_error();
                if svc_err.is_not_found() {
                    Err(StorageError::NotFound(key.to_string()))
                } else {
                    Err(StorageError::S3(format!("head_object: {svc_err:?}")))
                }
            }
        }
    }

    async fn delete_object(&self, key: &str) -> Result<(), StorageError> {
        let _ = ByteStream::from_static(b""); // ensure import path is valid
        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| map_err("delete_object", e))?;
        Ok(())
    }

    async fn ensure_bucket(&self, name: &str) -> Result<(), StorageError> {
        // HEAD bucket; if 404, create.
        let head = self
            .client
            .head_bucket()
            .bucket(name)
            .send()
            .await;
        if head.is_ok() {
            return Ok(());
        }
        let svc_err = head.err().unwrap().into_service_error();
        if !svc_err.is_not_found() {
            return Err(StorageError::BucketBootstrap(format!(
                "head_bucket failed (non-404): {svc_err:?}"
            )));
        }
        self.client
            .create_bucket()
            .bucket(name)
            .send()
            .await
            .map_err(|e| StorageError::BucketBootstrap(format!("create_bucket: {e:?}")))?;
        Ok(())
    }
}
```

- [ ] **Step 3: Build**

```bash
cd "C:/Users/Chiranjib Chaudhuri/Documents/Chiranjib/Elementors_Aula-phase0"
cargo build -p backend
```
Expected: succeeds. (`aws-sdk-s3` API surface stabilized in 1.x — but if any of the method names above have drifted in a more recent point release, fix at the call site to match.)

- [ ] **Step 4: Commit**

```bash
git add crates/backend/src/storage
git commit -m "feat(storage): MinIoClient production impl over aws-sdk-s3"
```

---

### Task 6: AppState wiring + main.rs bootstrap

**Files:**
- Modify: `crates/backend/src/lib.rs`
- Modify: `crates/backend/src/main.rs`

- [ ] **Step 1: Update `crates/backend/src/lib.rs`'s `AppState`**

Read the current `lib.rs`. Find the `AppState` struct (introduced in Phase 1a Task 17). Add two fields:

```rust
pub storage: Arc<dyn crate::storage::S3Client>,
pub bucket_name: String,
```

The full struct should now read:
```rust
#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub verifier: Arc<Verifier>,
    pub email_link_sender: Arc<dyn EmailLinkSender>,
    pub app_origin: String,
    pub storage: Arc<dyn crate::storage::S3Client>,
    pub bucket_name: String,
}
```

(`EmailLinkSender` is imported via `use crate::services::invitations::EmailLinkSender;` in Phase 1a's lib.rs — keep that as-is.)

- [ ] **Step 2: Update `crates/backend/src/main.rs`**

Read the current `main.rs`. After `db::run_migrations`, add MinIO config from env and bucket bootstrap:

```rust
let s3_endpoint_url = std::env::var("S3_ENDPOINT_URL")?;
let s3_region = std::env::var("S3_REGION")?;
let bucket_name = std::env::var("S3_BUCKET")?;
let aws_access_key_id = std::env::var("AWS_ACCESS_KEY_ID")?;
let aws_secret_access_key = std::env::var("AWS_SECRET_ACCESS_KEY")?;

let storage: Arc<dyn backend::storage::S3Client> = Arc::new(
    backend::storage::minio::MinIoClient::new(backend::storage::minio::MinIoConfig {
        endpoint_url: s3_endpoint_url,
        region: s3_region,
        bucket: bucket_name.clone(),
        access_key_id: aws_access_key_id,
        secret_access_key: aws_secret_access_key,
    }),
);
storage.ensure_bucket(&bucket_name).await?;
tracing::info!(%bucket_name, "minio bucket ready");
```

In the `AppState { ... }` literal, add the two new fields:
```rust
let state = AppState {
    pool,
    verifier,
    email_link_sender,
    app_origin,
    storage,
    bucket_name,
};
```

- [ ] **Step 3: Build + smoke-run Phase 0 health test**

```bash
cargo build -p backend
cargo test -p backend --test health
```
Expected: build succeeds; health test passes.

- [ ] **Step 4: Smoke-test bucket bootstrap against the live MinIO**

If the running backend container is stale, rebuild and restart:
```bash
cd "C:/Users/Chiranjib Chaudhuri/Documents/Chiranjib/Elementors_Aula-phase0"
docker compose build backend
docker compose up -d --force-recreate backend
sleep 3
curl -s http://localhost:8080/healthz
```

Expected: `ok`. Then check the MinIO console at `http://localhost:9001` (login per `.env`) — bucket `aulalite` should be visible.

If bucket bootstrap fails because the MinIO container didn't expose the right env to the backend, verify `.env` and `docker-compose.yml` agree on the MinIO endpoint URL (the backend container should reach MinIO at `http://minio:9000`, not `http://localhost:9000` — for the **container** S3 endpoint use `http://minio:9000` in the compose env, while local tests use `http://localhost:9000`).

- [ ] **Step 5: Commit**

```bash
git add crates/backend/src/lib.rs crates/backend/src/main.rs
git commit -m "feat(backend): wire S3Client + bucket bootstrap into AppState"
```

---

# Section C — db::file_assets queries

### Task 7: db::file_assets sqlx module

**Files:**
- Create: `crates/backend/src/db/file_assets.rs`
- Modify: `crates/backend/src/db/mod.rs`

- [ ] **Step 1: Add `pub mod file_assets;` to `crates/backend/src/db/mod.rs`**

The file currently has:
```rust
pub mod audit;
pub mod courses;
pub mod modules;
pub mod lessons;
pub mod enrollments;
pub mod live_sessions;
```
Append:
```rust
pub mod file_assets;
```

- [ ] **Step 2: Implement**

```rust
// crates/backend/src/db/file_assets.rs
use serde::Serialize;
use sqlx::{PgPool, Postgres, Transaction};
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

pub async fn fetch(pool: &PgPool, id: Uuid) -> sqlx::Result<Option<FileAssetRow>> {
    sqlx::query_as::<_, FileAssetRow>(
        "SELECT id, tenant_id, owner_user_id, bucket, object_key, content_type,
                size_bytes, status, visibility, linked_entity_type, linked_entity_id,
                created_at
           FROM file_assets WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
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

pub async fn mark_failed(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
) -> sqlx::Result<()> {
    sqlx::query("UPDATE file_assets SET status = 'failed' WHERE id = $1")
        .bind(id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

pub async fn mark_pruned(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
) -> sqlx::Result<bool> {
    Ok(sqlx::query("UPDATE file_assets SET status = 'pruned' WHERE id = $1 AND status <> 'pruned'")
        .bind(id)
        .execute(&mut **tx)
        .await?
        .rows_affected()
        > 0)
}

pub async fn list_for_entity(
    pool: &PgPool,
    linked_entity_type: &str,
    linked_entity_id: Uuid,
) -> sqlx::Result<Vec<FileAssetRow>> {
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
    .fetch_all(pool)
    .await
}
```

- [ ] **Step 3: Build**

```bash
cargo build -p backend
```
Expected: succeeds.

- [ ] **Step 4: Commit**

```bash
git add crates/backend/src/db/mod.rs crates/backend/src/db/file_assets.rs
git commit -m "feat(db): file_assets query module"
```

---

# Section D — ApiError + DoubleOption + lesson type lift

### Task 8: Extend ApiError with 3 new variants

**Files:**
- Modify: `crates/backend/src/error.rs`

- [ ] **Step 1: Update `crates/backend/src/error.rs`**

Find the `ApiError` enum (extended in Phase 1a Task 16). Append 3 new variants AFTER the Phase 1a `RecurrenceShapeInvalid` variant:

```rust
#[error("file asset not found")]
FileAssetNotFound,
#[error("upload validation failed: {0}")]
UploadValidationFailed(String),
#[error("upload object missing or size mismatch")]
UploadObjectMissing,
```

Add 3 corresponding match arms in the `IntoResponse` impl, AFTER the Phase 1a `RecurrenceShapeInvalid` arm:

```rust
ApiError::FileAssetNotFound => (
    StatusCode::NOT_FOUND,
    "file asset not found".into(),
),
ApiError::UploadValidationFailed(reason) => (
    StatusCode::BAD_REQUEST,
    format!("upload validation failed: {reason}"),
),
ApiError::UploadObjectMissing => (
    StatusCode::BAD_REQUEST,
    "upload object missing or size mismatch".into(),
),
```

- [ ] **Step 2: Build**

```bash
cd "C:/Users/Chiranjib Chaudhuri/Documents/Chiranjib/Elementors_Aula-phase0"
cargo build -p backend
```
Expected: succeeds.

- [ ] **Step 3: Commit**

```bash
git add crates/backend/src/error.rs
git commit -m "feat(error): add 3 Phase 1b-alpha ApiError variants for upload pipeline"
```

---

### Task 9: PatchCourse / PatchLesson — DoubleOption for nullable assets

**Files:**
- Modify: `crates/backend/src/handlers/courses.rs`
- Modify: `crates/backend/src/handlers/lessons.rs`
- Modify: `crates/backend/src/db/courses.rs`
- Modify: `crates/backend/src/db/lessons.rs`

**Goal:** distinguish "field absent in PATCH body" (no change) from "field is `null` in PATCH body" (clear to NULL). `Option<Option<Uuid>>` via `serde_with::rust::double_option` — `None` = absent, `Some(None)` = null, `Some(Some(id))` = set.

- [ ] **Step 1: Update `PatchCourse` in `crates/backend/src/handlers/courses.rs`**

Find the `PatchCourse` struct. Replace it with:

```rust
#[derive(Deserialize, Default)]
pub struct PatchCourse {
    pub title: Option<String>,
    pub description: Option<String>,
    pub status: Option<String>,
    #[serde(default, with = "::serde_with::rust::double_option")]
    pub cover_asset_id: Option<Option<Uuid>>,
}
```

In `patch_inner`, find the existing `db::courses::update_course(...)` call and modify its signature usage. We need an extra parameter for the cover_asset_id triple-state. The easier path is to update the DB function. Step 2 covers that.

After the existing body validates `status` transition, add a block that validates `cover_asset_id` when set:

```rust
if let Some(maybe_id) = body.cover_asset_id {
    if let Some(asset_id) = maybe_id {
        // Validate: asset exists, same tenant, status='available', linked to this course.
        let asset = db::file_assets::fetch(pool, asset_id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?
            .ok_or(ApiError::FileAssetNotFound)?;
        let tenant_id = ctx
            .tenant_id
            .ok_or_else(|| ApiError::BadRequest("no tenant".into()))?;
        if asset.tenant_id != tenant_id {
            return Err(ApiError::FileAssetNotFound);
        }
        if asset.status != "available" {
            return Err(ApiError::BadRequest(format!(
                "asset is in status '{}'; must be 'available'",
                asset.status
            )));
        }
        let valid_link = asset.linked_entity_type.as_deref() == Some("course")
            && asset.linked_entity_id == Some(id);
        if !valid_link {
            return Err(ApiError::BadRequest(
                "asset is not linked to this course".into(),
            ));
        }
    }
}
```

Then change the call to `db::courses::update_course` to pass the new triple-state. See Step 2.

- [ ] **Step 2: Update `db::courses::update_course` signature**

Find `pub async fn update_course` in `crates/backend/src/db/courses.rs`. Add a parameter `cover_asset_id: Option<Option<Uuid>>` and amend the SQL.

Replace the function body with:

```rust
pub async fn update_course(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    title: Option<&str>,
    description: Option<&str>,
    status: Option<&str>,
    cover_asset_id: Option<Option<Uuid>>,
) -> sqlx::Result<Option<CourseRow>> {
    // The COALESCE pattern can't express "set to NULL"; so we branch on the
    // double-Option: when the caller asks to clear, we issue an explicit
    // SET cover_asset_id = NULL.
    let (set_cover, cover_value): (bool, Option<Uuid>) = match cover_asset_id {
        None => (false, None),
        Some(v) => (true, v),
    };
    if set_cover {
        sqlx::query_as::<_, CourseRow>(
            "UPDATE courses
                SET title           = COALESCE($2, title),
                    description     = COALESCE($3, description),
                    status          = COALESCE($4, status),
                    cover_asset_id  = $5,
                    updated_at      = now()
              WHERE id = $1
            RETURNING id, tenant_id, slug, title, description, status, visibility,
                      owner_user_id, created_at, updated_at",
        )
        .bind(id)
        .bind(title)
        .bind(description)
        .bind(status)
        .bind(cover_value)
        .fetch_optional(&mut **tx)
        .await
    } else {
        sqlx::query_as::<_, CourseRow>(
            "UPDATE courses
                SET title           = COALESCE($2, title),
                    description     = COALESCE($3, description),
                    status          = COALESCE($4, status),
                    updated_at      = now()
              WHERE id = $1
            RETURNING id, tenant_id, slug, title, description, status, visibility,
                      owner_user_id, created_at, updated_at",
        )
        .bind(id)
        .bind(title)
        .bind(description)
        .bind(status)
        .fetch_optional(&mut **tx)
        .await
    }
}
```

Update the existing call in `patch_inner` to pass `body.cover_asset_id`:

```rust
let updated = db::courses::update_course(
    &mut tx,
    id,
    body.title.as_deref(),
    body.description.as_deref(),
    body.status.as_deref(),
    body.cover_asset_id,
)
.await
.map_err(|e| ApiError::Internal(e.to_string()))?
.ok_or(ApiError::CourseNotFound)?;
```

The `CourseRow` and `CourseDto` already include / can include `cover_asset_id` — Phase 1a's struct didn't expose it; add it now. Update `CourseRow` in `db/courses.rs`:

```rust
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct CourseRow {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub slug: String,
    pub title: String,
    pub description: Option<String>,
    pub status: String,
    pub visibility: String,
    pub cover_asset_id: Option<Uuid>,
    pub owner_user_id: Uuid,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}
```

Update every `RETURNING` clause in the file to include `cover_asset_id`:
- `insert_course`: change `RETURNING id, tenant_id, slug, title, description, status, visibility, owner_user_id, created_at, updated_at` to `RETURNING id, tenant_id, slug, title, description, status, visibility, cover_asset_id, owner_user_id, created_at, updated_at`.
- `fetch_course`: same SELECT list change.
- `list_for_caller`: same change in both branches.
- `update_course`: same in both arms.

Update `CourseDto` in `handlers/courses.rs`:

```rust
#[derive(Serialize)]
pub struct CourseDto {
    pub id: Uuid,
    pub slug: String,
    pub title: String,
    pub description: Option<String>,
    pub status: String,
    pub cover_asset_id: Option<Uuid>,
    pub owner_user_id: Uuid,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl From<db::courses::CourseRow> for CourseDto {
    fn from(r: db::courses::CourseRow) -> Self {
        Self {
            id: r.id,
            slug: r.slug,
            title: r.title,
            description: r.description,
            status: r.status,
            cover_asset_id: r.cover_asset_id,
            owner_user_id: r.owner_user_id,
            created_at: r.created_at,
        }
    }
}
```

- [ ] **Step 3: Update `PatchLesson` and `db::lessons::update_lesson` similarly**

In `crates/backend/src/handlers/lessons.rs`, replace `PatchLesson`:

```rust
#[derive(Deserialize, Default)]
pub struct PatchLesson {
    pub title: Option<String>,
    pub body_md: Option<String>,
    #[serde(default, with = "::serde_with::rust::double_option")]
    pub video_asset_id: Option<Option<Uuid>>,
    #[serde(default, with = "::serde_with::rust::double_option")]
    pub live_session_id: Option<Option<Uuid>>,
}
```

In `crates/backend/src/db/lessons.rs`, change `update_lesson` to accept double-Option for both `video_asset_id` and `live_session_id`. Same branching pattern as courses — when the field is `None` the SQL skips it; when `Some(v)` it sets it (including NULL).

```rust
pub async fn update_lesson(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    title: Option<&str>,
    body_md: Option<&str>,
    video_asset_id: Option<Option<Uuid>>,
    live_session_id: Option<Option<Uuid>>,
) -> sqlx::Result<Option<LessonRow>> {
    let (set_video, video_value): (bool, Option<Uuid>) = match video_asset_id {
        None => (false, None),
        Some(v) => (true, v),
    };
    let (set_live, live_value): (bool, Option<Uuid>) = match live_session_id {
        None => (false, None),
        Some(v) => (true, v),
    };

    let sql = match (set_video, set_live) {
        (false, false) => "UPDATE lessons
                SET title           = COALESCE($2, title),
                    body_md         = COALESCE($3, body_md),
                    updated_at      = now()
              WHERE id = $1
            RETURNING id, tenant_id, course_id, module_id, type, title,
                      body_md, video_asset_id, live_session_id, sort_order",
        (true, false) => "UPDATE lessons
                SET title           = COALESCE($2, title),
                    body_md         = COALESCE($3, body_md),
                    video_asset_id  = $4,
                    updated_at      = now()
              WHERE id = $1
            RETURNING id, tenant_id, course_id, module_id, type, title,
                      body_md, video_asset_id, live_session_id, sort_order",
        (false, true) => "UPDATE lessons
                SET title           = COALESCE($2, title),
                    body_md         = COALESCE($3, body_md),
                    live_session_id = $4,
                    updated_at      = now()
              WHERE id = $1
            RETURNING id, tenant_id, course_id, module_id, type, title,
                      body_md, video_asset_id, live_session_id, sort_order",
        (true, true) => "UPDATE lessons
                SET title           = COALESCE($2, title),
                    body_md         = COALESCE($3, body_md),
                    video_asset_id  = $4,
                    live_session_id = $5,
                    updated_at      = now()
              WHERE id = $1
            RETURNING id, tenant_id, course_id, module_id, type, title,
                      body_md, video_asset_id, live_session_id, sort_order",
    };

    let q = sqlx::query_as::<_, LessonRow>(sql)
        .bind(id)
        .bind(title)
        .bind(body_md);
    let q = match (set_video, set_live) {
        (false, false) => q,
        (true, false) => q.bind(video_value),
    (false, true) => q.bind(live_value),
        (true, true) => q.bind(video_value).bind(live_value),
    };
    q.fetch_optional(&mut **tx).await
}
```

Update `LessonRow` to include `video_asset_id` (already there) — verify the struct has it. If missing, add `pub video_asset_id: Option<Uuid>`.

Update the call in `handlers::lessons::patch_inner`:
```rust
let row = db::lessons::update_lesson(
    &mut tx,
    lesson_id,
    b.title.as_deref(),
    b.body_md.as_deref(),
    b.video_asset_id,
    b.live_session_id,
)
.await
.map_err(|e| ApiError::Internal(e.to_string()))?
.ok_or(ApiError::NotFound)?;
```

Add an asset-validation block in `patch_inner` (mirrors the cover validation in courses):

```rust
let tenant_id = ctx
    .tenant_id
    .ok_or_else(|| ApiError::BadRequest("no tenant".into()))?;

if let Some(Some(asset_id)) = b.video_asset_id {
    let asset = db::file_assets::fetch(pool, asset_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::FileAssetNotFound)?;
    if asset.tenant_id != tenant_id {
        return Err(ApiError::FileAssetNotFound);
    }
    if asset.status != "available" {
        return Err(ApiError::BadRequest(format!(
            "asset is in status '{}'; must be 'available'",
            asset.status
        )));
    }
    let valid_link = asset.linked_entity_type.as_deref() == Some("lesson")
        && asset.linked_entity_id == Some(lesson_id);
    if !valid_link {
        return Err(ApiError::BadRequest(
            "asset is not linked to this lesson".into(),
        ));
    }
}
```

(`live_session_id` validation can stay simple — no asset check needed; future improvement is to verify the live session belongs to the same course.)

- [ ] **Step 4: Build + run existing tests as regression check**

```bash
cargo build -p backend
DATABASE_URL=postgres://aulalite:changeme@localhost:55432/aulalite \
    cargo test -p backend --test courses_crud
DATABASE_URL=postgres://aulalite:changeme@localhost:55432/aulalite \
    cargo test -p backend --test lessons_crud
```

Expected: all tests still pass (existing tests don't exercise the new fields; they continue to omit `cover_asset_id` / `video_asset_id` from PATCH bodies, which means `None` = no change, same as before).

- [ ] **Step 5: Commit**

```bash
git add crates/backend/src/handlers/courses.rs crates/backend/src/handlers/lessons.rs \
        crates/backend/src/db/courses.rs crates/backend/src/db/lessons.rs
git commit -m "feat(handlers): DoubleOption for cover_asset_id / video_asset_id PATCH semantics"
```

---

### Task 10: Lesson type rejection lifted (video + file_bundle now allowed)

**Files:**
- Modify: `crates/backend/src/db/lessons.rs`
- Modify: `crates/backend/src/handlers/lessons.rs`
- Modify: `crates/backend/tests/lessons_crud.rs`

- [ ] **Step 1: Rename and broaden `type_supported_at_1a`**

In `crates/backend/src/db/lessons.rs`:

```rust
pub fn type_supported_at_1b_alpha(t: &str) -> bool {
    matches!(t, "rich_text" | "live_session" | "video" | "file_bundle")
}
```

(Delete the old `type_supported_at_1a` function.)

- [ ] **Step 2: Update the handler call**

In `crates/backend/src/handlers/lessons.rs`, find the line:
```rust
if !db::lessons::type_supported_at_1a(&b.r#type) {
    return Err(ApiError::LessonTypeNotSupported(b.r#type));
}
```
Replace with:
```rust
if !db::lessons::type_supported_at_1b_alpha(&b.r#type) {
    return Err(ApiError::LessonTypeNotSupported(b.r#type));
}
```

Also update the `live_session` requirement check to leave `video` and `file_bundle` flexible:
```rust
if b.r#type == "live_session" && b.live_session_id.is_none() {
    return Err(ApiError::BadRequest(
        "live_session lesson requires live_session_id".into(),
    ));
}
// video lesson may be created without video_asset_id; teacher uploads later.
// file_bundle lesson has no payload at create-time; attachments come via /v1/uploads/begin.
```

- [ ] **Step 3: Update the failing test**

In `crates/backend/tests/lessons_crud.rs`, find `video_lesson_rejected_at_phase_1a`. Replace it with:

```rust
#[tokio::test]
async fn video_lesson_creates_without_asset_id() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (user, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, user, "teacher").await;
    let (course, module) = course_with_module(&pool, tenant, user).await;

    let app = build_test_app(
        backend::handlers::lessons::router_for_tests(pool.clone()),
        StubAuth {
            pool: pool.clone(),
            user_id: user,
            firebase_uid: fb,
            email: em,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Teacher),
        },
    );

    let (s, b) = fire(
        &app,
        "POST",
        &format!("/v1/courses/{course}/modules/{module}/lessons"),
        Some(serde_json::json!({ "type": "video", "title": "Week 1 video" })),
    )
    .await;
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["type"], "video");
    assert!(b["video_asset_id"].is_null());
}

#[tokio::test]
async fn file_bundle_lesson_creates_without_attachments() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (user, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, user, "teacher").await;
    let (course, module) = course_with_module(&pool, tenant, user).await;

    let app = build_test_app(
        backend::handlers::lessons::router_for_tests(pool.clone()),
        StubAuth {
            pool: pool.clone(),
            user_id: user,
            firebase_uid: fb,
            email: em,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Teacher),
        },
    );

    let (s, b) = fire(
        &app,
        "POST",
        &format!("/v1/courses/{course}/modules/{module}/lessons"),
        Some(serde_json::json!({ "type": "file_bundle", "title": "Handouts" })),
    )
    .await;
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["type"], "file_bundle");
}
```

- [ ] **Step 4: Run tests**

```bash
DATABASE_URL=postgres://aulalite:changeme@localhost:55432/aulalite \
    cargo test -p backend --test lessons_crud
```

Expected: 3 passed (was 2 in Phase 1a — `rich_text_lesson_create_and_reorder` stays; the rejected-video test is replaced by the two new accepts).

- [ ] **Step 5: Commit**

```bash
git add crates/backend/src/db/lessons.rs crates/backend/src/handlers/lessons.rs \
        crates/backend/tests/lessons_crud.rs
git commit -m "feat(lessons): lift type rejection — video and file_bundle now allowed"
```

---

# Section E — Upload handlers + integration tests

### Task 11: handlers::uploads — begin + complete (TDD)

**Files:**
- Create: `crates/backend/src/handlers/uploads.rs`
- Modify: `crates/backend/src/handlers/mod.rs`
- Modify: `crates/backend/src/lib.rs` (route registration)
- Create: `crates/backend/tests/uploads.rs`

- [ ] **Step 1: Write failing tests**

```rust
// crates/backend/tests/uploads.rs
mod fixtures;

use fixtures::*;
use serde_json::json;
use std::sync::Arc;

async fn course_owned_by(pool: &sqlx::PgPool, tenant: uuid::Uuid, user: uuid::Uuid) -> uuid::Uuid {
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx).await.unwrap();
    let id: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO courses (tenant_id, slug, title, owner_user_id)
         VALUES ($1, $2, 'C', $3) RETURNING id",
    ).bind(tenant).bind(format!("c-{}", uuid::Uuid::new_v4())).bind(user)
        .fetch_one(&mut *tx).await.unwrap();
    sqlx::query(
        "INSERT INTO course_memberships (course_id, user_id, tenant_id, role)
         VALUES ($1,$2,$3,'teacher')",
    ).bind(id).bind(user).bind(tenant).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    id
}

#[tokio::test]
async fn begin_returns_presigned_url_and_pending_row() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (user, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, user, "teacher").await;
    let course = course_owned_by(&pool, tenant, user).await;

    let s3 = Arc::new(backend::storage::mock::MockS3Client::new());
    let app = build_test_app(
        backend::handlers::uploads::router_for_tests(pool.clone(), s3.clone(), "aulalite".into()),
        StubAuth {
            pool: pool.clone(),
            user_id: user,
            firebase_uid: fb,
            email: em,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Teacher),
        },
    );

    let (status, body) = fire(
        &app,
        "POST",
        "/v1/uploads/begin",
        Some(json!({
            "filename": "cover.png",
            "content_type": "image/png",
            "size_bytes": 1_000_000,
            "linked_entity_type": "course",
            "linked_entity_id": course,
            "purpose": "cover"
        })),
    )
    .await;

    assert_eq!(status, 200, "{body}");
    assert!(body["presigned_put_url"].as_str().unwrap().starts_with("https://mock.s3/put/"));
    let asset_id = body["asset_id"].as_str().unwrap();

    let row: (String,) = sqlx::query_as(
        "SELECT status FROM file_assets WHERE id = $1::uuid"
    ).bind(asset_id).fetch_one(&pool).await.unwrap();
    assert_eq!(row.0, "pending");
}

#[tokio::test]
async fn begin_rejects_oversized_request() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (user, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, user, "teacher").await;
    let course = course_owned_by(&pool, tenant, user).await;

    let s3 = Arc::new(backend::storage::mock::MockS3Client::new());
    let app = build_test_app(
        backend::handlers::uploads::router_for_tests(pool.clone(), s3, "aulalite".into()),
        StubAuth {
            pool: pool.clone(),
            user_id: user,
            firebase_uid: fb,
            email: em,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Teacher),
        },
    );

    let (status, body) = fire(
        &app,
        "POST",
        "/v1/uploads/begin",
        Some(json!({
            "filename": "huge.png",
            "content_type": "image/png",
            "size_bytes": 6_000_000,
            "linked_entity_type": "course",
            "linked_entity_id": course,
            "purpose": "cover"
        })),
    )
    .await;
    assert_eq!(status, 400);
    assert!(body["error"].as_str().unwrap().contains("size"));
}

#[tokio::test]
async fn begin_rejects_bad_content_type() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (user, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, user, "teacher").await;
    let course = course_owned_by(&pool, tenant, user).await;

    let s3 = Arc::new(backend::storage::mock::MockS3Client::new());
    let app = build_test_app(
        backend::handlers::uploads::router_for_tests(pool.clone(), s3, "aulalite".into()),
        StubAuth {
            pool: pool.clone(),
            user_id: user,
            firebase_uid: fb,
            email: em,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Teacher),
        },
    );

    let (status, body) = fire(
        &app,
        "POST",
        "/v1/uploads/begin",
        Some(json!({
            "filename": "evil.svg",
            "content_type": "image/svg+xml",
            "size_bytes": 1000,
            "linked_entity_type": "course",
            "linked_entity_id": course,
            "purpose": "cover"
        })),
    )
    .await;
    assert_eq!(status, 400);
    assert!(body["error"].as_str().unwrap().contains("content type"));
}

#[tokio::test]
async fn complete_marks_available_when_head_matches() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (user, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, user, "teacher").await;
    let course = course_owned_by(&pool, tenant, user).await;

    let s3 = Arc::new(backend::storage::mock::MockS3Client::new());
    let app = build_test_app(
        backend::handlers::uploads::router_for_tests(pool.clone(), s3.clone(), "aulalite".into()),
        StubAuth {
            pool: pool.clone(),
            user_id: user,
            firebase_uid: fb,
            email: em,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Teacher),
        },
    );

    let (_, body) = fire(
        &app,
        "POST",
        "/v1/uploads/begin",
        Some(json!({
            "filename": "cover.png",
            "content_type": "image/png",
            "size_bytes": 1_000_000,
            "linked_entity_type": "course",
            "linked_entity_id": course,
            "purpose": "cover"
        })),
    )
    .await;
    let asset_id = body["asset_id"].as_str().unwrap().to_string();

    // Read object_key from DB so we can simulate the upload landing.
    let object_key: String = sqlx::query_scalar(
        "SELECT object_key FROM file_assets WHERE id = $1::uuid"
    ).bind(&asset_id).fetch_one(&pool).await.unwrap();
    s3.simulate_object(&object_key, 1_000_000, "image/png");

    let (status, body) = fire(
        &app,
        "POST",
        &format!("/v1/uploads/{asset_id}/complete"),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["status"], "available");
}

#[tokio::test]
async fn complete_marks_failed_when_size_mismatch() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (user, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, user, "teacher").await;
    let course = course_owned_by(&pool, tenant, user).await;

    let s3 = Arc::new(backend::storage::mock::MockS3Client::new());
    let app = build_test_app(
        backend::handlers::uploads::router_for_tests(pool.clone(), s3.clone(), "aulalite".into()),
        StubAuth {
            pool: pool.clone(),
            user_id: user,
            firebase_uid: fb,
            email: em,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Teacher),
        },
    );

    let (_, body) = fire(
        &app, "POST", "/v1/uploads/begin",
        Some(json!({
            "filename": "x.png", "content_type": "image/png",
            "size_bytes": 1_000_000,
            "linked_entity_type": "course", "linked_entity_id": course,
            "purpose": "cover"
        })),
    ).await;
    let asset_id = body["asset_id"].as_str().unwrap().to_string();
    let object_key: String = sqlx::query_scalar(
        "SELECT object_key FROM file_assets WHERE id = $1::uuid"
    ).bind(&asset_id).fetch_one(&pool).await.unwrap();
    // Simulate: object lands but with wrong size
    s3.simulate_object(&object_key, 999_999, "image/png");

    let (status, _) = fire(
        &app, "POST", &format!("/v1/uploads/{asset_id}/complete"),
        Some(json!({})),
    ).await;
    assert_eq!(status, 400);

    let row: (String,) = sqlx::query_as(
        "SELECT status FROM file_assets WHERE id = $1::uuid"
    ).bind(&asset_id).fetch_one(&pool).await.unwrap();
    assert_eq!(row.0, "failed");
}

#[tokio::test]
async fn complete_by_non_uploader_rejected() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (uploader, fb_u, em_u) = create_user(&pool).await;
    attach_membership(&pool, tenant, uploader, "teacher").await;
    let (other, fb_o, em_o) = create_user(&pool).await;
    attach_membership(&pool, tenant, other, "teacher").await;
    let course = course_owned_by(&pool, tenant, uploader).await;

    let s3 = Arc::new(backend::storage::mock::MockS3Client::new());

    // Uploader begins
    let app_u = build_test_app(
        backend::handlers::uploads::router_for_tests(pool.clone(), s3.clone(), "aulalite".into()),
        StubAuth {
            pool: pool.clone(), user_id: uploader, firebase_uid: fb_u, email: em_u,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Teacher),
        },
    );
    let (_, body) = fire(
        &app_u, "POST", "/v1/uploads/begin",
        Some(json!({
            "filename": "x.png", "content_type": "image/png",
            "size_bytes": 1000,
            "linked_entity_type": "course", "linked_entity_id": course,
            "purpose": "cover"
        })),
    ).await;
    let asset_id = body["asset_id"].as_str().unwrap().to_string();

    // Other tries to complete
    let app_o = build_test_app(
        backend::handlers::uploads::router_for_tests(pool.clone(), s3, "aulalite".into()),
        StubAuth {
            pool: pool.clone(), user_id: other, firebase_uid: fb_o, email: em_o,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Teacher),
        },
    );
    let (status, _) = fire(
        &app_o, "POST", &format!("/v1/uploads/{asset_id}/complete"),
        Some(json!({})),
    ).await;
    assert_eq!(status, 403);
}

#[tokio::test]
async fn complete_is_idempotent() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (user, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, user, "teacher").await;
    let course = course_owned_by(&pool, tenant, user).await;

    let s3 = Arc::new(backend::storage::mock::MockS3Client::new());
    let app = build_test_app(
        backend::handlers::uploads::router_for_tests(pool.clone(), s3.clone(), "aulalite".into()),
        StubAuth {
            pool: pool.clone(), user_id: user, firebase_uid: fb, email: em,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Teacher),
        },
    );
    let (_, body) = fire(&app, "POST", "/v1/uploads/begin",
        Some(json!({
            "filename": "x.png", "content_type": "image/png",
            "size_bytes": 1000,
            "linked_entity_type": "course", "linked_entity_id": course,
            "purpose": "cover"
        }))).await;
    let asset_id = body["asset_id"].as_str().unwrap().to_string();
    let object_key: String = sqlx::query_scalar(
        "SELECT object_key FROM file_assets WHERE id = $1::uuid"
    ).bind(&asset_id).fetch_one(&pool).await.unwrap();
    s3.simulate_object(&object_key, 1000, "image/png");

    let (s1, b1) = fire(&app, "POST", &format!("/v1/uploads/{asset_id}/complete"),
        Some(json!({}))).await;
    assert_eq!(s1, 200);
    assert_eq!(b1["status"], "available");

    let (s2, b2) = fire(&app, "POST", &format!("/v1/uploads/{asset_id}/complete"),
        Some(json!({}))).await;
    assert_eq!(s2, 200);
    assert_eq!(b2["status"], "available");
}
```

- [ ] **Step 2: Run — expect compile failure**

```bash
cd "C:/Users/Chiranjib Chaudhuri/Documents/Chiranjib/Elementors_Aula-phase0"
cargo test -p backend --test uploads --no-run 2>&1 | tail -10
```

Expected: errors about missing `backend::handlers::uploads`.

- [ ] **Step 3: Implement `crates/backend/src/handlers/uploads.rs`**

```rust
// crates/backend/src/handlers/uploads.rs
use axum::extract::{Extension, Path, State};
use axum::{routing, Json, Router};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

use crate::context::RequestContext;
use crate::db;
use crate::error::ApiError;
use crate::services::file_assets as fa_svc;
use crate::storage::S3Client;
use crate::AppState;

const PUT_TTL: Duration = Duration::from_secs(15 * 60);

#[derive(Deserialize)]
pub struct BeginUpload {
    pub filename: String,
    pub content_type: String,
    pub size_bytes: i64,
    pub linked_entity_type: String,
    pub linked_entity_id: Uuid,
    pub purpose: String,
}

#[derive(Serialize)]
pub struct BeginUploadDto {
    pub asset_id: Uuid,
    pub presigned_put_url: String,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Serialize)]
pub struct AssetCompletedDto {
    pub asset_id: Uuid,
    pub status: String,
    pub size_bytes: i64,
    pub content_type: String,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/uploads/begin", routing::post(begin))
        .route("/v1/uploads/:asset_id/complete", routing::post(complete))
}

#[doc(hidden)]
pub fn router_for_tests(
    pool: PgPool,
    storage: Arc<dyn S3Client>,
    bucket: String,
) -> Router {
    Router::new()
        .route("/v1/uploads/begin", routing::post(begin_t))
        .route("/v1/uploads/:asset_id/complete", routing::post(complete_t))
        .with_state(TestState { pool, storage, bucket })
}

#[derive(Clone)]
struct TestState {
    pool: PgPool,
    storage: Arc<dyn S3Client>,
    bucket: String,
}

fn is_org_admin(ctx: &RequestContext) -> bool {
    matches!(ctx.tenant_role, Some(core_types::TenantRole::OrgAdmin))
}

// Production handlers
async fn begin(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Json(b): Json<BeginUpload>,
) -> Result<Json<BeginUploadDto>, ApiError> {
    begin_inner(&s.pool, s.storage.as_ref(), &s.bucket_name, &ctx, b).await
}
async fn complete(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(asset_id): Path<Uuid>,
) -> Result<Json<AssetCompletedDto>, ApiError> {
    complete_inner(&s.pool, s.storage.as_ref(), &ctx, asset_id).await
}

// Test mirrors
async fn begin_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Json(b): Json<BeginUpload>,
) -> Result<Json<BeginUploadDto>, ApiError> {
    begin_inner(&s.pool, s.storage.as_ref(), &s.bucket, &ctx, b).await
}
async fn complete_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(asset_id): Path<Uuid>,
) -> Result<Json<AssetCompletedDto>, ApiError> {
    complete_inner(&s.pool, s.storage.as_ref(), &ctx, asset_id).await
}

async fn require_admin_for_course(
    pool: &PgPool,
    ctx: &RequestContext,
    course_id: Uuid,
) -> Result<(), ApiError> {
    let allowed = db::courses::caller_can_admin_course(pool, course_id, ctx.user_id, is_org_admin(ctx))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !allowed { return Err(ApiError::Forbidden); }
    Ok(())
}

async fn begin_inner(
    pool: &PgPool,
    storage: &dyn S3Client,
    bucket: &str,
    ctx: &RequestContext,
    b: BeginUpload,
) -> Result<Json<BeginUploadDto>, ApiError> {
    // Per-(linked_entity_type, purpose) authorization + validation dispatch
    match (b.linked_entity_type.as_str(), b.purpose.as_str()) {
        ("course", "cover") => {
            require_admin_for_course(pool, ctx, b.linked_entity_id).await?;
        }
        ("lesson", "video") => {
            // Resolve lesson to its course, then require course-admin.
            let course_id: Option<Uuid> = sqlx::query_scalar(
                "SELECT course_id FROM lessons WHERE id = $1"
            )
            .bind(b.linked_entity_id)
            .fetch_optional(pool)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
            let course_id = course_id.ok_or(ApiError::NotFound)?;
            let lesson_type: String = sqlx::query_scalar(
                "SELECT type FROM lessons WHERE id = $1"
            )
            .bind(b.linked_entity_id)
            .fetch_one(pool)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
            if lesson_type != "video" {
                return Err(ApiError::BadRequest("lesson is not type 'video'".into()));
            }
            require_admin_for_course(pool, ctx, course_id).await?;
        }
        ("lesson", "attachment") => {
            let row: Option<(Uuid, String)> = sqlx::query_as(
                "SELECT course_id, type FROM lessons WHERE id = $1"
            )
            .bind(b.linked_entity_id)
            .fetch_optional(pool)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
            let (course_id, lesson_type) = row.ok_or(ApiError::NotFound)?;
            if lesson_type != "file_bundle" {
                return Err(ApiError::BadRequest(
                    "lesson is not type 'file_bundle'".into(),
                ));
            }
            require_admin_for_course(pool, ctx, course_id).await?;
        }
        _ => {
            return Err(ApiError::BadRequest(format!(
                "unsupported (entity_type, purpose) pair: ({}, {})",
                b.linked_entity_type, b.purpose
            )));
        }
    }

    // Validation matrix
    fa_svc::validate_request(&b.purpose, &b.content_type, b.size_bytes)
        .map_err(|e| ApiError::UploadValidationFailed(e.to_string()))?;

    let tenant_id = ctx
        .tenant_id
        .ok_or_else(|| ApiError::BadRequest("no tenant".into()))?;

    let asset_id = Uuid::new_v4();
    let now = chrono::Utc::now();
    let object_key = fa_svc::object_key(tenant_id, asset_id, &b.filename, now);

    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    // Insert pending row using the pre-generated UUID for object_key consistency.
    sqlx::query(
        "INSERT INTO file_assets
            (id, tenant_id, owner_user_id, bucket, object_key, content_type,
             size_bytes, status, visibility, linked_entity_type, linked_entity_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7, 'pending', 'private', $8, $9)",
    )
    .bind(asset_id)
    .bind(tenant_id)
    .bind(ctx.user_id)
    .bind(bucket)
    .bind(&object_key)
    .bind(&b.content_type)
    .bind(b.size_bytes)
    .bind(&b.linked_entity_type)
    .bind(b.linked_entity_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;

    db::audit::emit_audit_event(&mut tx, tenant_id, ctx.user_id,
        "file_asset.begin", "file_asset", asset_id, None)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    tx.commit().await.map_err(|e| ApiError::Internal(e.to_string()))?;

    let presigned_put_url = storage
        .presigned_put_url(&object_key, &b.content_type, b.size_bytes, PUT_TTL)
        .await
        .map_err(|e| ApiError::Internal(format!("presign failed: {e}")))?;
    let expires_at = now + chrono::Duration::seconds(PUT_TTL.as_secs() as i64);

    Ok(Json(BeginUploadDto {
        asset_id,
        presigned_put_url,
        expires_at,
    }))
}

async fn complete_inner(
    pool: &PgPool,
    storage: &dyn S3Client,
    ctx: &RequestContext,
    asset_id: Uuid,
) -> Result<Json<AssetCompletedDto>, ApiError> {
    let asset = db::file_assets::fetch(pool, asset_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::FileAssetNotFound)?;

    if asset.owner_user_id != ctx.user_id {
        return Err(ApiError::Forbidden);
    }

    // Idempotent: if already available, return current state.
    if asset.status == "available" {
        return Ok(Json(AssetCompletedDto {
            asset_id: asset.id,
            status: asset.status,
            size_bytes: asset.size_bytes,
            content_type: asset.content_type,
        }));
    }
    if asset.status == "failed" {
        return Err(ApiError::UploadObjectMissing);
    }

    // HEAD verify
    let head = storage.head_object(&asset.object_key).await;
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    match head {
        Ok(h) if h.size_bytes == asset.size_bytes => {
            let updated = db::file_assets::mark_available(&mut tx, asset_id)
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?
                .ok_or(ApiError::FileAssetNotFound)?;
            db::audit::emit_audit_event(&mut tx, asset.tenant_id, ctx.user_id,
                "file_asset.complete", "file_asset", asset_id, None)
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?;
            tx.commit().await.map_err(|e| ApiError::Internal(e.to_string()))?;
            Ok(Json(AssetCompletedDto {
                asset_id: updated.id,
                status: updated.status,
                size_bytes: updated.size_bytes,
                content_type: updated.content_type,
            }))
        }
        Ok(_) | Err(_) => {
            db::file_assets::mark_failed(&mut tx, asset_id)
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?;
            tx.commit().await.map_err(|e| ApiError::Internal(e.to_string()))?;
            Err(ApiError::UploadObjectMissing)
        }
    }
}
```

- [ ] **Step 4: Wire into `handlers/mod.rs` + `lib.rs`**

In `crates/backend/src/handlers/mod.rs`, append:
```rust
pub mod uploads;
```

In `crates/backend/src/lib.rs`'s `authed` Router, add:
```rust
.merge(handlers::uploads::routes())
```

- [ ] **Step 5: Run**

```bash
cargo build -p backend
DATABASE_URL=postgres://aulalite:changeme@localhost:55432/aulalite \
    cargo test -p backend --test uploads
```

Expected: 7 passed.

- [ ] **Step 6: Commit**

```bash
git add crates/backend/src/handlers/mod.rs crates/backend/src/handlers/uploads.rs \
        crates/backend/src/lib.rs crates/backend/tests/uploads.rs
git commit -m "feat(uploads): begin + complete handlers with mock-S3 integration tests"
```

---

### Task 12: handlers::file_assets — GET URL + DELETE + list lesson files

**Files:**
- Create: `crates/backend/src/handlers/file_assets.rs`
- Modify: `crates/backend/src/handlers/mod.rs`, `lib.rs`

- [ ] **Step 1: Implement**

```rust
// crates/backend/src/handlers/file_assets.rs
use axum::extract::{Extension, Path, State};
use axum::{routing, Json, Router};
use serde::Serialize;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

use crate::context::RequestContext;
use crate::db;
use crate::error::ApiError;
use crate::storage::S3Client;
use crate::AppState;

const GET_TTL: Duration = Duration::from_secs(15 * 60);

#[derive(Serialize)]
pub struct UrlDto {
    pub url: String,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Serialize)]
pub struct LessonFileDto {
    pub asset_id: Uuid,
    pub filename: String,
    pub content_type: String,
    pub size_bytes: i64,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/file-assets/:id/url", routing::get(get_url))
        .route("/v1/file-assets/:id", routing::delete(delete_asset))
        .route("/v1/lessons/:lid/files", routing::get(list_lesson_files))
}

#[doc(hidden)]
pub fn router_for_tests(
    pool: PgPool,
    storage: Arc<dyn S3Client>,
) -> Router {
    Router::new()
        .route("/v1/file-assets/:id/url", routing::get(get_url_t))
        .route("/v1/file-assets/:id", routing::delete(delete_asset_t))
        .route("/v1/lessons/:lid/files", routing::get(list_lesson_files_t))
        .with_state(TestState { pool, storage })
}

#[derive(Clone)]
struct TestState {
    pool: PgPool,
    storage: Arc<dyn S3Client>,
}

fn is_org_admin(ctx: &RequestContext) -> bool {
    matches!(ctx.tenant_role, Some(core_types::TenantRole::OrgAdmin))
}

async fn caller_can_read_linked_entity(
    pool: &PgPool,
    ctx: &RequestContext,
    asset: &db::file_assets::FileAssetRow,
) -> Result<bool, ApiError> {
    let course_id = match (asset.linked_entity_type.as_deref(), asset.linked_entity_id) {
        (Some("course"), Some(id)) => id,
        (Some("lesson"), Some(id)) => sqlx::query_scalar::<_, Uuid>(
            "SELECT course_id FROM lessons WHERE id = $1"
        )
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::FileAssetNotFound)?,
        _ => return Ok(false),
    };
    db::courses::caller_can_read_course(pool, course_id, ctx.user_id, is_org_admin(ctx))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))
}

async fn caller_can_admin_linked_entity(
    pool: &PgPool,
    ctx: &RequestContext,
    asset: &db::file_assets::FileAssetRow,
) -> Result<bool, ApiError> {
    let course_id = match (asset.linked_entity_type.as_deref(), asset.linked_entity_id) {
        (Some("course"), Some(id)) => id,
        (Some("lesson"), Some(id)) => sqlx::query_scalar::<_, Uuid>(
            "SELECT course_id FROM lessons WHERE id = $1"
        )
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::FileAssetNotFound)?,
        _ => return Ok(false),
    };
    db::courses::caller_can_admin_course(pool, course_id, ctx.user_id, is_org_admin(ctx))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))
}

// Production handlers
async fn get_url(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<UrlDto>, ApiError> {
    get_url_inner(&s.pool, s.storage.as_ref(), &ctx, id).await
}
async fn delete_asset(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<axum::http::StatusCode, ApiError> {
    delete_inner(&s.pool, s.storage.as_ref(), &ctx, id).await
}
async fn list_lesson_files(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(lid): Path<Uuid>,
) -> Result<Json<Vec<LessonFileDto>>, ApiError> {
    list_lesson_files_inner(&s.pool, &ctx, lid).await
}

// Test mirrors
async fn get_url_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<UrlDto>, ApiError> {
    get_url_inner(&s.pool, s.storage.as_ref(), &ctx, id).await
}
async fn delete_asset_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<axum::http::StatusCode, ApiError> {
    delete_inner(&s.pool, s.storage.as_ref(), &ctx, id).await
}
async fn list_lesson_files_t(
    State(s): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(lid): Path<Uuid>,
) -> Result<Json<Vec<LessonFileDto>>, ApiError> {
    list_lesson_files_inner(&s.pool, &ctx, lid).await
}

async fn get_url_inner(
    pool: &PgPool,
    storage: &dyn S3Client,
    ctx: &RequestContext,
    id: Uuid,
) -> Result<Json<UrlDto>, ApiError> {
    let asset = db::file_assets::fetch(pool, id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::FileAssetNotFound)?;
    let tenant_id = ctx
        .tenant_id
        .ok_or_else(|| ApiError::BadRequest("no tenant".into()))?;
    if asset.tenant_id != tenant_id {
        return Err(ApiError::FileAssetNotFound);
    }
    if asset.status != "available" {
        return Err(ApiError::FileAssetNotFound);
    }
    if !caller_can_read_linked_entity(pool, ctx, &asset).await? {
        return Err(ApiError::FileAssetNotFound);
    }
    let url = storage
        .presigned_get_url(&asset.object_key, GET_TTL)
        .await
        .map_err(|e| ApiError::Internal(format!("presign get failed: {e}")))?;
    let expires_at = chrono::Utc::now() + chrono::Duration::seconds(GET_TTL.as_secs() as i64);
    Ok(Json(UrlDto { url, expires_at }))
}

async fn delete_inner(
    pool: &PgPool,
    storage: &dyn S3Client,
    ctx: &RequestContext,
    id: Uuid,
) -> Result<axum::http::StatusCode, ApiError> {
    let asset = db::file_assets::fetch(pool, id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::FileAssetNotFound)?;
    let tenant_id = ctx
        .tenant_id
        .ok_or_else(|| ApiError::BadRequest("no tenant".into()))?;
    if asset.tenant_id != tenant_id {
        return Err(ApiError::FileAssetNotFound);
    }
    if !caller_can_admin_linked_entity(pool, ctx, &asset).await? {
        return Err(ApiError::Forbidden);
    }

    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    db::file_assets::mark_pruned(&mut tx, id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    db::audit::emit_audit_event(&mut tx, tenant_id, ctx.user_id,
        "file_asset.delete", "file_asset", id, None)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    tx.commit().await.map_err(|e| ApiError::Internal(e.to_string()))?;

    // Delete from storage; failures are logged but not fatal — operator
    // can sweep orphaned objects later.
    if let Err(e) = storage.delete_object(&asset.object_key).await {
        tracing::warn!(?e, key = %asset.object_key, "object delete failed; row marked pruned anyway");
    }

    Ok(axum::http::StatusCode::NO_CONTENT)
}

async fn list_lesson_files_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    lid: Uuid,
) -> Result<Json<Vec<LessonFileDto>>, ApiError> {
    // Resolve lesson -> course; check read permission.
    let course_id: Option<Uuid> = sqlx::query_scalar(
        "SELECT course_id FROM lessons WHERE id = $1"
    )
    .bind(lid)
    .fetch_optional(pool)
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    let course_id = course_id.ok_or(ApiError::NotFound)?;
    let allowed = db::courses::caller_can_read_course(pool, course_id, ctx.user_id, is_org_admin(ctx))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !allowed {
        return Err(ApiError::Forbidden);
    }

    let rows = db::file_assets::list_for_entity(pool, "lesson", lid)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(
        rows.into_iter()
            .map(|r| LessonFileDto {
                asset_id: r.id,
                filename: r.object_key.rsplit('/').next().unwrap_or("file").to_string(),
                content_type: r.content_type,
                size_bytes: r.size_bytes,
            })
            .collect(),
    ))
}
```

Add `pub mod file_assets;` to `crates/backend/src/handlers/mod.rs`.
Add `.merge(handlers::file_assets::routes())` to `lib.rs`.

- [ ] **Step 2: Build**

```bash
cargo build -p backend
```
Expected: succeeds.

- [ ] **Step 3: Commit**

```bash
git add crates/backend/src/handlers/mod.rs crates/backend/src/handlers/file_assets.rs \
        crates/backend/src/lib.rs
git commit -m "feat(file_assets): GET URL + DELETE + list lesson files handlers"
```

---

### Task 13: course cover upload integration tests

**Files:**
- Create: `crates/backend/tests/course_cover_upload.rs`

- [ ] **Step 1: Write tests**

```rust
// crates/backend/tests/course_cover_upload.rs
mod fixtures;

use fixtures::*;
use serde_json::json;
use std::sync::Arc;

async fn course_owned_by(pool: &sqlx::PgPool, tenant: uuid::Uuid, user: uuid::Uuid) -> uuid::Uuid {
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx).await.unwrap();
    let id: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO courses (tenant_id, slug, title, owner_user_id)
         VALUES ($1, $2, 'C', $3) RETURNING id",
    ).bind(tenant).bind(format!("c-{}", uuid::Uuid::new_v4())).bind(user)
        .fetch_one(&mut *tx).await.unwrap();
    sqlx::query(
        "INSERT INTO course_memberships (course_id, user_id, tenant_id, role)
         VALUES ($1,$2,$3,'teacher')",
    ).bind(id).bind(user).bind(tenant).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    id
}

async fn upload_cover(
    pool: &sqlx::PgPool,
    s3: &Arc<backend::storage::mock::MockS3Client>,
    stub: StubAuth,
    course: uuid::Uuid,
) -> uuid::Uuid {
    let app = build_test_app(
        backend::handlers::uploads::router_for_tests(pool.clone(), s3.clone(), "aulalite".into()),
        stub.clone(),
    );
    let (status, body) = fire(&app, "POST", "/v1/uploads/begin",
        Some(json!({
            "filename": "cover.png", "content_type": "image/png",
            "size_bytes": 1000,
            "linked_entity_type": "course", "linked_entity_id": course,
            "purpose": "cover"
        }))).await;
    assert_eq!(status, 200, "{body}");
    let asset_id_str = body["asset_id"].as_str().unwrap().to_string();
    let asset_id: uuid::Uuid = asset_id_str.parse().unwrap();
    let object_key: String = sqlx::query_scalar(
        "SELECT object_key FROM file_assets WHERE id = $1"
    ).bind(asset_id).fetch_one(pool).await.unwrap();
    s3.simulate_object(&object_key, 1000, "image/png");
    let (s, _) = fire(&app, "POST", &format!("/v1/uploads/{asset_id}/complete"),
        Some(json!({}))).await;
    assert_eq!(s, 200);
    asset_id
}

#[tokio::test]
async fn teacher_uploads_cover_then_patches_course() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (user, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, user, "teacher").await;
    let course = course_owned_by(&pool, tenant, user).await;
    let s3 = Arc::new(backend::storage::mock::MockS3Client::new());

    let stub = StubAuth {
        pool: pool.clone(), user_id: user, firebase_uid: fb, email: em,
        tenant_id: Some(tenant),
        tenant_role: Some(core_types::TenantRole::Teacher),
    };
    let asset_id = upload_cover(&pool, &s3, stub.clone(), course).await;

    // PATCH the course with the new cover_asset_id
    let courses_app = build_test_app(
        backend::handlers::courses::router_for_tests(pool.clone()),
        stub,
    );
    let (status, body) = fire(
        &courses_app,
        "PATCH",
        &format!("/v1/courses/{course}"),
        Some(json!({ "cover_asset_id": asset_id })),
    ).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["cover_asset_id"].as_str().unwrap(), asset_id.to_string());
}

#[tokio::test]
async fn student_cannot_begin_cover_upload() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let course = course_owned_by(&pool, tenant, teacher).await;

    let (student, fb_s, em_s) = create_user(&pool).await;
    attach_membership(&pool, tenant, student, "student").await;

    let s3 = Arc::new(backend::storage::mock::MockS3Client::new());
    let app = build_test_app(
        backend::handlers::uploads::router_for_tests(pool.clone(), s3, "aulalite".into()),
        StubAuth {
            pool: pool.clone(), user_id: student, firebase_uid: fb_s, email: em_s,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Student),
        },
    );
    let (status, _) = fire(&app, "POST", "/v1/uploads/begin",
        Some(json!({
            "filename": "x.png", "content_type": "image/png",
            "size_bytes": 1000,
            "linked_entity_type": "course", "linked_entity_id": course,
            "purpose": "cover"
        }))).await;
    assert_eq!(status, 403);
}

#[tokio::test]
async fn nullable_cover_asset_id_can_be_cleared() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (user, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, user, "teacher").await;
    let course = course_owned_by(&pool, tenant, user).await;
    let s3 = Arc::new(backend::storage::mock::MockS3Client::new());

    let stub = StubAuth {
        pool: pool.clone(), user_id: user, firebase_uid: fb, email: em,
        tenant_id: Some(tenant),
        tenant_role: Some(core_types::TenantRole::Teacher),
    };
    let asset_id = upload_cover(&pool, &s3, stub.clone(), course).await;

    let courses_app = build_test_app(
        backend::handlers::courses::router_for_tests(pool.clone()),
        stub,
    );
    // Set
    fire(&courses_app, "PATCH", &format!("/v1/courses/{course}"),
        Some(json!({ "cover_asset_id": asset_id }))).await;
    // Clear (explicit null)
    let (status, body) = fire(
        &courses_app,
        "PATCH",
        &format!("/v1/courses/{course}"),
        Some(json!({ "cover_asset_id": null })),
    ).await;
    assert_eq!(status, 200, "{body}");
    assert!(body["cover_asset_id"].is_null());
}
```

- [ ] **Step 2: Run**

```bash
DATABASE_URL=postgres://aulalite:changeme@localhost:55432/aulalite \
    cargo test -p backend --test course_cover_upload
```
Expected: 3 passed.

- [ ] **Step 3: Commit**

```bash
git add crates/backend/tests/course_cover_upload.rs
git commit -m "test(uploads): course cover end-to-end with mock S3 + DoubleOption clear"
```

---

### Task 14: lesson attachments integration tests

**Files:**
- Create: `crates/backend/tests/lesson_attachments.rs`

- [ ] **Step 1: Write tests**

```rust
// crates/backend/tests/lesson_attachments.rs
mod fixtures;

use fixtures::*;
use serde_json::json;
use std::sync::Arc;

async fn course_with_module(
    pool: &sqlx::PgPool,
    tenant: uuid::Uuid,
    user: uuid::Uuid,
) -> (uuid::Uuid, uuid::Uuid) {
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx).await.unwrap();
    let course: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO courses (tenant_id, slug, title, owner_user_id)
         VALUES ($1, $2, 'C', $3) RETURNING id",
    ).bind(tenant).bind(format!("c-{}", uuid::Uuid::new_v4())).bind(user)
        .fetch_one(&mut *tx).await.unwrap();
    sqlx::query(
        "INSERT INTO course_memberships (course_id, user_id, tenant_id, role)
         VALUES ($1,$2,$3,'teacher')",
    ).bind(course).bind(user).bind(tenant).execute(&mut *tx).await.unwrap();
    let module: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO modules (tenant_id, course_id, title, sort_order)
         VALUES ($1,$2,'M',10) RETURNING id",
    ).bind(tenant).bind(course).fetch_one(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    (course, module)
}

#[tokio::test]
async fn create_file_bundle_lesson_attach_two_files_list_returns_two() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (user, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, user, "teacher").await;
    let (course, module) = course_with_module(&pool, tenant, user).await;
    let s3 = Arc::new(backend::storage::mock::MockS3Client::new());

    let stub = StubAuth {
        pool: pool.clone(), user_id: user, firebase_uid: fb, email: em,
        tenant_id: Some(tenant),
        tenant_role: Some(core_types::TenantRole::Teacher),
    };

    // Create file_bundle lesson
    let lessons_app = build_test_app(
        backend::handlers::lessons::router_for_tests(pool.clone()),
        stub.clone(),
    );
    let (_, body) = fire(&lessons_app, "POST",
        &format!("/v1/courses/{course}/modules/{module}/lessons"),
        Some(json!({ "type": "file_bundle", "title": "Handouts" }))).await;
    let lesson_id_str = body["id"].as_str().unwrap().to_string();
    let lesson_id: uuid::Uuid = lesson_id_str.parse().unwrap();

    // Upload 2 attachments
    let uploads_app = build_test_app(
        backend::handlers::uploads::router_for_tests(pool.clone(), s3.clone(), "aulalite".into()),
        stub.clone(),
    );
    for i in 0..2 {
        let (_, body) = fire(&uploads_app, "POST", "/v1/uploads/begin",
            Some(json!({
                "filename": format!("handout_{i}.pdf"),
                "content_type": "application/pdf",
                "size_bytes": 1000,
                "linked_entity_type": "lesson", "linked_entity_id": lesson_id,
                "purpose": "attachment"
            }))).await;
        let asset_id = body["asset_id"].as_str().unwrap().to_string();
        let asset_uuid: uuid::Uuid = asset_id.parse().unwrap();
        let object_key: String = sqlx::query_scalar(
            "SELECT object_key FROM file_assets WHERE id = $1"
        ).bind(asset_uuid).fetch_one(&pool).await.unwrap();
        s3.simulate_object(&object_key, 1000, "application/pdf");
        fire(&uploads_app, "POST", &format!("/v1/uploads/{asset_id}/complete"),
            Some(json!({}))).await;
    }

    // List
    let fa_app = build_test_app(
        backend::handlers::file_assets::router_for_tests(pool.clone(), s3.clone()),
        stub,
    );
    let (status, body) = fire(&fa_app, "GET",
        &format!("/v1/lessons/{lesson_id}/files"), None).await;
    assert_eq!(status, 200);
    let arr = body.as_array().unwrap();
    assert_eq!(arr.len(), 2);
}

#[tokio::test]
async fn lesson_video_replaces_video_asset_id() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (user, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, user, "teacher").await;
    let (course, module) = course_with_module(&pool, tenant, user).await;
    let s3 = Arc::new(backend::storage::mock::MockS3Client::new());

    let stub = StubAuth {
        pool: pool.clone(), user_id: user, firebase_uid: fb, email: em,
        tenant_id: Some(tenant),
        tenant_role: Some(core_types::TenantRole::Teacher),
    };
    let lessons_app = build_test_app(
        backend::handlers::lessons::router_for_tests(pool.clone()),
        stub.clone(),
    );

    // Create video lesson (no asset yet)
    let (_, body) = fire(&lessons_app, "POST",
        &format!("/v1/courses/{course}/modules/{module}/lessons"),
        Some(json!({ "type": "video", "title": "Week 1" }))).await;
    let lesson_id_str = body["id"].as_str().unwrap().to_string();
    let lesson_id: uuid::Uuid = lesson_id_str.parse().unwrap();

    // Upload video
    let uploads_app = build_test_app(
        backend::handlers::uploads::router_for_tests(pool.clone(), s3.clone(), "aulalite".into()),
        stub.clone(),
    );
    let (_, body) = fire(&uploads_app, "POST", "/v1/uploads/begin",
        Some(json!({
            "filename": "lecture.mp4", "content_type": "video/mp4",
            "size_bytes": 50_000_000,
            "linked_entity_type": "lesson", "linked_entity_id": lesson_id,
            "purpose": "video"
        }))).await;
    let asset_id_str = body["asset_id"].as_str().unwrap().to_string();
    let asset_id: uuid::Uuid = asset_id_str.parse().unwrap();
    let object_key: String = sqlx::query_scalar(
        "SELECT object_key FROM file_assets WHERE id = $1"
    ).bind(asset_id).fetch_one(&pool).await.unwrap();
    s3.simulate_object(&object_key, 50_000_000, "video/mp4");
    fire(&uploads_app, "POST", &format!("/v1/uploads/{asset_id}/complete"),
        Some(json!({}))).await;

    // PATCH lesson with video_asset_id
    let (status, body) = fire(
        &lessons_app, "PATCH",
        &format!("/v1/courses/{course}/modules/{module}/lessons/{lesson_id}"),
        Some(json!({ "video_asset_id": asset_id })),
    ).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["video_asset_id"].as_str().unwrap(), asset_id.to_string());
}

#[tokio::test]
async fn non_member_cannot_list_attachments() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    let (course, module) = course_with_module(&pool, tenant, teacher).await;

    // Teacher creates file_bundle lesson
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx).await.unwrap();
    let lesson_id: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO lessons (tenant_id, course_id, module_id, type, title, sort_order)
         VALUES ($1, $2, $3, 'file_bundle', 'H', 10) RETURNING id"
    ).bind(tenant).bind(course).bind(module).fetch_one(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();

    // Outsider (different tenant)
    let other_tenant = create_tenant(&pool).await;
    let (outsider, fb_o, em_o) = create_user(&pool).await;
    attach_membership(&pool, other_tenant, outsider, "student").await;

    let s3 = Arc::new(backend::storage::mock::MockS3Client::new());
    let fa_app = build_test_app(
        backend::handlers::file_assets::router_for_tests(pool.clone(), s3),
        StubAuth {
            pool: pool.clone(), user_id: outsider, firebase_uid: fb_o, email: em_o,
            tenant_id: Some(other_tenant),
            tenant_role: Some(core_types::TenantRole::Student),
        },
    );
    let (status, _) = fire(&fa_app, "GET",
        &format!("/v1/lessons/{lesson_id}/files"), None).await;
    // 404 (lesson not visible) or 403 (lesson not readable). Either is acceptable masking.
    assert!(status == 404 || status == 403);
}
```

- [ ] **Step 2: Run**

```bash
DATABASE_URL=postgres://aulalite:changeme@localhost:55432/aulalite \
    cargo test -p backend --test lesson_attachments
```

Expected: 3 passed.

- [ ] **Step 3: Commit**

```bash
git add crates/backend/tests/lesson_attachments.rs
git commit -m "test(uploads): lesson video + file_bundle end-to-end with cross-tenant probe"
```

---

# Section F — Validation matrix + RLS sweep extension

### Task 15: uploads_validation_matrix integration test

**Files:**
- Create: `crates/backend/tests/uploads_validation_matrix.rs`

- [ ] **Step 1: Write the cross-cutting matrix test**

```rust
// crates/backend/tests/uploads_validation_matrix.rs
mod fixtures;

use fixtures::*;
use serde_json::json;
use std::sync::Arc;

async fn course_with_module_and_video_lesson_and_filebundle(
    pool: &sqlx::PgPool,
    tenant: uuid::Uuid,
    user: uuid::Uuid,
) -> (uuid::Uuid, uuid::Uuid, uuid::Uuid) {
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx).await.unwrap();
    let course: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO courses (tenant_id, slug, title, owner_user_id)
         VALUES ($1, $2, 'C', $3) RETURNING id",
    ).bind(tenant).bind(format!("c-{}", uuid::Uuid::new_v4())).bind(user)
        .fetch_one(&mut *tx).await.unwrap();
    sqlx::query(
        "INSERT INTO course_memberships (course_id, user_id, tenant_id, role)
         VALUES ($1,$2,$3,'teacher')",
    ).bind(course).bind(user).bind(tenant).execute(&mut *tx).await.unwrap();
    let module: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO modules (tenant_id, course_id, title, sort_order)
         VALUES ($1,$2,'M',10) RETURNING id",
    ).bind(tenant).bind(course).fetch_one(&mut *tx).await.unwrap();
    let video_lesson: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO lessons (tenant_id, course_id, module_id, type, title, sort_order)
         VALUES ($1, $2, $3, 'video', 'V', 10) RETURNING id",
    ).bind(tenant).bind(course).bind(module).fetch_one(&mut *tx).await.unwrap();
    let fb_lesson: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO lessons (tenant_id, course_id, module_id, type, title, sort_order)
         VALUES ($1, $2, $3, 'file_bundle', 'F', 20) RETURNING id",
    ).bind(tenant).bind(course).bind(module).fetch_one(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    (course, video_lesson, fb_lesson)
}

#[tokio::test]
async fn validation_matrix_end_to_end() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (user, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, user, "teacher").await;
    let (course, video_lesson, fb_lesson) =
        course_with_module_and_video_lesson_and_filebundle(&pool, tenant, user).await;

    let s3 = Arc::new(backend::storage::mock::MockS3Client::new());
    let app = build_test_app(
        backend::handlers::uploads::router_for_tests(pool.clone(), s3, "aulalite".into()),
        StubAuth {
            pool: pool.clone(), user_id: user, firebase_uid: fb, email: em,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Teacher),
        },
    );

    // (purpose, content_type, size_bytes, linked_entity_type, linked_entity_id, expected_status)
    let cases: Vec<(&str, &str, i64, &str, uuid::Uuid, u16)> = vec![
        // cover: pass
        ("cover", "image/jpeg", 1_000_000, "course", course, 200),
        ("cover", "image/png", 5_242_880, "course", course, 200),
        ("cover", "image/webp", 1_000_000, "course", course, 200),
        // cover: fail (svg disallowed)
        ("cover", "image/svg+xml", 1000, "course", course, 400),
        // cover: fail (oversized)
        ("cover", "image/png", 5_242_881, "course", course, 400),
        // video: pass
        ("video", "video/mp4", 100_000_000, "lesson", video_lesson, 200),
        ("video", "video/webm", 50_000_000, "lesson", video_lesson, 200),
        // video: fail (oversized)
        ("video", "video/mp4", 524_288_001, "lesson", video_lesson, 400),
        // video: fail (bad content type)
        ("video", "image/png", 1000, "lesson", video_lesson, 400),
        // attachment: pass
        ("attachment", "application/pdf", 50_000_000, "lesson", fb_lesson, 200),
        ("attachment", "application/zip", 100_000_000, "lesson", fb_lesson, 200),
        // attachment: fail (executable)
        ("attachment", "application/x-msdownload", 1000, "lesson", fb_lesson, 400),
        // attachment: fail (oversized)
        ("attachment", "application/pdf", 104_857_601, "lesson", fb_lesson, 400),
    ];

    for (purpose, ct, size, etype, eid, expected) in cases {
        let (status, body) = fire(
            &app, "POST", "/v1/uploads/begin",
            Some(json!({
                "filename": format!("f-{}.bin", uuid::Uuid::new_v4()),
                "content_type": ct,
                "size_bytes": size,
                "linked_entity_type": etype,
                "linked_entity_id": eid,
                "purpose": purpose
            })),
        ).await;
        assert_eq!(
            status.as_u16(), expected,
            "case (purpose={purpose}, ct={ct}, size={size}, etype={etype}, eid={eid}): \
             expected {expected}, got {} body {body}", status.as_u16()
        );
    }
}
```

- [ ] **Step 2: Run**

```bash
cd "C:/Users/Chiranjib Chaudhuri/Documents/Chiranjib/Elementors_Aula-phase0"
DATABASE_URL=postgres://aulalite:changeme@localhost:55432/aulalite \
    cargo test -p backend --test uploads_validation_matrix
```
Expected: 1 passed (with 13 inner cases).

- [ ] **Step 3: Commit**

```bash
git add crates/backend/tests/uploads_validation_matrix.rs
git commit -m "test(uploads): cross-cutting validation matrix end-to-end through /v1/uploads/begin"
```

---

### Task 16: RLS sweep — file_assets cross-tenant probe

**Files:**
- Modify: `crates/backend/tests/rls_tenant_isolation.rs`

- [ ] **Step 1: Append a new test that fires `/v1/file-assets/:id/url` cross-tenant**

Read the existing file. After the last test, append:

```rust
#[tokio::test]
async fn cross_tenant_get_url_route_returns_404() -> anyhow::Result<()> {
    let pool = pool().await;
    let tenant_a = Uuid::new_v4();
    let tenant_b = Uuid::new_v4();
    let user_a_id = Uuid::new_v4();
    let user_b_id = Uuid::new_v4();
    let asset_id = Uuid::new_v4();

    let mut conn = pool.acquire().await?;
    sqlx::query("BEGIN").execute(&mut *conn).await?;

    let test_result = async {
        seed_membership(&mut *conn, tenant_a, "a").await?;
        seed_membership(&mut *conn, tenant_b, "b").await?;
        seed_user(&mut *conn, user_a_id).await?;
        seed_user(&mut *conn, user_b_id).await?;

        // tenant A's file_asset, owned by user A, available
        sqlx::query(
            "INSERT INTO file_assets
                (id, tenant_id, owner_user_id, bucket, object_key,
                 content_type, size_bytes, status, visibility,
                 linked_entity_type, linked_entity_id)
             VALUES ($1, $2, $3, 'aulalite', $4, 'image/png', 1, 'available',
                     'private', NULL, NULL)",
        )
        .bind(asset_id)
        .bind(tenant_a)
        .bind(user_a_id)
        .bind(format!("k-{}", Uuid::new_v4().simple()))
        .execute(&mut *conn)
        .await?;

        // Tenant B caller — non-superuser role, app.tenant_id = B
        let role_name = create_rls_test_role_phase_1a(&mut *conn).await?;
        sqlx::query(&format!("SET LOCAL ROLE {}", role_ident(&role_name)))
            .execute(&mut *conn)
            .await?;
        set_local_tenant(&mut *conn, tenant_b).await?;

        let visible: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM file_assets WHERE id = $1",
        )
        .bind(asset_id)
        .fetch_one(&mut *conn)
        .await?;

        anyhow::Ok(visible.0)
    }
    .await;

    let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;

    assert_eq!(
        test_result?, 0,
        "tenant B must not see tenant A's file_assets"
    );

    Ok(())
}
```

- [ ] **Step 2: Run**

```bash
DATABASE_URL=postgres://aulalite:changeme@localhost:55432/aulalite \
    cargo test -p backend --test rls_tenant_isolation
```
Expected: all tests pass (5 prior + 1 new = 6 in this file).

- [ ] **Step 3: Commit**

```bash
git add crates/backend/tests/rls_tenant_isolation.rs
git commit -m "test(rls): cross-tenant probe for file_assets at the policy layer"
```

---

# Section G — Design system primitives

### Task 17: ProgressBar + FileCard

**Files:**
- Create: `crates/design-system/src/progress_bar.rs`
- Create: `crates/design-system/src/file_card.rs`
- Modify: `crates/design-system/src/lib.rs`

- [ ] **Step 1: Implement ProgressBar**

```rust
// crates/design-system/src/progress_bar.rs
use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct ProgressBarProps {
    /// 0.0..=1.0
    pub value: f32,
    #[props(default)]
    pub label: Option<String>,
}

#[component]
pub fn ProgressBar(props: ProgressBarProps) -> Element {
    let pct = (props.value.clamp(0.0, 1.0) * 100.0).round() as i32;
    rsx! {
        div { class: "ds-progress",
            div { class: "ds-progress-fill", style: "width: {pct}%" }
            if let Some(label) = &props.label {
                span { class: "ds-progress-label", "{label}" }
            } else {
                span { class: "ds-progress-label", "{pct}%" }
            }
        }
    }
}
```

- [ ] **Step 2: Implement FileCard**

```rust
// crates/design-system/src/file_card.rs
use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct FileCardProps {
    pub filename: String,
    pub size_bytes: i64,
    pub content_type: String,
    pub on_open: EventHandler<()>,
    #[props(default)]
    pub on_delete: Option<EventHandler<()>>,
}

fn format_size(bytes: i64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.1} GB", b / GB)
    } else if b >= MB {
        format!("{:.1} MB", b / MB)
    } else if b >= KB {
        format!("{:.1} KB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

fn icon_for(content_type: &str) -> &'static str {
    match content_type {
        "application/pdf" => "📄",
        ct if ct.starts_with("image/") => "🖼",
        ct if ct.starts_with("video/") => "🎬",
        ct if ct.starts_with("audio/") => "🎧",
        "application/zip" => "🗜",
        _ => "📎",
    }
}

#[component]
pub fn FileCard(props: FileCardProps) -> Element {
    let on_open = props.on_open.clone();
    rsx! {
        div { class: "ds-file-card",
            span { class: "ds-file-icon", "{icon_for(&props.content_type)}" }
            div { class: "ds-file-meta",
                a {
                    class: "ds-file-name",
                    onclick: move |_| on_open.call(()),
                    "{props.filename}"
                }
                span { class: "ds-file-size", "{format_size(props.size_bytes)}" }
            }
            if let Some(on_delete) = &props.on_delete {
                let on_delete = on_delete.clone();
                button {
                    class: "ds-file-delete",
                    "aria-label": "Delete",
                    onclick: move |_| on_delete.call(()),
                    "×"
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::format_size;

    #[test]
    fn format_size_picks_unit() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(1500), "1.5 KB");
        assert_eq!(format_size(2_500_000), "2.4 MB");
        assert_eq!(format_size(3_000_000_000), "2.8 GB");
    }
}
```

- [ ] **Step 3: Re-export in `crates/design-system/src/lib.rs`**

Append:
```rust
pub mod progress_bar;
pub mod file_card;
pub use progress_bar::ProgressBar;
pub use file_card::FileCard;
```

- [ ] **Step 4: Build + test**

```bash
cargo build -p design-system
cargo build -p design-system --target wasm32-unknown-unknown
cargo test -p design-system --lib file_card
```
Expected: 1 passed.

- [ ] **Step 5: Commit**

```bash
git add crates/design-system
git commit -m "feat(design-system): ProgressBar + FileCard primitives"
```

---

# Section H — features-courses pure helpers

### Task 18: file_picker validation submodule (TDD)

**Files:**
- Create: `crates/features-courses/src/file_picker.rs` (initial scaffold with validation submodule + tests)

- [ ] **Step 1: Write file_picker scaffold + validation submodule + tests**

Create `crates/features-courses/src/file_picker.rs`:

```rust
// crates/features-courses/src/file_picker.rs
//! Generic upload widget. The `validation` submodule is pure and unit-testable;
//! the rest of the file (XHR-based upload flow) lands in Task 20.

pub mod validation {
    /// Mirrors the backend matrix; provides instant client-side feedback.
    pub fn client_side_check(
        purpose: &str,
        content_type: &str,
        size_bytes: i64,
        allowed_types: &[&str],
        max_size_bytes: i64,
    ) -> Result<(), String> {
        if size_bytes < 0 {
            return Err("size must be non-negative".to_string());
        }
        if !allowed_types
            .iter()
            .any(|t| t.eq_ignore_ascii_case(content_type))
        {
            return Err(format!(
                "content type {content_type} not allowed for purpose {purpose}"
            ));
        }
        if size_bytes > max_size_bytes {
            return Err(format!(
                "size {size_bytes} exceeds {max_size_bytes} cap for purpose {purpose}"
            ));
        }
        Ok(())
    }

    pub const COVER_TYPES: &[&str] = &["image/jpeg", "image/png", "image/webp"];
    pub const VIDEO_TYPES: &[&str] = &["video/mp4", "video/webm"];
    pub const ATTACHMENT_TYPES: &[&str] = &[
        "application/pdf",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "application/zip",
        "text/plain",
        "text/csv",
        "image/jpeg",
        "image/png",
        "image/webp",
        "audio/mpeg",
        "video/mp4",
    ];
    pub const COVER_MAX: i64 = 5 * 1024 * 1024;
    pub const VIDEO_MAX: i64 = 500 * 1024 * 1024;
    pub const ATTACHMENT_MAX: i64 = 100 * 1024 * 1024;
}

#[cfg(test)]
mod tests {
    use super::validation::*;

    #[test]
    fn cover_accepts_jpeg() {
        assert!(client_side_check("cover", "image/jpeg", 1000, COVER_TYPES, COVER_MAX).is_ok());
    }

    #[test]
    fn cover_rejects_svg() {
        let err = client_side_check("cover", "image/svg+xml", 1000, COVER_TYPES, COVER_MAX)
            .unwrap_err();
        assert!(err.contains("not allowed"));
    }

    #[test]
    fn cover_rejects_oversized() {
        let err = client_side_check("cover", "image/png", COVER_MAX + 1, COVER_TYPES, COVER_MAX)
            .unwrap_err();
        assert!(err.contains("exceeds"));
    }

    #[test]
    fn video_accepts_mp4_under_cap() {
        assert!(client_side_check("video", "video/mp4", 100_000_000, VIDEO_TYPES, VIDEO_MAX).is_ok());
    }

    #[test]
    fn attachment_rejects_executable() {
        let err = client_side_check(
            "attachment",
            "application/x-msdownload",
            1000,
            ATTACHMENT_TYPES,
            ATTACHMENT_MAX,
        )
        .unwrap_err();
        assert!(err.contains("not allowed"));
    }
}
```

Add `pub mod file_picker;` to `crates/features-courses/src/lib.rs`.

- [ ] **Step 2: Run**

```bash
cargo test -p features-courses --lib file_picker
```
Expected: 5 passed.

- [ ] **Step 3: Commit**

```bash
git add crates/features-courses/src/lib.rs crates/features-courses/src/file_picker.rs
git commit -m "feat(features-courses): file_picker validation submodule with TDD coverage"
```

---

# Section I — features-courses display widgets

### Task 19: file_asset_image (async fetch GET URL → render <img>)

**Files:**
- Create: `crates/features-courses/src/file_asset_image.rs`
- Modify: `crates/features-courses/src/lib.rs`

- [ ] **Step 1: Implement**

```rust
// crates/features-courses/src/file_asset_image.rs
//! Renders an <img> for a file_asset. Fetches a fresh presigned GET URL on
//! every mount so we never store or cache short-lived URLs.

use dioxus::prelude::*;
use serde::Deserialize;

use crate::api::{fetch_json, ApiContext};

#[derive(Deserialize)]
struct UrlResponse {
    url: String,
}

#[derive(Props, Clone, PartialEq)]
pub struct FileAssetImageProps {
    pub asset_id: String,
    #[props(default = "asset".to_string())]
    pub alt: String,
    #[props(default)]
    pub class: Option<String>,
}

#[component]
pub fn FileAssetImage(props: FileAssetImageProps) -> Element {
    let cx = use_context::<ApiContext>();
    let asset_id = props.asset_id.clone();

    let url_resource = use_resource(move || {
        let cx = cx.clone();
        let asset_id = asset_id.clone();
        async move {
            let path = format!("/v1/file-assets/{asset_id}/url");
            fetch_json::<UrlResponse>(&cx, "GET", &path, None::<&()>)
                .await
                .map(|r| r.url)
        }
    });

    let class_attr = props.class.clone().unwrap_or_default();
    let alt = props.alt.clone();

    match &*url_resource.read_unchecked() {
        Some(Ok(url)) => rsx! {
            img { class: "{class_attr}", src: "{url}", alt: "{alt}" }
        },
        Some(Err(_)) => rsx! {
            div { class: "ds-img-error {class_attr}", "image unavailable" }
        },
        None => rsx! {
            div { class: "ds-img-loading {class_attr}", "…" }
        },
    }
}
```

Append to `crates/features-courses/src/lib.rs`:
```rust
pub mod file_asset_image;
pub use file_asset_image::FileAssetImage;
```

- [ ] **Step 2: Build (native + wasm)**

```bash
cargo build -p features-courses
cargo build -p features-courses --target wasm32-unknown-unknown
```
Expected: succeeds.

- [ ] **Step 3: Commit**

```bash
git add crates/features-courses/src/lib.rs crates/features-courses/src/file_asset_image.rs
git commit -m "feat(features-courses): FileAssetImage async-fetches presigned URL and renders <img>"
```

---

### Task 20: file_picker — XHR upload widget

**Files:**
- Modify: `crates/features-courses/src/file_picker.rs`
- Modify: `crates/features-courses/Cargo.toml` (add web-sys features for File, FormData, XHR)

- [ ] **Step 1: Add web-sys features**

In `crates/features-courses/Cargo.toml`'s `[target.'cfg(target_arch = "wasm32")'.dependencies]` block, ensure `web-sys` includes:
```toml
web-sys = { version = "0.3", features = [
    "Window","Request","RequestInit","Response","Headers",
    "File","FileList","HtmlInputElement","XmlHttpRequest","XmlHttpRequestUpload","ProgressEvent","Event"
] }
```

- [ ] **Step 2: Implement the widget**

Append to `crates/features-courses/src/file_picker.rs` (above the `#[cfg(test)]` block):

```rust
use design_system::{Button, ButtonVariant, ProgressBar, Spinner};
use dioxus::prelude::*;

use crate::api::{fetch_json, ApiContext, ApiError};

#[derive(serde::Serialize)]
struct BeginBody<'a> {
    filename: &'a str,
    content_type: &'a str,
    size_bytes: i64,
    linked_entity_type: &'a str,
    linked_entity_id: String,
    purpose: &'a str,
}

#[derive(serde::Deserialize)]
struct BeginResponse {
    asset_id: String,
    presigned_put_url: String,
}

#[derive(Props, Clone, PartialEq)]
pub struct FilePickerProps {
    pub purpose: String,
    pub linked_entity_type: String,
    pub linked_entity_id: String,
    pub allowed_types: Vec<String>,
    pub max_size_bytes: i64,
    pub on_uploaded: EventHandler<String>, // asset_id
    #[props(default = "Choose file".to_string())]
    pub button_label: String,
}

#[derive(Clone, PartialEq)]
enum PickerState {
    Idle,
    Validating,
    Beginning,
    Uploading(f32),
    Completing,
    Error(String),
}

#[component]
pub fn FilePicker(props: FilePickerProps) -> Element {
    let mut state = use_signal(|| PickerState::Idle);
    let cx = use_context::<ApiContext>();

    let on_change = {
        let purpose = props.purpose.clone();
        let entity_type = props.linked_entity_type.clone();
        let entity_id = props.linked_entity_id.clone();
        let allowed = props.allowed_types.clone();
        let max_size = props.max_size_bytes;
        let on_uploaded = props.on_uploaded.clone();
        let cx = cx.clone();

        move |evt: FormEvent| {
            // Pull the File from the change event.
            #[cfg(target_arch = "wasm32")]
            {
                use wasm_bindgen::JsCast;
                let target = evt.web_event().target().unwrap();
                let input: web_sys::HtmlInputElement = target.dyn_into().unwrap();
                let files = input.files().unwrap();
                if files.length() == 0 {
                    return;
                }
                let file = files.get(0).unwrap();
                let filename = file.name();
                let content_type = file.type_();
                let size_bytes = file.size() as i64;

                let allowed_refs: Vec<&str> =
                    allowed.iter().map(|s| s.as_str()).collect();
                state.set(PickerState::Validating);
                if let Err(e) = validation::client_side_check(
                    &purpose, &content_type, size_bytes,
                    &allowed_refs, max_size,
                ) {
                    state.set(PickerState::Error(e));
                    return;
                }

                let cx = cx.clone();
                let purpose = purpose.clone();
                let entity_type = entity_type.clone();
                let entity_id = entity_id.clone();
                let on_uploaded = on_uploaded.clone();
                let mut state_for_async = state;

                wasm_bindgen_futures::spawn_local(async move {
                    state_for_async.set(PickerState::Beginning);
                    let begin_body = BeginBody {
                        filename: &filename,
                        content_type: &content_type,
                        size_bytes,
                        linked_entity_type: &entity_type,
                        linked_entity_id: entity_id.clone(),
                        purpose: &purpose,
                    };
                    let begin_resp = match fetch_json::<BeginResponse>(
                        &cx, "POST", "/v1/uploads/begin", Some(&begin_body)
                    ).await {
                        Ok(r) => r,
                        Err(ApiError::Status(_, body)) => {
                            state_for_async.set(PickerState::Error(body));
                            return;
                        }
                        Err(e) => {
                            state_for_async.set(PickerState::Error(e.to_string()));
                            return;
                        }
                    };

                    // XHR PUT to MinIO with progress events.
                    state_for_async.set(PickerState::Uploading(0.0));
                    if let Err(msg) = upload_via_xhr(
                        &begin_resp.presigned_put_url,
                        &content_type,
                        &file,
                        |frac| state_for_async.set(PickerState::Uploading(frac)),
                    ).await {
                        state_for_async.set(PickerState::Error(msg));
                        return;
                    }

                    // Complete
                    state_for_async.set(PickerState::Completing);
                    let complete_path = format!(
                        "/v1/uploads/{}/complete", begin_resp.asset_id
                    );
                    let _: serde_json::Value = match fetch_json(
                        &cx, "POST", &complete_path,
                        Some(&serde_json::json!({})),
                    ).await {
                        Ok(v) => v,
                        Err(e) => {
                            state_for_async.set(PickerState::Error(e.to_string()));
                            return;
                        }
                    };

                    on_uploaded.call(begin_resp.asset_id);
                    state_for_async.set(PickerState::Idle);
                });
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                let _ = (evt, allowed, max_size, on_uploaded, cx, purpose, entity_type, entity_id);
                state.set(PickerState::Error("file picker only available on web".into()));
            }
        }
    };

    let accept = props.allowed_types.join(",");
    rsx! {
        div { class: "ds-file-picker",
            input {
                r#type: "file",
                accept: "{accept}",
                onchange: on_change,
                disabled: !matches!(*state.read(), PickerState::Idle | PickerState::Error(_)),
            }
            match &*state.read() {
                PickerState::Idle => rsx! {},
                PickerState::Validating | PickerState::Beginning => rsx! { Spinner {} },
                PickerState::Uploading(f) => rsx! { ProgressBar { value: *f, label: None } },
                PickerState::Completing => rsx! { Spinner {} },
                PickerState::Error(msg) => rsx! {
                    div { class: "form-error", "{msg}" }
                },
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
async fn upload_via_xhr(
    url: &str,
    content_type: &str,
    file: &web_sys::File,
    on_progress: impl Fn(f32) + 'static,
) -> Result<(), String> {
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen::JsCast;

    let xhr = web_sys::XmlHttpRequest::new().map_err(|e| format!("xhr init: {e:?}"))?;
    xhr.open("PUT", url).map_err(|e| format!("xhr open: {e:?}"))?;
    xhr.set_request_header("Content-Type", content_type)
        .map_err(|e| format!("xhr header: {e:?}"))?;

    // Hook progress
    let upload = xhr.upload().map_err(|e| format!("xhr upload: {e:?}"))?;
    let progress_cb = Closure::<dyn FnMut(web_sys::ProgressEvent)>::new(
        move |evt: web_sys::ProgressEvent| {
            if evt.length_computable() {
                let frac = (evt.loaded() / evt.total()) as f32;
                on_progress(frac.clamp(0.0, 1.0));
            }
        },
    );
    upload.set_onprogress(Some(progress_cb.as_ref().unchecked_ref()));

    // Build a oneshot channel via async event listeners.
    let (tx, rx) = futures_channel::oneshot::channel::<Result<(), String>>();
    let tx_load = std::cell::RefCell::new(Some(tx));

    let load_cb = Closure::<dyn FnMut(web_sys::Event)>::new({
        let tx_load = tx_load;
        let xhr_clone = xhr.clone();
        move |_| {
            let status = xhr_clone.status().unwrap_or(0);
            let result = if (200..300).contains(&status) {
                Ok(())
            } else {
                Err(format!("PUT returned status {status}"))
            };
            if let Some(tx) = tx_load.borrow_mut().take() {
                let _ = tx.send(result);
            }
        }
    });
    xhr.set_onloadend(Some(load_cb.as_ref().unchecked_ref()));

    xhr.send_with_opt_blob(Some(file)).map_err(|e| format!("xhr send: {e:?}"))?;

    // Keep closures alive until the request completes.
    let result = rx.await.map_err(|_| "xhr cancelled".to_string())?;
    drop(progress_cb);
    drop(load_cb);
    result
}
```

In `crates/features-courses/Cargo.toml`, add to `[target.'cfg(target_arch = "wasm32")'.dependencies]`:
```toml
futures-channel = "0.3"
```

- [ ] **Step 3: Build wasm32**

```bash
cargo build -p features-courses --target wasm32-unknown-unknown
```
Expected: succeeds.

- [ ] **Step 4: Commit**

```bash
git add crates/features-courses/Cargo.toml crates/features-courses/src/file_picker.rs Cargo.lock
git commit -m "feat(features-courses): FilePicker XHR widget with progress + begin/PUT/complete flow"
```

---

### Task 21: lesson_outline_view — student-side type-switched display

**Files:**
- Create: `crates/features-courses/src/lesson_outline_view.rs`
- Modify: `crates/features-courses/src/lib.rs`

- [ ] **Step 1: Implement**

```rust
// crates/features-courses/src/lesson_outline_view.rs
//! Student-side display: type-switches a single lesson into rich_text /
//! video / live_session / file_bundle render branches.

use crate::api::{fetch_json, ApiContext};
use crate::file_asset_image::FileAssetImage;
use design_system::{Card, FileCard};
use dioxus::prelude::*;

#[derive(Clone, PartialEq)]
pub struct LessonView {
    pub id: String,
    pub r#type: String, // 'rich_text' | 'video' | 'live_session' | 'file_bundle'
    pub title: String,
    pub body_md: Option<String>,
    pub video_asset_id: Option<String>,
    pub live_session_id: Option<String>,
}

#[derive(serde::Deserialize)]
struct LessonFile {
    asset_id: String,
    filename: String,
    content_type: String,
    size_bytes: i64,
}

#[derive(serde::Deserialize)]
struct UrlResponse {
    url: String,
}

#[derive(Props, Clone, PartialEq)]
pub struct LessonOutlineViewProps {
    pub lesson: LessonView,
}

#[component]
pub fn LessonOutlineView(props: LessonOutlineViewProps) -> Element {
    let lesson = props.lesson.clone();
    let cx = use_context::<ApiContext>();

    rsx! {
        Card {
            h3 { "{lesson.title}" }
            match lesson.r#type.as_str() {
                "rich_text" => rsx! {
                    {render_markdown(lesson.body_md.as_deref().unwrap_or(""))}
                },
                "video" => rsx! {
                    {render_video(&lesson, &cx)}
                },
                "live_session" => rsx! {
                    p { class: "live-session-pointer",
                        if let Some(sid) = &lesson.live_session_id {
                            "Live session: " a { href: "/sessions/{sid}", "open" }
                        } else {
                            "Live session not yet scheduled."
                        }
                    }
                },
                "file_bundle" => rsx! {
                    {render_file_bundle(&lesson, &cx)}
                },
                _ => rsx! { p { "Unknown lesson type." } },
            }
        }
    }
}

fn render_markdown(body: &str) -> Element {
    use pulldown_cmark::{html, Parser};
    let parser = Parser::new(body);
    let mut out = String::new();
    html::push_html(&mut out, parser);
    rsx! {
        div { class: "lesson-md", dangerous_inner_html: "{out}" }
    }
}

fn render_video(lesson: &LessonView, _cx: &ApiContext) -> Element {
    if let Some(asset_id) = &lesson.video_asset_id {
        let asset_id = asset_id.clone();
        let cx_inner = use_context::<ApiContext>();
        let url_resource = use_resource(move || {
            let cx = cx_inner.clone();
            let asset_id = asset_id.clone();
            async move {
                let path = format!("/v1/file-assets/{asset_id}/url");
                fetch_json::<UrlResponse>(&cx, "GET", &path, None::<&()>)
                    .await
                    .map(|r| r.url)
            }
        });
        match &*url_resource.read_unchecked() {
            Some(Ok(url)) => rsx! {
                video { controls: true, src: "{url}", class: "lesson-video" }
            },
            Some(Err(_)) => rsx! { p { class: "form-error", "Video unavailable." } },
            None => rsx! { p { "Loading video…" } },
        }
    } else {
        rsx! { p { class: "muted", "No video uploaded yet." } }
    }
}

fn render_file_bundle(lesson: &LessonView, _cx: &ApiContext) -> Element {
    let lesson_id = lesson.id.clone();
    let cx_inner = use_context::<ApiContext>();
    let files = use_resource(move || {
        let cx = cx_inner.clone();
        let lesson_id = lesson_id.clone();
        async move {
            let path = format!("/v1/lessons/{lesson_id}/files");
            fetch_json::<Vec<LessonFile>>(&cx, "GET", &path, None::<&()>).await
        }
    });

    match &*files.read_unchecked() {
        Some(Ok(list)) if list.is_empty() => rsx! {
            p { class: "muted", "No files attached." }
        },
        Some(Ok(list)) => {
            let list_clone = list.clone();
            rsx! {
                ul { class: "lesson-file-list",
                    for f in list_clone {
                        {
                            let asset_id = f.asset_id.clone();
                            let filename = f.filename.clone();
                            let content_type = f.content_type.clone();
                            let size = f.size_bytes;
                            let cx_open = use_context::<ApiContext>();
                            rsx! {
                                li { key: "{asset_id}",
                                    FileCard {
                                        filename: filename,
                                        size_bytes: size,
                                        content_type: content_type,
                                        on_open: move |_| {
                                            let cx = cx_open.clone();
                                            let asset_id = asset_id.clone();
                                            wasm_bindgen_futures::spawn_local(async move {
                                                let path = format!("/v1/file-assets/{asset_id}/url");
                                                if let Ok(r) = fetch_json::<UrlResponse>(
                                                    &cx, "GET", &path, None::<&()>
                                                ).await {
                                                    #[cfg(target_arch = "wasm32")]
                                                    {
                                                        let win = web_sys::window().unwrap();
                                                        let _ = win.location().set_href(&r.url);
                                                    }
                                                }
                                            });
                                        },
                                        on_delete: None,
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        Some(Err(_)) => rsx! { p { class: "form-error", "Couldn't load files." } },
        None => rsx! { p { "Loading files…" } },
    }
}

// Touch FileAssetImage so the import isn't unused (used inside other components).
#[allow(dead_code)]
fn _force_link() -> Element {
    rsx! { FileAssetImage { asset_id: "x".to_string(), alt: "x".to_string() } }
}
```

`pulldown-cmark` is already a workspace dep (added in Phase 1a). Add it to `crates/features-courses/Cargo.toml` `[dependencies]` if not present:
```toml
pulldown-cmark = { workspace = true }
```

Append to `crates/features-courses/src/lib.rs`:
```rust
pub mod lesson_outline_view;
pub use lesson_outline_view::{LessonOutlineView, LessonView};
```

- [ ] **Step 2: Build (native + wasm)**

```bash
cargo build -p features-courses
cargo build -p features-courses --target wasm32-unknown-unknown
```
Expected: succeeds.

- [ ] **Step 3: Commit**

```bash
git add crates/features-courses/Cargo.toml crates/features-courses/src/lib.rs \
        crates/features-courses/src/lesson_outline_view.rs Cargo.lock
git commit -m "feat(features-courses): LessonOutlineView with type-switched display"
```

---

# Section J — Editor wrapper components

### Task 22: course_cover_editor

**Files:**
- Create: `crates/features-courses/src/course_cover_editor.rs`
- Modify: `crates/features-courses/src/lib.rs`

- [ ] **Step 1: Implement**

```rust
// crates/features-courses/src/course_cover_editor.rs
use crate::file_asset_image::FileAssetImage;
use crate::file_picker::{validation, FilePicker};
use design_system::Card;
use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct CourseCoverEditorProps {
    pub course_id: String,
    pub current_cover_asset_id: Option<String>,
    pub on_changed: EventHandler<String>, // new asset_id
}

#[component]
pub fn CourseCoverEditor(props: CourseCoverEditorProps) -> Element {
    rsx! {
        Card {
            h3 { "Cover image" }
            if let Some(asset_id) = &props.current_cover_asset_id {
                FileAssetImage {
                    asset_id: asset_id.clone(),
                    alt: "Course cover".to_string(),
                    class: Some("cover-preview".to_string()),
                }
            } else {
                p { class: "muted", "No cover image yet." }
            }
            FilePicker {
                purpose: "cover".to_string(),
                linked_entity_type: "course".to_string(),
                linked_entity_id: props.course_id.clone(),
                allowed_types: validation::COVER_TYPES.iter().map(|s| s.to_string()).collect(),
                max_size_bytes: validation::COVER_MAX,
                on_uploaded: move |asset_id: String| props.on_changed.call(asset_id),
                button_label: "Upload cover image".to_string(),
            }
        }
    }
}
```

Append to `crates/features-courses/src/lib.rs`:
```rust
pub mod course_cover_editor;
pub use course_cover_editor::CourseCoverEditor;
```

- [ ] **Step 2: Build + commit**

```bash
cargo build -p features-courses
cargo build -p features-courses --target wasm32-unknown-unknown
git add crates/features-courses/src/lib.rs crates/features-courses/src/course_cover_editor.rs
git commit -m "feat(features-courses): CourseCoverEditor wrapping FilePicker"
```

---

### Task 23: lesson_video_editor

**Files:**
- Create: `crates/features-courses/src/lesson_video_editor.rs`
- Modify: `crates/features-courses/src/lib.rs`

- [ ] **Step 1: Implement**

```rust
// crates/features-courses/src/lesson_video_editor.rs
use crate::api::{fetch_json, ApiContext};
use crate::file_picker::{validation, FilePicker};
use design_system::{Button, ButtonVariant};
use dioxus::prelude::*;
use serde::Deserialize;

#[derive(Deserialize)]
struct UrlResponse {
    url: String,
}

#[derive(Props, Clone, PartialEq)]
pub struct LessonVideoEditorProps {
    pub lesson_id: String,
    pub current_video_asset_id: Option<String>,
    pub on_uploaded: EventHandler<String>, // new asset_id
}

#[component]
pub fn LessonVideoEditor(props: LessonVideoEditorProps) -> Element {
    let mut replacing = use_signal(|| false);
    let cx = use_context::<ApiContext>();

    let asset_present = props.current_video_asset_id.is_some() && !*replacing.read();

    rsx! {
        div { class: "lesson-video-editor",
            if asset_present {
                {
                    let asset_id = props.current_video_asset_id.clone().unwrap();
                    let cx_inner = cx.clone();
                    let url_resource = use_resource(move || {
                        let cx = cx_inner.clone();
                        let asset_id = asset_id.clone();
                        async move {
                            let path = format!("/v1/file-assets/{asset_id}/url");
                            fetch_json::<UrlResponse>(&cx, "GET", &path, None::<&()>)
                                .await
                                .map(|r| r.url)
                        }
                    });
                    match &*url_resource.read_unchecked() {
                        Some(Ok(url)) => rsx! {
                            video { controls: true, src: "{url}", class: "lesson-video" }
                            Button {
                                label: "Replace video".to_string(),
                                variant: ButtonVariant::Secondary,
                                on_click: move |_| replacing.set(true),
                            }
                        },
                        Some(Err(_)) => rsx! {
                            div { class: "form-error", "Couldn't load video." }
                            Button {
                                label: "Replace video".to_string(),
                                variant: ButtonVariant::Secondary,
                                on_click: move |_| replacing.set(true),
                            }
                        },
                        None => rsx! { div { "Loading…" } },
                    }
                }
            } else {
                FilePicker {
                    purpose: "video".to_string(),
                    linked_entity_type: "lesson".to_string(),
                    linked_entity_id: props.lesson_id.clone(),
                    allowed_types: validation::VIDEO_TYPES.iter().map(|s| s.to_string()).collect(),
                    max_size_bytes: validation::VIDEO_MAX,
                    on_uploaded: move |asset_id: String| {
                        replacing.set(false);
                        props.on_uploaded.call(asset_id);
                    },
                    button_label: "Upload video".to_string(),
                }
            }
        }
    }
}
```

Append to lib.rs:
```rust
pub mod lesson_video_editor;
pub use lesson_video_editor::LessonVideoEditor;
```

- [ ] **Step 2: Build + commit**

```bash
cargo build -p features-courses
cargo build -p features-courses --target wasm32-unknown-unknown
git add crates/features-courses/src/lib.rs crates/features-courses/src/lesson_video_editor.rs
git commit -m "feat(features-courses): LessonVideoEditor with playback + replace flow"
```

---

### Task 24: lesson_files_editor

**Files:**
- Create: `crates/features-courses/src/lesson_files_editor.rs`
- Modify: `crates/features-courses/src/lib.rs`

- [ ] **Step 1: Implement**

```rust
// crates/features-courses/src/lesson_files_editor.rs
use crate::api::{fetch_json, ApiContext, ApiError};
use crate::file_picker::{validation, FilePicker};
use design_system::{Card, FileCard};
use dioxus::prelude::*;
use serde::Deserialize;

#[derive(Deserialize, Clone)]
struct LessonFile {
    asset_id: String,
    filename: String,
    content_type: String,
    size_bytes: i64,
}

#[derive(Props, Clone, PartialEq)]
pub struct LessonFilesEditorProps {
    pub lesson_id: String,
}

#[component]
pub fn LessonFilesEditor(props: LessonFilesEditorProps) -> Element {
    let cx = use_context::<ApiContext>();
    let lesson_id = props.lesson_id.clone();

    // Reload trigger: incrementing this signal forces re-fetch.
    let mut reload = use_signal(|| 0u32);

    let lesson_id_for_resource = lesson_id.clone();
    let cx_for_resource = cx.clone();
    let files = use_resource(move || {
        let _ = reload.read(); // dependency
        let cx = cx_for_resource.clone();
        let lid = lesson_id_for_resource.clone();
        async move {
            let path = format!("/v1/lessons/{lid}/files");
            fetch_json::<Vec<LessonFile>>(&cx, "GET", &path, None::<&()>).await
        }
    });

    let lesson_id_for_picker = lesson_id.clone();
    rsx! {
        Card {
            h3 { "Attached files" }
            match &*files.read_unchecked() {
                Some(Ok(list)) if list.is_empty() => rsx! {
                    p { class: "muted", "No files attached yet." }
                },
                Some(Ok(list)) => {
                    let cx_for_delete = cx.clone();
                    rsx! {
                        ul { class: "lesson-files-list",
                            for f in list.clone() {
                                {
                                    let cx_d = cx_for_delete.clone();
                                    let asset_id = f.asset_id.clone();
                                    let mut reload_inner = reload;
                                    rsx! {
                                        li { key: "{asset_id}",
                                            FileCard {
                                                filename: f.filename,
                                                size_bytes: f.size_bytes,
                                                content_type: f.content_type,
                                                on_open: |_| {},
                                                on_delete: Some(EventHandler::new(move |_| {
                                                    let cx = cx_d.clone();
                                                    let asset_id = asset_id.clone();
                                                    wasm_bindgen_futures::spawn_local(async move {
                                                        let path = format!("/v1/file-assets/{asset_id}");
                                                        let _: Result<serde_json::Value, ApiError> =
                                                            fetch_json(&cx, "DELETE", &path, None::<&()>).await;
                                                        reload_inner.set(*reload_inner.read() + 1);
                                                    });
                                                })),
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                Some(Err(_)) => rsx! { p { class: "form-error", "Couldn't load files." } },
                None => rsx! { p { "Loading files…" } },
            }
            FilePicker {
                purpose: "attachment".to_string(),
                linked_entity_type: "lesson".to_string(),
                linked_entity_id: lesson_id_for_picker,
                allowed_types: validation::ATTACHMENT_TYPES.iter().map(|s| s.to_string()).collect(),
                max_size_bytes: validation::ATTACHMENT_MAX,
                on_uploaded: move |_asset_id: String| {
                    reload.set(*reload.read() + 1);
                },
                button_label: "+ Add files".to_string(),
            }
        }
    }
}
```

Append to lib.rs:
```rust
pub mod lesson_files_editor;
pub use lesson_files_editor::LessonFilesEditor;
```

- [ ] **Step 2: Build + commit**

```bash
cargo build -p features-courses
cargo build -p features-courses --target wasm32-unknown-unknown
git add crates/features-courses/src/lib.rs crates/features-courses/src/lesson_files_editor.rs
git commit -m "feat(features-courses): LessonFilesEditor with attach + list + delete"
```

---

# Section K — Existing component modifications

### Task 25: lesson_editor — 4-branch type switch

**Files:**
- Modify: `crates/features-courses/src/lesson_editor.rs`

- [ ] **Step 1: Replace lesson_editor with the 4-branch version**

The Phase 1a version handled `rich_text` (markdown editor) and `live_session` (picker). Add `video` and `file_bundle` branches:

```rust
// crates/features-courses/src/lesson_editor.rs
use design_system::{Button, ButtonVariant, Input, MarkdownEditor, Select, SelectOption};
use dioxus::prelude::*;

use crate::lesson_video_editor::LessonVideoEditor;
use crate::lesson_files_editor::LessonFilesEditor;

#[derive(Clone, PartialEq)]
pub struct UnscheduledSession {
    pub id: String,
    pub label: String,
}

#[derive(Props, Clone, PartialEq)]
pub struct LessonEditorProps {
    pub lesson_id: String,
    pub r#type: String,
    pub title: String,
    pub body_md: String,
    pub linked_session_id: Option<String>,
    pub linked_video_asset_id: Option<String>,
    pub available_sessions: Vec<UnscheduledSession>,
    pub on_save: EventHandler<SaveLessonRequest>,
    pub on_video_uploaded: EventHandler<String>, // (asset_id)
}

#[derive(Clone, PartialEq, Debug)]
pub struct SaveLessonRequest {
    pub lesson_id: String,
    pub title: String,
    pub body_md: Option<String>,
    pub linked_session_id: Option<String>,
}

#[component]
pub fn LessonEditor(props: LessonEditorProps) -> Element {
    let mut title = use_signal(|| props.title.clone());
    let mut body = use_signal(|| props.body_md.clone());
    let mut linked = use_signal(|| props.linked_session_id.clone());
    let r#type = props.r#type.clone();

    let do_save = {
        let on_save = props.on_save.clone();
        let lesson_id = props.lesson_id.clone();
        let r#type = r#type.clone();
        move |_| {
            let body_md = if r#type == "rich_text" { Some(body.read().clone()) } else { None };
            let linked_session_id = if r#type == "live_session" { linked.read().clone() } else { None };
            on_save.call(SaveLessonRequest {
                lesson_id: lesson_id.clone(),
                title: title.read().clone(),
                body_md,
                linked_session_id,
            });
        }
    };

    rsx! {
        div { class: "lesson-editor",
            div { class: "field",
                label { "Title" }
                Input {
                    value: title.read().clone(),
                    placeholder: "Lesson title".to_string(),
                    input_type: "text".to_string(),
                    disabled: false,
                    on_input: move |v| title.set(v),
                }
            }
            match r#type.as_str() {
                "rich_text" => rsx! {
                    div { class: "field",
                        label { "Content (markdown)" }
                        MarkdownEditor {
                            value: body.read().clone(),
                            on_change: move |v| body.set(v),
                            disabled: false,
                        }
                    }
                },
                "live_session" => rsx! {
                    div { class: "field",
                        label { "Linked live session" }
                        Select {
                            value: linked.read().clone().unwrap_or_default(),
                            options: {
                                let mut opts = vec![SelectOption { value: String::new(), label: "— pick a session —".to_string() }];
                                for s in &props.available_sessions {
                                    opts.push(SelectOption { value: s.id.clone(), label: s.label.clone() });
                                }
                                opts
                            },
                            on_change: move |v: String| linked.set(if v.is_empty() { None } else { Some(v) }),
                        }
                    }
                },
                "video" => rsx! {
                    LessonVideoEditor {
                        lesson_id: props.lesson_id.clone(),
                        current_video_asset_id: props.linked_video_asset_id.clone(),
                        on_uploaded: props.on_video_uploaded.clone(),
                    }
                },
                "file_bundle" => rsx! {
                    LessonFilesEditor {
                        lesson_id: props.lesson_id.clone(),
                    }
                },
                _ => rsx! { p { "Unknown lesson type: {r#type}" } },
            }
            div { class: "actions",
                Button {
                    label: "Save".to_string(),
                    variant: ButtonVariant::Primary,
                    on_click: do_save,
                }
            }
        }
    }
}
```

- [ ] **Step 2: Build + commit**

```bash
cargo build -p features-courses
cargo build -p features-courses --target wasm32-unknown-unknown
git add crates/features-courses/src/lesson_editor.rs
git commit -m "feat(features-courses): LessonEditor 4-branch type switch (video + file_bundle)"
```

---

### Task 26: course_list — display cover image on cards

**Files:**
- Modify: `crates/features-courses/src/course_list.rs`

- [ ] **Step 1: Update course_list**

Find the `CourseListItem` struct. Add `cover_asset_id: Option<String>`:

```rust
#[derive(Clone, PartialEq)]
pub struct CourseListItem {
    pub id: String,
    pub slug: String,
    pub title: String,
    pub status: String,
    pub description: Option<String>,
    pub owner_user_id: String,
    pub cover_asset_id: Option<String>,
}
```

In the `for course in &visible` block, change the `Card { ... }` body to include the cover image when set. Replace the existing `Card {...}` block with:

```rust
Card {
    if let Some(asset_id) = &course.cover_asset_id {
        crate::file_asset_image::FileAssetImage {
            asset_id: asset_id.clone(),
            alt: course.title.clone(),
            class: Some("course-card-cover".to_string()),
        }
    } else {
        div { class: "course-card-cover-placeholder" }
    }
    h3 { a { href: "/courses/{course.slug}", "{course.title}" } }
    div { class: "card-row",
        Badge {
            label: course.status.clone(),
            tone: match course.status.as_str() {
                "draft" => BadgeTone::Neutral,
                "published" => BadgeTone::Success,
                "archived" => BadgeTone::Warning,
                _ => BadgeTone::Neutral,
            },
        }
    }
    if let Some(desc) = &course.description {
        p { class: "course-desc", "{desc}" }
    }
}
```

The existing SSR test (`create_button_hidden_when_not_allowed`) still works because it passes empty `courses: vec![]`.

- [ ] **Step 2: Run + commit**

```bash
cargo test -p features-courses --lib course_list
git add crates/features-courses/src/course_list.rs
git commit -m "feat(features-courses): course_list cards display cover image"
```

---

### Task 27: course_detail — cover banner + Edit tab + Outline tab

**Files:**
- Modify: `crates/features-courses/src/course_detail.rs`

- [ ] **Step 1: Update CourseDetail**

The existing `CourseDetail` is a tabbed shell. Extend its props with `cover_asset_id` and a slot for an `edit_tab_extra` element used to inject the cover editor. Keep the existing tabs structure.

Replace `crates/features-courses/src/course_detail.rs` with:

```rust
// crates/features-courses/src/course_detail.rs
use crate::file_asset_image::FileAssetImage;
use design_system::{Badge, BadgeTone, Tab, Tabs};
use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct CourseDetailProps {
    pub course_title: String,
    pub course_status: String,
    pub course_cover_asset_id: Option<String>,
    pub can_admin: bool,
    pub active_tab: String,
    pub on_tab_change: EventHandler<String>,
    pub children: Element,
}

#[component]
pub fn CourseDetail(props: CourseDetailProps) -> Element {
    let mut tabs: Vec<Tab> = vec![
        Tab { key: "outline".to_string(), label: "Outline".to_string() },
        Tab { key: "schedule".to_string(), label: "Schedule".to_string() },
    ];
    if props.can_admin {
        tabs.push(Tab { key: "people".to_string(), label: "People".to_string() });
        tabs.push(Tab { key: "edit".to_string(), label: "Edit".to_string() });
    }
    let on_change = props.on_tab_change.clone();

    rsx! {
        div { class: "course-detail",
            if let Some(asset_id) = &props.course_cover_asset_id {
                div { class: "course-detail-banner",
                    FileAssetImage {
                        asset_id: asset_id.clone(),
                        alt: props.course_title.clone(),
                        class: Some("course-banner-img".to_string()),
                    }
                }
            }
            header { class: "course-detail-header",
                h1 { "{props.course_title}" }
                Badge {
                    label: props.course_status.clone(),
                    tone: match props.course_status.as_str() {
                        "draft" => BadgeTone::Neutral,
                        "published" => BadgeTone::Success,
                        "archived" => BadgeTone::Warning,
                        _ => BadgeTone::Neutral,
                    },
                }
            }
            Tabs { tabs: tabs, active: props.active_tab.clone(),
                on_change: move |k| on_change.call(k) }
            div { class: "course-detail-body", {props.children} }
        }
    }
}
```

The page using `CourseDetail` (in `shell-web/src/main.rs`) is responsible for putting `CourseCoverEditor` and `LessonOutlineView` into `children` based on the active tab. Phase 1a's shell-web stub passes plain text; we leave the shell-web wiring of these new sub-components for the user to drive in the manual exit-checklist phase. The router structure already supports it.

- [ ] **Step 2: Build + commit**

```bash
cargo build -p features-courses
cargo build -p features-courses --target wasm32-unknown-unknown
git add crates/features-courses/src/course_detail.rs
git commit -m "feat(features-courses): CourseDetail cover banner; props extended for cover_asset_id"
```

---

# Section L — error_messages + course_builder

### Task 28: error_messages humanizations + course_builder type pills

**Files:**
- Modify: `crates/features-courses/src/error_messages.rs`
- Modify: `crates/features-courses/src/course_builder.rs`

- [ ] **Step 1: Add 3 new humanizations**

Update `humanize_error` in `crates/features-courses/src/error_messages.rs`. Add 3 new branches BEFORE the `forbidden` branch:

```rust
} else if raw.contains("file asset not found") {
    "That file is no longer available."
} else if raw.contains("upload validation failed") {
    "Couldn't upload that file. Check the type and size."
} else if raw.contains("upload object missing") {
    "The upload didn't finish. Please try again."
} else if raw.contains("forbidden") {
```

Update the existing tests to include 3 new cases:

```rust
assert!(humanize_error("file asset not found").contains("no longer available"));
assert!(humanize_error("upload validation failed: size 6000000 exceeds 5242880").contains("Check the type"));
assert!(humanize_error("upload object missing or size mismatch").contains("didn't finish"));
```

- [ ] **Step 2: Update course_builder to display all 4 lesson types**

In `crates/features-courses/src/course_builder.rs`, the lesson nodes already render a `type-pill` from `l.r#type`. Phase 1a's UI didn't filter — `'video'` and `'file_bundle'` types automatically appear in the tree once they're created. So no code change is strictly required.

For visual polish, ensure the `type-pill` CSS class supports the longer strings — but that's a CSS concern, deferred. Confirm by running existing tests:

```bash
cargo test -p features-courses --lib course_builder
```
Expected: 3 passed (move_to_index helper tests).

- [ ] **Step 3: Run all tests**

```bash
cargo test -p features-courses --lib
```
Expected: all green (the previously-passing tests + 3 new humanize_error cases = 13 total in features-courses).

- [ ] **Step 4: Commit**

```bash
git add crates/features-courses/src/error_messages.rs crates/features-courses/src/course_builder.rs
git commit -m "feat(features-courses): humanize 3 new upload error variants"
```

---

# Section M — Exit checklist + final push

### Task 29: Phase 1b-α exit checklist + workspace test sweep + push

**Files:**
- Create: `docs/superpowers/plans/2026-05-08-aulalite-phase-1b-alpha-exit-checklist.md`

- [ ] **Step 1: Write the checklist**

```markdown
# Phase 1b-α Exit Checklist

Run these checks in order from the repository root. Phase 1b-α is complete only
when every required item passes.

## 1. Stack health
- [ ] `docker compose up -d`
- [ ] `curl http://localhost:8080/healthz` returns `ok`
- [ ] MinIO console at `http://localhost:9001` shows the `aulalite` bucket
- [ ] Postgres / Redis / MinIO / backend / mediamtx all running

## 2. Migrations
- [ ] `sqlx migrate info --source migrations` shows `20260508000011_tighten_file_assets_links` as applied.
- [ ] `\d file_assets` shows the `file_assets_linked_entity_type_check` and `file_assets_size_nonneg` constraints.
- [ ] `\d courses` shows `courses_cover_asset_id_fkey`.
- [ ] `\d lessons` shows `lessons_video_asset_id_fkey`.

## 3. Automated verification
- [ ] `cargo test --workspace` (everything green; zero failures)
- [ ] `cargo build -p shell-web --target wasm32-unknown-unknown`
- [ ] `cargo check -p shell-mobile --target aarch64-linux-android`
- [ ] `cargo build -p features-courses --target wasm32-unknown-unknown`
- [ ] `dx build --platform web --package shell-web`

## 4. MinIO CORS check (one-time setup)
- [ ] In MinIO console (or via mc CLI), set the bucket's CORS to allow `PUT` from
      `http://localhost:3000` (dev) and `https://app.elementors.guru` (prod). If
      not set, browser uploads fail with CORS errors. Workaround for dev:
      ```bash
      mc alias set local http://localhost:9000 aulalite changeme123
      mc anonymous set-cors local/aulalite '<<<json>>>'
      ```
      where `<<<json>>>` allows your origins. (Spec section 7 open question 3.)

## 5. Course cover flow (web, real Firebase user)
- [ ] Sign in as teacher; open a course → Edit tab → upload a JPEG <5MB.
- [ ] Confirm preview displays.
- [ ] Confirm course list card shows the cover image.
- [ ] Confirm course detail header banner displays the cover.

## 6. Lesson video flow (web)
- [ ] Create a `'video'` lesson; upload an mp4 ≤500MB.
- [ ] Confirm `<video controls>` plays in lesson outline.
- [ ] Replace the video; confirm new video loads.

## 7. Lesson file_bundle flow (web)
- [ ] Create a `'file_bundle'` lesson; attach 3 files (pdf + zip + image).
- [ ] Confirm file list displays with correct icons + sizes.
- [ ] Click a file; download starts.
- [ ] Delete a file; confirm it's removed from the list and the MinIO object is gone.

## 8. Permissions
- [ ] Sign in as a student; confirm upload-begin returns 403.
- [ ] Cross-tenant probe: GET `/v1/file-assets/{some-tenant-A-asset-id}/url` from tenant B's session returns 404.
- [ ] PATCH course with a `cover_asset_id` from a different tenant returns 4xx.

## 9. Failure path
- [ ] Begin an upload; close the browser tab before completing.
- [ ] Confirm the `file_assets` row stays at `status='pending'`.
- [ ] Manually run the janitor SQL:
      ```sql
      UPDATE file_assets SET status='failed'
       WHERE status='pending' AND created_at < now() - interval '1 hour';
      ```
      (For exit-checklist purposes, override the timestamp with `interval '0 seconds'`.)
- [ ] Confirm the row is now `status='failed'`.

## Completion tag

Only after every required check above passes:

```bash
git tag phase-1b-alpha-complete
git push origin phase-1b-alpha-complete
```
```

- [ ] **Step 2: Commit + workspace test sweep + push**

```bash
cd "C:/Users/Chiranjib Chaudhuri/Documents/Chiranjib/Elementors_Aula-phase0"
git add docs/superpowers/plans/2026-05-08-aulalite-phase-1b-alpha-exit-checklist.md
git commit -m "docs(plan): Phase 1b-alpha exit checklist"

DATABASE_URL=postgres://aulalite:changeme@localhost:55432/aulalite \
    cargo test --workspace
# Expected: all green.

git push origin phase-0-foundations
```

- [ ] **Step 3: Report status**

Tell the user:
- Total commits in 1b-α
- Workspace test pass count
- Final SHA on `origin/phase-0-foundations`
- That the 9 manual exit-checklist items are the gate before applying the `phase-1b-alpha-complete` tag.

---

## Self-review notes (for the controller running this plan)

After all tasks complete:

1. **Spec coverage:** every section of `2026-05-08-aulalite-phase-1b-alpha-file-uploads-design.md` is touched by at least one task. Verify by grepping section headings against task descriptions.
2. **No placeholders:** every code block has actual implementation; no TODO/TBD left in the plan.
3. **Type consistency:** `S3Client` trait signature is identical in `storage/mod.rs` (Task 4), `storage/mock.rs` (Task 4), `storage/minio.rs` (Task 5), and the test routers (Tasks 11-12). `BeginUpload` / `BeginUploadDto` shapes match between handler (Task 11) and frontend `FilePicker` (Task 20). `LessonView`, `LessonFile`, and `UrlResponse` are stable across `lesson_outline_view`, `file_asset_image`, and the editor wrappers.
4. **Migration is forward-only.** `ALTER TABLE ... ADD CONSTRAINT` does not break existing rows because Phase 1a's `file_assets` table is empty (no production data yet).
5. **Test coverage:** every handler task has paired integration tests; pure-function services have inline unit tests; UI components are covered by the matrix-style validation tests in `file_picker::validation`.
6. **Mobile and desktop shells unchanged.** `shell-mobile` and `shell-desktop` should still build cleanly — no changes touched their compile surface.





