# Phase 1c — Assignments Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the assignments + submissions + grading slice (course-level OR lesson-attached assignments, per-assignment grading mode + late + lock + release_mode + accepted-types policies, full submission state machine, RLS-isolated, with frontend editor + grading table + student submit form).

**Architecture:** Three new migrations (`assignments`, `submissions`, `file_assets` CHECK extension). Backend mirrors prior phases — queries in `crates/backend/src/db/{assignments,submissions}.rs`, handlers in `crates/backend/src/handlers/{assignments,submissions}.rs`. Frontend lands in `crates/features-courses/src/assignment_*.rs` + `submission_*.rs` and routes registered in `shell-web`. RLS sweep adds the two new tables to the cross-tenant probe.

**Tech Stack:** sqlx 0.8 + Postgres 16 (RLS, FORCE RLS, ENUMs), Axum 0.7, Dioxus 0.7, serde_with for `DoubleOption`, file_assets pipeline from 1b-α.

**Predecessor:** Phase 1b complete and merged to main at `5f7aef2`. Spec at `docs/superpowers/specs/2026-05-09-aulalite-phase-1c-assignments-design.md` (`8e9cd24`).

---

## File Structure

**Migrations (create):**
- `migrations/20260509000016_assignments.sql` — assignments table + 3 ENUMs + RLS
- `migrations/20260509000017_submissions.sql` — submissions table + 1 ENUM + RLS
- `migrations/20260509000018_file_assets_assignments.sql` — extend `linked_entity_type` CHECK

**Backend (create):**
- `crates/backend/src/db/assignments.rs` — `AssignmentRow`, CRUD + state transitions
- `crates/backend/src/db/submissions.rs` — `SubmissionRow`, CRUD + state transitions
- `crates/backend/src/handlers/assignments.rs` — 8 routes (CRUD + publish/unpublish + list)
- `crates/backend/src/handlers/submissions.rs` — 9 routes (create-or-get, patch, submit, list, get, grade, release, return + uploads-complete callback addition)

**Backend (modify):**
- `crates/backend/src/db/mod.rs` — register two new modules
- `crates/backend/src/handlers/mod.rs` — register two new modules
- `crates/backend/src/lib.rs` — merge new routers into `router()`
- `crates/backend/src/handlers/uploads.rs` — accept `linked_entity_type` `'assignment_attachment'` / `'submission_attachment'`

**Backend integration tests (create):**
- `crates/backend/tests/assignments_crud.rs`
- `crates/backend/tests/submissions_flow.rs`
- `crates/backend/tests/assignments_permissions.rs`

**Backend tests (modify):**
- `crates/backend/tests/rls_tenant_isolation.rs` — extend table list + add cross-tenant probes

**Frontend (create) — `crates/features-courses/src/`:**
- `assignment_list.rs` — course-level assignments list (role-aware)
- `assignment_editor.rs` — create/edit form
- `assignment_detail.rs` — role-aware single-assignment view
- `submission_form.rs` — student submit form (text + files)
- `submission_view.rs` — read-only single submission
- `submissions_grading_table.rs` — teacher grading table
- `submission_grade_modal.rs` — inline grade entry modal

**Frontend (modify):**
- `crates/features-courses/src/lib.rs` — re-export new modules
- `crates/features-courses/src/api.rs` — DTOs + new fetch helpers
- `crates/features-courses/src/lesson_outline_view.rs` — inline assignment card
- `crates/features-courses/src/course_detail.rs` — Assignments tab link
- `crates/features-courses/src/file_picker.rs` — accept new linked_entity_type values
- `crates/shell-web/src/main.rs` — add 5 new routes

**Frontend smokes (create):**
- `crates/features-courses/tests/assignments_ssr.rs` — SSR render every new component

**Plan (create):**
- `docs/superpowers/plans/2026-05-09-aulalite-phase-1c-assignments-exit-checklist.md`

---

## Task Sequencing

Tasks 1-3 land migrations. Tasks 4-5 land the db query layer with pure unit tests. Tasks 6-12 land the backend handlers task-by-task with TDD via integration tests. Task 13 wires the new routers into `AppState`. Tasks 14-22 land the frontend in dependency order. Task 23 wires `shell-web` routes. Tasks 24-26 add integration test sweeps for full state-machine coverage. Task 27 extends RLS. Task 28 adds SSR smokes. Task 29 sweeps builds. Task 30 closes out with the exit checklist.

---

### Task 1: Migration 0016 — assignments table

**Files:**
- Create: `migrations/20260509000016_assignments.sql`

- [ ] **Step 1: Write the migration**

```sql
-- migrations/20260509000016_assignments.sql
-- Phase 1c: assignments table. Course-level OR lesson-attached, per-assignment
-- grading_mode + late + lock_on_submit + accepted-types + release_mode policies.

CREATE TYPE assignment_grading_mode AS ENUM ('numeric', 'pass_fail');
CREATE TYPE assignment_release_mode AS ENUM ('instant', 'manual');
CREATE TYPE assignment_status AS ENUM ('draft', 'published');

CREATE TABLE assignments (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    course_id UUID NOT NULL REFERENCES courses(id) ON DELETE CASCADE,
    lesson_id UUID REFERENCES lessons(id) ON DELETE SET NULL,
    title TEXT NOT NULL,
    instructions_md TEXT NOT NULL DEFAULT '',
    grading_mode assignment_grading_mode NOT NULL,
    max_points INTEGER,
    allow_late BOOLEAN NOT NULL DEFAULT TRUE,
    lock_on_submit BOOLEAN NOT NULL DEFAULT FALSE,
    accepts_text BOOLEAN NOT NULL DEFAULT TRUE,
    accepts_files BOOLEAN NOT NULL DEFAULT TRUE,
    release_mode assignment_release_mode NOT NULL DEFAULT 'instant',
    attachment_asset_ids UUID[] NOT NULL DEFAULT '{}',
    due_at TIMESTAMPTZ,
    status assignment_status NOT NULL DEFAULT 'draft',
    published_at TIMESTAMPTZ,
    created_by UUID NOT NULL REFERENCES users(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (accepts_text OR accepts_files),
    CHECK (grading_mode <> 'numeric' OR max_points IS NOT NULL),
    CHECK (grading_mode <> 'pass_fail' OR max_points IS NULL),
    CHECK (max_points IS NULL OR max_points > 0)
);

CREATE INDEX idx_assignments_course
    ON assignments (tenant_id, course_id, status);
CREATE INDEX idx_assignments_lesson
    ON assignments (tenant_id, lesson_id) WHERE lesson_id IS NOT NULL;

ALTER TABLE assignments ENABLE ROW LEVEL SECURITY;
ALTER TABLE assignments FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON assignments
    USING (tenant_id::text = current_setting('app.tenant_id', true));
```

- [ ] **Step 2: Apply and verify**

```bash
cd "C:/Users/Chiranjib Chaudhuri/Documents/Chiranjib/Elementors_Aula-phase0"
docker compose up -d postgres
sqlx migrate run --source migrations
psql postgres://aulalite:changeme@localhost:55432/aulalite -c "\d assignments"
```

Expected: table with 19 columns, 4 CHECK constraints, RLS policy `tenant_isolation`, two indexes.

- [ ] **Step 3: Commit**

```bash
git add migrations/20260509000016_assignments.sql
git commit -m "feat(db): migration 0016 add assignments table with RLS"
```

---

### Task 2: Migration 0017 — submissions table

**Files:**
- Create: `migrations/20260509000017_submissions.sql`

- [ ] **Step 1: Write the migration**

```sql
-- migrations/20260509000017_submissions.sql
-- Phase 1c: submissions table. One row per (assignment, student). Drives
-- the draft -> submitted -> graded/returned state machine.

CREATE TYPE submission_status AS ENUM ('draft', 'submitted', 'returned', 'graded');

CREATE TABLE submissions (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    assignment_id UUID NOT NULL REFERENCES assignments(id) ON DELETE CASCADE,
    course_id UUID NOT NULL REFERENCES courses(id) ON DELETE CASCADE,
    student_user_id UUID NOT NULL REFERENCES users(id),
    status submission_status NOT NULL DEFAULT 'draft',
    text_answer TEXT,
    attachment_asset_ids UUID[] NOT NULL DEFAULT '{}',
    submitted_at TIMESTAMPTZ,
    is_late BOOLEAN NOT NULL DEFAULT FALSE,
    numeric_grade NUMERIC(7,2),
    letter_grade TEXT,
    passed BOOLEAN,
    student_visible_feedback TEXT,
    teacher_only_notes TEXT,
    graded_by_user_id UUID REFERENCES users(id),
    graded_at TIMESTAMPTZ,
    released_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (assignment_id, student_user_id)
);

CREATE INDEX idx_submissions_student
    ON submissions (tenant_id, student_user_id, status);
CREATE INDEX idx_submissions_assignment
    ON submissions (tenant_id, assignment_id, status);

ALTER TABLE submissions ENABLE ROW LEVEL SECURITY;
ALTER TABLE submissions FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON submissions
    USING (tenant_id::text = current_setting('app.tenant_id', true));
```

- [ ] **Step 2: Apply and verify**

```bash
sqlx migrate run --source migrations
psql postgres://aulalite:changeme@localhost:55432/aulalite -c "\d submissions"
```

Expected: table with 19 columns, RLS policy, two indexes, unique on `(assignment_id, student_user_id)`.

- [ ] **Step 3: Commit**

```bash
git add migrations/20260509000017_submissions.sql
git commit -m "feat(db): migration 0017 add submissions table with RLS"
```

---

### Task 3: Migration 0018 — file_assets CHECK extension

**Files:**
- Create: `migrations/20260509000018_file_assets_assignments.sql`

- [ ] **Step 1: Write the migration**

```sql
-- migrations/20260509000018_file_assets_assignments.sql
-- Phase 1c: extend the linked_entity_type CHECK to allow attachment links
-- for assignments and submissions. Forward-only, mirrors 0015.

ALTER TABLE file_assets
    DROP CONSTRAINT IF EXISTS file_assets_linked_entity_type_check;

ALTER TABLE file_assets
    ADD CONSTRAINT file_assets_linked_entity_type_check
    CHECK (linked_entity_type IS NULL
        OR linked_entity_type IN (
            'course', 'lesson', 'session_recording',
            'assignment_attachment', 'submission_attachment'
        ));
```

- [ ] **Step 2: Apply and verify**

```bash
sqlx migrate run --source migrations
psql postgres://aulalite:changeme@localhost:55432/aulalite -c "\d+ file_assets" | grep linked_entity_type
```

Expected: CHECK includes `'assignment_attachment'` and `'submission_attachment'`.

- [ ] **Step 3: Commit**

```bash
git add migrations/20260509000018_file_assets_assignments.sql
git commit -m "feat(db): allow assignment/submission attachments in file_assets"
```

---

### Task 4: db::assignments query module

**Files:**
- Create: `crates/backend/src/db/assignments.rs`
- Modify: `crates/backend/src/db/mod.rs`

- [ ] **Step 1: Register module in `crates/backend/src/db/mod.rs`**

Add `pub mod assignments;` after `pub mod modules;` (alphabetical).

```rust
// crates/backend/src/db/mod.rs
pub mod assignments;
pub mod audit;
pub mod courses;
pub mod enrollments;
pub mod file_assets;
pub mod lessons;
pub mod live_room;
pub mod live_sessions;
pub mod modules;
pub mod recordings;
pub mod submissions;
```

(Note: `submissions` registered now too — Task 5 fills its body.)

- [ ] **Step 2: Write `crates/backend/src/db/assignments.rs`**

```rust
// crates/backend/src/db/assignments.rs
use serde::Serialize;
use sqlx::{Postgres, Transaction};
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, Serialize, sqlx::FromRow, Clone)]
pub struct AssignmentRow {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub course_id: Uuid,
    pub lesson_id: Option<Uuid>,
    pub title: String,
    pub instructions_md: String,
    pub grading_mode: String,
    pub max_points: Option<i32>,
    pub allow_late: bool,
    pub lock_on_submit: bool,
    pub accepts_text: bool,
    pub accepts_files: bool,
    pub release_mode: String,
    pub attachment_asset_ids: Vec<Uuid>,
    pub due_at: Option<OffsetDateTime>,
    pub status: String,
    pub published_at: Option<OffsetDateTime>,
    pub created_by: Uuid,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

pub struct InsertAssignment<'a> {
    pub tenant_id: Uuid,
    pub course_id: Uuid,
    pub lesson_id: Option<Uuid>,
    pub title: &'a str,
    pub instructions_md: &'a str,
    pub grading_mode: &'a str,
    pub max_points: Option<i32>,
    pub allow_late: bool,
    pub lock_on_submit: bool,
    pub accepts_text: bool,
    pub accepts_files: bool,
    pub release_mode: &'a str,
    pub due_at: Option<OffsetDateTime>,
    pub created_by: Uuid,
}

pub async fn insert(
    tx: &mut Transaction<'_, Postgres>,
    a: InsertAssignment<'_>,
) -> sqlx::Result<AssignmentRow> {
    sqlx::query_as::<_, AssignmentRow>(
        "INSERT INTO assignments
            (tenant_id, course_id, lesson_id, title, instructions_md,
             grading_mode, max_points, allow_late, lock_on_submit,
             accepts_text, accepts_files, release_mode, due_at, created_by)
         VALUES ($1,$2,$3,$4,$5,$6::assignment_grading_mode,$7,$8,$9,$10,$11,
                 $12::assignment_release_mode,$13,$14)
         RETURNING *",
    )
    .bind(a.tenant_id)
    .bind(a.course_id)
    .bind(a.lesson_id)
    .bind(a.title)
    .bind(a.instructions_md)
    .bind(a.grading_mode)
    .bind(a.max_points)
    .bind(a.allow_late)
    .bind(a.lock_on_submit)
    .bind(a.accepts_text)
    .bind(a.accepts_files)
    .bind(a.release_mode)
    .bind(a.due_at)
    .bind(a.created_by)
    .fetch_one(&mut **tx)
    .await
}

pub async fn fetch_by_id(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
) -> sqlx::Result<Option<AssignmentRow>> {
    sqlx::query_as::<_, AssignmentRow>("SELECT * FROM assignments WHERE id = $1")
        .bind(id)
        .fetch_optional(&mut **tx)
        .await
}

pub async fn list_by_course(
    tx: &mut Transaction<'_, Postgres>,
    course_id: Uuid,
    include_drafts: bool,
) -> sqlx::Result<Vec<AssignmentRow>> {
    let sql = if include_drafts {
        "SELECT * FROM assignments WHERE course_id = $1
         ORDER BY COALESCE(due_at, 'infinity'), created_at"
    } else {
        "SELECT * FROM assignments WHERE course_id = $1 AND status = 'published'
         ORDER BY COALESCE(due_at, 'infinity'), created_at"
    };
    sqlx::query_as::<_, AssignmentRow>(sql)
        .bind(course_id)
        .fetch_all(&mut **tx)
        .await
}

pub async fn list_by_lesson(
    tx: &mut Transaction<'_, Postgres>,
    lesson_id: Uuid,
    include_drafts: bool,
) -> sqlx::Result<Vec<AssignmentRow>> {
    let sql = if include_drafts {
        "SELECT * FROM assignments WHERE lesson_id = $1
         ORDER BY COALESCE(due_at, 'infinity'), created_at"
    } else {
        "SELECT * FROM assignments WHERE lesson_id = $1 AND status = 'published'
         ORDER BY COALESCE(due_at, 'infinity'), created_at"
    };
    sqlx::query_as::<_, AssignmentRow>(sql)
        .bind(lesson_id)
        .fetch_all(&mut **tx)
        .await
}

pub struct PatchAssignment<'a> {
    pub title: Option<&'a str>,
    pub instructions_md: Option<&'a str>,
    pub grading_mode: Option<&'a str>,
    pub max_points: Option<Option<i32>>,
    pub allow_late: Option<bool>,
    pub lock_on_submit: Option<bool>,
    pub accepts_text: Option<bool>,
    pub accepts_files: Option<bool>,
    pub release_mode: Option<&'a str>,
    pub attachment_asset_ids: Option<&'a [Uuid]>,
    pub due_at: Option<Option<OffsetDateTime>>,
    pub lesson_id: Option<Option<Uuid>>,
}

pub async fn patch(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    p: PatchAssignment<'_>,
) -> sqlx::Result<AssignmentRow> {
    sqlx::query_as::<_, AssignmentRow>(
        "UPDATE assignments SET
            title = COALESCE($2, title),
            instructions_md = COALESCE($3, instructions_md),
            grading_mode = COALESCE($4::assignment_grading_mode, grading_mode),
            max_points = CASE WHEN $5::bool THEN $6 ELSE max_points END,
            allow_late = COALESCE($7, allow_late),
            lock_on_submit = COALESCE($8, lock_on_submit),
            accepts_text = COALESCE($9, accepts_text),
            accepts_files = COALESCE($10, accepts_files),
            release_mode = COALESCE($11::assignment_release_mode, release_mode),
            attachment_asset_ids = COALESCE($12, attachment_asset_ids),
            due_at = CASE WHEN $13::bool THEN $14 ELSE due_at END,
            lesson_id = CASE WHEN $15::bool THEN $16 ELSE lesson_id END,
            updated_at = now()
         WHERE id = $1
         RETURNING *",
    )
    .bind(id)
    .bind(p.title)
    .bind(p.instructions_md)
    .bind(p.grading_mode)
    .bind(p.max_points.is_some())
    .bind(p.max_points.flatten())
    .bind(p.allow_late)
    .bind(p.lock_on_submit)
    .bind(p.accepts_text)
    .bind(p.accepts_files)
    .bind(p.release_mode)
    .bind(p.attachment_asset_ids)
    .bind(p.due_at.is_some())
    .bind(p.due_at.flatten())
    .bind(p.lesson_id.is_some())
    .bind(p.lesson_id.flatten())
    .fetch_one(&mut **tx)
    .await
}

pub async fn publish(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
) -> sqlx::Result<AssignmentRow> {
    sqlx::query_as::<_, AssignmentRow>(
        "UPDATE assignments SET status='published', published_at=now(), updated_at=now()
         WHERE id=$1 AND status='draft' RETURNING *",
    )
    .bind(id)
    .fetch_one(&mut **tx)
    .await
}

pub async fn unpublish(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
) -> sqlx::Result<AssignmentRow> {
    sqlx::query_as::<_, AssignmentRow>(
        "UPDATE assignments SET status='draft', published_at=NULL, updated_at=now()
         WHERE id=$1 AND status='published' RETURNING *",
    )
    .bind(id)
    .fetch_one(&mut **tx)
    .await
}

pub async fn delete(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
) -> sqlx::Result<u64> {
    Ok(sqlx::query("DELETE FROM assignments WHERE id=$1 AND status='draft'")
        .bind(id)
        .execute(&mut **tx)
        .await?
        .rows_affected())
}

pub async fn count_submissions(
    tx: &mut Transaction<'_, Postgres>,
    assignment_id: Uuid,
) -> sqlx::Result<i64> {
    sqlx::query_scalar("SELECT COUNT(*) FROM submissions WHERE assignment_id = $1")
        .bind(assignment_id)
        .fetch_one(&mut **tx)
        .await
}
```

