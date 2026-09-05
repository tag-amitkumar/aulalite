# Phase 1c — Assignments Design

**Status:** Approved 2026-05-09
**Predecessor:** Phase 1b complete (live classes + recording) merged at `5f7aef2`.
**Successor unlocks:** Phase 1d (notifications, role polish, parent role).

## Overview

Phase 1c adds the second pillar of AulaLite's MVP: assignments with student
submissions, grading, deadlines, and feedback. The flow is teacher-creates →
student-submits → teacher-grades → student-views-result. The phase reuses the
1b-α file uploads pipeline for attachments and follows the same
backend-handler / db-query / features-courses-component pattern established in
phases 1a–1b.

## Decisions captured during brainstorm

| # | Decision | Choice |
|---|---|---|
| 1 | Lesson coupling | `assignments.lesson_id` nullable — assignments can be course-level OR lesson-attached. Both surfaces in UI. |
| 2 | Grading model | Per-assignment enum `grading_mode`: `numeric` (with optional letter override) OR `pass_fail`. |
| 3 | Late submission | Per-assignment `allow_late BOOL` default true. When false, server rejects submission past `due_at`. When true, submission past `due_at` carries `is_late=true` flag. |
| 4 | Resubmission | Per-assignment `lock_on_submit BOOL` default false. When false, student can edit while `submitted`. When true, edits locked once submitted. Independent of that, teacher can always flip a graded submission back to `returned` for resubmission. |
| 5 | Submission types | Per-assignment `accepts_text BOOL` + `accepts_files BOOL`, CHECK at least one true. |
| 6 | Grade release | Per-assignment enum `release_mode`: `instant` default (grade visible to student on save) OR `manual` (grade staged until teacher clicks Release). |
| 7 | Codebase | Mirror prior phases — `handlers/{assignments,submissions}.rs`, `db/{assignments,submissions}.rs`, frontend components in `features-courses/src/assignment_*.rs` + `submission_*.rs`. No new crate. |

## Section A — Schema

Three new migrations.

### `migrations/20260509000016_assignments.sql`

```sql
CREATE TYPE assignment_grading_mode AS ENUM ('numeric', 'pass_fail');
CREATE TYPE assignment_release_mode AS ENUM ('instant', 'manual');
CREATE TYPE assignment_status AS ENUM ('draft', 'published');

CREATE TABLE assignments (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    course_id UUID NOT NULL REFERENCES courses(id) ON DELETE CASCADE,
    lesson_id UUID REFERENCES lessons(id) ON DELETE SET NULL,  -- NULL = course-level
    title TEXT NOT NULL,
    instructions_md TEXT NOT NULL DEFAULT '',
    grading_mode assignment_grading_mode NOT NULL,
    max_points INTEGER,                 -- required when grading_mode='numeric'
    allow_late BOOLEAN NOT NULL DEFAULT TRUE,
    lock_on_submit BOOLEAN NOT NULL DEFAULT FALSE,
    accepts_text BOOLEAN NOT NULL DEFAULT TRUE,
    accepts_files BOOLEAN NOT NULL DEFAULT TRUE,
    release_mode assignment_release_mode NOT NULL DEFAULT 'instant',
    attachment_asset_ids UUID[] NOT NULL DEFAULT '{}',
    due_at TIMESTAMPTZ,                 -- NULL = no due date
    status assignment_status NOT NULL DEFAULT 'draft',
    published_at TIMESTAMPTZ,
    created_by UUID NOT NULL REFERENCES users(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (accepts_text OR accepts_files),
    CHECK (grading_mode <> 'numeric' OR max_points IS NOT NULL),
    CHECK (grading_mode <> 'pass_fail' OR max_points IS NULL)
);
CREATE INDEX idx_assignments_course
    ON assignments (tenant_id, course_id, status);
CREATE INDEX idx_assignments_lesson
    ON assignments (tenant_id, lesson_id) WHERE lesson_id IS NOT NULL;
ALTER TABLE assignments ENABLE ROW LEVEL SECURITY;
ALTER TABLE assignments FORCE ROW LEVEL SECURITY;
CREATE POLICY assignments_isolation ON assignments
    USING (tenant_id::text = current_setting('app.tenant_id', true));
```

