# AulaLite — Phase 1b-α Design (File Upload Pipeline)

**Date:** 2026-05-08
**Status:** Approved for plan generation
**Parent design:** `docs/superpowers/specs/2026-05-03-aulalite-scope-design.md`
**Predecessor spec:** `docs/superpowers/specs/2026-05-08-aulalite-phase-1a-courses-enrollment-design.md`

---

## Executive Summary

Phase 1b-α is the first sub-slice of the design spec's Phase 1b ("Live class subsystem"). It delivers the generic **client-driven file upload pipeline** — `POST /v1/uploads/begin` → presigned PUT to MinIO → `POST /v1/uploads/:id/complete` — and three concrete consumers: course cover images, lesson `video` attachments, and lesson `file_bundle` multi-attachments. Plus the read-side display surfaces (cover image in course list/detail, `<video>` in lesson view, file list in `file_bundle` lesson view).

**End-of-1b-α deliverable:** a teacher signs in, uploads a cover image for their course, creates a lesson of type `'video'` and uploads an mp4 to it, creates a lesson of type `'file_bundle'` and attaches three documents. Students viewing the course see the cover image, can play the video, and can download the documents. All authorization checks enforced; cross-tenant isolation verified.

**Scope guardrail:** the pipeline shape is generic (any future consumer adds a `(linked_entity_type, purpose)` arm to the begin-handler dispatch), but at 1b-α only three consumers are wired. Recording-uploader's server-side `register-segment` flow lands with 1b-δ. Mobile pickers, transcoding, multi-part / resumable uploads, virus scanning, and thumbnail generation are all out of scope.

---

## Scoping Decisions (Captured From Brainstorming)