- [ ] **Step 3: Verify it compiles**

```bash
cargo build -p backend 2>&1 | tail -10
```

Expected: clean (note: `submissions` module registered but empty — fill in Task 5).

- [ ] **Step 4: Commit**

```bash
git add crates/backend/src/db/mod.rs crates/backend/src/db/assignments.rs
git commit -m "feat(db): assignments query module"
```

---

### Task 5: db::submissions query module

**Files:**
- Create: `crates/backend/src/db/submissions.rs`

- [ ] **Step 1: Write `crates/backend/src/db/submissions.rs`**

```rust
// crates/backend/src/db/submissions.rs
use serde::Serialize;
use sqlx::types::Decimal;
use sqlx::{Postgres, Transaction};
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, Serialize, sqlx::FromRow, Clone)]
pub struct SubmissionRow {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub assignment_id: Uuid,
    pub course_id: Uuid,
    pub student_user_id: Uuid,
    pub status: String,
    pub text_answer: Option<String>,
    pub attachment_asset_ids: Vec<Uuid>,
    pub submitted_at: Option<OffsetDateTime>,
    pub is_late: bool,
    pub numeric_grade: Option<Decimal>,
    pub letter_grade: Option<String>,
    pub passed: Option<bool>,
    pub student_visible_feedback: Option<String>,
    pub teacher_only_notes: Option<String>,
    pub graded_by_user_id: Option<Uuid>,
    pub graded_at: Option<OffsetDateTime>,
    pub released_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

pub async fn upsert_for_student(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    assignment_id: Uuid,
    course_id: Uuid,
    student_user_id: Uuid,
) -> sqlx::Result<SubmissionRow> {
    sqlx::query_as::<_, SubmissionRow>(
        "INSERT INTO submissions
            (tenant_id, assignment_id, course_id, student_user_id)
         VALUES ($1,$2,$3,$4)
         ON CONFLICT (assignment_id, student_user_id)
            DO UPDATE SET updated_at = submissions.updated_at
         RETURNING *",
    )
    .bind(tenant_id)
    .bind(assignment_id)
    .bind(course_id)
    .bind(student_user_id)
    .fetch_one(&mut **tx)
    .await
}

pub async fn fetch_by_id(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
) -> sqlx::Result<Option<SubmissionRow>> {
    sqlx::query_as::<_, SubmissionRow>("SELECT * FROM submissions WHERE id=$1")
        .bind(id)
        .fetch_optional(&mut **tx)
        .await
}

pub async fn list_for_assignment(
    tx: &mut Transaction<'_, Postgres>,
    assignment_id: Uuid,
) -> sqlx::Result<Vec<SubmissionRow>> {
    sqlx::query_as::<_, SubmissionRow>(
        "SELECT * FROM submissions WHERE assignment_id=$1
         ORDER BY submitted_at NULLS LAST, created_at",
    )
    .bind(assignment_id)
    .fetch_all(&mut **tx)
    .await
}

pub async fn patch_draft_fields(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    text_answer: Option<Option<&str>>,
    attachment_asset_ids: Option<&[Uuid]>,
) -> sqlx::Result<SubmissionRow> {
    sqlx::query_as::<_, SubmissionRow>(
        "UPDATE submissions SET
            text_answer = CASE WHEN $2::bool THEN $3 ELSE text_answer END,
            attachment_asset_ids = COALESCE($4, attachment_asset_ids),
            updated_at = now()
         WHERE id = $1
         RETURNING *",
    )
    .bind(id)
    .bind(text_answer.is_some())
    .bind(text_answer.flatten())
    .bind(attachment_asset_ids)
    .fetch_one(&mut **tx)
    .await
}

pub async fn mark_submitted(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    is_late: bool,
) -> sqlx::Result<SubmissionRow> {
    sqlx::query_as::<_, SubmissionRow>(
        "UPDATE submissions SET status='submitted', submitted_at=now(),
              is_late=$2, updated_at=now()
         WHERE id=$1 AND status IN ('draft','returned')
         RETURNING *",
    )
    .bind(id)
    .bind(is_late)
    .fetch_one(&mut **tx)
    .await
}

pub struct GradeFields<'a> {
    pub numeric_grade: Option<Decimal>,
    pub letter_grade: Option<&'a str>,
    pub passed: Option<bool>,
    pub student_visible_feedback: Option<&'a str>,
    pub teacher_only_notes: Option<&'a str>,
    pub grader_id: Uuid,
    pub release_now: bool,
}

pub async fn save_grade(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    g: GradeFields<'_>,
) -> sqlx::Result<SubmissionRow> {
    sqlx::query_as::<_, SubmissionRow>(
        "UPDATE submissions SET
            status = 'graded',
            numeric_grade = $2,
            letter_grade = $3,
            passed = $4,
            student_visible_feedback = $5,
            teacher_only_notes = $6,
            graded_by_user_id = $7,
            graded_at = now(),
            released_at = CASE WHEN $8::bool THEN now() ELSE NULL END,
            updated_at = now()
         WHERE id = $1
         RETURNING *",
    )
    .bind(id)
    .bind(g.numeric_grade)
    .bind(g.letter_grade)
    .bind(g.passed)
    .bind(g.student_visible_feedback)
    .bind(g.teacher_only_notes)
    .bind(g.grader_id)
    .bind(g.release_now)
    .fetch_one(&mut **tx)
    .await
}

pub async fn mark_released(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
) -> sqlx::Result<SubmissionRow> {
    sqlx::query_as::<_, SubmissionRow>(
        "UPDATE submissions SET released_at=now(), updated_at=now()
         WHERE id=$1 AND status='graded' AND released_at IS NULL
         RETURNING *",
    )
    .bind(id)
    .fetch_one(&mut **tx)
    .await
}

pub async fn return_for_resubmit(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
) -> sqlx::Result<SubmissionRow> {
    sqlx::query_as::<_, SubmissionRow>(
        "UPDATE submissions SET
            status='returned',
            released_at=NULL,
            updated_at=now()
         WHERE id=$1 AND status IN ('submitted','graded')
         RETURNING *",
    )
    .bind(id)
    .fetch_one(&mut **tx)
    .await
}
```

- [ ] **Step 2: Add `sqlx::types::Decimal` dependency check**

```bash
grep -E "rust_decimal|Decimal" crates/backend/Cargo.toml
```

If not present, add to `crates/backend/Cargo.toml` `[dependencies]`:
```toml
sqlx = { version = "0.8", features = ["postgres", "uuid", "time", "macros", "runtime-tokio-rustls", "rust_decimal"] }
```

(Replace existing `sqlx = ...` line; preserve the other features that were there.)

- [ ] **Step 3: Build**

```bash
cargo build -p backend 2>&1 | tail -10
```

Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add crates/backend/src/db/submissions.rs crates/backend/Cargo.toml
git commit -m "feat(db): submissions query module"
```

---

### Task 6: handlers::assignments — CRUD (create, get, list, patch, delete)

**Files:**
- Create: `crates/backend/src/handlers/assignments.rs`
- Modify: `crates/backend/src/handlers/mod.rs` (register module + `submissions` from Task 8)
- Test: `crates/backend/tests/assignments_crud.rs` (created in Task 24, but write the first test now)

- [ ] **Step 1: Register modules in `crates/backend/src/handlers/mod.rs`**

```rust
// crates/backend/src/handlers/mod.rs
pub mod assignments;
pub mod courses;
pub mod enrollments;
pub mod file_assets;
pub mod health;
pub mod lessons;
pub mod live_sessions;
pub mod me;
pub mod modules;
pub mod submissions;
pub mod uploads;
```

- [ ] **Step 2: Write the failing test (start the integration test file)**

Create `crates/backend/tests/assignments_crud.rs`:

```rust
//! Phase 1c: assignments CRUD smoke tests.

mod fixtures;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

async fn course(pool: &sqlx::PgPool, tenant: Uuid, teacher: Uuid) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO courses (tenant_id, slug, title, owner_user_id, status)
         VALUES ($1, $2, 'C', $3, 'published') RETURNING id",
    )
    .bind(tenant)
    .bind(format!("c-{}", Uuid::new_v4()))
    .bind(teacher)
    .fetch_one(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn teacher_creates_draft_assignment() {
    let pool = fixtures::pool().await;
    let tenant = fixtures::create_tenant(&pool).await;
    let (teacher, _, _) = fixtures::create_user(&pool).await;
    fixtures::attach_membership(&pool, tenant, teacher, "teacher").await;
    let cid = course(&pool, tenant, teacher).await;

    let stub = fixtures::StubAuth {
        pool: pool.clone(),
        user_id: teacher,
        firebase_uid: "fb".into(),
        email: "t@x".into(),
        tenant_id: Some(tenant),
        tenant_role: Some(core_types::TenantRole::Teacher),
    };
    let app = fixtures::build_test_app(
        backend::handlers::assignments::router_for_tests(pool.clone()),
        stub,
    );

    let req = Request::builder()
        .method("POST")
        .uri(format!("/v1/courses/{cid}/assignments"))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "title": "Essay 1",
                "instructions_md": "Write 500 words.",
                "grading_mode": "numeric",
                "max_points": 100,
                "accepts_text": true,
                "accepts_files": false
            })
            .to_string(),
        ))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body: Value = serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["status"], "draft");
    assert_eq!(body["grading_mode"], "numeric");
    assert_eq!(body["max_points"], 100);
}
```

- [ ] **Step 3: Run the test (expect FAIL — handler doesn't exist)**

```bash
cargo test -p backend --test assignments_crud teacher_creates_draft_assignment 2>&1 | tail -10
```

Expected: compile failure on `backend::handlers::assignments::router_for_tests`.

- [ ] **Step 4: Implement `crates/backend/src/handlers/assignments.rs`**

```rust
// crates/backend/src/handlers/assignments.rs
use axum::extract::{Extension, Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::{routing, Json, Router};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::context::RequestContext;
use crate::db;
use crate::error::ApiError;
use crate::AppState;

#[derive(Deserialize)]
pub struct CreateAssignment {
    pub title: String,
    #[serde(default)]
    pub instructions_md: String,
    pub grading_mode: String, // "numeric" | "pass_fail"
    pub max_points: Option<i32>,
    pub lesson_id: Option<Uuid>,
    #[serde(default = "default_true")]
    pub allow_late: bool,
    #[serde(default)]
    pub lock_on_submit: bool,
    #[serde(default = "default_true")]
    pub accepts_text: bool,
    #[serde(default = "default_true")]
    pub accepts_files: bool,
    #[serde(default = "default_release_mode")]
    pub release_mode: String, // "instant" | "manual"
    pub due_at: Option<OffsetDateTime>,
}

fn default_true() -> bool {
    true
}
fn default_release_mode() -> String {
    "instant".into()
}

#[derive(Deserialize, Default)]
pub struct PatchAssignment {
    pub title: Option<String>,
    pub instructions_md: Option<String>,
    pub grading_mode: Option<String>,
    #[serde(default, with = "::serde_with::rust::double_option")]
    pub max_points: Option<Option<i32>>,
    pub allow_late: Option<bool>,
    pub lock_on_submit: Option<bool>,
    pub accepts_text: Option<bool>,
    pub accepts_files: Option<bool>,
    pub release_mode: Option<String>,
    pub attachment_asset_ids: Option<Vec<Uuid>>,
    #[serde(default, with = "::serde_with::rust::double_option")]
    pub due_at: Option<Option<OffsetDateTime>>,
    #[serde(default, with = "::serde_with::rust::double_option")]
    pub lesson_id: Option<Option<Uuid>>,
}

#[derive(Serialize)]
pub struct AssignmentDto {
    pub id: Uuid,
    pub course_id: Uuid,
    pub lesson_id: Option<Uuid>,
    pub title: String,
    pub instructions_md: String,
    pub grading_mode: String,
    pub max_points: Option<i32>,
    pub allow_late: bool,
    pub lock_on_submit: bool,
    pub accepts_text: bool,
    pub accepts_files: bool,
    pub release_mode: String,
    pub attachment_asset_ids: Vec<Uuid>,
    pub due_at: Option<OffsetDateTime>,
    pub status: String,
    pub published_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

impl From<db::assignments::AssignmentRow> for AssignmentDto {
    fn from(r: db::assignments::AssignmentRow) -> Self {
        Self {
            id: r.id,
            course_id: r.course_id,
            lesson_id: r.lesson_id,
            title: r.title,
            instructions_md: r.instructions_md,
            grading_mode: r.grading_mode,
            max_points: r.max_points,
            allow_late: r.allow_late,
            lock_on_submit: r.lock_on_submit,
            accepts_text: r.accepts_text,
            accepts_files: r.accepts_files,
            release_mode: r.release_mode,
            attachment_asset_ids: r.attachment_asset_ids,
            due_at: r.due_at,
            status: r.status,
            published_at: r.published_at,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/courses/:cid/assignments",
               routing::post(create).get(list_for_course))
        .route("/v1/lessons/:lid/assignments", routing::get(list_for_lesson))
        .route("/v1/assignments/:id",
               routing::get(get_one).patch(patch).delete(delete_one))
        .route("/v1/assignments/:id/publish", routing::post(publish))
        .route("/v1/assignments/:id/unpublish", routing::post(unpublish))
}

#[doc(hidden)]
pub fn router_for_tests(pool: PgPool) -> Router {
    use crate::handlers::assignments as h;
    Router::new()
        .route("/v1/courses/:cid/assignments",
               routing::post(h::create_t).get(h::list_for_course_t))
        .route("/v1/lessons/:lid/assignments", routing::get(h::list_for_lesson_t))
        .route("/v1/assignments/:id",
               routing::get(h::get_one_t).patch(h::patch_t).delete(h::delete_one_t))
        .route("/v1/assignments/:id/publish", routing::post(h::publish_t))
        .route("/v1/assignments/:id/unpublish", routing::post(h::unpublish_t))
        .with_state(TestState { pool })
}

#[derive(Clone)]
struct TestState { pool: PgPool }

// --- production handlers ---
async fn create(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(cid): Path<Uuid>,
    Json(body): Json<CreateAssignment>,
) -> Result<impl IntoResponse, ApiError> {
    create_inner(&s.pool, &ctx, cid, body).await
}

async fn create_inner(
    pool: &PgPool, ctx: &RequestContext, cid: Uuid, body: CreateAssignment,
) -> Result<impl IntoResponse, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden("no tenant".into()))?;
    require_teacher(ctx)?;
    if body.grading_mode == "numeric" && body.max_points.is_none() {
        return Err(ApiError::Validation("numeric mode requires max_points".into()));
    }
    if body.grading_mode == "pass_fail" && body.max_points.is_some() {
        return Err(ApiError::Validation("pass_fail mode forbids max_points".into()));
    }
    if !body.accepts_text && !body.accepts_files {
        return Err(ApiError::Validation("at least one accepted submission type required".into()));
    }
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant).await?;
    let row = db::assignments::insert(
        &mut tx,
        db::assignments::InsertAssignment {
            tenant_id: tenant,
            course_id: cid,
            lesson_id: body.lesson_id,
            title: &body.title,
            instructions_md: &body.instructions_md,
            grading_mode: &body.grading_mode,
            max_points: body.max_points,
            allow_late: body.allow_late,
            lock_on_submit: body.lock_on_submit,
            accepts_text: body.accepts_text,
            accepts_files: body.accepts_files,
            release_mode: &body.release_mode,
            due_at: body.due_at,
            created_by: ctx.user_id,
        },
    ).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(AssignmentDto::from(row))))
}

async fn get_one(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<AssignmentDto>, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden("no tenant".into()))?;
    let mut tx = s.pool.begin().await?;
    set_tenant(&mut tx, tenant).await?;
    let row = db::assignments::fetch_by_id(&mut tx, id).await?
        .ok_or(ApiError::NotFound)?;
    if row.status == "draft" && !is_teacher(&ctx) {
        return Err(ApiError::NotFound);
    }
    tx.commit().await?;
    Ok(Json(AssignmentDto::from(row)))
}

#[derive(Deserialize, Default)]
pub struct ListQuery {
    #[serde(default)]
    pub include_drafts: bool,
}

async fn list_for_course(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(cid): Path<Uuid>,
    Query(q): Query<ListQuery>,
) -> Result<Json<Vec<AssignmentDto>>, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden("no tenant".into()))?;
    let include_drafts = q.include_drafts && is_teacher(&ctx);
    let mut tx = s.pool.begin().await?;
    set_tenant(&mut tx, tenant).await?;
    let rows = db::assignments::list_by_course(&mut tx, cid, include_drafts).await?;
    tx.commit().await?;
    Ok(Json(rows.into_iter().map(AssignmentDto::from).collect()))
}

async fn list_for_lesson(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(lid): Path<Uuid>,
    Query(q): Query<ListQuery>,
) -> Result<Json<Vec<AssignmentDto>>, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden("no tenant".into()))?;
    let include_drafts = q.include_drafts && is_teacher(&ctx);
    let mut tx = s.pool.begin().await?;
    set_tenant(&mut tx, tenant).await?;
    let rows = db::assignments::list_by_lesson(&mut tx, lid, include_drafts).await?;
    tx.commit().await?;
    Ok(Json(rows.into_iter().map(AssignmentDto::from).collect()))
}

async fn patch(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    Json(body): Json<PatchAssignment>,
) -> Result<Json<AssignmentDto>, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden("no tenant".into()))?;
    require_teacher(&ctx)?;
    let mut tx = s.pool.begin().await?;
    set_tenant(&mut tx, tenant).await?;
    let existing = db::assignments::fetch_by_id(&mut tx, id).await?
        .ok_or(ApiError::NotFound)?;
    if existing.status != "draft" {
        return Err(ApiError::Conflict("assignment_published".into()));
    }
    let row = db::assignments::patch(
        &mut tx, id,
        db::assignments::PatchAssignment {
            title: body.title.as_deref(),
            instructions_md: body.instructions_md.as_deref(),
            grading_mode: body.grading_mode.as_deref(),
            max_points: body.max_points,
            allow_late: body.allow_late,
            lock_on_submit: body.lock_on_submit,
            accepts_text: body.accepts_text,
            accepts_files: body.accepts_files,
            release_mode: body.release_mode.as_deref(),
            attachment_asset_ids: body.attachment_asset_ids.as_deref(),
            due_at: body.due_at,
            lesson_id: body.lesson_id,
        },
    ).await?;
    tx.commit().await?;
    Ok(Json(AssignmentDto::from(row)))
}