### `migrations/20260509000017_submissions.sql`

```sql
CREATE TYPE submission_status AS ENUM ('draft', 'submitted', 'returned', 'graded');

CREATE TABLE submissions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    assignment_id UUID NOT NULL REFERENCES assignments(id) ON DELETE CASCADE,
    course_id UUID NOT NULL REFERENCES courses(id) ON DELETE CASCADE,
    student_user_id UUID NOT NULL REFERENCES users(id),
    status submission_status NOT NULL DEFAULT 'draft',
    text_answer TEXT,
    attachment_asset_ids UUID[] NOT NULL DEFAULT '{}',
    submitted_at TIMESTAMPTZ,
    is_late BOOLEAN NOT NULL DEFAULT FALSE,
    -- grading
    numeric_grade NUMERIC(7,2),
    letter_grade TEXT,
    passed BOOLEAN,
    student_visible_feedback TEXT,
    teacher_only_notes TEXT,
    graded_by_user_id UUID REFERENCES users(id),
    graded_at TIMESTAMPTZ,              -- when teacher saved grade
    released_at TIMESTAMPTZ,            -- when grade became visible to student
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
CREATE POLICY submissions_isolation ON submissions
    USING (tenant_id::text = current_setting('app.tenant_id', true));
```

### `migrations/20260509000018_file_assets_assignments.sql`

Extends the `linked_entity_type` CHECK constraint added in 1b-δ migration 0015
to permit `'assignment_attachment'` and `'submission_attachment'`. Same
forward-only pattern.

```sql
ALTER TABLE file_assets DROP CONSTRAINT IF EXISTS file_assets_link_type_check;
ALTER TABLE file_assets ADD CONSTRAINT file_assets_link_type_check
    CHECK (linked_entity_type IS NULL
           OR linked_entity_type IN ('course', 'lesson', 'session_recording',
                                     'assignment_attachment', 'submission_attachment'));
```

## Section B — REST API

All routes under `/v1/`, JWT auth, RLS via `app.tenant_id` GUC. Roles checked
per route via the existing `RoleGuard` middleware established in 1a.

### Assignments (teacher/TA write; student read of published only)

| Method | Path | Purpose | Roles |
|---|---|---|---|
| POST | `/courses/:course_id/assignments` | Create draft | teacher, ta, org_admin |
| PATCH | `/assignments/:id` | Edit draft fields | author, teacher, org_admin |
| POST | `/assignments/:id/publish` | `draft` → `published`, sets `published_at` | author, teacher, org_admin |
| POST | `/assignments/:id/unpublish` | `published` → `draft` (only if no submissions) | author, teacher, org_admin |
| GET | `/courses/:course_id/assignments` | List for course (students see published only) | enrolled |
| GET | `/lessons/:lesson_id/assignments` | List for lesson | enrolled |
| GET | `/assignments/:id` | Detail | enrolled |
| DELETE | `/assignments/:id` | Hard delete (only if `draft` AND no submissions) | author, org_admin |

### Submissions (one row per student per assignment)

| Method | Path | Purpose | Roles |
|---|---|---|---|
| POST | `/assignments/:id/submissions` | Idempotent create-or-get of student's submission row | enrolled student |
| PATCH | `/submissions/:id` | Edit `text_answer` + `attachment_asset_ids`. Allowed when status `draft` or `returned`. Allowed for `submitted` only when assignment `lock_on_submit=false`. | submission owner |
| POST | `/submissions/:id/submit` | `draft`/`returned` → `submitted`; sets `submitted_at`, `is_late` | submission owner |
| GET | `/assignments/:id/submissions` | List all submissions for grading | teacher, ta, org_admin |
| GET | `/submissions/:id` | Detail. Student sees grade fields only when `released_at IS NOT NULL`. | owner OR grader |
| POST | `/submissions/:id/grade` | Save grade + feedback. If `release_mode='instant'`, sets `released_at=now()`; else leaves NULL. Sets status to `graded`. | teacher, ta, org_admin |
| POST | `/submissions/:id/release` | Sets `released_at=now()` (manual mode only) | teacher, org_admin |
| POST | `/submissions/:id/return` | `graded`/`submitted` → `returned`; clears `released_at` | teacher, org_admin |