| # | Question | Decision |
|---|---|---|
| 1 | Decompose Phase 1b? | Yes. Split into 1b-α (file upload pipeline), 1b-β (live class core: MediaMTX + WebRTC publish/watch), 1b-γ (live room UX: chat, hand-raise, presence), 1b-δ (recording: auto-record + uploader sidecar + playback). This spec covers 1b-α only. |
| 2 | Which consumers at 1b-α? | Course cover image, lesson `video`, lesson `file_bundle`. *(Recording-uploader's server-side metadata route is a separate shape and ships with 1b-δ.)* |
| 3 | Validation matrix | Cover: `image/{jpeg,png,webp}`, 5 MB cap. Video: `video/{mp4,webm}`, 500 MB cap. Attachment: pdf/docx/pptx/xlsx/txt/csv/zip/jpeg/png/webp/mp3/mp4, 100 MB cap per file. No SVG (XSS risk). |
| 4 | UI scope | Minimal-but-demo-able UI: cover-image picker on course detail Edit tab, video picker in lesson editor (video branch), multi-file picker in lesson editor (file_bundle branch). Display surfaces: cover image on course list cards + course detail header, `<video>` in lesson outline, file-card list in lesson outline. |
| 5 | S3 SDK | `aws-sdk-s3` (industry standard, MinIO-compatible via `endpoint_url` override). |
| 6 | Bucket bootstrap | Backend startup creates `aulalite` bucket if missing (idempotent). |
| 7 | Janitor sweep for stale `pending` rows | **Stubbed at 1b-α.** SQL one-liner documented; no daemon. Real daemon lands in 1b-δ. |
| 8 | Concurrent multi-upload in `file_bundle` UI | Sequential at 1b-α; parallel is a Phase 2 perf concern. |
| 9 | Mobile UI | Read-only display of cover images and video playback via existing watch flows. No mobile pickers at 1b-α. |
| 10 | Test coverage shape | Real-Postgres integration tests + mock-S3 trait for upload pipeline + pure-function unit tests for validation/sanitization + SSR-render tests for new screens. No browser E2E. |

---

## Section 1 — Scope

### In scope

- **One new migration** (`20260508000011_tighten_file_assets_links.sql`): wires deferred FKs `courses.cover_asset_id → file_assets(id)` and `lessons.video_asset_id → file_assets(id)` with `ON DELETE SET NULL`; adds CHECK on `file_assets.linked_entity_type IN ('course','lesson')`; adds `CHECK size_bytes >= 0`.
- **Backend services:**
  - `services::file_assets` — pure validation matrix + `sanitize_filename` + `object_key` formatter.
  - `storage::S3Client` trait + `MinIoClient` (production, `aws-sdk-s3`) + `MockS3Client` (test).
- **Backend `db::file_assets`** — sqlx queries: `insert_pending`, `mark_available`, `mark_failed`, `fetch`, `list_for_entity`, `delete`.
- **Backend handlers:**
  - `handlers::uploads` — `POST /v1/uploads/begin`, `POST /v1/uploads/:id/complete`.
  - `handlers::file_assets` — `GET /v1/file-assets/:id/url`, `DELETE /v1/file-assets/:id`, `GET /v1/lessons/:lid/files`.
  - `handlers::courses` — extended: `PATCH /v1/courses/:id` accepts `cover_asset_id` (with double-Option for null vs absent).
  - `handlers::lessons` — extended: `'video'` and `'file_bundle'` types accepted; `video_asset_id` PATCH supported.
- **`AppState`** gains `storage: Arc<dyn S3Client>` + `bucket_name: String`. Backend startup ensures the bucket exists.
- **`features-courses`** UI additions: `file_picker.rs`, `course_cover_editor.rs`, `lesson_video_editor.rs`, `lesson_files_editor.rs`, `file_asset_image.rs`, `lesson_outline_view.rs`. Plus modifications to `lesson_editor.rs`, `course_builder.rs`, `course_list.rs`, `course_detail.rs`.
- **Design system:** `ProgressBar`, `FileCard`.
- **Three new `ApiError` variants:** `FileAssetNotFound`, `UploadValidationFailed(String)`, `UploadObjectMissing`.
- **Tests:** integration tests with mock S3 for the upload pipeline, course cover flow, lesson attachments; pure unit tests for validation matrix and sanitize_filename; SSR-render tests for new components; extended cross-tenant RLS sweep covering `file_assets`.

### Out of scope (explicit deferrals)

- **Recording-uploader's server-side `POST /v1/internal/recordings/segment`** route (different auth shape — service-to-service). Lands with 1b-δ.
- **Live class media** (WebRTC publish, MediaMTX HTTP auth). 1b-β.
- **Live room UX** (chat, hand-raise, presence). 1b-γ.
- **Recording playback** UI. 1b-δ.
- **Mobile picker UI** — `shell-mobile` reads but doesn't write at 1b-α.
- **Transcoding / thumbnail generation** — videos play in-browser as uploaded; no poster auto-generation; no transcoded variants.
- **Multi-part / resumable uploads** — single-shot PUT with 15-min TTL. >500MB videos that fail mid-upload start over.
- **Virus scanning** — out of scope at MVP.
- **Janitor daemon** that auto-transitions stale `pending` rows to `failed`. SQL is documented; operator runs manually.
- **Public-share URLs** — every GET URL is short-lived presigned. No long-lived public links at 1b-α.
- **Per-tenant storage quotas** — Phase 2 with billing.
- **Concurrent multi-file uploads** in `file_bundle` UI. Sequential only.

---

## Section 2 — Architecture

### Crate / module boundaries

```
backend/
  src/
    storage/                              # NEW module
      mod.rs                              # S3Client trait + S3Call enum (for mock)
      minio.rs                            # MinIoClient — production impl over aws-sdk-s3
      mock.rs                             # MockS3Client — captures calls; pub mod mock for tests
    services/
      file_assets.rs                      # NEW — sanitize_filename, object_key, validate_request
    db/
      file_assets.rs                      # NEW — sqlx queries
    handlers/
      uploads.rs                          # NEW — begin + complete
      file_assets.rs                      # NEW — get-url + delete + list-lesson-files
      courses.rs                          # MODIFIED — PATCH accepts cover_asset_id (DoubleOption)
      lessons.rs                          # MODIFIED — type rejection lifted; video_asset_id PATCH (DoubleOption)
  src/lib.rs                              # MODIFIED — AppState gets storage + bucket_name; bootstrap on startup
  src/main.rs                             # MODIFIED — construct MinIoClient from env; ensure_bucket call
  tests/
    uploads.rs                            # NEW
    uploads_validation_matrix.rs          # NEW
    course_cover_upload.rs                # NEW
    lesson_attachments.rs                 # NEW
    rls_tenant_isolation.rs               # extended — file_assets cross-tenant probe via /v1/file-assets/:id/url

features-courses/
  src/
    file_picker.rs                        # NEW — generic XHR-based upload widget + validation submodule
    course_cover_editor.rs                # NEW
    lesson_video_editor.rs                # NEW
    lesson_files_editor.rs                # NEW
    file_asset_image.rs                   # NEW — async fetch GET URL + render <img>
    lesson_outline_view.rs                # NEW — student-side type-switched display
    lesson_editor.rs                      # MODIFIED — 4-branch type switch
    course_list.rs                        # MODIFIED — cover image on cards
    course_detail.rs                      # MODIFIED — cover banner, Edit tab adds CourseCoverEditor; Outline tab uses LessonOutlineView
    course_builder.rs                     # MODIFIED — show video/file_bundle types

design-system/
  src/
    progress_bar.rs                       # NEW
    file_card.rs                          # NEW
```

### Key architectural decisions

1. **`storage::S3Client` trait abstraction.** Production wraps `aws-sdk-s3::Client`; tests inject `MockS3Client`. The trait surfaces the four operations the handlers actually need: `presigned_put_url(key, content_type, size, ttl)`, `presigned_get_url(key, ttl)`, `head_object(key)`, `delete_object(key)`. Plus `ensure_bucket(name)` for startup. The mock records every call and exposes `simulate_object(key, size, content_type)` so tests can prime the HEAD response.

2. **Two-phase upload, never via the API.** Backend never sees the blob — the API is stateless w.r.t. upload bandwidth. `/begin` validates + inserts `file_assets(status='pending')` + returns presigned URL. Client `PUT`s directly. `/complete` HEADs to verify size + presence, flips `status='available'`. This lets the API scale independently of upload throughput.

3. **Per-`(linked_entity_type, purpose)` authorization dispatch in `/v1/uploads/begin`.** Small `match` block; each arm resolves the linked entity, checks the caller's permission against the existing `db::courses::caller_can_admin_course` helper, and runs the validation matrix arm. New consumers add a match arm — no abstraction layer.

4. **Idempotent `/v1/uploads/:id/complete`.** Calling on an already-`available` row returns the current row without re-HEADing. Calling on `failed` rejects. Calling on `pending` does the HEAD + transition.

5. **Caller-of-`/begin` is the only valid caller of `/complete`.** `/complete` checks `file_assets.owner_user_id == ctx.user_id`. Prevents racing-uploader-claim attacks.

6. **Bucket bootstrap fails fast.** `main.rs` runs `storage.ensure_bucket(&bucket_name).await?` after `db::run_migrations`. If MinIO is unreachable or auth is wrong, the binary exits with a clear error before serving any traffic.

7. **`linked_entity_type` CHECK tightens forward-compatibly.** Migration 0011 restricts to `('course', 'lesson')`. Future consumers (recordings, submissions, avatars) extend this CHECK in their own migration.

8. **`DoubleOption` for nullable PATCH fields.** `PatchCourse.cover_asset_id` and `PatchLesson.video_asset_id` use `Option<Option<Uuid>>` (`None` = absent / no change; `Some(None)` = explicit null / clear; `Some(Some(id))` = set). Implemented via `serde_with::rust::double_option` (added as workspace dep).

9. **Lesson `video_asset_id` is optional at lesson create.** A teacher can create the lesson skeleton, then upload, then PATCH the video onto it. Or upload a video first, get the asset_id, then create the lesson with it. Both flows work.

10. **GET URLs minted on-demand, never cached.** Every display fetches `/v1/file-assets/:id/url` to get a fresh 15-min signed URL. The frontend re-fetches on every component mount. This avoids expired-URL bugs entirely at the cost of one extra API round-trip per displayed asset (cheap).

11. **Mobile reads, doesn't write.** `shell-mobile` consumes cover images and video playback via the same `/v1/file-assets/:id/url` route — same auth, same RLS. No mobile-specific code added at 1b-α.

12. **No janitor daemon.** A single SQL one-liner is part of the runbook:
    ```sql
    UPDATE file_assets SET status='failed' WHERE status='pending' AND created_at < now() - interval '1 hour';
    ```
    A real daemon (cron-style) lands in 1b-δ alongside the recording-uploader's lifecycle.

---

## Section 3 — Data Model

### Migration 0011 — `tighten_file_assets_links.sql`

```sql
-- migrations/20260508000011_tighten_file_assets_links.sql

-- Wire deferred FKs from Phase 1a.
ALTER TABLE courses
    ADD CONSTRAINT courses_cover_asset_id_fkey
    FOREIGN KEY (cover_asset_id) REFERENCES file_assets(id) ON DELETE SET NULL;

ALTER TABLE lessons
    ADD CONSTRAINT lessons_video_asset_id_fkey
    FOREIGN KEY (video_asset_id) REFERENCES file_assets(id) ON DELETE SET NULL;

-- Tighten polymorphic linked_entity_type to known values.
ALTER TABLE file_assets
    ADD CONSTRAINT file_assets_linked_entity_type_check
    CHECK (linked_entity_type IS NULL
        OR linked_entity_type IN ('course', 'lesson'));

-- Defensive: size_bytes must be non-negative.
ALTER TABLE file_assets
    ADD CONSTRAINT file_assets_size_nonneg
    CHECK (size_bytes >= 0);
```

### What stays as-is (already in Phase 1a)

- `file_assets` table, RLS+FORCE, `file_assets_tenant_isolation` policy, `file_assets_owner_idx`, `file_assets_linked_idx` — all from Migration 0009.
- `courses.cover_asset_id` (nullable), `lessons.video_asset_id` (nullable), `lessons.type CHECK` accepting all four types.

### How `file_bundle` lessons are modeled (no schema change)

A lesson with `type = 'file_bundle'` has zero or more rows in `file_assets` where:
- `linked_entity_type = 'lesson'`
- `linked_entity_id = <lesson.id>`
- `tenant_id = <lesson.tenant_id>` (RLS enforces)
- `status = 'available'` (the application filters out `pending`/`failed`)

Display order: `created_at ASC` — first-attached-first-displayed. A future requirement for custom ordering would add a `sort_order` column.

### Object-key format

```
{tenant_id_simple}/{yyyy}/{mm}/{file_assets.id_simple}/{sanitized_filename}
```

`_simple` is the UUID without dashes (32 hex). Filename is for human readability in the MinIO console only — never used as a database lookup key.

### Pure helpers (`services::file_assets`)

```rust
pub fn sanitize_filename(input: &str) -> String;  // strips path seps, control chars; collapses spaces; truncates 200; preserves extension
pub fn object_key(tenant_id: Uuid, asset_id: Uuid, filename: &str, now: DateTime<Utc>) -> String;
pub fn validate_request(purpose: &str, content_type: &str, size_bytes: i64) -> Result<(), ValidateError>;
```

### Validation matrix

| `purpose` | `linked_entity_type` | Allowed `content_type` | Max `size_bytes` |
|---|---|---|---|
| `"cover"` | `"course"` | `image/jpeg`, `image/png`, `image/webp` | 5 MB (5_242_880) |
| `"video"` | `"lesson"` | `video/mp4`, `video/webm` | 500 MB (524_288_000) |
| `"attachment"` | `"lesson"` | `application/pdf`, `application/vnd.openxmlformats-officedocument.wordprocessingml.document`, `application/vnd.openxmlformats-officedocument.spreadsheetml.sheet`, `application/vnd.openxmlformats-officedocument.presentationml.presentation`, `application/zip`, `text/plain`, `text/csv`, `image/jpeg`, `image/png`, `image/webp`, `audio/mpeg`, `video/mp4` | 100 MB (104_857_600) |

---

## Section 4 — API Surface

All under `/v1/`, all return JSON, all require valid Firebase ID token.

### New routes

| Method | Path | Auth | Notes |
|---|---|---|---|
| `POST` | `/v1/uploads/begin` | per-`(linked_entity_type, purpose)` dispatch | Body: `{ filename, content_type, size_bytes, linked_entity_type, linked_entity_id, purpose }`. Returns `{ asset_id, presigned_put_url, expires_at }`. The presigned URL has the exact `Content-Type` header baked in; client must `PUT` with that header. |
| `POST` | `/v1/uploads/:asset_id/complete` | uploader (`file_assets.owner_user_id == ctx.user_id`) | Body empty. HEADs the object; verifies size matches `file_assets.size_bytes`, content-type matches. On success: `status='available'`. On size mismatch: `status='failed'` and `UploadObjectMissing` error. Idempotent on already-`available` rows. |
| `GET` | `/v1/file-assets/:id/url` | linked-entity reader (course-scoped) | Returns `{ url, expires_at }` for a 15-min presigned GET. Frontend re-fetches on every display (URL is short-lived). |
| `DELETE` | `/v1/file-assets/:id` | linked-entity admin | Sets `status='pruned'` in DB; calls S3 `delete_object`; emits audit. DB row stays for history; only the blob disappears. |
| `GET` | `/v1/lessons/:lid/files` | course member or org_admin | Lists `status='available'` `file_assets` linked to this lesson. Sorted `created_at ASC`. |

### Modified routes

| Method | Path | Change |
|---|---|---|
| `PATCH` | `/v1/courses/:id` | `PatchCourse` gains `cover_asset_id: DoubleOption<Uuid>`. Validation: if `Some(Some(id))`, asset must exist, same tenant, `status='available'`, `linked_entity_type='course' AND linked_entity_id=:id`. `Some(None)` clears (sets to NULL). |
| `POST` | `/v1/courses/:cid/modules/:mid/lessons` | Type rejection lifted: all four types accepted. `type='video'` may omit `video_asset_id`. |
| `PATCH` | `/v1/courses/:cid/modules/:mid/lessons/:lid` | `PatchLesson.video_asset_id` becomes `DoubleOption`. Same validation as cover above (asset must be lesson-linked, same tenant, available). |

### Authorization dispatch (`/v1/uploads/begin`)

```rust
match (body.linked_entity_type.as_str(), body.purpose.as_str()) {
    ("course", "cover") => {
        // course owner or org_admin
        require_admin_for_course(pool, &ctx, body.linked_entity_id).await?;
        validate_request("cover", &body.content_type, body.size_bytes)?;
    }
    ("lesson", "video") => {
        let lesson = db::lessons::fetch_lesson(pool, body.linked_entity_id).await?
            .ok_or(ApiError::NotFound)?;
        require_admin_for_course(pool, &ctx, lesson.course_id).await?;
        if lesson.r#type != "video" {
            return Err(ApiError::BadRequest("lesson is not type 'video'".into()));
        }
        validate_request("video", &body.content_type, body.size_bytes)?;
    }
    ("lesson", "attachment") => {
        let lesson = db::lessons::fetch_lesson(pool, body.linked_entity_id).await?
            .ok_or(ApiError::NotFound)?;
        require_admin_for_course(pool, &ctx, lesson.course_id).await?;
        if lesson.r#type != "file_bundle" {
            return Err(ApiError::BadRequest("lesson is not type 'file_bundle'".into()));
        }
        validate_request("attachment", &body.content_type, body.size_bytes)?;
    }
    _ => return Err(ApiError::BadRequest("unsupported (entity_type, purpose) pair".into())),
}
```

### GET-URL authorization

| `linked_entity_type` | `purpose` (inferred) | Who can read |
|---|---|---|
| `'course'` | cover | course member or org_admin |
| `'lesson'` | video | course member or org_admin |
| `'lesson'` | attachment | course member or org_admin |

### Permission matrix (recap)

| Role | Begin (cover) | Begin (video / attachment) | Complete | Get URL | List lesson files | Delete asset |
|---|---|---|---|---|---|---|
| `org_admin` | any course in tenant | any course in tenant | only as uploader | as course member | as course member | any in tenant |
| `teacher` (course owner) | own | own | only as uploader | as member | as member | own |
| `teacher` (other) | — | — | — | as member | as member | — |
| `ta` | — | — | — | as member | as member | — |
| `student` | — | — | — | as member | as member | — |

### New `ApiError` variants

```rust
#[error("file asset not found")]
FileAssetNotFound,                              // 404
#[error("upload validation failed: {0}")]
UploadValidationFailed(String),                 // 400, with structured reason
#[error("upload object missing or size mismatch")]
UploadObjectMissing,                            // 400
```

---

## Section 5 — UI Surface (web)

All in `shell-web` via `features-courses`. **No new shell-web routes** — all editing happens inside existing pages.

### Component additions in `features-courses/src/`

- `file_picker.rs` — generic XHR upload widget. Props: `accept_types: Vec<String>`, `max_size_bytes: u64`, `purpose: String`, `linked_entity_type: String`, `linked_entity_id: Uuid`, `on_uploaded: EventHandler<Uuid>`. Uses `web-sys::XmlHttpRequest` for native upload-progress events. Submodule `validation` exposes pure `client_side_check` mirroring backend matrix for instant feedback.
- `course_cover_editor.rs` — wraps `FilePicker` constrained to image types + 5MB; shows current cover via `FileAssetImage`. On upload, fires `on_changed(asset_id)` so parent does `PATCH /v1/courses/:id`.
- `lesson_video_editor.rs` — type='video' branch. Two states: no video → picker; has video → `<video controls>` + replace button.
- `lesson_files_editor.rs` — type='file_bundle' branch. Lists current attachments (`FileCard` per row) + multi-file picker (sequential upload) + per-file delete.
- `file_asset_image.rs` — async fetch GET URL + render `<img>`. Loading and error states inline. Re-fetches on every mount.
- `lesson_outline_view.rs` — student-side display, type-switched: rich_text → markdown render, video → `<video>`, live_session → "Joins at <time>" link, file_bundle → list of `FileCard` download links.

### Modified components

- `lesson_editor.rs` — type switch grows from 2 to 4 branches.
- `course_builder.rs` — show video/file_bundle types in the tree (1a's hide-rule removed).
- `course_list.rs` — cards display cover image when set.
- `course_detail.rs` — cover banner; Edit tab adds `CourseCoverEditor`; Outline tab uses `LessonOutlineView`.

### New design-system primitives

- `ProgressBar` — determinate linear bar, `value: f32` (0.0..=1.0), `label: Option<String>`.
- `FileCard` — icon + filename + formatted size + optional delete button; click `on_open`.

### Authorization-aware UI

Replace/Add/Delete buttons hidden when `caller_can_admin_course == false`. GET URL fetches that 403 surface inline as "you don't have access to this file".

### Out of scope for 1b-α UI

- Transcoding / thumbnail generation.
- Drag-drop reordering of file_bundle attachments.
- Shared multi-upload queue (sequential only).
- Mobile picker UI.
- Resumable / multi-part uploads.

---

## Section 6 — Testing Strategy

### Backend pure unit tests

`services::file_assets`:

- `sanitize_filename` — table-driven across path-sep / control-char / spaces / truncation cases.
- `object_key` — deterministic key formatter; pure.
- `validate_request` — table-driven over `(purpose, content_type, size_bytes)` matrix; both pass and fail directions.

`storage::mock`:

- One unit test verifies the mock records calls correctly.

### Backend integration tests

All against real Postgres on `localhost:55432`. Mock S3 injected via `router_for_tests_with_storage(pool, storage)`.

`tests/uploads.rs`:
- `begin_returns_presigned_url_and_pending_row`
- `begin_rejects_oversized_request`
- `begin_rejects_bad_content_type`
- `complete_marks_available_when_head_matches`
- `complete_marks_failed_when_size_mismatch`
- `complete_by_non_uploader_rejected`
- `complete_is_idempotent`

`tests/uploads_validation_matrix.rs`:
- One table-driven test iterating the full validation matrix end-to-end through `/v1/uploads/begin`.

`tests/course_cover_upload.rs`:
- `teacher_uploads_cover_then_patches_course`
- `student_cannot_begin_cover_upload`
- `cross_tenant_cover_asset_id_rejected_by_patch`
- `nullable_cover_asset_id_can_be_cleared`

`tests/lesson_attachments.rs`:
- `create_file_bundle_lesson_attach_two_files_list_returns_two`
- `delete_attached_file_removes_from_list_and_calls_s3_delete`
- `course_member_can_list_attachments`
- `non_member_cannot_list_attachments`
- `lesson_video_replaces_video_asset_id`

`tests/rls_tenant_isolation.rs` extended:
- New test fires `GET /v1/file-assets/:id/url` as tenant B against a tenant-A asset, asserts 404.

### Frontend tests

`features-courses::file_picker::validation`:
- `client_side_check` — mirrors backend matrix; pure, table-driven.

SSR-render tests (inline per component):
- `course_cover_editor_renders_picker_when_no_cover`
- `course_cover_editor_renders_preview_and_replace_when_set`
- `lesson_video_editor_no_asset_renders_picker`
- `lesson_video_editor_with_asset_renders_video_element`
- `lesson_files_editor_empty_list_renders_add_button`
- `file_card_renders_filename_and_size`

These render the static initial state — they don't exercise the async fetch (would need a browser-level fetch mock; defer).

### CI gates

- `cargo test --workspace` green.
- `cargo build -p shell-web --target wasm32-unknown-unknown` green.
- `cargo check -p shell-mobile --target aarch64-linux-android` green.
- `dx build --platform web` runs only on tagged releases.

### What testing does NOT cover at 1b-α

- Real MinIO bytes round-trip (manual exit-checklist gate).
- XHR upload progress events (manual).
- Concurrent multi-file uploads (sequential only at 1b-α).
- Resumable / multi-part uploads (out of scope).
- Image transcoding / thumbnail generation (out of scope).

---

## Section 7 — Phase 1b-α Exit Checklist (high-level)

Full checklist lives with the implementation plan.

1. **Stack health** — `docker compose up -d`, all services healthy, MinIO bucket `aulalite` exists.
2. **Migration applied** — `20260508000011_tighten_file_assets_links` shows in `_sqlx_migrations`.
3. **Automated** — `cargo test --workspace` green; wasm32 + android target builds green.
4. **Course cover flow (web)** — sign in as teacher → open course → Edit tab → upload an image → confirm preview → confirm course list shows cover + course detail header banner displays.
5. **Lesson video flow (web)** — create a `'video'` lesson → upload an mp4 → confirm `<video controls>` plays in lesson outline.
6. **Lesson file_bundle flow (web)** — create a `'file_bundle'` lesson → attach 3 files → confirm file list displays in lesson outline → click each → download.
7. **Permissions** — non-owner teacher cannot upload cover for someone else's course; student cannot upload anything; cross-tenant asset references rejected by PATCH.
8. **Cleanup** — DELETE an asset; confirm `status='pruned'` in DB; confirm MinIO console shows the object is gone; confirm GET URL fetch now 404s.

Tag `phase-1b-alpha-complete` only after every required check passes.

---

## Open questions / risks / next steps

1. **`aws-sdk-s3` binary size impact.** Pulling in the AWS SDK adds ~3 MB to the backend binary. Acceptable for a server binary; not relevant for shell-web (which doesn't depend on it). Worth noting in case future tightening of binary size becomes a goal.
2. **MinIO presigned-URL signature compatibility.** `aws-sdk-s3` defaults to AWS Signature V4 with virtual-host-style addressing; MinIO supports both V4 and path-style. We'll force path-style via `force_path_style(true)` on the client builder. If MinIO ever rejects a signed URL, the fix is in that one place.
3. **Browser CORS on direct PUT to MinIO.** The MinIO container needs a CORS policy that allows `PUT` from `app.elementors.guru` (and `localhost:3000` for dev). The bootstrap step in `main.rs` should set the CORS policy on the `aulalite` bucket if not already set. This is a small extra startup step worth flagging as a known sharp edge.
4. **5GB MinIO single-PUT cap.** `aws-sdk-s3` and MinIO both support multi-part uploads; we don't use them at 1b-α. A single-shot `PUT` >5GB will fail. The 500MB video cap is well under this, so not a practical issue, but call it out.
5. **`DoubleOption` introduction.** Adds `serde_with` as a workspace dep. One-time cost for clean nullable-PATCH semantics. Worth it.
6. **Janitor sweep deferred to 1b-δ.** Until that ships, operators occasionally need to run the documented SQL to clean up orphaned `pending` rows. If the rate of orphans is high enough to be annoying before 1b-δ lands, we'd add a cron stub earlier — track but don't pre-build.