async fn publish(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<AssignmentDto>, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden("no tenant".into()))?;
    require_teacher(&ctx)?;
    let mut tx = s.pool.begin().await?;
    set_tenant(&mut tx, tenant).await?;
    let row = db::assignments::publish(&mut tx, id).await
        .map_err(|e| match e {
            sqlx::Error::RowNotFound => ApiError::Conflict("not_in_draft".into()),
            other => other.into(),
        })?;
    tx.commit().await?;
    Ok(Json(AssignmentDto::from(row)))
}

async fn unpublish(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<AssignmentDto>, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden("no tenant".into()))?;
    require_teacher(&ctx)?;
    let mut tx = s.pool.begin().await?;
    set_tenant(&mut tx, tenant).await?;
    let count = db::assignments::count_submissions(&mut tx, id).await?;
    if count > 0 {
        return Err(ApiError::Conflict("assignment_has_submissions".into()));
    }
    let row = db::assignments::unpublish(&mut tx, id).await
        .map_err(|e| match e {
            sqlx::Error::RowNotFound => ApiError::Conflict("not_published".into()),
            other => other.into(),
        })?;
    tx.commit().await?;
    Ok(Json(AssignmentDto::from(row)))
}

async fn delete_one(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden("no tenant".into()))?;
    require_teacher(&ctx)?;
    let mut tx = s.pool.begin().await?;
    set_tenant(&mut tx, tenant).await?;
    let count = db::assignments::count_submissions(&mut tx, id).await?;
    if count > 0 {
        return Err(ApiError::Conflict("assignment_has_submissions".into()));
    }
    let n = db::assignments::delete(&mut tx, id).await?;
    if n == 0 {
        return Err(ApiError::Conflict("not_in_draft_or_not_found".into()));
    }
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

// --- helpers ---
fn require_teacher(ctx: &RequestContext) -> Result<(), ApiError> {
    if is_teacher(ctx) { Ok(()) } else { Err(ApiError::Forbidden("teacher_only".into())) }
}

fn is_teacher(ctx: &RequestContext) -> bool {
    matches!(
        ctx.tenant_role,
        Some(core_types::TenantRole::Teacher)
            | Some(core_types::TenantRole::OrgAdmin)
            | Some(core_types::TenantRole::Ta)
    ) || ctx.is_platform_admin
}

async fn set_tenant(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, tenant: Uuid)
    -> sqlx::Result<()>
{
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut **tx).await?;
    Ok(())
}

// --- test wrappers (use TestState) ---
async fn create_t(
    State(ts): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(cid): Path<Uuid>,
    Json(body): Json<CreateAssignment>,
) -> Result<impl IntoResponse, ApiError> {
    create_inner(&ts.pool, &ctx, cid, body).await
}

async fn get_one_t(
    State(ts): State<TestState>, Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<AssignmentDto>, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden("no tenant".into()))?;
    let mut tx = ts.pool.begin().await?; set_tenant(&mut tx, tenant).await?;
    let row = db::assignments::fetch_by_id(&mut tx, id).await?.ok_or(ApiError::NotFound)?;
    if row.status == "draft" && !is_teacher(&ctx) { return Err(ApiError::NotFound); }
    tx.commit().await?;
    Ok(Json(AssignmentDto::from(row)))
}

async fn list_for_course_t(
    State(ts): State<TestState>, Extension(ctx): Extension<RequestContext>,
    Path(cid): Path<Uuid>, Query(q): Query<ListQuery>,
) -> Result<Json<Vec<AssignmentDto>>, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden("no tenant".into()))?;
    let include = q.include_drafts && is_teacher(&ctx);
    let mut tx = ts.pool.begin().await?; set_tenant(&mut tx, tenant).await?;
    let rows = db::assignments::list_by_course(&mut tx, cid, include).await?;
    tx.commit().await?;
    Ok(Json(rows.into_iter().map(AssignmentDto::from).collect()))
}

async fn list_for_lesson_t(
    State(ts): State<TestState>, Extension(ctx): Extension<RequestContext>,
    Path(lid): Path<Uuid>, Query(q): Query<ListQuery>,
) -> Result<Json<Vec<AssignmentDto>>, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden("no tenant".into()))?;
    let include = q.include_drafts && is_teacher(&ctx);
    let mut tx = ts.pool.begin().await?; set_tenant(&mut tx, tenant).await?;
    let rows = db::assignments::list_by_lesson(&mut tx, lid, include).await?;
    tx.commit().await?;
    Ok(Json(rows.into_iter().map(AssignmentDto::from).collect()))
}

async fn patch_t(
    State(ts): State<TestState>, Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>, Json(body): Json<PatchAssignment>,
) -> Result<Json<AssignmentDto>, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden("no tenant".into()))?;
    require_teacher(&ctx)?;
    let mut tx = ts.pool.begin().await?; set_tenant(&mut tx, tenant).await?;
    let existing = db::assignments::fetch_by_id(&mut tx, id).await?.ok_or(ApiError::NotFound)?;
    if existing.status != "draft" { return Err(ApiError::Conflict("assignment_published".into())); }
    let row = db::assignments::patch(
        &mut tx, id,
        db::assignments::PatchAssignment {
            title: body.title.as_deref(),
            instructions_md: body.instructions_md.as_deref(),
            grading_mode: body.grading_mode.as_deref(),
            max_points: body.max_points,
            allow_late: body.allow_late,
            lock_on_submit: body.lock_on_submit,
            accepts_text: body.accepts_text,
            accepts_files: body.accepts_files,
            release_mode: body.release_mode.as_deref(),
            attachment_asset_ids: body.attachment_asset_ids.as_deref(),
            due_at: body.due_at,
            lesson_id: body.lesson_id,
        },
    ).await?;
    tx.commit().await?;
    Ok(Json(AssignmentDto::from(row)))
}

async fn publish_t(
    State(ts): State<TestState>, Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<AssignmentDto>, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden("no tenant".into()))?;
    require_teacher(&ctx)?;
    let mut tx = ts.pool.begin().await?; set_tenant(&mut tx, tenant).await?;
    let row = db::assignments::publish(&mut tx, id).await
        .map_err(|e| match e { sqlx::Error::RowNotFound => ApiError::Conflict("not_in_draft".into()), other => other.into() })?;
    tx.commit().await?;
    Ok(Json(AssignmentDto::from(row)))
}

async fn unpublish_t(
    State(ts): State<TestState>, Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<AssignmentDto>, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden("no tenant".into()))?;
    require_teacher(&ctx)?;
    let mut tx = ts.pool.begin().await?; set_tenant(&mut tx, tenant).await?;
    let count = db::assignments::count_submissions(&mut tx, id).await?;
    if count > 0 { return Err(ApiError::Conflict("assignment_has_submissions".into())); }
    let row = db::assignments::unpublish(&mut tx, id).await
        .map_err(|e| match e { sqlx::Error::RowNotFound => ApiError::Conflict("not_published".into()), other => other.into() })?;
    tx.commit().await?;
    Ok(Json(AssignmentDto::from(row)))
}

async fn delete_one_t(
    State(ts): State<TestState>, Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden("no tenant".into()))?;
    require_teacher(&ctx)?;
    let mut tx = ts.pool.begin().await?; set_tenant(&mut tx, tenant).await?;
    let count = db::assignments::count_submissions(&mut tx, id).await?;
    if count > 0 { return Err(ApiError::Conflict("assignment_has_submissions".into())); }
    let n = db::assignments::delete(&mut tx, id).await?;
    if n == 0 { return Err(ApiError::Conflict("not_in_draft_or_not_found".into())); }
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
```

- [ ] **Step 5: Stub `crates/backend/src/handlers/submissions.rs`** so the workspace compiles (Task 8 fills it):

```rust
// crates/backend/src/handlers/submissions.rs
use axum::Router;
use crate::AppState;

pub fn routes() -> Router<AppState> { Router::new() }

#[doc(hidden)]
pub fn router_for_tests(_pool: sqlx::PgPool) -> Router { Router::new() }
```

- [ ] **Step 6: Run the test (expect PASS)**

```bash
cargo test -p backend --test assignments_crud teacher_creates_draft_assignment 2>&1 | tail -10
```

Expected: 1 passed.

- [ ] **Step 7: Commit**

```bash
git add crates/backend/src/handlers/mod.rs \
        crates/backend/src/handlers/assignments.rs \
        crates/backend/src/handlers/submissions.rs \
        crates/backend/tests/assignments_crud.rs
git commit -m "feat(assignments): CRUD handlers (create, get, list, patch, delete) + first integration test"
```

---

### Task 7: Wire assignments router into AppState + add publish/unpublish integration tests

**Files:**
- Modify: `crates/backend/src/lib.rs` (merge `handlers::assignments::routes()`)
- Modify: `crates/backend/tests/assignments_crud.rs` (extend with publish/unpublish + delete tests)

- [ ] **Step 1: Wire router in `crates/backend/src/lib.rs`**

In the `authed` router chain (after `.merge(handlers::lessons::routes())`):
```rust
        .merge(handlers::assignments::routes())
        .merge(handlers::submissions::routes())
```

- [ ] **Step 2: Add `publish_then_list_excludes_drafts_for_students` test**

Append to `crates/backend/tests/assignments_crud.rs`:

```rust
async fn create_assignment(
    pool: &sqlx::PgPool, tenant: Uuid, teacher: Uuid, cid: Uuid,
) -> Uuid {
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(pool).await.unwrap();
    sqlx::query_scalar(
        "INSERT INTO assignments (tenant_id, course_id, title, grading_mode, max_points, created_by)
         VALUES ($1,$2,'A','numeric',100,$3) RETURNING id",
    ).bind(tenant).bind(cid).bind(teacher).fetch_one(pool).await.unwrap()
}