### Attachment flow (reuses 1b-α)

1. Client → `POST /uploads/begin` with size/content_type → presigned PUT URL.
2. Client → MinIO direct PUT.
3. Client → `POST /uploads/complete` → backend creates `file_assets` row with
   `linked_entity_type IN ('assignment_attachment','submission_attachment')`,
   `linked_entity_id` = assignment/submission UUID. Returns `asset_id`.
4. Client → `PATCH /assignments/:id` or `PATCH /submissions/:id` with the new
   `asset_id` appended to `attachment_asset_ids`.

## Section C — Frontend

### Routes (`shell-web` Dioxus router)

```
/courses/:slug/assignments              → AssignmentList (course-level)
/courses/:slug/assignments/new          → AssignmentEditor (teacher)
/courses/:slug/assignments/:id          → AssignmentDetail (role-aware)
/courses/:slug/assignments/:id/edit     → AssignmentEditor (teacher, draft only)
/courses/:slug/assignments/:id/grade    → SubmissionsGradingTable (teacher)
```

Lesson-attached assignments surface inline as a card in
`lesson_outline_view.rs` linking to the same `:id` detail route.

### New components in `crates/features-courses/src/`

| File | Responsibility |
|---|---|
| `assignment_list.rs` | Course-detail tab. Teachers see drafts + published; students see published only. |
| `assignment_editor.rs` | Create/edit form: title, instructions (markdown), grading_mode toggle, max_points, due_at, allow_late, lock_on_submit, accepts_text/files, release_mode, attachments via `file_picker.rs`. |
| `assignment_detail.rs` | Role-aware. Student sees their own submission card with edit/submit/grade buttons depending on state. Teacher sees "12 / 30 submitted" link to grading table. |
| `submission_form.rs` | Student-facing. Renders text_answer textarea (if `accepts_text`), file_picker (if `accepts_files`), Save Draft / Submit buttons. Disabled per status × `lock_on_submit`. |
| `submission_view.rs` | Read-only single-submission display. Used by student post-submit and by teacher in grading table. Hides grade fields when `released_at` null and viewer is student. |
| `submissions_grading_table.rs` | Teacher view: row per enrolled student. Columns: name, status, submitted_at, late?, grade. Click row → grade modal. "Release all" button when `release_mode='manual'`. |
| `submission_grade_modal.rs` | Inline grade entry: numeric input + optional letter, OR pass/fail radio. Feedback markdown. Buttons: Save & Next, Save & Return for resubmit, Save & Release (manual only). |

All components are `pub fn` (no `#[component]` macro per project convention).
SSR smoke test per component. Reuses `file_picker.rs`, `error_messages.rs`.

## Section D — Status state machine + business rules

### Submission status transitions (server-enforced)

```
                    student submit
              ┌──────────────────────┐
              │                      ▼
     ┌──────────┐                ┌───────────┐  teacher grade   ┌──────────┐
     │   draft  │                │ submitted │ ───────────────▶ │  graded  │
     └──────────┘                └───────────┘                  └──────────┘
              ▲                       │  ▲                            │
              │                       │  │ teacher return             │
   teacher    │                       │  └────────────────────────────┘
   return     │  student edits        │
              │  (if !lock_on_submit) │
              │                       ▼
              │                ┌──────────┐
              └────────────────│ returned │
                               └──────────┘
                                    │
                          student submit
                                    ▼
                               (back to submitted)
```

### Server-enforced invariants

1. **Late detection.** On `POST /submissions/:id/submit`:
   `is_late = (assignment.due_at IS NOT NULL AND now() > due_at)`.
   If `assignment.allow_late=false AND is_late=true`, reject with HTTP 422
   `submission_late_not_allowed`.
2. **Lock on submit.** `PATCH /submissions/:id` returns HTTP 409
   `submission_locked` when status is `submitted` and the assignment has
   `lock_on_submit=true`.