#[tokio::test]
async fn publish_then_list_excludes_drafts_for_students() {
    let pool = fixtures::pool().await;
    let tenant = fixtures::create_tenant(&pool).await;
    let (teacher, _, _) = fixtures::create_user(&pool).await;
    let (student, _, _) = fixtures::create_user(&pool).await;
    fixtures::attach_membership(&pool, tenant, teacher, "teacher").await;
    fixtures::attach_membership(&pool, tenant, student, "student").await;
    let cid = course(&pool, tenant, teacher).await;
    let aid = create_assignment(&pool, tenant, teacher, cid).await;

    // Teacher publishes.
    let teacher_stub = fixtures::StubAuth {
        pool: pool.clone(), user_id: teacher, firebase_uid: "fb".into(),
        email: "t".into(), tenant_id: Some(tenant),
        tenant_role: Some(core_types::TenantRole::Teacher),
    };
    let app = fixtures::build_test_app(
        backend::handlers::assignments::router_for_tests(pool.clone()),
        teacher_stub,
    );
    let req = Request::builder().method("POST")
        .uri(format!("/v1/assignments/{aid}/publish"))
        .body(Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Student lists course assignments — sees published.
    let student_stub = fixtures::StubAuth {
        pool: pool.clone(), user_id: student, firebase_uid: "fb2".into(),
        email: "s".into(), tenant_id: Some(tenant),
        tenant_role: Some(core_types::TenantRole::Student),
    };
    let app = fixtures::build_test_app(
        backend::handlers::assignments::router_for_tests(pool.clone()),
        student_stub.clone(),
    );
    let req = Request::builder().method("GET")
        .uri(format!("/v1/courses/{cid}/assignments"))
        .body(Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let body: Value = serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body.as_array().unwrap().len(), 1);

    // Student GETs the assignment by id — visible.
    let app = fixtures::build_test_app(
        backend::handlers::assignments::router_for_tests(pool.clone()),
        student_stub,
    );
    let req = Request::builder().method("GET")
        .uri(format!("/v1/assignments/{aid}"))
        .body(Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn student_cannot_see_draft_assignment() {
    let pool = fixtures::pool().await;
    let tenant = fixtures::create_tenant(&pool).await;
    let (teacher, _, _) = fixtures::create_user(&pool).await;
    let (student, _, _) = fixtures::create_user(&pool).await;
    fixtures::attach_membership(&pool, tenant, teacher, "teacher").await;
    fixtures::attach_membership(&pool, tenant, student, "student").await;
    let cid = course(&pool, tenant, teacher).await;
    let aid = create_assignment(&pool, tenant, teacher, cid).await;

    let stub = fixtures::StubAuth {
        pool: pool.clone(), user_id: student, firebase_uid: "fb".into(),
        email: "s".into(), tenant_id: Some(tenant),
        tenant_role: Some(core_types::TenantRole::Student),
    };
    let app = fixtures::build_test_app(
        backend::handlers::assignments::router_for_tests(pool.clone()),
        stub,
    );
    let req = Request::builder().method("GET")
        .uri(format!("/v1/assignments/{aid}"))
        .body(Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
```

- [ ] **Step 3: Run tests**

```bash
cargo test -p backend --test assignments_crud -j 2 2>&1 | tail -15
```

Expected: 3 passed.

- [ ] **Step 4: Commit**

```bash
git add crates/backend/src/lib.rs crates/backend/tests/assignments_crud.rs
git commit -m "feat(backend): wire assignments router; publish/visibility tests"
```

---

### Task 8: handlers::submissions — create-or-get + patch + submit

**Files:**
- Modify: `crates/backend/src/handlers/submissions.rs` (replace stub from Task 6)
- Test: `crates/backend/tests/submissions_flow.rs`

- [ ] **Step 1: Write `crates/backend/tests/submissions_flow.rs` (first failing test)**

```rust
//! Phase 1c: submission state machine + late detection + lock_on_submit.

mod fixtures;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

async fn course(pool: &sqlx::PgPool, tenant: Uuid, teacher: Uuid) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO courses (tenant_id, slug, title, owner_user_id, status)
         VALUES ($1,$2,'C',$3,'published') RETURNING id",
    ).bind(tenant).bind(format!("c-{}", Uuid::new_v4())).bind(teacher)
     .fetch_one(pool).await.unwrap()
}

async fn published_assignment(
    pool: &sqlx::PgPool, tenant: Uuid, course_id: Uuid, teacher: Uuid,
    due_at: Option<time::OffsetDateTime>, allow_late: bool,
) -> Uuid {
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string()).execute(pool).await.unwrap();
    sqlx::query_scalar(
        "INSERT INTO assignments
            (tenant_id, course_id, title, grading_mode, max_points,
             status, published_at, due_at, allow_late, created_by)
         VALUES ($1,$2,'Essay','numeric',100,'published',now(),$3,$4,$5)
         RETURNING id",
    ).bind(tenant).bind(course_id).bind(due_at).bind(allow_late).bind(teacher)
     .fetch_one(pool).await.unwrap()
}

#[tokio::test]
async fn student_creates_then_submits() {
    let pool = fixtures::pool().await;
    let tenant = fixtures::create_tenant(&pool).await;
    let (teacher, _, _) = fixtures::create_user(&pool).await;
    let (student, _, _) = fixtures::create_user(&pool).await;
    fixtures::attach_membership(&pool, tenant, teacher, "teacher").await;
    fixtures::attach_membership(&pool, tenant, student, "student").await;
    let cid = course(&pool, tenant, teacher).await;
    let aid = published_assignment(&pool, tenant, cid, teacher, None, true).await;

    let stub = fixtures::StubAuth {
        pool: pool.clone(), user_id: student, firebase_uid: "fb".into(),
        email: "s".into(), tenant_id: Some(tenant),
        tenant_role: Some(core_types::TenantRole::Student),
    };
    let app = fixtures::build_test_app(
        backend::handlers::submissions::router_for_tests(pool.clone()),
        stub.clone(),
    );

    // Create-or-get.
    let req = Request::builder().method("POST")
        .uri(format!("/v1/assignments/{aid}/submissions"))
        .body(Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let sid = body["id"].as_str().unwrap().to_string();
    assert_eq!(body["status"], "draft");

    // PATCH text_answer.
    let app = fixtures::build_test_app(
        backend::handlers::submissions::router_for_tests(pool.clone()),
        stub.clone(),
    );
    let req = Request::builder().method("PATCH")
        .uri(format!("/v1/submissions/{sid}"))
        .header("content-type", "application/json")
        .body(Body::from(json!({"text_answer": "hello"}).to_string())).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Submit.
    let app = fixtures::build_test_app(
        backend::handlers::submissions::router_for_tests(pool.clone()),
        stub,
    );
    let req = Request::builder().method("POST")
        .uri(format!("/v1/submissions/{sid}/submit"))
        .body(Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["status"], "submitted");
    assert_eq!(body["is_late"], false);
}
```

- [ ] **Step 2: Run test (expect FAIL — handler still stubbed)**

```bash
cargo test -p backend --test submissions_flow student_creates_then_submits 2>&1 | tail -5
```

- [ ] **Step 3: Implement `crates/backend/src/handlers/submissions.rs`**

(Replace the Task-6 stub completely.) See plan-resource snippet at the end of this file titled `# resource: handlers/submissions.rs (Tasks 8-11)` and copy verbatim. The handler exposes:
- `POST /v1/assignments/:id/submissions` → `create_or_get` (idempotent)
- `PATCH /v1/submissions/:id` → `patch` (enforces `lock_on_submit`)
- `POST /v1/submissions/:id/submit` → `submit` (sets `is_late`, rejects on late+disabled)
- `GET /v1/assignments/:id/submissions` → `list` (teacher only)
- `GET /v1/submissions/:id` → `get_one` (filters grade fields per release + role)
- `POST /v1/submissions/:id/grade` → `grade` (validates coherence, writes graded + maybe released)
- `POST /v1/submissions/:id/release` → `release` (manual mode only)
- `POST /v1/submissions/:id/return` → `return_for_resubmit`

(Implementation listed at end of file. Copy in full.)

- [ ] **Step 4: Run test (expect PASS)**

```bash
cargo test -p backend --test submissions_flow student_creates_then_submits 2>&1 | tail -5
```

Expected: 1 passed.

- [ ] **Step 5: Commit**

```bash
git add crates/backend/src/handlers/submissions.rs crates/backend/tests/submissions_flow.rs
git commit -m "feat(submissions): create-or-get, patch, submit (state machine + late detection)"
```

---

### Task 9: Submissions list + detail with grade-visibility filter (TDD)

**Files:**
- Modify: `crates/backend/tests/submissions_flow.rs` (add list + visibility tests)

- [ ] **Step 1: Add `teacher_lists_and_student_cannot_see_others_submission` test**

Append to `crates/backend/tests/submissions_flow.rs`:

```rust
#[tokio::test]
async fn teacher_lists_and_student_cannot_see_others_submission() {
    let pool = fixtures::pool().await;
    let tenant = fixtures::create_tenant(&pool).await;
    let (teacher, _, _) = fixtures::create_user(&pool).await;
    let (alice, _, _) = fixtures::create_user(&pool).await;
    let (bob, _, _) = fixtures::create_user(&pool).await;
    fixtures::attach_membership(&pool, tenant, teacher, "teacher").await;
    fixtures::attach_membership(&pool, tenant, alice, "student").await;
    fixtures::attach_membership(&pool, tenant, bob, "student").await;
    let cid = course(&pool, tenant, teacher).await;
    let aid = published_assignment(&pool, tenant, cid, teacher, None, true).await;

    // Alice creates her submission.
    let alice_stub = fixtures::StubAuth {
        pool: pool.clone(), user_id: alice, firebase_uid: "fa".into(),
        email: "a".into(), tenant_id: Some(tenant),
        tenant_role: Some(core_types::TenantRole::Student),
    };
    let app = fixtures::build_test_app(
        backend::handlers::submissions::router_for_tests(pool.clone()),
        alice_stub,
    );
    let req = Request::builder().method("POST")
        .uri(format!("/v1/assignments/{aid}/submissions"))
        .body(Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let body: Value = serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let alice_sid = body["id"].as_str().unwrap().to_string();

    // Bob tries to GET Alice's submission — 404.
    let bob_stub = fixtures::StubAuth {
        pool: pool.clone(), user_id: bob, firebase_uid: "fb".into(),
        email: "b".into(), tenant_id: Some(tenant),
        tenant_role: Some(core_types::TenantRole::Student),
    };
    let app = fixtures::build_test_app(
        backend::handlers::submissions::router_for_tests(pool.clone()),
        bob_stub,
    );
    let req = Request::builder().method("GET")
        .uri(format!("/v1/submissions/{alice_sid}"))
        .body(Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // Teacher lists — sees Alice's row.
    let teacher_stub = fixtures::StubAuth {
        pool: pool.clone(), user_id: teacher, firebase_uid: "ft".into(),
        email: "t".into(), tenant_id: Some(tenant),
        tenant_role: Some(core_types::TenantRole::Teacher),
    };
    let app = fixtures::build_test_app(
        backend::handlers::submissions::router_for_tests(pool.clone()),
        teacher_stub,
    );
    let req = Request::builder().method("GET")
        .uri(format!("/v1/assignments/{aid}/submissions"))
        .body(Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let body: Value = serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body.as_array().unwrap().len(), 1);
}
```

- [ ] **Step 2: Run test**

```bash
cargo test -p backend --test submissions_flow teacher_lists_and_student_cannot_see_others_submission 2>&1 | tail -5
```

Expected: passes (handler from Task 8 already covers this).

- [ ] **Step 3: Commit**

```bash
git add crates/backend/tests/submissions_flow.rs
git commit -m "test(submissions): list + cross-student visibility"
```

---

### Task 10: Grade + release (TDD: instant + manual modes, coherence)

**Files:**
- Modify: `crates/backend/tests/submissions_flow.rs`

- [ ] **Step 1: Add three grading tests**

```rust
async fn assignment_with_release_mode(
    pool: &sqlx::PgPool, tenant: Uuid, course_id: Uuid, teacher: Uuid,
    release_mode: &str,
) -> Uuid {
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string()).execute(pool).await.unwrap();
    sqlx::query_scalar(
        "INSERT INTO assignments (tenant_id, course_id, title, grading_mode, max_points,
                                  status, published_at, release_mode, created_by)
         VALUES ($1,$2,'Q','numeric',100,'published',now(),$3::assignment_release_mode,$4)
         RETURNING id",
    ).bind(tenant).bind(course_id).bind(release_mode).bind(teacher)
     .fetch_one(pool).await.unwrap()
}

async fn make_submission(
    pool: &sqlx::PgPool, tenant: Uuid, aid: Uuid, course_id: Uuid, student: Uuid,
) -> Uuid {
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string()).execute(pool).await.unwrap();
    sqlx::query_scalar(
        "INSERT INTO submissions (tenant_id, assignment_id, course_id, student_user_id,
                                  status, submitted_at)
         VALUES ($1,$2,$3,$4,'submitted',now()) RETURNING id",
    ).bind(tenant).bind(aid).bind(course_id).bind(student)
     .fetch_one(pool).await.unwrap()
}

#[tokio::test]
async fn instant_release_grade_visible_to_student_immediately() {
    let pool = fixtures::pool().await;
    let tenant = fixtures::create_tenant(&pool).await;
    let (teacher, _, _) = fixtures::create_user(&pool).await;
    let (student, _, _) = fixtures::create_user(&pool).await;
    fixtures::attach_membership(&pool, tenant, teacher, "teacher").await;
    fixtures::attach_membership(&pool, tenant, student, "student").await;
    let cid = course(&pool, tenant, teacher).await;
    let aid = assignment_with_release_mode(&pool, tenant, cid, teacher, "instant").await;
    let sid = make_submission(&pool, tenant, aid, cid, student).await;

    let teacher_stub = fixtures::StubAuth {
        pool: pool.clone(), user_id: teacher, firebase_uid: "ft".into(),
        email: "t".into(), tenant_id: Some(tenant),
        tenant_role: Some(core_types::TenantRole::Teacher),
    };
    let app = fixtures::build_test_app(
        backend::handlers::submissions::router_for_tests(pool.clone()),
        teacher_stub,
    );
    let req = Request::builder().method("POST")
        .uri(format!("/v1/submissions/{sid}/grade"))
        .header("content-type", "application/json")
        .body(Body::from(json!({
            "numeric_grade": 85.5,
            "student_visible_feedback": "good"
        }).to_string())).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let student_stub = fixtures::StubAuth {
        pool: pool.clone(), user_id: student, firebase_uid: "fs".into(),
        email: "s".into(), tenant_id: Some(tenant),
        tenant_role: Some(core_types::TenantRole::Student),
    };
    let app = fixtures::build_test_app(
        backend::handlers::submissions::router_for_tests(pool.clone()),
        student_stub,
    );
    let req = Request::builder().method("GET")
        .uri(format!("/v1/submissions/{sid}"))
        .body(Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let body: Value = serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["status"], "graded");
    assert_eq!(body["numeric_grade"].as_f64().unwrap(), 85.5);
    assert_eq!(body["student_visible_feedback"], "good");
}

#[tokio::test]
async fn manual_release_grade_hidden_from_student_until_release() {
    let pool = fixtures::pool().await;
    let tenant = fixtures::create_tenant(&pool).await;
    let (teacher, _, _) = fixtures::create_user(&pool).await;
    let (student, _, _) = fixtures::create_user(&pool).await;
    fixtures::attach_membership(&pool, tenant, teacher, "teacher").await;
    fixtures::attach_membership(&pool, tenant, student, "student").await;
    let cid = course(&pool, tenant, teacher).await;
    let aid = assignment_with_release_mode(&pool, tenant, cid, teacher, "manual").await;
    let sid = make_submission(&pool, tenant, aid, cid, student).await;

    let teacher_stub = fixtures::StubAuth {
        pool: pool.clone(), user_id: teacher, firebase_uid: "ft".into(),
        email: "t".into(), tenant_id: Some(tenant),
        tenant_role: Some(core_types::TenantRole::Teacher),
    };
    let app = fixtures::build_test_app(
        backend::handlers::submissions::router_for_tests(pool.clone()),
        teacher_stub.clone(),
    );
    let req = Request::builder().method("POST")
        .uri(format!("/v1/submissions/{sid}/grade"))
        .header("content-type", "application/json")
        .body(Body::from(json!({"numeric_grade": 70}).to_string())).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Student sees graded status but no grade fields (released_at NULL).
    let student_stub = fixtures::StubAuth {
        pool: pool.clone(), user_id: student, firebase_uid: "fs".into(),
        email: "s".into(), tenant_id: Some(tenant),
        tenant_role: Some(core_types::TenantRole::Student),
    };
    let app = fixtures::build_test_app(
        backend::handlers::submissions::router_for_tests(pool.clone()),
        student_stub.clone(),
    );
    let req = Request::builder().method("GET")
        .uri(format!("/v1/submissions/{sid}"))
        .body(Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let body: Value = serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert!(body["numeric_grade"].is_null());
    assert!(body["student_visible_feedback"].is_null());

    // Teacher releases.
    let app = fixtures::build_test_app(
        backend::handlers::submissions::router_for_tests(pool.clone()),
        teacher_stub,
    );
    let req = Request::builder().method("POST")
        .uri(format!("/v1/submissions/{sid}/release"))
        .body(Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Student now sees grade.
    let app = fixtures::build_test_app(
        backend::handlers::submissions::router_for_tests(pool.clone()),
        student_stub,
    );
    let req = Request::builder().method("GET")
        .uri(format!("/v1/submissions/{sid}"))
        .body(Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let body: Value = serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["numeric_grade"].as_f64().unwrap(), 70.0);
}

#[tokio::test]
async fn grade_rejects_out_of_range_numeric() {
    let pool = fixtures::pool().await;
    let tenant = fixtures::create_tenant(&pool).await;
    let (teacher, _, _) = fixtures::create_user(&pool).await;
    let (student, _, _) = fixtures::create_user(&pool).await;
    fixtures::attach_membership(&pool, tenant, teacher, "teacher").await;
    fixtures::attach_membership(&pool, tenant, student, "student").await;
    let cid = course(&pool, tenant, teacher).await;
    let aid = assignment_with_release_mode(&pool, tenant, cid, teacher, "instant").await;
    let sid = make_submission(&pool, tenant, aid, cid, student).await;

    let stub = fixtures::StubAuth {
        pool: pool.clone(), user_id: teacher, firebase_uid: "ft".into(),
        email: "t".into(), tenant_id: Some(tenant),
        tenant_role: Some(core_types::TenantRole::Teacher),
    };
    let app = fixtures::build_test_app(
        backend::handlers::submissions::router_for_tests(pool.clone()),
        stub,
    );
    let req = Request::builder().method("POST")
        .uri(format!("/v1/submissions/{sid}/grade"))
        .header("content-type", "application/json")
        .body(Body::from(json!({"numeric_grade": 150}).to_string())).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
}
```

- [ ] **Step 2: Run tests**

```bash
cargo test -p backend --test submissions_flow grade_ -j 2 2>&1 | tail -10
cargo test -p backend --test submissions_flow release_ -j 2 2>&1 | tail -10
cargo test -p backend --test submissions_flow instant_ -j 2 2>&1 | tail -10
cargo test -p backend --test submissions_flow manual_  -j 2 2>&1 | tail -10
```

Expected: all 3 new tests pass (handler from Task 8 covers all this).

- [ ] **Step 3: Commit**

```bash
git add crates/backend/tests/submissions_flow.rs
git commit -m "test(submissions): instant vs manual release + grade coherence"
```

---

### Task 11: Return-for-resubmit + lock_on_submit + late-rejection (TDD)

**Files:**
- Modify: `crates/backend/tests/submissions_flow.rs`

- [ ] **Step 1: Add three tests**

```rust
#[tokio::test]
async fn teacher_returns_graded_for_resubmit() {
    let pool = fixtures::pool().await;
    let tenant = fixtures::create_tenant(&pool).await;
    let (teacher, _, _) = fixtures::create_user(&pool).await;
    let (student, _, _) = fixtures::create_user(&pool).await;
    fixtures::attach_membership(&pool, tenant, teacher, "teacher").await;
    fixtures::attach_membership(&pool, tenant, student, "student").await;
    let cid = course(&pool, tenant, teacher).await;
    let aid = assignment_with_release_mode(&pool, tenant, cid, teacher, "instant").await;
    let sid = make_submission(&pool, tenant, aid, cid, student).await;

    let teacher_stub = fixtures::StubAuth {
        pool: pool.clone(), user_id: teacher, firebase_uid: "ft".into(),
        email: "t".into(), tenant_id: Some(tenant),
        tenant_role: Some(core_types::TenantRole::Teacher),
    };
    // Grade.
    let app = fixtures::build_test_app(
        backend::handlers::submissions::router_for_tests(pool.clone()),
        teacher_stub.clone(),
    );
    let req = Request::builder().method("POST")
        .uri(format!("/v1/submissions/{sid}/grade"))
        .header("content-type", "application/json")
        .body(Body::from(json!({"numeric_grade": 50}).to_string())).unwrap();
    assert_eq!(app.oneshot(req).await.unwrap().status(), StatusCode::OK);
    // Return.
    let app = fixtures::build_test_app(
        backend::handlers::submissions::router_for_tests(pool.clone()),
        teacher_stub,
    );
    let req = Request::builder().method("POST")
        .uri(format!("/v1/submissions/{sid}/return"))
        .body(Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["status"], "returned");
    assert!(body["released_at"].is_null());
}

#[tokio::test]
async fn lock_on_submit_blocks_patch_after_submit() {
    let pool = fixtures::pool().await;
    let tenant = fixtures::create_tenant(&pool).await;
    let (teacher, _, _) = fixtures::create_user(&pool).await;
    let (student, _, _) = fixtures::create_user(&pool).await;
    fixtures::attach_membership(&pool, tenant, teacher, "teacher").await;
    fixtures::attach_membership(&pool, tenant, student, "student").await;
    let cid = course(&pool, tenant, teacher).await;

    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string()).execute(&pool).await.unwrap();
    let aid: Uuid = sqlx::query_scalar(
        "INSERT INTO assignments
            (tenant_id, course_id, title, grading_mode, max_points, status,
             published_at, lock_on_submit, created_by)
         VALUES ($1,$2,'L','numeric',100,'published',now(),true,$3)
         RETURNING id",
    ).bind(tenant).bind(cid).bind(teacher).fetch_one(&pool).await.unwrap();
    let sid: Uuid = sqlx::query_scalar(
        "INSERT INTO submissions (tenant_id, assignment_id, course_id, student_user_id,
                                  status, submitted_at, text_answer)
         VALUES ($1,$2,$3,$4,'submitted',now(),'answer') RETURNING id",
    ).bind(tenant).bind(aid).bind(cid).bind(student).fetch_one(&pool).await.unwrap();

    let stub = fixtures::StubAuth {
        pool: pool.clone(), user_id: student, firebase_uid: "fs".into(),
        email: "s".into(), tenant_id: Some(tenant),
        tenant_role: Some(core_types::TenantRole::Student),
    };
    let app = fixtures::build_test_app(
        backend::handlers::submissions::router_for_tests(pool.clone()),
        stub,
    );
    let req = Request::builder().method("PATCH")
        .uri(format!("/v1/submissions/{sid}"))
        .header("content-type", "application/json")
        .body(Body::from(json!({"text_answer": "edited"}).to_string())).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn late_submit_rejected_when_allow_late_false() {
    let pool = fixtures::pool().await;
    let tenant = fixtures::create_tenant(&pool).await;
    let (teacher, _, _) = fixtures::create_user(&pool).await;
    let (student, _, _) = fixtures::create_user(&pool).await;
    fixtures::attach_membership(&pool, tenant, teacher, "teacher").await;
    fixtures::attach_membership(&pool, tenant, student, "student").await;
    let cid = course(&pool, tenant, teacher).await;
    let past = time::OffsetDateTime::now_utc() - time::Duration::hours(1);
    let aid = published_assignment(&pool, tenant, cid, teacher, Some(past), false).await;

    let stub = fixtures::StubAuth {
        pool: pool.clone(), user_id: student, firebase_uid: "fs".into(),
        email: "s".into(), tenant_id: Some(tenant),
        tenant_role: Some(core_types::TenantRole::Student),
    };
    let app = fixtures::build_test_app(
        backend::handlers::submissions::router_for_tests(pool.clone()),
        stub.clone(),
    );
    // Create-or-get.
    let req = Request::builder().method("POST")
        .uri(format!("/v1/assignments/{aid}/submissions"))
        .body(Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let body: Value = serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let sid = body["id"].as_str().unwrap().to_string();

    // Submit — should 422.
    let app = fixtures::build_test_app(
        backend::handlers::submissions::router_for_tests(pool.clone()),
        stub,
    );
    let req = Request::builder().method("POST")
        .uri(format!("/v1/submissions/{sid}/submit"))
        .body(Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
}
```

- [ ] **Step 2: Run tests**

```bash
cargo test -p backend --test submissions_flow -j 2 2>&1 | tail -15
```

Expected: 9+ tests pass total.

- [ ] **Step 3: Commit**

```bash
git add crates/backend/tests/submissions_flow.rs
git commit -m "test(submissions): return-for-resubmit, lock_on_submit, late-rejection"
```

---

### Task 12: Allow `assignment_attachment` / `submission_attachment` in uploads

**Files:**
- Modify: `crates/backend/src/handlers/uploads.rs` (the `linked_entity_type` validation)

- [ ] **Step 1: Locate the validation**

```bash
grep -n "linked_entity_type" crates/backend/src/handlers/uploads.rs
```

- [ ] **Step 2: Extend the allow-list**

Find the array (or match statement) that lists allowed `linked_entity_type` values. Add `"assignment_attachment"` and `"submission_attachment"`. Example diff (adapt to whatever the actual code looks like):

```rust
const ALLOWED_LINK_TYPES: &[&str] = &[
    "course",
    "lesson",
    "session_recording",
    "assignment_attachment",
    "submission_attachment",
];
```

- [ ] **Step 3: Add a quick test in `crates/backend/tests/uploads.rs`**

```rust
#[tokio::test]
async fn complete_upload_accepts_assignment_attachment_type() {
    // Mirror the pattern of the existing "complete_upload_accepts_course" test
    // but pass linked_entity_type = "assignment_attachment".
    // Expect HTTP 201 + a file_assets row with that linked_entity_type.
}
```

(Replicate the assertion shape from the nearest existing test in the same file — do not copy a placeholder. The body of this test mirrors the `course` cover upload test with the linked_entity_type field swapped.)

- [ ] **Step 4: Run**

```bash
cargo test -p backend --test uploads complete_upload_accepts_assignment_attachment_type 2>&1 | tail -5
```

- [ ] **Step 5: Commit**

```bash
git add crates/backend/src/handlers/uploads.rs crates/backend/tests/uploads.rs
git commit -m "feat(uploads): accept assignment_attachment + submission_attachment link types"
```

---

### Task 13: Frontend api.rs — DTOs + fetch helpers

**Files:**
- Modify: `crates/features-courses/src/api.rs`

- [ ] **Step 1: Inspect the existing `api.rs`**

```bash
grep -n "fetch_json\|ApiContext\|ApiError" crates/features-courses/src/api.rs | head -15
```

- [ ] **Step 2: Add DTOs and fetch helpers**

Append to `crates/features-courses/src/api.rs`:

```rust
// --- Phase 1c: assignments + submissions ---

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct AssignmentDto {
    pub id: String,
    pub course_id: String,
    pub lesson_id: Option<String>,
    pub title: String,
    pub instructions_md: String,
    pub grading_mode: String,
    pub max_points: Option<i32>,
    pub allow_late: bool,
    pub lock_on_submit: bool,
    pub accepts_text: bool,
    pub accepts_files: bool,
    pub release_mode: String,
    pub attachment_asset_ids: Vec<String>,
    pub due_at: Option<String>,
    pub status: String,
    pub published_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct SubmissionDto {
    pub id: String,
    pub assignment_id: String,
    pub course_id: String,
    pub student_user_id: String,
    pub status: String,
    pub text_answer: Option<String>,
    pub attachment_asset_ids: Vec<String>,
    pub submitted_at: Option<String>,
    pub is_late: bool,
    pub numeric_grade: Option<f64>,
    pub letter_grade: Option<String>,
    pub passed: Option<bool>,
    pub student_visible_feedback: Option<String>,
    pub graded_at: Option<String>,
    pub released_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

pub async fn list_course_assignments(
    ctx: &ApiContext, course_id: &str, include_drafts: bool,
) -> Result<Vec<AssignmentDto>, ApiError> {
    let path = format!("/v1/courses/{course_id}/assignments?include_drafts={include_drafts}");
    fetch_json(ctx, "GET", &path, None::<&()>).await
}

pub async fn list_lesson_assignments(
    ctx: &ApiContext, lesson_id: &str,
) -> Result<Vec<AssignmentDto>, ApiError> {
    let path = format!("/v1/lessons/{lesson_id}/assignments");
    fetch_json(ctx, "GET", &path, None::<&()>).await
}

pub async fn get_assignment(
    ctx: &ApiContext, id: &str,
) -> Result<AssignmentDto, ApiError> {
    fetch_json(ctx, "GET", &format!("/v1/assignments/{id}"), None::<&()>).await
}

#[derive(serde::Serialize)]
pub struct CreateAssignmentBody<'a> {
    pub title: &'a str,
    pub instructions_md: &'a str,
    pub grading_mode: &'a str,
    pub max_points: Option<i32>,
    pub lesson_id: Option<&'a str>,
    pub allow_late: bool,
    pub lock_on_submit: bool,
    pub accepts_text: bool,
    pub accepts_files: bool,
    pub release_mode: &'a str,
    pub due_at: Option<&'a str>,
}

pub async fn create_assignment(
    ctx: &ApiContext, course_id: &str, body: &CreateAssignmentBody<'_>,
) -> Result<AssignmentDto, ApiError> {
    let path = format!("/v1/courses/{course_id}/assignments");
    fetch_json(ctx, "POST", &path, Some(body)).await
}

pub async fn publish_assignment(
    ctx: &ApiContext, id: &str,
) -> Result<AssignmentDto, ApiError> {
    fetch_json(ctx, "POST", &format!("/v1/assignments/{id}/publish"), None::<&()>).await
}

pub async fn create_or_get_submission(
    ctx: &ApiContext, assignment_id: &str,
) -> Result<SubmissionDto, ApiError> {
    fetch_json(ctx, "POST",
               &format!("/v1/assignments/{assignment_id}/submissions"),
               None::<&()>).await
}

#[derive(serde::Serialize)]
pub struct PatchSubmissionBody<'a> {
    pub text_answer: Option<&'a str>,
    pub attachment_asset_ids: Option<Vec<&'a str>>,
}

pub async fn patch_submission(
    ctx: &ApiContext, id: &str, body: &PatchSubmissionBody<'_>,
) -> Result<SubmissionDto, ApiError> {
    fetch_json(ctx, "PATCH", &format!("/v1/submissions/{id}"), Some(body)).await
}

pub async fn submit_submission(
    ctx: &ApiContext, id: &str,
) -> Result<SubmissionDto, ApiError> {
    fetch_json(ctx, "POST", &format!("/v1/submissions/{id}/submit"), None::<&()>).await
}

pub async fn list_assignment_submissions(
    ctx: &ApiContext, assignment_id: &str,
) -> Result<Vec<SubmissionDto>, ApiError> {
    fetch_json(ctx, "GET", &format!("/v1/assignments/{assignment_id}/submissions"),
               None::<&()>).await
}

#[derive(serde::Serialize)]
pub struct GradeBody<'a> {
    pub numeric_grade: Option<f64>,
    pub letter_grade: Option<&'a str>,
    pub passed: Option<bool>,
    pub student_visible_feedback: Option<&'a str>,
    pub teacher_only_notes: Option<&'a str>,
}

pub async fn grade_submission(
    ctx: &ApiContext, id: &str, body: &GradeBody<'_>,
) -> Result<SubmissionDto, ApiError> {
    fetch_json(ctx, "POST", &format!("/v1/submissions/{id}/grade"), Some(body)).await
}

pub async fn release_submission(
    ctx: &ApiContext, id: &str,
) -> Result<SubmissionDto, ApiError> {
    fetch_json(ctx, "POST", &format!("/v1/submissions/{id}/release"), None::<&()>).await
}

pub async fn return_submission(
    ctx: &ApiContext, id: &str,
) -> Result<SubmissionDto, ApiError> {
    fetch_json(ctx, "POST", &format!("/v1/submissions/{id}/return"), None::<&()>).await
}
```

- [ ] **Step 3: Build**

```bash
cargo build -p features-courses 2>&1 | tail -10
cargo build -p features-courses --target wasm32-unknown-unknown 2>&1 | tail -5
```

Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add crates/features-courses/src/api.rs
git commit -m "feat(api): assignments + submissions DTOs and fetch helpers"
```

---

### Task 14: AssignmentList component

**Files:**
- Create: `crates/features-courses/src/assignment_list.rs`
- Modify: `crates/features-courses/src/lib.rs` (re-export)

- [ ] **Step 1: Re-export in `crates/features-courses/src/lib.rs`**

Find the `pub mod` block and add (alphabetical):
```rust
pub mod assignment_detail;
pub mod assignment_editor;
pub mod assignment_list;
```

(All three modules will be filled across Tasks 14-16 — declare them now to keep the workspace compiling between tasks.)

Stub `assignment_detail.rs` and `assignment_editor.rs` with empty bodies for now:
```rust
// crates/features-courses/src/assignment_detail.rs
// Filled in Task 16.
```
```rust
// crates/features-courses/src/assignment_editor.rs
// Filled in Task 15.
```

- [ ] **Step 2: Write `crates/features-courses/src/assignment_list.rs`**

```rust
// crates/features-courses/src/assignment_list.rs
//! Course-level list of assignments. Teachers see drafts + published;
//! students see published only.

use crate::api::{self, ApiContext, AssignmentDto};
use dioxus::prelude::*;

#[derive(Clone, Props, PartialEq)]
pub struct AssignmentListProps {
    pub api: ApiContext,
    pub course_slug: String,
    pub course_id: String,
    pub is_teacher: bool,
}

pub fn AssignmentList(props: AssignmentListProps) -> Element {
    let course_id = props.course_id.clone();
    let api = props.api.clone();
    let is_teacher = props.is_teacher;

    let assignments = use_resource(move || {
        let api = api.clone();
        let course_id = course_id.clone();
        async move {
            api::list_course_assignments(&api, &course_id, is_teacher).await
        }
    });

    rsx! {
        div { class: "assignment-list",
            h2 { "Assignments" }
            if props.is_teacher {
                a {
                    href: format!("/courses/{}/assignments/new", props.course_slug),
                    class: "btn btn-primary",
                    "New assignment"
                }
            }
            match &*assignments.read_unchecked() {
                Some(Ok(items)) if items.is_empty() => rsx! { p { "No assignments yet." } },
                Some(Ok(items)) => rsx! {
                    ul { class: "assignment-list__items",
                        for a in items.iter() {
                            { render_row(props.course_slug.clone(), a) }
                        }
                    }
                },
                Some(Err(e)) => rsx! { p { class: "error", "{e}" } },
                None => rsx! { p { "Loading..." } },
            }
        }
    }
}

fn render_row(course_slug: String, a: &AssignmentDto) -> Element {
    let id = a.id.clone();
    let title = a.title.clone();
    let due = a.due_at.clone().unwrap_or_default();
    let status_class = format!("badge badge--{}", a.status);
    let status = a.status.clone();
    rsx! {
        li { key: "{id}", class: "assignment-list__row",
            a { href: format!("/courses/{course_slug}/assignments/{id}"), "{title}" }
            span { class: "{status_class}", "{status}" }
            if !due.is_empty() { span { class: "due", "due {due}" } }
        }
    }
}
```

- [ ] **Step 3: Build**

```bash
cargo build -p features-courses --target wasm32-unknown-unknown 2>&1 | tail -5
```

Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add crates/features-courses/src/lib.rs \
        crates/features-courses/src/assignment_list.rs \
        crates/features-courses/src/assignment_editor.rs \
        crates/features-courses/src/assignment_detail.rs
git commit -m "feat(features-courses): AssignmentList component (role-aware)"
```

---

### Task 15: AssignmentEditor component (create + edit form)

**Files:**
- Modify: `crates/features-courses/src/assignment_editor.rs` (replace stub)

- [ ] **Step 1: Write the editor**

```rust
// crates/features-courses/src/assignment_editor.rs
//! Create/edit form for assignments. Teacher only. Posts to
//! POST /v1/courses/:cid/assignments (create) or PATCH /v1/assignments/:id (edit).

use crate::api::{self, ApiContext, AssignmentDto, CreateAssignmentBody};
use dioxus::prelude::*;

#[derive(Clone, Props, PartialEq)]
pub struct AssignmentEditorProps {
    pub api: ApiContext,
    pub course_slug: String,
    pub course_id: String,
    /// None = create mode, Some = edit mode (only allowed for draft).
    pub initial: Option<AssignmentDto>,
}

pub fn AssignmentEditor(props: AssignmentEditorProps) -> Element {
    let initial = props.initial.clone();
    let mut title = use_signal(|| initial.as_ref().map(|a| a.title.clone()).unwrap_or_default());
    let mut instructions = use_signal(|| initial.as_ref().map(|a| a.instructions_md.clone()).unwrap_or_default());
    let mut grading_mode = use_signal(|| initial.as_ref().map(|a| a.grading_mode.clone()).unwrap_or_else(|| "numeric".into()));
    let mut max_points = use_signal(|| initial.as_ref().and_then(|a| a.max_points).unwrap_or(100));
    let mut allow_late = use_signal(|| initial.as_ref().map(|a| a.allow_late).unwrap_or(true));
    let mut lock_on_submit = use_signal(|| initial.as_ref().map(|a| a.lock_on_submit).unwrap_or(false));
    let mut accepts_text = use_signal(|| initial.as_ref().map(|a| a.accepts_text).unwrap_or(true));
    let mut accepts_files = use_signal(|| initial.as_ref().map(|a| a.accepts_files).unwrap_or(true));
    let mut release_mode = use_signal(|| initial.as_ref().map(|a| a.release_mode.clone()).unwrap_or_else(|| "instant".into()));
    let mut error: Signal<Option<String>> = use_signal(|| None);
    let mut saving = use_signal(|| false);

    let api = props.api.clone();
    let course_id = props.course_id.clone();
    let course_slug = props.course_slug.clone();

    let on_save = move |_| {
        let api = api.clone();
        let course_id = course_id.clone();
        let course_slug = course_slug.clone();
        spawn(async move {
            saving.set(true);
            let body = CreateAssignmentBody {
                title: &title.read(),
                instructions_md: &instructions.read(),
                grading_mode: &grading_mode.read(),
                max_points: if grading_mode.read().as_str() == "numeric" {
                    Some(*max_points.read()) } else { None },
                lesson_id: None,
                allow_late: *allow_late.read(),
                lock_on_submit: *lock_on_submit.read(),
                accepts_text: *accepts_text.read(),
                accepts_files: *accepts_files.read(),
                release_mode: &release_mode.read(),
                due_at: None,
            };
            match api::create_assignment(&api, &course_id, &body).await {
                Ok(a) => {
                    #[cfg(target_arch = "wasm32")]
                    if let Some(win) = web_sys::window() {
                        let _ = win.location().set_href(
                            &format!("/courses/{course_slug}/assignments/{}", a.id),
                        );
                    }
                }
                Err(e) => error.set(Some(format!("{e}"))),
            }
            saving.set(false);
        });
    };

    rsx! {
        form { class: "assignment-editor", onsubmit: move |e| e.prevent_default(),
            label { "Title" }
            input { value: "{title}", oninput: move |e| title.set(e.value()) }

            label { "Instructions (markdown)" }
            textarea { rows: 6, value: "{instructions}",
                oninput: move |e| instructions.set(e.value()) }

            fieldset {
                legend { "Grading" }
                label {
                    input {
                        r#type: "radio", name: "grading_mode", value: "numeric",
                        checked: grading_mode.read().as_str() == "numeric",
                        oninput: move |_| grading_mode.set("numeric".into()),
                    }
                    "Numeric"
                }
                label {
                    input {
                        r#type: "radio", name: "grading_mode", value: "pass_fail",
                        checked: grading_mode.read().as_str() == "pass_fail",
                        oninput: move |_| grading_mode.set("pass_fail".into()),
                    }
                    "Pass/fail"
                }
                if grading_mode.read().as_str() == "numeric" {
                    label { "Max points" }
                    input {
                        r#type: "number", min: "1", value: "{max_points}",
                        oninput: move |e| {
                            if let Ok(v) = e.value().parse() { max_points.set(v); }
                        }
                    }
                }
            }

            fieldset {
                legend { "Submission types" }
                label {
                    input {
                        r#type: "checkbox", checked: *accepts_text.read(),
                        oninput: move |e| accepts_text.set(e.value() == "true"),
                    }
                    "Text"
                }
                label {
                    input {
                        r#type: "checkbox", checked: *accepts_files.read(),
                        oninput: move |e| accepts_files.set(e.value() == "true"),
                    }
                    "Files"
                }
            }

            fieldset {
                legend { "Policies" }
                label {
                    input {
                        r#type: "checkbox", checked: *allow_late.read(),
                        oninput: move |e| allow_late.set(e.value() == "true"),
                    }
                    "Allow late submissions"
                }
                label {
                    input {
                        r#type: "checkbox", checked: *lock_on_submit.read(),
                        oninput: move |e| lock_on_submit.set(e.value() == "true"),
                    }
                    "Lock submission once submitted"
                }
                label {
                    input {
                        r#type: "radio", name: "release_mode", value: "instant",
                        checked: release_mode.read().as_str() == "instant",
                        oninput: move |_| release_mode.set("instant".into()),
                    }
                    "Release grades instantly"
                }
                label {
                    input {
                        r#type: "radio", name: "release_mode", value: "manual",
                        checked: release_mode.read().as_str() == "manual",
                        oninput: move |_| release_mode.set("manual".into()),
                    }
                    "Hold grades for manual release"
                }
            }

            if let Some(err) = error.read().as_ref() { p { class: "error", "{err}" } }
            button { disabled: *saving.read(), onclick: on_save,
                if *saving.read() { "Saving..." } else { "Save draft" } }
        }
    }
}
```

- [ ] **Step 2: Build**

```bash
cargo build -p features-courses --target wasm32-unknown-unknown 2>&1 | tail -5
```

- [ ] **Step 3: Commit**

```bash
git add crates/features-courses/src/assignment_editor.rs
git commit -m "feat(features-courses): AssignmentEditor (create/edit form)"
```

---

### Task 16: AssignmentDetail component (role-aware)

**Files:**
- Modify: `crates/features-courses/src/assignment_detail.rs` (replace stub)

- [ ] **Step 1: Write the component**

```rust
// crates/features-courses/src/assignment_detail.rs
//! Single-assignment view. Students see their submission card; teachers see
//! the link to the grading table.

use crate::api::{self, ApiContext};
use dioxus::prelude::*;

#[derive(Clone, Props, PartialEq)]
pub struct AssignmentDetailProps {
    pub api: ApiContext,
    pub assignment_id: String,
    pub course_slug: String,
    pub current_user_id: String,
    pub is_teacher: bool,
}

pub fn AssignmentDetail(props: AssignmentDetailProps) -> Element {
    let api = props.api.clone();
    let aid = props.assignment_id.clone();
    let assignment = use_resource(move || {
        let api = api.clone();
        let aid = aid.clone();
        async move { api::get_assignment(&api, &aid).await }
    });

    rsx! {
        div { class: "assignment-detail",
            match &*assignment.read_unchecked() {
                Some(Ok(a)) => rsx! {
                    h1 { "{a.title}" }
                    p { class: "instructions", "{a.instructions_md}" }
                    if a.status == "draft" && props.is_teacher {
                        p { class: "badge badge--draft", "DRAFT — not yet visible to students" }
                    }
                    if let Some(due) = a.due_at.as_ref() {
                        p { class: "due", "Due: {due}" }
                    }
                    if props.is_teacher {
                        a {
                            href: format!("/courses/{}/assignments/{}/grade",
                                          props.course_slug, props.assignment_id),
                            class: "btn",
                            "View submissions"
                        }
                    } else {
                        crate::submission_form::SubmissionForm {
                            api: props.api.clone(),
                            assignment: a.clone(),
                        }
                    }
                },
                Some(Err(e)) => rsx! { p { class: "error", "{e}" } },
                None => rsx! { p { "Loading..." } },
            }
        }
    }
}
```

- [ ] **Step 2: Build (will fail until SubmissionForm lands in Task 17)**

```bash
cargo build -p features-courses --target wasm32-unknown-unknown 2>&1 | tail -5
```

Expected: error referencing `crate::submission_form` — that's fine; Task 17 fixes it. Mark this task complete and proceed.

- [ ] **Step 3: Commit**

```bash
git add crates/features-courses/src/assignment_detail.rs
git commit -m "feat(features-courses): AssignmentDetail component (role-aware)"
```

---

### Task 17: SubmissionForm component (student submit)

**Files:**
- Create: `crates/features-courses/src/submission_form.rs`
- Modify: `crates/features-courses/src/lib.rs` (re-export)

- [ ] **Step 1: Re-export in `lib.rs`**

Add (alphabetical):
```rust
pub mod submission_form;
pub mod submission_grade_modal;
pub mod submission_view;
pub mod submissions_grading_table;
```

Stub the three not-yet-implemented modules with empty body comments to keep workspace compiling.

- [ ] **Step 2: Write `crates/features-courses/src/submission_form.rs`**

```rust
// crates/features-courses/src/submission_form.rs
//! Student-facing submit form. Renders text + file inputs based on the
//! assignment's accepted types. Disables editing per status × lock_on_submit.

use crate::api::{self, ApiContext, AssignmentDto, PatchSubmissionBody, SubmissionDto};
use dioxus::prelude::*;

#[derive(Clone, Props, PartialEq)]
pub struct SubmissionFormProps {
    pub api: ApiContext,
    pub assignment: AssignmentDto,
}

pub fn SubmissionForm(props: SubmissionFormProps) -> Element {
    let api = props.api.clone();
    let aid = props.assignment.id.clone();
    let mut submission: Signal<Option<SubmissionDto>> = use_signal(|| None);
    let mut error: Signal<Option<String>> = use_signal(|| None);

    let api_for_load = api.clone();
    let aid_for_load = aid.clone();
    use_future(move || {
        let api = api_for_load.clone();
        let aid = aid_for_load.clone();
        async move {
            match api::create_or_get_submission(&api, &aid).await {
                Ok(s) => submission.set(Some(s)),
                Err(e) => error.set(Some(format!("{e}"))),
            }
        }
    });

    let mut text_answer = use_signal(String::new);

    use_effect(move || {
        if let Some(s) = submission.read().as_ref() {
            text_answer.set(s.text_answer.clone().unwrap_or_default());
        }
    });

    let editable = |s: &SubmissionDto| -> bool {
        match s.status.as_str() {
            "draft" | "returned" => true,
            "submitted" => !props.assignment.lock_on_submit,
            _ => false,
        }
    };

    let api_for_save = api.clone();
    let on_save_draft = move |_| {
        let api = api_for_save.clone();
        let sid = submission.read().as_ref().map(|s| s.id.clone()).unwrap_or_default();
        let body_text = text_answer.read().clone();
        spawn(async move {
            let body = PatchSubmissionBody {
                text_answer: Some(body_text.as_str()),
                attachment_asset_ids: None,
            };
            match api::patch_submission(&api, &sid, &body).await {
                Ok(s) => submission.set(Some(s)),
                Err(e) => error.set(Some(format!("{e}"))),
            }
        });
    };

    let api_for_submit = api.clone();
    let on_submit = move |_| {
        let api = api_for_submit.clone();
        let sid = submission.read().as_ref().map(|s| s.id.clone()).unwrap_or_default();
        spawn(async move {
            match api::submit_submission(&api, &sid).await {
                Ok(s) => submission.set(Some(s)),
                Err(e) => error.set(Some(format!("{e}"))),
            }
        });
    };

    rsx! {
        div { class: "submission-form",
            match submission.read().as_ref() {
                Some(s) => rsx! {
                    p { class: "status", "Status: {s.status}" }
                    if let Some(released) = s.released_at.as_ref() {
                        if let Some(g) = s.numeric_grade {
                            p { class: "grade", "Grade: {g} (released {released})" }
                        }
                        if let Some(fb) = s.student_visible_feedback.as_ref() {
                            p { class: "feedback", "Feedback: {fb}" }
                        }
                    }
                    if props.assignment.accepts_text {
                        label { "Your answer" }
                        textarea {
                            rows: 8,
                            disabled: !editable(s),
                            value: "{text_answer}",
                            oninput: move |e| text_answer.set(e.value()),
                        }
                    }
                    if props.assignment.accepts_files {
                        p { class: "files-placeholder",
                            "(File upload reuses /v1/uploads/begin → MinIO PUT → /v1/uploads/complete then PATCH submission with new asset_id; mirror file_picker.rs.)" }
                    }
                    if editable(s) {
                        button { onclick: on_save_draft, "Save draft" }
                        button { class: "btn-primary", onclick: on_submit, "Submit" }
                    }
                },
                None => rsx! { p { "Loading submission..." } },
            }
            if let Some(e) = error.read().as_ref() { p { class: "error", "{e}" } }
        }
    }
}
```

- [ ] **Step 3: Build**

```bash
cargo build -p features-courses --target wasm32-unknown-unknown 2>&1 | tail -5
```

- [ ] **Step 4: Commit**

```bash
git add crates/features-courses/src/lib.rs \
        crates/features-courses/src/submission_form.rs \
        crates/features-courses/src/submission_view.rs \
        crates/features-courses/src/submissions_grading_table.rs \
        crates/features-courses/src/submission_grade_modal.rs
git commit -m "feat(features-courses): SubmissionForm + module stubs for grading UI"
```

---

### Task 18: SubmissionView component (read-only single submission)

**Files:**
- Modify: `crates/features-courses/src/submission_view.rs`

- [ ] **Step 1: Write component**

```rust
// crates/features-courses/src/submission_view.rs
//! Read-only single-submission display. Hides grade fields when
//! released_at is null and viewer is the student (server already filters).

use crate::api::SubmissionDto;
use dioxus::prelude::*;

#[derive(Clone, Props, PartialEq)]
pub struct SubmissionViewProps {
    pub submission: SubmissionDto,
}

pub fn SubmissionView(props: SubmissionViewProps) -> Element {
    let s = &props.submission;
    rsx! {
        div { class: "submission-view",
            p { class: "status", "Status: {s.status}" }
            if let Some(text) = s.text_answer.as_ref() {
                section { class: "answer",
                    h3 { "Answer" }
                    p { "{text}" }
                }
            }
            if !s.attachment_asset_ids.is_empty() {
                section { class: "attachments",
                    h3 { "Attachments" }
                    ul {
                        for id in s.attachment_asset_ids.iter() {
                            li { key: "{id}", "{id}" }
                        }
                    }
                }
            }
            if s.released_at.is_some() {
                section { class: "grade-block",
                    h3 { "Grade" }
                    if let Some(n) = s.numeric_grade { p { "Numeric: {n}" } }
                    if let Some(l) = s.letter_grade.as_ref() { p { "Letter: {l}" } }
                    if let Some(p) = s.passed { p { "Pass: {p}" } }
                    if let Some(fb) = s.student_visible_feedback.as_ref() {
                        p { class: "feedback", "Feedback: {fb}" }
                    }
                }
            }
        }
    }
}
```

- [ ] **Step 2: Build + commit**

```bash
cargo build -p features-courses --target wasm32-unknown-unknown 2>&1 | tail -5
git add crates/features-courses/src/submission_view.rs
git commit -m "feat(features-courses): SubmissionView read-only display"
```

---

### Task 19: SubmissionsGradingTable component

**Files:**
- Modify: `crates/features-courses/src/submissions_grading_table.rs`

- [ ] **Step 1: Write component**

```rust
// crates/features-courses/src/submissions_grading_table.rs
//! Teacher-only table listing all submissions for an assignment. Click a row
//! to open SubmissionGradeModal.

use crate::api::{self, ApiContext, AssignmentDto, SubmissionDto};
use dioxus::prelude::*;

#[derive(Clone, Props, PartialEq)]
pub struct SubmissionsGradingTableProps {
    pub api: ApiContext,
    pub assignment: AssignmentDto,
}

pub fn SubmissionsGradingTable(props: SubmissionsGradingTableProps) -> Element {
    let api = props.api.clone();
    let aid = props.assignment.id.clone();
    let submissions = use_resource(move || {
        let api = api.clone();
        let aid = aid.clone();
        async move { api::list_assignment_submissions(&api, &aid).await }
    });

    let mut selected: Signal<Option<SubmissionDto>> = use_signal(|| None);

    rsx! {
        div { class: "submissions-grading-table",
            h2 { "Submissions: {props.assignment.title}" }
            match &*submissions.read_unchecked() {
                Some(Ok(rows)) => rsx! {
                    table {
                        thead { tr {
                            th { "Student" }
                            th { "Status" }
                            th { "Submitted" }
                            th { "Late?" }
                            th { "Grade" }
                            th { "Released?" }
                        } }
                        tbody {
                            for s in rows.iter() {
                                tr { key: "{s.id}", onclick: {
                                    let s = s.clone();
                                    move |_| selected.set(Some(s.clone()))
                                },
                                    td { "{s.student_user_id}" }
                                    td { "{s.status}" }
                                    td { "{s.submitted_at.clone().unwrap_or_default()}" }
                                    td { if s.is_late { "LATE" } else { "" } }
                                    td {
                                        if let Some(n) = s.numeric_grade { "{n}" }
                                        else if let Some(p) = s.passed { if p { "PASS" } else { "FAIL" } }
                                        else { "—" }
                                    }
                                    td { if s.released_at.is_some() { "yes" } else { "no" } }
                                }
                            }
                        }
                    }
                },
                Some(Err(e)) => rsx! { p { class: "error", "{e}" } },
                None => rsx! { p { "Loading..." } },
            }
            if let Some(s) = selected.read().clone() {
                crate::submission_grade_modal::SubmissionGradeModal {
                    api: props.api.clone(),
                    assignment: props.assignment.clone(),
                    submission: s,
                    on_close: move || selected.set(None),
                }
            }
        }
    }
}
```

- [ ] **Step 2: Build (will fail until Task 20 lands SubmissionGradeModal)**

Continue regardless.

- [ ] **Step 3: Commit**

```bash
git add crates/features-courses/src/submissions_grading_table.rs
git commit -m "feat(features-courses): SubmissionsGradingTable teacher view"
```

---

### Task 20: SubmissionGradeModal component

**Files:**
- Modify: `crates/features-courses/src/submission_grade_modal.rs`

- [ ] **Step 1: Write component**

```rust
// crates/features-courses/src/submission_grade_modal.rs
//! Inline grade-entry modal. Numeric input + optional letter, OR pass/fail
//! radio (per assignment.grading_mode). Buttons: Save & Release, Save & Return.

use crate::api::{self, ApiContext, AssignmentDto, GradeBody, SubmissionDto};
use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct SubmissionGradeModalProps {
    pub api: ApiContext,
    pub assignment: AssignmentDto,
    pub submission: SubmissionDto,
    pub on_close: EventHandler<()>,
}

pub fn SubmissionGradeModal(props: SubmissionGradeModalProps) -> Element {
    let mut numeric: Signal<Option<f64>> = use_signal(|| props.submission.numeric_grade);
    let mut letter: Signal<String> = use_signal(|| props.submission.letter_grade.clone().unwrap_or_default());
    let mut passed: Signal<Option<bool>> = use_signal(|| props.submission.passed);
    let mut feedback: Signal<String> = use_signal(|| props.submission.student_visible_feedback.clone().unwrap_or_default());
    let mut error: Signal<Option<String>> = use_signal(|| None);

    let api = props.api.clone();
    let sid = props.submission.id.clone();
    let mode = props.assignment.grading_mode.clone();
    let release_mode = props.assignment.release_mode.clone();

    let on_save = {
        let api = api.clone();
        let sid = sid.clone();
        let mode = mode.clone();
        move |_| {
            let api = api.clone();
            let sid = sid.clone();
            let mode = mode.clone();
            spawn(async move {
                let n = if mode == "numeric" { *numeric.read() } else { None };
                let l = if mode == "numeric" {
                    let s = letter.read().clone();
                    if s.is_empty() { None } else { Some(s) }
                } else { None };
                let p = if mode == "pass_fail" { *passed.read() } else { None };
                let fb = feedback.read().clone();
                let body = GradeBody {
                    numeric_grade: n,
                    letter_grade: l.as_deref(),
                    passed: p,
                    student_visible_feedback: if fb.is_empty() { None } else { Some(fb.as_str()) },
                    teacher_only_notes: None,
                };
                match api::grade_submission(&api, &sid, &body).await {
                    Ok(_) => props.on_close.call(()),
                    Err(e) => error.set(Some(format!("{e}"))),
                }
            });
        }
    };

    let on_save_release = {
        let api = api.clone();
        let sid = sid.clone();
        move |_| {
            let api = api.clone();
            let sid = sid.clone();
            spawn(async move {
                if let Err(e) = api::release_submission(&api, &sid).await {
                    error.set(Some(format!("{e}")));
                } else {
                    props.on_close.call(());
                }
            });
        }
    };

    let on_return = {
        let api = api.clone();
        let sid = sid.clone();
        move |_| {
            let api = api.clone();
            let sid = sid.clone();
            spawn(async move {
                if let Err(e) = api::return_submission(&api, &sid).await {
                    error.set(Some(format!("{e}")));
                } else {
                    props.on_close.call(());
                }
            });
        }
    };

    rsx! {
        div { class: "modal-backdrop", onclick: move |_| props.on_close.call(()),
            div { class: "modal", onclick: move |e| e.stop_propagation(),
                h3 { "Grade submission" }
                if mode == "numeric" {
                    label { "Numeric grade (max {props.assignment.max_points.unwrap_or(0)})" }
                    input {
                        r#type: "number",
                        value: "{numeric.read().unwrap_or(0.0)}",
                        oninput: move |e| {
                            if let Ok(v) = e.value().parse::<f64>() { numeric.set(Some(v)); }
                        }
                    }
                    label { "Letter grade (optional)" }
                    input {
                        value: "{letter}",
                        oninput: move |e| letter.set(e.value()),
                    }
                } else {
                    fieldset {
                        legend { "Pass/fail" }
                        label {
                            input {
                                r#type: "radio", name: "passed", value: "true",
                                checked: matches!(*passed.read(), Some(true)),
                                oninput: move |_| passed.set(Some(true)),
                            }
                            "Pass"
                        }
                        label {
                            input {
                                r#type: "radio", name: "passed", value: "false",
                                checked: matches!(*passed.read(), Some(false)),
                                oninput: move |_| passed.set(Some(false)),
                            }
                            "Fail"
                        }
                    }
                }
                label { "Feedback (visible to student after release)" }
                textarea { rows: 4, value: "{feedback}",
                    oninput: move |e| feedback.set(e.value()) }

                if let Some(e) = error.read().as_ref() { p { class: "error", "{e}" } }
                div { class: "modal-actions",
                    button { onclick: on_save, "Save grade" }
                    if release_mode == "manual" {
                        button { class: "btn-primary", onclick: on_save_release,
                            "Release grade" }
                    }
                    button { onclick: on_return, "Return for resubmit" }
                    button { onclick: move |_| props.on_close.call(()), "Cancel" }
                }
            }
        }
    }
}
```

- [ ] **Step 2: Build entire crate**

```bash
cargo build -p features-courses --target wasm32-unknown-unknown 2>&1 | tail -10
```

Expected: clean (the `submissions_grading_table.rs` reference now resolves).

- [ ] **Step 3: Commit**

```bash
git add crates/features-courses/src/submission_grade_modal.rs
git commit -m "feat(features-courses): SubmissionGradeModal grade entry"
```

---

### Task 21: Inline assignment card in lesson outline

**Files:**
- Modify: `crates/features-courses/src/lesson_outline_view.rs`

- [ ] **Step 1: Inspect existing structure**

```bash
grep -n "fn render\|fn lesson\|use_resource\|use_future" crates/features-courses/src/lesson_outline_view.rs | head -10
```

- [ ] **Step 2: Add a `LessonAssignmentsCard` rendering inline**

Append a new component to `lesson_outline_view.rs`:

```rust
// --- Phase 1c addition: lesson-attached assignments inline card ---

use crate::api::{self as api_mod, ApiContext, AssignmentDto};

#[derive(Clone, Props, PartialEq)]
pub struct LessonAssignmentsCardProps {
    pub api: ApiContext,
    pub course_slug: String,
    pub lesson_id: String,
}

pub fn LessonAssignmentsCard(props: LessonAssignmentsCardProps) -> Element {
    let api = props.api.clone();
    let lid = props.lesson_id.clone();
    let assignments = use_resource(move || {
        let api = api.clone();
        let lid = lid.clone();
        async move { api_mod::list_lesson_assignments(&api, &lid).await }
    });
    let course_slug = props.course_slug.clone();

    rsx! {
        section { class: "lesson-assignments",
            h3 { "Assignments for this lesson" }
            match &*assignments.read_unchecked() {
                Some(Ok(items)) if items.is_empty() => rsx! { p { "None yet." } },
                Some(Ok(items)) => rsx! {
                    ul {
                        for a in items.iter() {
                            { render_lesson_card_row(&course_slug, a) }
                        }
                    }
                },
                Some(Err(e)) => rsx! { p { class: "error", "{e}" } },
                None => rsx! { p { "Loading..." } },
            }
        }
    }
}

fn render_lesson_card_row(course_slug: &str, a: &AssignmentDto) -> Element {
    let id = a.id.clone();
    let title = a.title.clone();
    rsx! {
        li { key: "{id}",
            a { href: format!("/courses/{course_slug}/assignments/{id}"), "{title}" }
            if let Some(d) = a.due_at.as_ref() { span { class: "due", " · due {d}" } }
        }
    }
}
```

Then wire `LessonAssignmentsCard { api, course_slug, lesson_id }` into the existing lesson body render (under the existing content, above the close of the lesson view block). Adjust the existing `LessonOutlineViewProps` to carry `course_slug` + `api` if it doesn't already.

- [ ] **Step 3: Build + commit**

```bash
cargo build -p features-courses --target wasm32-unknown-unknown 2>&1 | tail -5
git add crates/features-courses/src/lesson_outline_view.rs
git commit -m "feat(features-courses): inline assignment card in lesson outline"
```

---

### Task 22: shell-web routes

**Files:**
- Modify: `crates/shell-web/src/main.rs`

- [ ] **Step 1: Inspect router**

```bash
grep -n "enum Route\|#\[layout\]\|#\[route\]" crates/shell-web/src/main.rs | head -20
```

- [ ] **Step 2: Add five new variants to `enum Route`**

```rust
#[route("/courses/:slug/assignments")]
AssignmentList { slug: String },
#[route("/courses/:slug/assignments/new")]
AssignmentNew { slug: String },
#[route("/courses/:slug/assignments/:id")]
AssignmentDetail { slug: String, id: String },
#[route("/courses/:slug/assignments/:id/edit")]
AssignmentEdit { slug: String, id: String },
#[route("/courses/:slug/assignments/:id/grade")]
AssignmentGrade { slug: String, id: String },
```

- [ ] **Step 3: Add render functions for each**

Each new function constructs the appropriate component from `features_courses` (e.g., `AssignmentList`, `AssignmentEditor`, `AssignmentDetail`, `SubmissionsGradingTable`). Mirror the pattern of the existing `LiveSession` route render.

```rust
#[component]
fn AssignmentListRoute(slug: String) -> Element {
    let api = use_api_context();
    let course = use_resource_course_by_slug(&slug);
    rsx! {
        match &*course.read_unchecked() {
            Some(Ok(c)) => rsx! {
                features_courses::assignment_list::AssignmentList {
                    api: api.clone(),
                    course_slug: slug.clone(),
                    course_id: c.id.clone(),
                    is_teacher: c.viewer_role_is_teacher,
                }
            },
            Some(Err(e)) => rsx! { p { "{e}" } },
            None => rsx! { p { "Loading..." } },
        }
    }
}
```

(Repeat for the other 4 routes — substitute the right component. `use_api_context` and `use_resource_course_by_slug` already exist in `shell-web`; reuse them.)

- [ ] **Step 4: Build**

```bash
dx build --platform web --package shell-web 2>&1 | tail -10
cargo build -p shell-web --target wasm32-unknown-unknown 2>&1 | tail -5
```

- [ ] **Step 5: Commit**

```bash
git add crates/shell-web/src/main.rs
git commit -m "feat(shell-web): add 5 assignment routes"
```

---

### Task 23: assignments_permissions.rs integration tests

**Files:**
- Create: `crates/backend/tests/assignments_permissions.rs`

- [ ] **Step 1: Write 4 permission tests**

```rust
//! Phase 1c: assignments + submissions role permissions matrix.

mod fixtures;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::json;
use tower::ServiceExt;
use uuid::Uuid;

async fn course(pool: &sqlx::PgPool, tenant: Uuid, teacher: Uuid) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO courses (tenant_id, slug, title, owner_user_id, status)
         VALUES ($1,$2,'C',$3,'published') RETURNING id",
    ).bind(tenant).bind(format!("c-{}", Uuid::new_v4())).bind(teacher)
     .fetch_one(pool).await.unwrap()
}

#[tokio::test]
async fn student_cannot_create_assignment() {
    let pool = fixtures::pool().await;
    let tenant = fixtures::create_tenant(&pool).await;
    let (teacher, _, _) = fixtures::create_user(&pool).await;
    let (student, _, _) = fixtures::create_user(&pool).await;
    fixtures::attach_membership(&pool, tenant, teacher, "teacher").await;
    fixtures::attach_membership(&pool, tenant, student, "student").await;
    let cid = course(&pool, tenant, teacher).await;

    let stub = fixtures::StubAuth {
        pool: pool.clone(), user_id: student, firebase_uid: "fs".into(),
        email: "s".into(), tenant_id: Some(tenant),
        tenant_role: Some(core_types::TenantRole::Student),
    };
    let app = fixtures::build_test_app(
        backend::handlers::assignments::router_for_tests(pool.clone()), stub,
    );
    let req = Request::builder().method("POST")
        .uri(format!("/v1/courses/{cid}/assignments"))
        .header("content-type", "application/json")
        .body(Body::from(json!({
            "title": "x", "grading_mode": "numeric", "max_points": 100
        }).to_string())).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn student_cannot_grade_submission() {
    let pool = fixtures::pool().await;
    let tenant = fixtures::create_tenant(&pool).await;
    let (teacher, _, _) = fixtures::create_user(&pool).await;
    let (student, _, _) = fixtures::create_user(&pool).await;
    fixtures::attach_membership(&pool, tenant, teacher, "teacher").await;
    fixtures::attach_membership(&pool, tenant, student, "student").await;
    let cid = course(&pool, tenant, teacher).await;

    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string()).execute(&pool).await.unwrap();
    let aid: Uuid = sqlx::query_scalar(
        "INSERT INTO assignments (tenant_id, course_id, title, grading_mode, max_points,
                                  status, published_at, created_by)
         VALUES ($1,$2,'A','numeric',100,'published',now(),$3) RETURNING id",
    ).bind(tenant).bind(cid).bind(teacher).fetch_one(&pool).await.unwrap();
    let sid: Uuid = sqlx::query_scalar(
        "INSERT INTO submissions (tenant_id, assignment_id, course_id, student_user_id,
                                  status, submitted_at)
         VALUES ($1,$2,$3,$4,'submitted',now()) RETURNING id",
    ).bind(tenant).bind(aid).bind(cid).bind(student).fetch_one(&pool).await.unwrap();

    let stub = fixtures::StubAuth {
        pool: pool.clone(), user_id: student, firebase_uid: "fs".into(),
        email: "s".into(), tenant_id: Some(tenant),
        tenant_role: Some(core_types::TenantRole::Student),
    };
    let app = fixtures::build_test_app(
        backend::handlers::submissions::router_for_tests(pool.clone()), stub,
    );
    let req = Request::builder().method("POST")
        .uri(format!("/v1/submissions/{sid}/grade"))
        .header("content-type", "application/json")
        .body(Body::from(json!({"numeric_grade": 100}).to_string())).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn delete_published_assignment_with_submissions_409() {
    let pool = fixtures::pool().await;
    let tenant = fixtures::create_tenant(&pool).await;
    let (teacher, _, _) = fixtures::create_user(&pool).await;
    let (student, _, _) = fixtures::create_user(&pool).await;
    fixtures::attach_membership(&pool, tenant, teacher, "teacher").await;
    fixtures::attach_membership(&pool, tenant, student, "student").await;
    let cid = course(&pool, tenant, teacher).await;

    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string()).execute(&pool).await.unwrap();
    let aid: Uuid = sqlx::query_scalar(
        "INSERT INTO assignments (tenant_id, course_id, title, grading_mode, max_points,
                                  status, published_at, created_by)
         VALUES ($1,$2,'A','numeric',100,'published',now(),$3) RETURNING id",
    ).bind(tenant).bind(cid).bind(teacher).fetch_one(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO submissions (tenant_id, assignment_id, course_id, student_user_id, status)
         VALUES ($1,$2,$3,$4,'draft')",
    ).bind(tenant).bind(aid).bind(cid).bind(student).execute(&pool).await.unwrap();

    let stub = fixtures::StubAuth {
        pool: pool.clone(), user_id: teacher, firebase_uid: "ft".into(),
        email: "t".into(), tenant_id: Some(tenant),
        tenant_role: Some(core_types::TenantRole::Teacher),
    };
    let app = fixtures::build_test_app(
        backend::handlers::assignments::router_for_tests(pool.clone()), stub,
    );
    let req = Request::builder().method("DELETE")
        .uri(format!("/v1/assignments/{aid}"))
        .body(Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn unpublish_with_submissions_409() {
    let pool = fixtures::pool().await;
    let tenant = fixtures::create_tenant(&pool).await;
    let (teacher, _, _) = fixtures::create_user(&pool).await;
    let (student, _, _) = fixtures::create_user(&pool).await;
    fixtures::attach_membership(&pool, tenant, teacher, "teacher").await;
    fixtures::attach_membership(&pool, tenant, student, "student").await;
    let cid = course(&pool, tenant, teacher).await;

    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string()).execute(&pool).await.unwrap();
    let aid: Uuid = sqlx::query_scalar(
        "INSERT INTO assignments (tenant_id, course_id, title, grading_mode, max_points,
                                  status, published_at, created_by)
         VALUES ($1,$2,'A','numeric',100,'published',now(),$3) RETURNING id",
    ).bind(tenant).bind(cid).bind(teacher).fetch_one(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO submissions (tenant_id, assignment_id, course_id, student_user_id, status)
         VALUES ($1,$2,$3,$4,'draft')",
    ).bind(tenant).bind(aid).bind(cid).bind(student).execute(&pool).await.unwrap();

    let stub = fixtures::StubAuth {
        pool: pool.clone(), user_id: teacher, firebase_uid: "ft".into(),
        email: "t".into(), tenant_id: Some(tenant),
        tenant_role: Some(core_types::TenantRole::Teacher),
    };
    let app = fixtures::build_test_app(
        backend::handlers::assignments::router_for_tests(pool.clone()), stub,
    );
    let req = Request::builder().method("POST")
        .uri(format!("/v1/assignments/{aid}/unpublish"))
        .body(Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
}
```

- [ ] **Step 2: Run**

```bash
cargo test -p backend --test assignments_permissions -j 2 2>&1 | tail -10
```

Expected: 4 passed.

- [ ] **Step 3: Commit**

```bash
git add crates/backend/tests/assignments_permissions.rs
git commit -m "test(assignments): permissions matrix + 409 paths"
```

---

### Task 24: RLS sweep extension — assignments + submissions

**Files:**
- Modify: `crates/backend/tests/rls_tenant_isolation.rs`

- [ ] **Step 1: Extend the `PHASE_1B_GAMMA_TABLES` constant**

```rust
const PHASE_1B_GAMMA_TABLES: &[&str] = &[
    "live_room_messages",
    "live_room_kicks",
    "recordings",
    "assignments",
    "submissions",
];
```

(Rename if you prefer a `PHASE_1C_TABLES` block — match existing convention.)

- [ ] **Step 2: Add cross-tenant probes**

Append two tests:

```rust
#[tokio::test]
async fn cross_tenant_assignments_masked() -> anyhow::Result<()> {
    let pool = pool().await;
    let tenant_a = Uuid::new_v4();
    let tenant_b = Uuid::new_v4();
    let mut conn = pool.acquire().await?;
    seed_membership(&mut *conn, tenant_a, "a").await?;
    seed_membership(&mut *conn, tenant_b, "b").await?;

    let teacher_a = Uuid::new_v4();
    seed_user(&mut *conn, teacher_a).await?;
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_a.to_string()).execute(&mut *conn).await?;
    let course_a: Uuid = sqlx::query_scalar(
        "INSERT INTO courses (tenant_id, slug, title, owner_user_id, status)
         VALUES ($1,$2,'A',$3,'published') RETURNING id",
    ).bind(tenant_a).bind(format!("a-{}", tenant_a.simple())).bind(teacher_a)
     .fetch_one(&mut *conn).await?;
    sqlx::query(
        "INSERT INTO assignments (tenant_id, course_id, title, grading_mode, max_points,
                                  status, created_by)
         VALUES ($1,$2,'X','numeric',100,'published',$3)",
    ).bind(tenant_a).bind(course_a).bind(teacher_a).execute(&mut *conn).await?;

    let role_name = create_rls_test_role_phase_1a(&mut *conn).await?;
    sqlx::query(&format!("SET LOCAL ROLE \"{role_name}\""))
        .execute(&mut *conn).await?;
    set_local_tenant(&mut *conn, tenant_b).await?;
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM assignments")
        .fetch_one(&mut *conn).await?;
    assert_eq!(count, 0, "tenant B should not see tenant A assignments");
    Ok(())
}

#[tokio::test]
async fn cross_tenant_submissions_masked() -> anyhow::Result<()> {
    let pool = pool().await;
    let tenant_a = Uuid::new_v4();
    let tenant_b = Uuid::new_v4();
    let mut conn = pool.acquire().await?;
    seed_membership(&mut *conn, tenant_a, "a").await?;
    seed_membership(&mut *conn, tenant_b, "b").await?;

    let teacher_a = Uuid::new_v4();
    let student_a = Uuid::new_v4();
    seed_user(&mut *conn, teacher_a).await?;
    seed_user(&mut *conn, student_a).await?;
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_a.to_string()).execute(&mut *conn).await?;
    let course_a: Uuid = sqlx::query_scalar(
        "INSERT INTO courses (tenant_id, slug, title, owner_user_id, status)
         VALUES ($1,$2,'A',$3,'published') RETURNING id",
    ).bind(tenant_a).bind(format!("a-{}", tenant_a.simple())).bind(teacher_a)
     .fetch_one(&mut *conn).await?;
    let assn_a: Uuid = sqlx::query_scalar(
        "INSERT INTO assignments (tenant_id, course_id, title, grading_mode, max_points,
                                  status, created_by)
         VALUES ($1,$2,'X','numeric',100,'published',$3) RETURNING id",
    ).bind(tenant_a).bind(course_a).bind(teacher_a).fetch_one(&mut *conn).await?;
    sqlx::query(
        "INSERT INTO submissions (tenant_id, assignment_id, course_id, student_user_id, status)
         VALUES ($1,$2,$3,$4,'draft')",
    ).bind(tenant_a).bind(assn_a).bind(course_a).bind(student_a)
     .execute(&mut *conn).await?;

    let role_name = create_rls_test_role_phase_1a(&mut *conn).await?;
    sqlx::query(&format!("SET LOCAL ROLE \"{role_name}\""))
        .execute(&mut *conn).await?;
    set_local_tenant(&mut *conn, tenant_b).await?;
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM submissions")
        .fetch_one(&mut *conn).await?;
    assert_eq!(count, 0, "tenant B should not see tenant A submissions");
    Ok(())
}
```

- [ ] **Step 3: Run**

```bash
cargo test -p backend --test rls_tenant_isolation -j 2 2>&1 | tail -10
```

Expected: all RLS tests pass (12+ now).

- [ ] **Step 4: Commit**

```bash
git add crates/backend/tests/rls_tenant_isolation.rs
git commit -m "test(rls): cross-tenant probe for assignments + submissions"
```

---

### Task 25: SSR smokes for new components

**Files:**
- Create: `crates/features-courses/tests/assignments_ssr.rs`

- [ ] **Step 1: Write SSR smoke tests**

```rust
//! Phase 1c: SSR smokes — assignment + submission components mount without panic.

use dioxus::prelude::*;
use dioxus_ssr::render;
use features_courses::api::{ApiContext, AssignmentDto, SubmissionDto};

fn fake_api() -> ApiContext {
    ApiContext::new("http://localhost:8080".into(), None)
}

fn fake_assignment() -> AssignmentDto {
    AssignmentDto {
        id: "00000000-0000-0000-0000-000000000001".into(),
        course_id: "00000000-0000-0000-0000-000000000002".into(),
        lesson_id: None,
        title: "Essay 1".into(),
        instructions_md: "Write 500 words.".into(),
        grading_mode: "numeric".into(),
        max_points: Some(100),
        allow_late: true, lock_on_submit: false,
        accepts_text: true, accepts_files: true,
        release_mode: "instant".into(),
        attachment_asset_ids: vec![],
        due_at: None, status: "published".into(),
        published_at: None,
        created_at: "2026-05-09T00:00:00Z".into(),
        updated_at: "2026-05-09T00:00:00Z".into(),
    }
}

#[test]
fn assignment_list_renders() {
    let mut dom = VirtualDom::new_with_props(
        features_courses::assignment_list::AssignmentList,
        features_courses::assignment_list::AssignmentListProps {
            api: fake_api(),
            course_slug: "math".into(),
            course_id: "00000000-0000-0000-0000-000000000002".into(),
            is_teacher: true,
        },
    );
    let _ = dom.rebuild_in_place();
    let html = render(&dom);
    assert!(html.contains("Assignments"));
}

#[test]
fn assignment_editor_renders() {
    let mut dom = VirtualDom::new_with_props(
        features_courses::assignment_editor::AssignmentEditor,
        features_courses::assignment_editor::AssignmentEditorProps {
            api: fake_api(),
            course_slug: "math".into(),
            course_id: "00000000-0000-0000-0000-000000000002".into(),
            initial: None,
        },
    );
    let _ = dom.rebuild_in_place();
    let html = render(&dom);
    assert!(html.contains("Title"));
    assert!(html.contains("Pass/fail"));
}

#[test]
fn assignment_detail_renders_loading() {
    let mut dom = VirtualDom::new_with_props(
        features_courses::assignment_detail::AssignmentDetail,
        features_courses::assignment_detail::AssignmentDetailProps {
            api: fake_api(),
            assignment_id: "00000000-0000-0000-0000-000000000001".into(),
            course_slug: "math".into(),
            current_user_id: "00000000-0000-0000-0000-000000000003".into(),
            is_teacher: false,
        },
    );
    let _ = dom.rebuild_in_place();
    let html = render(&dom);
    assert!(html.contains("Loading"));
}

#[test]
fn submission_view_hides_grade_when_unreleased() {
    let s = SubmissionDto {
        id: "00000000-0000-0000-0000-000000000010".into(),
        assignment_id: "00000000-0000-0000-0000-000000000001".into(),
        course_id: "00000000-0000-0000-0000-000000000002".into(),
        student_user_id: "00000000-0000-0000-0000-000000000003".into(),
        status: "graded".into(),
        text_answer: Some("answer".into()),
        attachment_asset_ids: vec![],
        submitted_at: Some("2026-05-09T00:00:00Z".into()),
        is_late: false,
        numeric_grade: Some(80.0),
        letter_grade: None,
        passed: None,
        student_visible_feedback: Some("good".into()),
        graded_at: Some("2026-05-09T01:00:00Z".into()),
        released_at: None, // NOT released
        created_at: "2026-05-09T00:00:00Z".into(),
        updated_at: "2026-05-09T00:00:00Z".into(),
    };
    let mut dom = VirtualDom::new_with_props(
        features_courses::submission_view::SubmissionView,
        features_courses::submission_view::SubmissionViewProps { submission: s },
    );
    let _ = dom.rebuild_in_place();
    let html = render(&dom);
    assert!(!html.contains("Numeric: 80"), "grade leaked when released_at NULL");
    assert!(!html.contains("Feedback: good"), "feedback leaked when released_at NULL");
}
```

- [ ] **Step 2: Run**

```bash
cargo test -p features-courses --test assignments_ssr 2>&1 | tail -10
```

Expected: 4 passed.

- [ ] **Step 3: Commit**

```bash
git add crates/features-courses/tests/assignments_ssr.rs
git commit -m "test(features-courses): SSR smokes for assignment components"
```

---

### Task 26: Course detail — Assignments tab link

**Files:**
- Modify: `crates/features-courses/src/course_detail.rs`

- [ ] **Step 1: Add a navigation row**

Locate the existing tabs/nav in `course_detail.rs`. Add an `Assignments` tab pointing to `/courses/{slug}/assignments`.

```rust
a {
    href: format!("/courses/{}/assignments", props.slug),
    class: "course-nav__tab",
    "Assignments"
}
```

- [ ] **Step 2: Build + commit**

```bash
cargo build -p features-courses --target wasm32-unknown-unknown 2>&1 | tail -5
git add crates/features-courses/src/course_detail.rs
git commit -m "feat(features-courses): assignments tab in course detail"
```

---

### Task 27: Build sweeps + workspace test sweep

**Files:** none — verification only.

- [ ] **Step 1: Run wasm builds in parallel**

```bash
cargo build -p shell-web --target wasm32-unknown-unknown 2>&1 | tail -5
cargo build -p features-courses --target wasm32-unknown-unknown 2>&1 | tail -5
dx build --platform web --package shell-web 2>&1 | tail -10
```

Expected: all clean (warnings OK, no errors).

- [ ] **Step 2: Workspace test sweep**

```bash
DATABASE_URL=postgres://aulalite:changeme@localhost:55432/aulalite \
    cargo test --workspace -j 2 2>&1 | tail -20
```

Expected: zero failures across all binaries.

- [ ] **Step 3: If a Windows PDB linker error (LNK1318) appears**

```bash
cargo clean -p backend
DATABASE_URL=... cargo test --workspace -j 2
```

- [ ] **Step 4: Commit any incidental fixes** (if a flaky test surfaces a real bug; otherwise nothing to commit).

---

### Task 28: Phase 1c exit checklist + commit + report

**Files:**
- Create: `docs/superpowers/plans/2026-05-09-aulalite-phase-1c-assignments-exit-checklist.md`

- [ ] **Step 1: Write the checklist**

```markdown
# Phase 1c Exit Checklist

Run these checks in order from the repository root. Phase 1c is complete only
when every required item passes.

## 1. Stack health
- [ ] `docker compose up -d`
- [ ] `curl http://localhost:8080/healthz` returns `ok`

## 2. Migrations
- [ ] `sqlx migrate info --source migrations` shows `20260509000016_assignments` applied.
- [ ] `sqlx migrate info --source migrations` shows `20260509000017_submissions` applied.
- [ ] `sqlx migrate info --source migrations` shows `20260509000018_file_assets_assignments` applied.
- [ ] `\d assignments` shows table with RLS, 4 CHECK constraints, 2 indexes.
- [ ] `\d submissions` shows table with RLS, UNIQUE(assignment_id, student_user_id), 2 indexes.
- [ ] `\d+ file_assets` CHECK includes `'assignment_attachment'` and `'submission_attachment'`.

## 3. Automated verification
- [ ] `cargo test --workspace -j 2` (everything green; zero failures).
- [ ] `cargo build -p shell-web --target wasm32-unknown-unknown`
- [ ] `cargo build -p features-courses --target wasm32-unknown-unknown`
- [ ] `dx build --platform web --package shell-web` succeeds.

## 4. End-to-end teacher flow (manual)
- [ ] Sign in as teacher. Open a course → Assignments tab → "New assignment".
- [ ] Fill out form (title, instructions, grading_mode=numeric, max_points=100, accepts_text+files, release_mode=instant). Save draft.
- [ ] Click Publish. Confirm assignment status flips to `published` in DB.

## 5. End-to-end student flow (manual)
- [ ] Sign in as enrolled student. Open the same course → Assignments tab.
- [ ] Open the published assignment. Confirm SubmissionForm renders.
- [ ] Type a text answer. Click Save draft. Click Submit.
- [ ] Confirm `submissions.status='submitted'` and `is_late=false` in DB.

## 6. End-to-end grading (manual)
- [ ] Switch back to teacher. Open `/courses/<slug>/assignments/<id>/grade`.
- [ ] Click the student's row. Enter grade 85 + feedback. Save.
- [ ] (instant mode) Switch to student → see grade immediately.
- [ ] Repeat with a `release_mode=manual` assignment; confirm grade is hidden until teacher clicks Release.

## 7. Late-rejection (manual)
- [ ] Create an assignment with `due_at` in the past and `allow_late=false`.
- [ ] As student, try to submit. Confirm 422 + UI error.

## 8. Lock-on-submit (manual)
- [ ] Create an assignment with `lock_on_submit=true`. As student, submit.
- [ ] Try to PATCH the submission. Confirm 409 + UI prevents editing.

## 9. Return-for-resubmit (manual)
- [ ] Grade a submission. Click "Return for resubmit".
- [ ] As student, see status `returned` and grade hidden. Edit + resubmit.

## 10. Cross-tenant probe
- [ ] Tenant B's user fetches `/v1/courses/<tenant-A-course-id>/assignments` → 404 (masked).
- [ ] Tenant B's user fetches `/v1/assignments/<tenant-A-assignment-id>` → 404.
- [ ] Tenant B's user fetches `/v1/submissions/<tenant-A-submission-id>` → 404.

## 11. Open follow-ups carried into next phase

From 1b-γ (still open):
- [ ] Persistent WebSocket polish (WhipPublisher close on Demoted, toast surfacing).
- [ ] RedisLiveRoomBroker production exercise.

These do NOT block tagging `phase-1c-complete`.

## Completion tag

```bash
git tag phase-1c-complete
git push origin phase-1c-complete
```

Tagging `phase-1c-complete` unlocks Phase 1d (notifications, role polish, parent role).
```

- [ ] **Step 2: Commit**

```bash
git add docs/superpowers/plans/2026-05-09-aulalite-phase-1c-assignments-exit-checklist.md
git commit -m "docs(plan): Phase 1c assignments exit checklist"
git rev-parse HEAD
```

- [ ] **Step 3: Report**

Tell the user:
- Total commits added in 1c.
- Test pass count.
- Final SHA on `phase-0-foundations`.
- Pending manual exit-checklist items.
- That Phase 1c unlocks Phase 1d (notifications, role polish, parent role).

---

## resource: handlers/submissions.rs (Tasks 8-11)

This is the full body to drop into `crates/backend/src/handlers/submissions.rs`
in Task 8 Step 3.

```rust
// crates/backend/src/handlers/submissions.rs
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::{routing, Json, Router};
use serde::{Deserialize, Serialize};
use sqlx::types::Decimal;
use sqlx::PgPool;
use std::str::FromStr;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::context::RequestContext;
use crate::db;
use crate::error::ApiError;
use crate::AppState;

#[derive(Deserialize, Default)]
pub struct PatchSubmission {
    #[serde(default, with = "::serde_with::rust::double_option")]
    pub text_answer: Option<Option<String>>,
    pub attachment_asset_ids: Option<Vec<Uuid>>,
}

#[derive(Deserialize, Default)]
pub struct GradeBody {
    pub numeric_grade: Option<f64>,
    pub letter_grade: Option<String>,
    pub passed: Option<bool>,
    pub student_visible_feedback: Option<String>,
    pub teacher_only_notes: Option<String>,
}

#[derive(Serialize)]
pub struct SubmissionDto {
    pub id: Uuid,
    pub assignment_id: Uuid,
    pub course_id: Uuid,
    pub student_user_id: Uuid,
    pub status: String,
    pub text_answer: Option<String>,
    pub attachment_asset_ids: Vec<Uuid>,
    pub submitted_at: Option<OffsetDateTime>,
    pub is_late: bool,
    pub numeric_grade: Option<f64>,
    pub letter_grade: Option<String>,
    pub passed: Option<bool>,
    pub student_visible_feedback: Option<String>,
    pub graded_at: Option<OffsetDateTime>,
    pub released_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

fn dto_for_viewer(
    row: db::submissions::SubmissionRow,
    is_teacher: bool,
) -> SubmissionDto {
    let released = row.released_at.is_some();
    let show_grade = is_teacher || released;
    SubmissionDto {
        id: row.id,
        assignment_id: row.assignment_id,
        course_id: row.course_id,
        student_user_id: row.student_user_id,
        status: row.status,
        text_answer: row.text_answer,
        attachment_asset_ids: row.attachment_asset_ids,
        submitted_at: row.submitted_at,
        is_late: row.is_late,
        numeric_grade: if show_grade {
            row.numeric_grade.and_then(|d| d.to_string().parse::<f64>().ok())
        } else { None },
        letter_grade: if show_grade { row.letter_grade } else { None },
        passed: if show_grade { row.passed } else { None },
        student_visible_feedback: if show_grade { row.student_visible_feedback } else { None },
        graded_at: if show_grade { row.graded_at } else { None },
        released_at: if show_grade { row.released_at } else { None },
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/assignments/:aid/submissions",
               routing::post(create_or_get).get(list_for_assignment))
        .route("/v1/submissions/:id", routing::get(get_one).patch(patch))
        .route("/v1/submissions/:id/submit", routing::post(submit))
        .route("/v1/submissions/:id/grade", routing::post(grade))
        .route("/v1/submissions/:id/release", routing::post(release))
        .route("/v1/submissions/:id/return", routing::post(return_resubmit))
}

#[doc(hidden)]
pub fn router_for_tests(pool: PgPool) -> Router {
    Router::new()
        .route("/v1/assignments/:aid/submissions",
               routing::post(create_or_get_t).get(list_for_assignment_t))
        .route("/v1/submissions/:id", routing::get(get_one_t).patch(patch_t))
        .route("/v1/submissions/:id/submit", routing::post(submit_t))
        .route("/v1/submissions/:id/grade", routing::post(grade_t))
        .route("/v1/submissions/:id/release", routing::post(release_t))
        .route("/v1/submissions/:id/return", routing::post(return_resubmit_t))
        .with_state(TestState { pool })
}

#[derive(Clone)]
struct TestState { pool: PgPool }

fn is_teacher(ctx: &RequestContext) -> bool {
    matches!(
        ctx.tenant_role,
        Some(core_types::TenantRole::Teacher)
            | Some(core_types::TenantRole::OrgAdmin)
            | Some(core_types::TenantRole::Ta)
    ) || ctx.is_platform_admin
}

async fn set_tenant(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, tenant: Uuid)
    -> sqlx::Result<()>
{
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut **tx).await?;
    Ok(())
}

// ----------------- production handlers -----------------

async fn create_or_get(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(aid): Path<Uuid>,
) -> Result<Json<SubmissionDto>, ApiError> {
    create_or_get_inner(&s.pool, &ctx, aid).await
}
async fn create_or_get_t(
    State(ts): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(aid): Path<Uuid>,
) -> Result<Json<SubmissionDto>, ApiError> {
    create_or_get_inner(&ts.pool, &ctx, aid).await
}

async fn create_or_get_inner(
    pool: &PgPool, ctx: &RequestContext, aid: Uuid,
) -> Result<Json<SubmissionDto>, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden("no tenant".into()))?;
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant).await?;
    let assignment = db::assignments::fetch_by_id(&mut tx, aid).await?
        .ok_or(ApiError::NotFound)?;
    if assignment.status != "published" {
        return Err(ApiError::NotFound);
    }
    let row = db::submissions::upsert_for_student(
        &mut tx, tenant, aid, assignment.course_id, ctx.user_id,
    ).await?;
    tx.commit().await?;
    Ok(Json(dto_for_viewer(row, is_teacher(ctx))))
}

async fn patch(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    Json(body): Json<PatchSubmission>,
) -> Result<Json<SubmissionDto>, ApiError> {
    patch_inner(&s.pool, &ctx, id, body).await
}
async fn patch_t(
    State(ts): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    Json(body): Json<PatchSubmission>,
) -> Result<Json<SubmissionDto>, ApiError> {
    patch_inner(&ts.pool, &ctx, id, body).await
}
async fn patch_inner(
    pool: &PgPool, ctx: &RequestContext, id: Uuid, body: PatchSubmission,
) -> Result<Json<SubmissionDto>, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden("no tenant".into()))?;
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant).await?;
    let row = db::submissions::fetch_by_id(&mut tx, id).await?
        .ok_or(ApiError::NotFound)?;
    if row.student_user_id != ctx.user_id {
        return Err(ApiError::NotFound);
    }
    let assn = db::assignments::fetch_by_id(&mut tx, row.assignment_id).await?
        .ok_or(ApiError::NotFound)?;
    let editable = match row.status.as_str() {
        "draft" | "returned" => true,
        "submitted" => !assn.lock_on_submit,
        _ => false,
    };
    if !editable {
        return Err(ApiError::Conflict("submission_locked".into()));
    }
    let updated = db::submissions::patch_draft_fields(
        &mut tx, id,
        body.text_answer.map(|opt| opt.as_deref()),
        body.attachment_asset_ids.as_deref(),
    ).await?;
    tx.commit().await?;
    Ok(Json(dto_for_viewer(updated, is_teacher(ctx))))
}