3. **Grade coherence.** `POST /submissions/:id/grade` validates:
   - Numeric mode ⇒ `numeric_grade` is non-null and 0 ≤ `numeric_grade` ≤ `max_points`.
   - Pass/fail mode ⇒ `passed` is non-null; `numeric_grade` and `letter_grade` must be null.
   - `letter_grade` allowed only with numeric mode (and is otherwise rejected).
4. **Release timing.** Instant mode: `released_at` set in the same transaction
   as `graded_at`. Manual mode: `released_at` stays NULL until explicit
   `/release`. Returning a graded submission clears `released_at` so a stale
   grade isn't shown if re-graded.
5. **Read filtering.** `GET /submissions/:id` returns `numeric_grade`,
   `letter_grade`, `passed`, `student_visible_feedback` only when
   `released_at IS NOT NULL` AND viewer is the student. Teachers always see
   everything. Server filters; client trusts.
6. **Delete assignment.** Refused with HTTP 409 `assignment_has_submissions` if
   any submissions exist or if status is `published`. Teacher must unpublish
   first.
7. **Unpublish assignment.** Refused with HTTP 409
   `assignment_has_submissions` if any submissions exist (any status).
8. **Idempotent submission creation.** `POST /assignments/:id/submissions`
   relies on `UNIQUE (assignment_id, student_user_id)` and returns the
   existing row when one already exists.
9. **Edit constraints on published.** `PATCH /assignments/:id` is permitted
   only when status is `draft`. Once `published`, the teacher must unpublish
   (which requires no submissions) to make changes. This prevents the grading
   contract from changing underneath students mid-flight.
10. **Submissions on draft.** `POST /assignments/:id/submissions` returns
    HTTP 404 if the assignment is `status='draft'` (students cannot see or
    interact with drafts).

### Audit events (reuses `audit_events` from 1a)

`assignment.published`, `assignment.deleted`, `submission.submitted`,
`submission.graded`, `submission.released`, `submission.returned`.

## Section E — Testing + RLS + scope cuts

### Test coverage

- **Backend integration** (`crates/backend/tests/`):
  - `assignments_crud.rs` — create draft, edit, publish, unpublish, delete (all
    edge cases including 409 paths).
  - `submissions_flow.rs` — full state machine: draft → submit → grade,
    late detection (both modes), `lock_on_submit` enforcement, grade coherence
    (numeric, pass_fail, letter), release_mode behavior (instant vs manual),
    return-for-resubmit loop.
  - `assignments_permissions.rs` — student cannot grade, cannot see drafts,
    cannot see another student's submission, cannot see grade before release.
- **RLS sweep** (`rls_tenant_isolation.rs` extended): `assignments` and
  `submissions` added to the GRANT-list. New cross-tenant probes confirm
  tenant B cannot read tenant A's assignments or submissions.
- **Frontend SSR smokes** (`crates/features-courses/tests/`): each new
  component renders without panic; status-conditional rendering verified
  (e.g., grade hidden when `released_at` null and viewer is student).
- **Pure helpers** in `db/assignments.rs`, `db/submissions.rs`: unit-tested for
  status-transition validation logic where it can live in pure Rust.

### Explicit scope cuts (deferred)

- **Notifications** (in-app/email/push when graded/returned/due-soon) →
  Phase 1d.
- **Parent dashboard view of grades** → Phase 1e.
- **Rubric-based grading**, **group submissions**, **anti-plagiarism** →
  Phase 2+.
- **Bulk download submissions as ZIP** → Phase 1d follow-up.
- **TA scoping (course-specific permissions)** → Phase 1d roles polish. For
  Phase 1c, TA = teacher equivalent.

## Open follow-ups carried into 1c (not blockers)

From the 1b-γ exit checklist (still open as of `5f7aef2`):

- §4a: Persistent WebSocket connection in `LiveRoomView` /
  `LiveRoomBroadcast` — partially landed (`use_persistent_socket` wired in
  `live_room_view.rs:47` and `live_room_broadcast.rs:64`); WhipPublisher
  storage on `Promoted`, close on `Demoted`, and toast surfacing of
  `RateLimited`/`Error` events still pending. Track as polish in 1d.
- §4b: Exercise `RedisLiveRoomBroker` end-to-end with `REDIS_URL` set —
  manual verification. Track in the 1b-complete tag gate.

These do not block 1c implementation.