async fn submit(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<SubmissionDto>, ApiError> { submit_inner(&s.pool, &ctx, id).await }
async fn submit_t(
    State(ts): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<SubmissionDto>, ApiError> { submit_inner(&ts.pool, &ctx, id).await }
async fn submit_inner(
    pool: &PgPool, ctx: &RequestContext, id: Uuid,
) -> Result<Json<SubmissionDto>, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden("no tenant".into()))?;
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant).await?;
    let row = db::submissions::fetch_by_id(&mut tx, id).await?
        .ok_or(ApiError::NotFound)?;
    if row.student_user_id != ctx.user_id {
        return Err(ApiError::NotFound);
    }
    if !matches!(row.status.as_str(), "draft" | "returned") {
        return Err(ApiError::Conflict("not_in_draft_or_returned".into()));
    }
    let assn = db::assignments::fetch_by_id(&mut tx, row.assignment_id).await?
        .ok_or(ApiError::NotFound)?;
    let now = OffsetDateTime::now_utc();
    let is_late = assn.due_at.map(|d| now > d).unwrap_or(false);
    if is_late && !assn.allow_late {
        return Err(ApiError::Validation("submission_late_not_allowed".into()));
    }
    let updated = db::submissions::mark_submitted(&mut tx, id, is_late).await?;
    tx.commit().await?;
    Ok(Json(dto_for_viewer(updated, is_teacher(ctx))))
}

async fn list_for_assignment(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(aid): Path<Uuid>,
) -> Result<Json<Vec<SubmissionDto>>, ApiError> {
    list_inner(&s.pool, &ctx, aid).await
}
async fn list_for_assignment_t(
    State(ts): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(aid): Path<Uuid>,
) -> Result<Json<Vec<SubmissionDto>>, ApiError> {
    list_inner(&ts.pool, &ctx, aid).await
}
async fn list_inner(
    pool: &PgPool, ctx: &RequestContext, aid: Uuid,
) -> Result<Json<Vec<SubmissionDto>>, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden("no tenant".into()))?;
    if !is_teacher(ctx) { return Err(ApiError::Forbidden("teacher_only".into())); }
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant).await?;
    let rows = db::submissions::list_for_assignment(&mut tx, aid).await?;
    tx.commit().await?;
    Ok(Json(rows.into_iter().map(|r| dto_for_viewer(r, true)).collect()))
}

async fn get_one(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<SubmissionDto>, ApiError> { get_one_inner(&s.pool, &ctx, id).await }
async fn get_one_t(
    State(ts): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<SubmissionDto>, ApiError> { get_one_inner(&ts.pool, &ctx, id).await }
async fn get_one_inner(
    pool: &PgPool, ctx: &RequestContext, id: Uuid,
) -> Result<Json<SubmissionDto>, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden("no tenant".into()))?;
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant).await?;
    let row = db::submissions::fetch_by_id(&mut tx, id).await?
        .ok_or(ApiError::NotFound)?;
    let teacher = is_teacher(ctx);
    if !teacher && row.student_user_id != ctx.user_id {
        return Err(ApiError::NotFound);
    }
    tx.commit().await?;
    Ok(Json(dto_for_viewer(row, teacher)))
}

async fn grade(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    Json(body): Json<GradeBody>,
) -> Result<Json<SubmissionDto>, ApiError> { grade_inner(&s.pool, &ctx, id, body).await }
async fn grade_t(
    State(ts): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    Json(body): Json<GradeBody>,
) -> Result<Json<SubmissionDto>, ApiError> { grade_inner(&ts.pool, &ctx, id, body).await }
async fn grade_inner(
    pool: &PgPool, ctx: &RequestContext, id: Uuid, body: GradeBody,
) -> Result<Json<SubmissionDto>, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden("no tenant".into()))?;
    if !is_teacher(ctx) { return Err(ApiError::Forbidden("teacher_only".into())); }
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant).await?;
    let row = db::submissions::fetch_by_id(&mut tx, id).await?
        .ok_or(ApiError::NotFound)?;
    let assn = db::assignments::fetch_by_id(&mut tx, row.assignment_id).await?
        .ok_or(ApiError::NotFound)?;

    // Coherence checks.
    match assn.grading_mode.as_str() {
        "numeric" => {
            let n = body.numeric_grade.ok_or(
                ApiError::Validation("numeric_grade_required".into()))?;
            let max = assn.max_points.unwrap_or(0);
            if n < 0.0 || n > max as f64 {
                return Err(ApiError::Validation("numeric_grade_out_of_range".into()));
            }
            if body.passed.is_some() {
                return Err(ApiError::Validation("passed_not_allowed_for_numeric".into()));
            }
        }
        "pass_fail" => {
            if body.passed.is_none() {
                return Err(ApiError::Validation("passed_required".into()));
            }
            if body.numeric_grade.is_some() || body.letter_grade.is_some() {
                return Err(ApiError::Validation("numeric_letter_not_allowed_for_pass_fail".into()));
            }
        }
        _ => return Err(ApiError::Internal("bad grading_mode".into())),
    }

    let release_now = assn.release_mode == "instant";
    let numeric = body.numeric_grade
        .and_then(|f| Decimal::from_str(&f.to_string()).ok());
    let updated = db::submissions::save_grade(
        &mut tx, id,
        db::submissions::GradeFields {
            numeric_grade: numeric,
            letter_grade: body.letter_grade.as_deref(),
            passed: body.passed,
            student_visible_feedback: body.student_visible_feedback.as_deref(),
            teacher_only_notes: body.teacher_only_notes.as_deref(),
            grader_id: ctx.user_id,
            release_now,
        },
    ).await?;
    tx.commit().await?;
    Ok(Json(dto_for_viewer(updated, true)))
}

async fn release(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<SubmissionDto>, ApiError> { release_inner(&s.pool, &ctx, id).await }
async fn release_t(
    State(ts): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<SubmissionDto>, ApiError> { release_inner(&ts.pool, &ctx, id).await }
async fn release_inner(
    pool: &PgPool, ctx: &RequestContext, id: Uuid,
) -> Result<Json<SubmissionDto>, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden("no tenant".into()))?;
    if !is_teacher(ctx) { return Err(ApiError::Forbidden("teacher_only".into())); }
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant).await?;
    let updated = db::submissions::mark_released(&mut tx, id).await
        .map_err(|e| match e {
            sqlx::Error::RowNotFound => ApiError::Conflict("not_in_graded_or_already_released".into()),
            other => other.into(),
        })?;
    tx.commit().await?;
    Ok(Json(dto_for_viewer(updated, true)))
}

async fn return_resubmit(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<SubmissionDto>, ApiError> { return_inner(&s.pool, &ctx, id).await }
async fn return_resubmit_t(
    State(ts): State<TestState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<SubmissionDto>, ApiError> { return_inner(&ts.pool, &ctx, id).await }
async fn return_inner(
    pool: &PgPool, ctx: &RequestContext, id: Uuid,
) -> Result<Json<SubmissionDto>, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden("no tenant".into()))?;
    if !is_teacher(ctx) { return Err(ApiError::Forbidden("teacher_only".into())); }
    let mut tx = pool.begin().await?;
    set_tenant(&mut tx, tenant).await?;
    let updated = db::submissions::return_for_resubmit(&mut tx, id).await
        .map_err(|e| match e {
            sqlx::Error::RowNotFound => ApiError::Conflict("not_in_submitted_or_graded".into()),
            other => other.into(),
        })?;
    tx.commit().await?;
    Ok(Json(dto_for_viewer(updated, true)))
}
```

---

## Self-review notes (for the controller running this plan)

After all tasks complete:

1. **Spec coverage.** Section A (schema) → Tasks 1-3. Section B (REST API) → Tasks 6-12. Section C (frontend) → Tasks 13-22, 26. Section D (state machine + invariants) → invariants 1-6 covered in Task 8 + 10-11 tests; invariants 7-10 covered in Tasks 7 + 23. Section E (testing) → Tasks 6-11, 23-25.

2. **Type consistency.** `AssignmentRow` (Task 4) used unchanged through Tasks 6, 8, 12. `SubmissionRow` (Task 5) used through Tasks 8-11. `AssignmentDto`/`SubmissionDto` consistent across backend (Tasks 6, 8) and frontend (Task 13). `grading_mode` values `"numeric"` / `"pass_fail"` and `release_mode` values `"instant"` / `"manual"` consistent everywhere.

3. **No placeholders.** Each code-bearing step has a literal code block. Three exceptions where the engineer must adapt to existing code (Task 12 uploads validation, Task 21 lesson outline integration, Task 22 shell-web router) are explicit about WHAT to add and reference the existing pattern to mirror.

4. **Migration is forward-only.** Three new migrations; ALTER on file_assets CHECK is the only schema rewrite, preserving prior values.

5. **Frontend follows project convention.** All new components are `pub fn` (no `#[component]` macro with explicit Props derive — same convention as 1b-γ/δ).

6. **Open follow-ups from prior phases.** 1b-γ §4a and §4b carried forward as documented in spec. They do not block 1c.
