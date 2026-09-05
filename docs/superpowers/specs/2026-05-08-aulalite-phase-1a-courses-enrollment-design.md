# AulaLite — Phase 1a Design (Courses, Enrollment, Live Session Metadata)

**Date:** 2026-05-08
**Status:** Approved for plan generation
**Parent design:** `docs/superpowers/specs/2026-05-03-aulalite-scope-design.md`
**Predecessor plan:** `docs/superpowers/plans/2026-05-03-aulalite-phase-0-foundations.md`

---

## Executive Summary

Phase 1a is the first sub-slice of the design spec's Phase 1 ("P0 Core Spine"). It delivers steps 1–3 of that section — Courses + modules + lessons CRUD, enrollment (admin invite + self-enroll via code), and live-session scheduling (metadata only) — plus the live-session recurrence model that turns "schedule a class" into "schedule a recurring class with per-occurrence overrides".

**End-of-1a deliverable:** an org admin or teacher signs in to the web shell, creates a course with modules and rich-text lessons, schedules a recurring live session, generates an enrollment code or sends an email-link invite, and a student joins via either path and sees their course outline + upcoming schedule. No live class actually streams (1b), no files upload (1b), no assignments (1c).

**Scope guardrails:** lesson types restricted to `rich_text` and `live_session` at the API layer. `video` and `file_bundle` lesson types are reserved in the schema but rejected at the route level. The `file_assets` table migration ships at 1a so 1b's upload pipeline doesn't churn the schema.

---

## Scoping Decisions (Captured From Brainstorming)

| # | Question | Decision |
|---|---|---|
| 1 | Decompose Phase 1? | Yes. Split into 1a (courses/enrollment/scheduling), 1b (live class subsystem), 1c (assignments/grading). This spec covers 1a only. |
| 2 | Email-based invites | Yes — via Firebase Auth's email-link sign-in flow. Backend calls `generateSignInWithEmailLink` with `continueUrl=/accept-invite/:token`. No Resend dependency. Codes-only enrollment also ships at 1a as a fallback. |
| 3 | Recurring live sessions | Yes. Simple-enum model: `frequency ∈ {none, daily, weekly, biweekly, monthly}` with `byweekday[]` for weekly/biweekly. End condition: `count`, `until`, or `open`. RFC 5545 RRULE rejected as overkill for tutoring. |
| 4 | Per-occurrence overrides | Cancel **and** reschedule a single occurrence. Each occurrence row has a `diverged` flag; series-level edits skip diverged rows. |
| 5 | File / video upload | Pipeline architecture defined at 1a (see "Out of scope" → "Future upload pipeline (deferred)"). Schema migration for `file_assets` ships at 1a. Routes and client uploader land in 1b. |
| 6 | Mobile UI at 1a | Deferred to 1b. `shell-mobile` stays consumer-stub. |
| 7 | Course discovery / search | Deferred to Phase 2. Courses are private; access is invite/code only. |
| 8 | Recurring scheduler daemon | Stub-only at 1a. The cron entry point is defined; running it as a real daemon (extending `open` series nightly) lands in 1b. |
| 9 | Audit log emission | On every write, in the same transaction. Surface (admin-facing log viewer UI) deferred to Phase 3 per parent spec. |
| 10 | Test coverage shape | Real-Postgres integration tests for CRUD + RLS + permissions matrix; pure unit tests for recurrence expansion and other logic-only modules; SSR-render tests for new screens; no browser E2E at 1a. |

---

## Section 1 — Scope

### In scope

- **9 new migrations:** `courses`, `modules`, `lessons`, `course_memberships`, `enrollment_codes`, `course_invitations`, `live_session_series`, `live_sessions` (occurrences), `file_assets` (table only).
- **Backend REST under `/v1/`:** course CRUD, modules CRUD + reorder, lessons CRUD + reorder (with `type` restricted to `rich_text` and `live_session`), course memberships read, enrollment codes (generate / list / revoke / redeem), course invitations (create / list / revoke / accept), live session series CRUD, single occurrence patch (cancel + reschedule), student conveniences (`/v1/me/courses`, `/v1/me/schedule`).
- **Pure-function services:** `services::recurrence::expand` (no IO), `services::invitations` (Firebase Admin SDK behind a trait).
- **Web UI in `shell-web`** (via new `features-courses` crate): role-aware app shell, dashboard, course list, course create form, course detail (outline / people / schedule / edit tabs), module/lesson builder with drag-to-reorder, lesson editor (markdown + live-session picker), people page (member list + invite modal + code modal), redeem-code page, accept-invite landing page, series scheduler with occurrence preview, schedule view with per-occurrence cancel/reschedule.
- **Design system additions:** `Modal`, `Tabs`, `Badge`, `Select`, `Checkbox`, `Toggle`, `EmptyState`, `DateTimePicker`, `MarkdownEditor`.
- **RLS** (with `FORCE`) on all 9 new tables, using the existing `app.tenant_id` GUC pattern.
- **Audit emission** on every write in the same transaction as the mutation.
- **Tests:** integration tests with real Postgres for every CRUD route + per-table RLS sweep + cross-cutting permissions matrix; pure unit tests for recurrence + token generators; SSR-render tests for new screens.

### Out of scope (explicit deferrals)

- **Live class room and media pipeline** — MediaMTX integration, WebRTC publish, HLS fallback, recording uploader, recording playback. (Phase 1b.)
- **File / video upload routes and client uploader** — even though `file_assets` schema ships at 1a, no route writes to that table at 1a. Lesson types `video` and `file_bundle` rejected at the API layer with `400`. (Phase 1b.)
- **Assignments + submissions + grading.** (Phase 1c.)
- **Mobile UI changes.** (Phase 1b adds consumer-only views.)
- **Course discovery / search.** (Phase 2.)
- **Email transactional via Resend / Postmark.** Phase 1a uses Firebase email-link only. (Phase 2 layers Resend in for non-invite emails.)
- **Recurring-series daemon** that extends `open` series nightly — only the cron entry point exists at 1a; the scheduler harness lands in 1b.
- **Audit log surface (admin-facing UI to read events).** Events are emitted at 1a; the viewer is Phase 3.
- **Stripe billing, parent role, push notifications, structured search, attendance reports, branding/whitelabel.** (Phase 2 or later, per parent spec.)

### Future upload pipeline (deferred to 1b — design captured here for stability)

The schema includes `file_assets`. The 1b pipeline:

1. Client requests upload: `POST /v1/uploads/begin { content_type, size_bytes, linked_entity_type, linked_entity_id }`.
2. Backend creates `file_assets` row with `status='pending'`, computes `object_key = "{tenant_id}/{yyyy}/{mm}/{uuid}/{filename}"`, returns `{ asset_id, presigned_put_url, upload_headers }`.
3. Client `PUT`s blob directly to MinIO using the presigned URL. Backend never sees the blob.
4. Client confirms completion: `POST /v1/uploads/:id/complete { etag, observed_size }`. Backend HEADs the object to verify size + presence, sets `status='available'`.
5. If client never confirms within an expiry (default 1 hour), a janitor job sets `status='failed'` and any orphan blob in MinIO is cleaned up.

**Why pre-define this:** the lesson types `video` and `file_bundle` reference `file_assets.id` columns that already need to exist in the schema. Locking the contract now prevents 1b from rewriting `lessons` columns or migrating `file_assets`.

---

## Section 2 — Architecture

### Crate / module boundaries

```
backend/
  src/handlers/
    courses.rs            # CRUD + list-mine
    modules.rs            # nested under course; includes reorder
    lessons.rs            # nested; type-discriminated; rejects video/file_bundle at 1a
    enrollments.rs        # codes (generate + list + revoke + redeem) + invitations (create + list + revoke + accept)
    live_sessions.rs      # series CRUD + single-occurrence PATCH
    me.rs                 # extended: + my-courses, + my-schedule
  src/db/
    courses.rs            # tenant-scoped sqlx queries; no business logic
    enrollments.rs
    live_sessions.rs      # includes occurrence materialization helpers
    audit.rs              # emit_audit_event(tx, ...)
  src/services/
    invitations.rs        # trait abstraction over Firebase Admin SDK email-link
    recurrence.rs         # pure function (series_spec) -> Vec<NewOccurrence>; no IO
    slugger.rs            # pure: title -> slug, with dedup helper

features-courses/                                # NEW crate (parallel to features-auth)
  src/lib.rs
  src/app_shell.rs                              # role-aware nav
  src/dashboard.rs                              # role-switched home
  src/course_list.rs
  src/course_create.rs
  src/course_detail.rs                          # tabbed: outline / people / schedule / edit
  src/course_builder.rs                         # module/lesson tree + DnD reorder
  src/lesson_editor.rs
  src/course_people.rs
  src/invite_modal.rs
  src/code_modal.rs
  src/redeem_code.rs
  src/accept_invite.rs
  src/series_scheduler.rs
  src/schedule_view.rs
  src/error_messages.rs                         # API-error -> user-text mapper

design-system/
  src/modal.rs, tabs.rs, badge.rs, select.rs,
  src/checkbox.rs, toggle.rs, empty_state.rs,
  src/datetime_picker.rs, markdown_editor.rs

shell-web/
  Routes registered through features-courses; existing dashboard becomes role-aware home.
```

### Key architectural decisions

1. **`features-courses` is a new crate** parallel to `features-auth`. Each features-* crate stays under ~500 LoC of feature code. Login/signup/forgot-password remain in `features-auth`; everything course-related moves to the new crate. This mirrors the parent spec's Section 6 "feature crates" boundary and keeps a clear add-feature pattern.

2. **Recurrence expansion is a pure function**, not part of any handler. `services::recurrence::expand(SeriesSpec, max_occurrences) -> Vec<NewOccurrence>` is unit-testable without a database. The handler calls it, then inserts the rows in a single transaction.

3. **Occurrence materialization strategy:** at series-create time, materialize all occurrences if `end_kind ∈ {count, until}`. For `end_kind='open'`, materialize 52 ahead and store the cursor. The nightly extension daemon (cron entry point at 1a; live daemon at 1b) advances the cursor on `open` series so there are always rows ahead.

4. **Invitation flow uses Firebase Admin SDK**, not the client SDK. Backend calls `auth.generate_sign_in_with_email_link(email, ActionCodeSettings { url, handleCodeInApp: true })`. Firebase sends the email; we never touch SMTP. The `course_invitations` table holds the token + expiry + target course/role. The token is encoded in the action URL's path (`/accept-invite/:token`), not in the Firebase action code itself — we rely on Firebase only to deliver the link and authenticate the user.

5. **RLS on every new table** with `FORCE ROW LEVEL SECURITY`. Same `app.tenant_id` GUC pattern Phase 0 established (the user added the explicit `force_rls` migration in Phase 0; we maintain that habit for new tables).

6. **Audit emission** on writes: every mutating handler calls `db::audit::emit_audit_event(&mut tx, ...)` inside the same transaction as the mutation. No separate audit service. The audit *surface* (UI for org admins to read events) is deferred per parent design spec.

7. **`file_assets` table migration** lands in 1a (schema only) so 1b can implement upload routes without schema churn. Lesson types `video` and `file_bundle` are accepted by the database but rejected at the route layer with `400 Bad Request: lesson type not yet supported`.

8. **No service layer abstraction over sqlx for CRUD.** Phase 0's "thin handlers + `db::*` query module" pattern is preserved. The two `services::*` modules are present only because they encapsulate logic that genuinely belongs outside the handler (Firebase SDK calls; recurrence math).

---

## Section 3 — Data Model

All migrations live in `migrations/`, keyed `2026050800000X_*.sql`. Each table gets `tenant_id`, RLS enabled with `FORCE`, and the standard `app.tenant_id` GUC policy. UUIDs use `uuid_generate_v7()` (already enabled by Phase 0's `extensions` migration).

### Migration 1 — `courses`

```sql
CREATE TABLE courses (
  id              UUID PRIMARY KEY DEFAULT uuid_generate_v7(),
  tenant_id       UUID NOT NULL REFERENCES tenants(id) ON DELETE RESTRICT,
  slug            TEXT NOT NULL,
  title           TEXT NOT NULL,
  description     TEXT,
  status          TEXT NOT NULL CHECK (status IN ('draft','published','archived')) DEFAULT 'draft',
  visibility      TEXT NOT NULL CHECK (visibility IN ('private')) DEFAULT 'private',
  cover_asset_id  UUID,                                                       -- nullable; references file_assets when populated (1b)
  owner_user_id   UUID NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE (tenant_id, slug)
);
CREATE INDEX courses_tenant_owner_idx ON courses(tenant_id, owner_user_id);
ALTER TABLE courses ENABLE ROW LEVEL SECURITY;
ALTER TABLE courses FORCE ROW LEVEL SECURITY;
CREATE POLICY courses_tenant_isolation ON courses
  USING (tenant_id = current_setting('app.tenant_id')::uuid);
```

The `visibility` enum reserves a single value (`'private'`) at 1a — column shape is forward-compatible with the eventual `'tenant' / 'public'` values.

### Migration 2 — `modules`

```sql
CREATE TABLE modules (
  id              UUID PRIMARY KEY DEFAULT uuid_generate_v7(),
  tenant_id       UUID NOT NULL,
  course_id       UUID NOT NULL REFERENCES courses(id) ON DELETE CASCADE,
  title           TEXT NOT NULL,
  sort_order      INT NOT NULL,
  created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX modules_course_sort_idx ON modules(course_id, sort_order);
ALTER TABLE modules ENABLE ROW LEVEL SECURITY;
ALTER TABLE modules FORCE ROW LEVEL SECURITY;
CREATE POLICY modules_tenant_isolation ON modules
  USING (tenant_id = current_setting('app.tenant_id')::uuid);
```

`sort_order` is gap-friendly (10, 20, 30) so insertions don't always require bulk renumbering.

### Migration 3 — `lessons`

```sql
CREATE TABLE lessons (
  id              UUID PRIMARY KEY DEFAULT uuid_generate_v7(),
  tenant_id       UUID NOT NULL,
  course_id       UUID NOT NULL REFERENCES courses(id) ON DELETE CASCADE,
  module_id       UUID NOT NULL REFERENCES modules(id) ON DELETE CASCADE,
  type            TEXT NOT NULL CHECK (type IN ('rich_text','video','live_session','file_bundle')),
  title           TEXT NOT NULL,
  body_md         TEXT,
  video_asset_id  UUID,                                                       -- references file_assets(id) when populated (1b)
  live_session_id UUID,                                                       -- forward-declared; FK added after live_sessions migration
  sort_order      INT NOT NULL,
  published_at    TIMESTAMPTZ,
  created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX lessons_module_sort_idx ON lessons(module_id, sort_order);
ALTER TABLE lessons ENABLE ROW LEVEL SECURITY;
ALTER TABLE lessons FORCE ROW LEVEL SECURITY;
CREATE POLICY lessons_tenant_isolation ON lessons
  USING (tenant_id = current_setting('app.tenant_id')::uuid);
```

`module_id` is required (no top-level lessons at 1a). API rejects `type IN ('video','file_bundle')` so we can store the values when 1b populates them.

### Migration 4 — `course_memberships`

```sql
CREATE TABLE course_memberships (
  course_id       UUID NOT NULL REFERENCES courses(id) ON DELETE CASCADE,
  user_id         UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  tenant_id       UUID NOT NULL,
  role            TEXT NOT NULL CHECK (role IN ('teacher','ta','student')),
  status          TEXT NOT NULL CHECK (status IN ('active','removed')) DEFAULT 'active',
  joined_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (course_id, user_id)
);
CREATE INDEX course_memberships_user_idx ON course_memberships(user_id, tenant_id);
ALTER TABLE course_memberships ENABLE ROW LEVEL SECURITY;
ALTER TABLE course_memberships FORCE ROW LEVEL SECURITY;
CREATE POLICY course_memberships_tenant_isolation ON course_memberships
  USING (tenant_id = current_setting('app.tenant_id')::uuid);
```

The course owner gets an implicit teacher membership inserted in the same transaction as the course.

### Migration 5 — `enrollment_codes`

```sql
CREATE TABLE enrollment_codes (
  id              UUID PRIMARY KEY DEFAULT uuid_generate_v7(),
  tenant_id       UUID NOT NULL,
  course_id       UUID NOT NULL REFERENCES courses(id) ON DELETE CASCADE,
  code            TEXT NOT NULL UNIQUE,
  max_uses        INT,
  uses            INT NOT NULL DEFAULT 0,
  expires_at      TIMESTAMPTZ,
  created_by      UUID NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);
ALTER TABLE enrollment_codes ENABLE ROW LEVEL SECURITY;
ALTER TABLE enrollment_codes FORCE ROW LEVEL SECURITY;
CREATE POLICY enrollment_codes_tenant_isolation ON enrollment_codes
  USING (tenant_id = current_setting('app.tenant_id')::uuid);
```

Code is **globally unique** (not per-tenant) so a student can paste any code without ambiguity. Generation: 8-char base32, alphabet excludes `0/O/1/I` (10 ambiguous-pair characters total), retry on collision (probability ~0.0003 at 1M codes).

**Cross-tenant RLS challenge.** A student redeeming a code might not yet be a member of any tenant (or might be a member of a *different* tenant). Their `app.tenant_id` GUC is therefore wrong (or null) when they hit `/v1/codes/redeem`, so a plain `SELECT` against `enrollment_codes` would return nothing. Same problem applies to invitation acceptance: the invitee's tenant context doesn't match the inviting tenant.

**Solution: a small set of `SECURITY DEFINER` lookup functions** that bypass RLS for exactly the cross-tenant resolution step:

```sql
CREATE FUNCTION lookup_enrollment_code(p_code TEXT)
  RETURNS TABLE(code_id UUID, tenant_id UUID, course_id UUID, max_uses INT, uses INT, expires_at TIMESTAMPTZ)
  LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public
AS $$ SELECT id, tenant_id, course_id, max_uses, uses, expires_at FROM enrollment_codes WHERE code = p_code $$;

CREATE FUNCTION lookup_invitation_by_token(p_token TEXT)
  RETURNS TABLE(invitation_id UUID, tenant_id UUID, course_id UUID, email CITEXT, role TEXT, status TEXT, expires_at TIMESTAMPTZ)
  LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public
AS $$ SELECT id, tenant_id, course_id, email, role, status, expires_at FROM course_invitations WHERE token = p_token $$;
```

The functions are owned by the migration role (which has BYPASSRLS implicitly for tables it owns). They return *only* enough fields to drive the next step — never expose the full code/invitation row to a caller who isn't permissioned. Redemption flow:

1. Handler calls `lookup_enrollment_code(p_code)` → resolves `tenant_id`. If the code doesn't exist or is expired, return `EnrollmentCodeInvalid` (constant-time response shape to avoid token-fishing).
2. Handler `SET LOCAL app.tenant_id = <resolved tenant_id>`.
3. Inside the now-correct tenant context: `SELECT … FOR UPDATE` the code via the regular table, validate `uses < max_uses`, increment `uses`, insert `tenant_memberships` (if missing) + `course_memberships`. All in one transaction.
4. Emit audit event in the same transaction.

Invitation acceptance follows the same pattern with `lookup_invitation_by_token`. The migration creating `enrollment_codes` and `course_invitations` also creates these helper functions and grants `EXECUTE` to the application role only.

### Migration 6 — `course_invitations`

```sql
CREATE TABLE course_invitations (
  id              UUID PRIMARY KEY DEFAULT uuid_generate_v7(),
  tenant_id       UUID NOT NULL,
  course_id       UUID NOT NULL REFERENCES courses(id) ON DELETE CASCADE,
  email           CITEXT NOT NULL,
  role            TEXT NOT NULL CHECK (role IN ('teacher','ta','student')),
  token           TEXT NOT NULL UNIQUE,                                       -- 32-byte base64url
  status          TEXT NOT NULL CHECK (status IN ('pending','accepted','revoked','expired')) DEFAULT 'pending',
  expires_at      TIMESTAMPTZ NOT NULL,                                       -- default now() + 14 days
  created_by      UUID NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  accepted_by     UUID REFERENCES users(id),
  accepted_at     TIMESTAMPTZ,
  created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX course_invitations_pending_unique
  ON course_invitations(course_id, lower(email)) WHERE status = 'pending';
ALTER TABLE course_invitations ENABLE ROW LEVEL SECURITY;
ALTER TABLE course_invitations FORCE ROW LEVEL SECURITY;
CREATE POLICY course_invitations_tenant_isolation ON course_invitations
  USING (tenant_id = current_setting('app.tenant_id')::uuid);
```

The partial unique index enforces "one pending invite per email per course" — a revoked invite doesn't block a fresh one. The token is what the `/accept-invite/:token` route extracts; it's separate from Firebase's email-link OOB code.

### Migration 7 — `live_session_series`

```sql
CREATE TABLE live_session_series (
  id                  UUID PRIMARY KEY DEFAULT uuid_generate_v7(),
  tenant_id           UUID NOT NULL,
  course_id           UUID NOT NULL REFERENCES courses(id) ON DELETE CASCADE,
  title               TEXT NOT NULL,
  starts_at           TIMESTAMPTZ NOT NULL,
  duration_minutes    INT NOT NULL CHECK (duration_minutes BETWEEN 5 AND 480),
  frequency           TEXT NOT NULL CHECK (frequency IN ('none','daily','weekly','biweekly','monthly')),
  byweekday           TEXT[],                                                 -- ['mon','wed','fri'] when weekly/biweekly
  end_kind            TEXT NOT NULL CHECK (end_kind IN ('count','until','open')),
  occurrence_count    INT,
  end_until           TIMESTAMPTZ,
  primary_teacher_id  UUID NOT NULL REFERENCES users(id),
  recording_enabled   BOOL,                                                   -- nullable; null = inherit tenant default
  open_cursor         TIMESTAMPTZ,                                            -- only set when end_kind='open'; tracks "next-occurrence-to-materialize" boundary
  created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
  CHECK (
    (end_kind = 'count' AND occurrence_count IS NOT NULL AND end_until IS NULL) OR
    (end_kind = 'until' AND end_until IS NOT NULL AND occurrence_count IS NULL) OR
    (end_kind = 'open'  AND occurrence_count IS NULL AND end_until IS NULL)
  ),
  CHECK (
    (frequency IN ('weekly','biweekly') AND byweekday IS NOT NULL AND array_length(byweekday, 1) > 0) OR
    (frequency NOT IN ('weekly','biweekly') AND byweekday IS NULL)
  )
);
ALTER TABLE live_session_series ENABLE ROW LEVEL SECURITY;
ALTER TABLE live_session_series FORCE ROW LEVEL SECURITY;
CREATE POLICY live_session_series_tenant_isolation ON live_session_series
  USING (tenant_id = current_setting('app.tenant_id')::uuid);
```

`frequency='none'` represents a one-off — the schema treats one-offs and recurrences uniformly so the API has one shape.

### Migration 8 — `live_sessions` (occurrences)

```sql
CREATE TABLE live_sessions (
  id                  UUID PRIMARY KEY DEFAULT uuid_generate_v7(),
  tenant_id           UUID NOT NULL,
  course_id           UUID NOT NULL REFERENCES courses(id) ON DELETE CASCADE,
  series_id           UUID NOT NULL REFERENCES live_session_series(id) ON DELETE CASCADE,
  occurrence_index    INT NOT NULL,
  title               TEXT NOT NULL,
  status              TEXT NOT NULL CHECK (status IN ('scheduled','live','ended','cancelled')) DEFAULT 'scheduled',
  starts_at           TIMESTAMPTZ NOT NULL,
  duration_minutes    INT NOT NULL,
  actual_started_at   TIMESTAMPTZ,
  actual_ended_at     TIMESTAMPTZ,
  primary_teacher_id  UUID NOT NULL REFERENCES users(id),
  ta_user_ids         UUID[] NOT NULL DEFAULT '{}',
  mode                TEXT NOT NULL CHECK (mode IN ('lecture','discussion')) DEFAULT 'lecture',
  recording_enabled   BOOL NOT NULL,                                          -- resolved at create from series + tenant
  main_path           TEXT,
  hls_fallback_enabled BOOL NOT NULL DEFAULT false,
  diverged            BOOL NOT NULL DEFAULT false,
  created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE (series_id, occurrence_index)
);
CREATE INDEX live_sessions_course_starts_idx ON live_sessions(course_id, starts_at);
ALTER TABLE live_sessions ENABLE ROW LEVEL SECURITY;
ALTER TABLE live_sessions FORCE ROW LEVEL SECURITY;
CREATE POLICY live_sessions_tenant_isolation ON live_sessions
  USING (tenant_id = current_setting('app.tenant_id')::uuid);

-- Now add the deferred FK from lessons.live_session_id
ALTER TABLE lessons
  ADD CONSTRAINT lessons_live_session_id_fkey
  FOREIGN KEY (live_session_id) REFERENCES live_sessions(id) ON DELETE SET NULL;
```

Series edits update only rows where `diverged = false`. Single-occurrence cancel/reschedule sets `diverged = true` so future series edits skip it.

### Migration 9 — `file_assets` (schema only)

```sql
CREATE TABLE file_assets (
  id                  UUID PRIMARY KEY DEFAULT uuid_generate_v7(),
  tenant_id           UUID NOT NULL,
  owner_user_id       UUID NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  bucket              TEXT NOT NULL,
  object_key          TEXT NOT NULL,
  content_type        TEXT NOT NULL,
  size_bytes          BIGINT NOT NULL,
  status              TEXT NOT NULL CHECK (status IN ('pending','available','failed','pruned')) DEFAULT 'pending',
  visibility          TEXT NOT NULL CHECK (visibility IN ('private','course','public')) DEFAULT 'private',
  linked_entity_type  TEXT,
  linked_entity_id    UUID,
  created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE (bucket, object_key)
);
ALTER TABLE file_assets ENABLE ROW LEVEL SECURITY;
ALTER TABLE file_assets FORCE ROW LEVEL SECURITY;
CREATE POLICY file_assets_tenant_isolation ON file_assets
  USING (tenant_id = current_setting('app.tenant_id')::uuid);
```

No routes write to this table at 1a. Migration only.

### Schema invariants confirmed

- Every new table has `tenant_id` + RLS with `FORCE`.
- All FKs to user-scoped tables have appropriate `ON DELETE`: `CASCADE` for course-scoped children, `RESTRICT` for owner relations to surface dangling-data attempts.
- Audit events emitted in same transaction as mutation.
- `lessons.live_session_id` FK is added in migration 8 (after `live_sessions` exists), avoiding circular dependency.

---

## Section 4 — API Surface

All routes under `/v1/`, all return JSON, all require a valid Firebase ID token (existing middleware extracts `RequestContext`). Permissions enforced in handlers; RLS is the second line of defense.

### Courses

| Method | Path | Auth | Notes |
|---|---|---|---|
| `POST` | `/v1/courses` | teacher / org_admin | Body: `{slug, title, description?}`. Sets `owner_user_id = caller`; `org_admin` may pass `owner_user_id` to create-on-behalf. Inserts owner course_membership(role='teacher'). |
| `GET` | `/v1/courses` | any | Returns courses the caller can access (teacher: owned + membered; org_admin: all in tenant; student: active memberships). |
| `GET` | `/v1/courses/:id` | course member or org_admin | Includes derived counts: `module_count`, `student_count`, `next_session_at`. |
| `PATCH` | `/v1/courses/:id` | course owner or org_admin | Partial. `status` transitions enforced (`draft → published → archived`, monotonic). |
| `DELETE` | `/v1/courses/:id` | course owner or org_admin | Hard delete cascades. Refuses if any submissions exist (1c-aware check; no-op at 1a). |

### Modules

| Method | Path | Auth | Notes |
|---|---|---|---|
| `POST` | `/v1/courses/:cid/modules` | course owner or org_admin | Body: `{title}`. Server assigns `sort_order = (max+10)` or 10 if first. |
| `PATCH` | `/v1/courses/:cid/modules/:mid` | course owner or org_admin | Partial — title only. |
| `POST` | `/v1/courses/:cid/modules/reorder` | course owner or org_admin | Body: `{module_ids: [uuid…]}`. Single tx, renumbers `sort_order` 10/20/30/… |
| `DELETE` | `/v1/courses/:cid/modules/:mid` | course owner or org_admin | Cascades lessons. |

### Lessons

| Method | Path | Auth | Notes |
|---|---|---|---|
| `POST` | `/v1/courses/:cid/modules/:mid/lessons` | course owner or org_admin | Body: `{type, title, body_md?, live_session_id?}`. **Server rejects `type ∈ {'video','file_bundle'}`** with `400`. For `'live_session'`, `live_session_id` must reference a session in the same course. |
| `PATCH` | `/v1/courses/:cid/modules/:mid/lessons/:lid` | course owner or org_admin | Partial. Same type-rejection rule. |
| `POST` | `/v1/courses/:cid/modules/:mid/lessons/reorder` | course owner or org_admin | Body: `{lesson_ids: [uuid…]}`. |
| `DELETE` | `/v1/courses/:cid/modules/:mid/lessons/:lid` | course owner or org_admin | Hard delete. |

### Memberships (read-only listing)

| Method | Path | Auth | Notes |
|---|---|---|---|
| `GET` | `/v1/courses/:cid/members` | course member or org_admin | Lists active memberships with display data. Students see only their own row + teachers (privacy default). |

### Enrollment codes

| Method | Path | Auth | Notes |
|---|---|---|---|
| `POST` | `/v1/courses/:cid/codes` | course owner or org_admin | Body: `{max_uses?, expires_at?}`. Returns `{code, …}` once — never re-displayed. |
| `GET` | `/v1/courses/:cid/codes` | course owner or org_admin | List active codes for the course (without re-revealing the code string — only metadata + last-4-of-code for ID). |
| `DELETE` | `/v1/courses/:cid/codes/:codeid` | course owner or org_admin | Marks code revoked (`expires_at = now()`). |
| `POST` | `/v1/codes/redeem` | any authenticated user | Body: `{code}`. Single tx; cross-tenant safe (handler escalates briefly to read code, then sets `app.tenant_id` from the code's tenant). Returns `{course_id, course_title}` for redirect. |

### Course invitations (Firebase email-link)

| Method | Path | Auth | Notes |
|---|---|---|---|
| `POST` | `/v1/courses/:cid/invitations` | course owner or org_admin | Body: `{email, role}`. Creates `course_invitations` row, calls Firebase Admin `generateSignInWithEmailLink(email, ActionCodeSettings { url: "{APP_URL}/accept-invite/{token}", handleCodeInApp: true })`. Email is sent by Firebase. Returns `{invitation_id, email, expires_at}`. |
| `GET` | `/v1/courses/:cid/invitations` | course owner or org_admin | List pending. |
| `DELETE` | `/v1/courses/:cid/invitations/:iid` | course owner or org_admin | Sets status='revoked'. |
| `POST` | `/v1/invitations/:token/accept` | any authenticated user | Caller's Firebase email must match invitation email. Single tx: marks invitation accepted, creates tenant_membership if missing, creates course_membership. |

### Live session series

| Method | Path | Auth | Notes |
|---|---|---|---|
| `POST` | `/v1/courses/:cid/sessions` | course owner or org_admin | Body: `{title, starts_at, duration_minutes, frequency, byweekday?, end_kind, occurrence_count?, end_until?, primary_teacher_id?, recording_enabled?}`. Calls `services::recurrence::expand`, inserts series row + N occurrences in one tx. `frequency='none'` ⇒ 1 occurrence. Returns `{series, occurrences: [...]}`. |
| `GET` | `/v1/courses/:cid/sessions` | course member or org_admin | Returns series list with next 5 occurrences pre-joined; `?after=<ts>` paginates. |
| `GET` | `/v1/series/:sid` | course member or org_admin | Series detail with all materialized occurrences. |
| `PATCH` | `/v1/series/:sid` | course owner or org_admin | Updates non-recurrence fields (title, default duration, recording_enabled, primary_teacher_id) on the series AND propagates to non-diverged occurrences. Recurrence shape changes (frequency/byweekday/end_kind) require `?regenerate=true` and rebuild future non-diverged occurrences. |
| `DELETE` | `/v1/series/:sid` | course owner or org_admin | Cascades occurrences. |

### Live session occurrences (single)

| Method | Path | Auth | Notes |
|---|---|---|---|
| `PATCH` | `/v1/sessions/:id` | course owner or org_admin | Body subset: `{starts_at?, duration_minutes?, title?, primary_teacher_id?, status?}`. Sets `diverged = true` if anything other than `status` changes. `status` transitions allowed at 1a: `scheduled → cancelled` (revertible to `scheduled`). `live` and `ended` rejected at 1a (1b only). |

### Student conveniences

| Method | Path | Auth | Notes |
|---|---|---|---|
| `GET` | `/v1/me/courses` | any | Active course_memberships with each course's next upcoming session. Powers student dashboard. |
| `GET` | `/v1/me/schedule` | any | Upcoming `live_sessions` across all enrolled courses, ordered by `starts_at`. Default window: next 30 days. |

### Permission matrix

| Role | Courses | Modules / Lessons | Codes | Invites | Sessions |
|---|---|---|---|---|---|
| `org_admin` | CRUD any in tenant | Full | Full | Full | Full |
| `teacher` (course owner) | Read own; write own; delete own (no submissions) | Full on own | Full on own | Full on own | Full on own |
| `teacher` (not owner) | Read course they're member of | Read | — | — | Read (no write at 1a) |
| `ta` | Read enrolled course | Read | — | — | Read |
| `student` | Read enrolled courses; redeem code; accept invite | Read | — | — | Read |

### Error shapes

Use existing `ApiError` enum. New variants added at 1a:

- `CourseNotFound`
- `EnrollmentCodeInvalid` (covers expired / revoked / over-cap)
- `InvitationInvalid` (covers expired / wrong-email / already-accepted / revoked)
- `LessonTypeNotSupported` (for `video` / `file_bundle` at 1a)
- `RecurrenceShapeInvalid` (CHECK-constraint friendly errors raised before INSERT for clearer messaging)

All return `{"error": "<message>"}` JSON consistent with existing `/v1/me` shape.

---

## Section 5 — UI Surface (web)

All in `shell-web` via the new `features-courses` crate. Phase 0 already shipped the in-app router (`/login`, `/signup`, `/forgot`, `/dashboard`); 1a adds routes underneath an authenticated layout shell.

### Route map

```
/                                   redirect to /login or /dashboard
/login, /signup, /forgot            (Phase 0)
/accept-invite/:token               invitation landing page (Firebase email-link target)
/dashboard                          role-aware home (replaces Phase 0 stub)

/courses                            my courses index
/courses/new                        teacher / org_admin — create course form
/courses/:slug                      course detail (role-aware view)
/courses/:slug/edit                 owner / org_admin — course settings
/courses/:slug/build                owner / org_admin — module/lesson builder
/courses/:slug/people               owner / org_admin — member list + invite + codes
/courses/:slug/schedule             all members — schedule view (read), owner/admin can edit

/redeem                             student code-paste form
/me/schedule                        unified upcoming-sessions view
```

### Component breakdown

| Module (`features-courses/src/`) | Responsibility | Used on |
|---|---|---|
| `app_shell.rs` | Top-bar + side nav + role-aware menu items + active-tenant indicator | All authenticated routes |
| `dashboard.rs` | Role-switched home. Teacher: My Courses + Upcoming. Org admin: same + All Tenant Courses. Student: Enrolled + Upcoming. | `/dashboard` |
| `course_list.rs` | Card grid; status filter chips; "+ New Course" CTA when authorized | `/courses` |
| `course_create.rs` | Form: title, slug (auto-generated, editable), description; org_admin gets owner-picker | `/courses/new` |
| `course_detail.rs` | Header + tabs: Outline (read), People (admin), Schedule, Edit | `/courses/:slug` |
| `course_builder.rs` | Module/lesson tree; HTML5 DnD reorder; inline rename; "Add module" / "Add lesson" buttons; lesson-type picker (limited to `rich_text` + `live_session` at 1a; disabled options show tooltip "available in 1b") | `/courses/:slug/build` |
| `lesson_editor.rs` | Side-panel/modal editor. `rich_text` → markdown textarea + preview tab. `live_session` → picker dropdown of unscheduled sessions in this course + "Schedule new" link | embedded |
| `course_people.rs` | Member table + "Invite by email" button + "Generate code" button; active codes list | `/courses/:slug/people` |
| `invite_modal.rs` | Email input + role select + submit; on success "Sent to <email>" toast | embedded |
| `code_modal.rs` | Form: max_uses ("unlimited") + expires (date or "never") + Generate; success displays code prominently with copy + "won't be shown again" banner | embedded |
| `redeem_code.rs` | Single text input + submit; on success redirects to course | `/redeem` |
| `accept_invite.rs` | Reads `:token` + Firebase email-link sign-in state; calls `/v1/invitations/:token/accept`; success → redirect to course | `/accept-invite/:token` |
| `series_scheduler.rs` | Form: title, start datetime, duration, frequency, byweekday chips, end-kind tabs, recording-enabled toggle. Live preview of generated occurrences. | `/courses/:slug/schedule?action=create` |
| `schedule_view.rs` | List grouped by week; per-occurrence kebab menu (owner/admin): Cancel, Reschedule, View Series. Cancelled = struck-through. Diverged = "edited" badge. | `/courses/:slug/schedule`, `/me/schedule` |
| `error_messages.rs` | API-error → user-text mapper | shared |

### Design system additions (`design-system`)

Phase 0 shipped Button, Input, Card, FormError, Spinner. 1a adds:

- `Modal` — overlay + backdrop + focus-trap + ESC-close.
- `Tabs` — top-level + within-page.
- `Badge` — status pills.
- `Select` — basic dropdown.
- `Checkbox`, `Toggle`.
- `EmptyState` — text + CTA.
- `DateTimePicker` — native `<input type="datetime-local">` with light styling at 1a.
- `MarkdownEditor` — textarea + preview tab; `pulldown-cmark` for render.

### Role-aware navigation

`app_shell` reads `RequestContext.tenant_role` from a context provider populated at app boot via `/v1/me`. Menu items:

- **Teacher / Org admin:** Dashboard, My Courses, *(admin only)* All Tenant Courses.
- **Student:** Dashboard, My Courses, My Schedule, Redeem Code.
- **TA:** Dashboard, My Courses (read-only badges).

Account menu: name/avatar → Sign out.

### Form / error UX

- Forms use existing `FormError` for inline validation + a top-of-form summary on submit failure.
- API errors are mapped to user-friendly text in one place (`features-courses::error_messages`):
  - `CourseNotFound` → "This course doesn't exist or you don't have access."
  - `EnrollmentCodeInvalid` → "That code is invalid, expired, or fully used."
  - `InvitationInvalid` → "This invitation is no longer valid. Ask the teacher for a new one."
  - `LessonTypeNotSupported` → "That lesson type isn't available yet. Use a rich-text or live-session lesson for now."
- Loading uses `Spinner` inline; navigation transitions use a top-bar progress bar.

### State management

Stay with Dioxus 0.7 signals; no global store. Each page fetches its own data via async resources. Cross-page invalidation is keyed by tenant + role only — when a course is created, the courses list re-fetches on next visit (no live invalidation; cheap and good enough for 1a).

### Out of scope for 1a UI

- Mobile shell touchups (`shell-mobile` stays consumer-stub).
- Drag-and-drop file upload (1b).
- Inline rich-text editing in the dashboard (1b — the 1a markdown editor is fine).
- Calendar widget on the dashboard (just a list).

---

## Section 6 — Testing Strategy

Carries forward Phase 0's "real-Postgres-with-RLS for integration tests, pure unit tests for logic-only modules" pattern. No new test infra required.

### Backend pure unit tests

- `services::recurrence::expand` — table-driven over `frequency × byweekday × end_kind`. Cases:
  - `frequency='none'` → exactly 1 occurrence at `starts_at`.
  - `weekly + byweekday=['mon','wed','fri'] + count=10` → 10 occurrences in pattern; indices 0..9.
  - `biweekly + byweekday=['tue']` skips alternating weeks.
  - `monthly` advances by `INTERVAL '1 month'`; Feb-29 starts_at falls back to Feb-28 in non-leap years.
  - `end_kind='until'` truncates at the cutoff.
  - `end_kind='open'` caps at 52.
- Invitation token generator — properties: 32-byte payload, base64url-safe (no `+/=`), unique across 10k samples.
- Course slug generator from title — properties: lowercase, dashes, ASCII-fold, dedup via `-2`/`-3` suffixes.

### Backend integration tests (`#[sqlx::test]`)

Per resource, three flavors:

1. **Happy path** — owner creates resource; reads back; updates; deletes.
2. **Permissions** — table-driven over role × action; each cell asserts 200 or 403 against expected.
3. **RLS** — same operation as caller from tenant A and tenant B; assert tenant B can't see tenant A's data even when bypassing handler-level checks.

Specific high-value tests:

- `courses`: hard-delete refuses with mock submission row; status transitions enforce monotonic forward.
- `modules` / `lessons`: cascade delete; reorder route renumbers atomically.
- `course_memberships`: implicit teacher membership inserted at course creation.
- `enrollment_codes`: redemption is atomic — concurrent two-redeemer race on `max_uses=1` proves only one wins.
- `course_invitations`: accept rejects when `accepted_by` email ≠ invitation email; partial unique-index enforces "one pending per email per course"; expiry honored.
- `live_session_series` + `live_sessions`: series-create writes both rows in one tx (rollback test by injecting a failing trigger). Per-occurrence cancel sets `diverged=true`. Series PATCH with `regenerate=false` skips diverged. Series PATCH with `regenerate=true` rebuilds non-diverged future occurrences.

### Firebase Admin SDK mocking

Mock at the service-layer boundary. `services::invitations::EmailLinkSender` is a trait; production impl wraps the Firebase SDK; tests inject `MockEmailLinkSender` that captures `(email, action_url)` calls without network. Route tests assert both DB state AND that the mock was called with the expected URL.

### Test layout

```
backend/tests/
  rls_tenant_isolation.rs         (Phase 0; we extend its sweep with new tables)
  health.rs                       (Phase 0)
  me_endpoint.rs                  (Phase 0)
  courses_crud.rs                 (NEW)
  modules_crud.rs                 (NEW)
  lessons_crud.rs                 (NEW)
  enrollment_codes.rs             (NEW)
  course_invitations.rs           (NEW)
  live_session_series.rs          (NEW)
  live_session_occurrences.rs     (NEW)
  permissions_matrix.rs           (NEW; cross-cutting role × action assertions)

backend/src/services/recurrence.rs  (unit tests inline)
```

### Frontend tests

Phase 0 established the SSR-render test pattern (commit `de24e9b` introduced the SSR dev-dep for the mobile shell). Continue that pattern:

- Per new screen, an SSR-render test asserts the component produces expected text/structure for representative state (logged out, teacher view, student view, error state).
- `services::error_messages` mapper — table-driven unit test over every error variant.
- `course_builder` reorder logic — pure unit test over the local state-mutation function (DnD wiring is integration-level and not unit-tested at 1a; manual verification covers it).

No browser-driven E2E at 1a. Pulling in Playwright is a substantial project of its own — defer.

### CI gates

- `cargo test --workspace` must remain green.
- `cargo build -p shell-web --target wasm32-unknown-unknown` must remain green.
- `cargo check -p shell-mobile --target aarch64-linux-android` must remain green.
- `dx build --platform web` runs only on tagged release builds (already the case).

### What testing does NOT cover at 1a (deliberately)

- Browser smoke for end-to-end course-creation → email-invite → accept flow. Manual, runbook-style verification — added to the Phase 1a exit checklist (analogous to Phase 0's exit checklist).
- Mobile UI (no UI changes shipped at 1a).
- Load testing of code-redemption concurrency above the two-redeemer race test. Defer to Phase 3 alongside other load work.
- Firebase email actually sending (the mock-SDK tests prove our call shape; whether the email lands in the inbox is verified manually per environment in the runbook).

---

## Section 7 — Phase 1a Exit Checklist (high-level — full version lives with the implementation plan)

1. **Stack health** — `docker compose up -d`, all five services healthy.
2. **Migrations** — `sqlx migrate info` shows all 9 new migrations applied; tables present.
3. **Automated** — `cargo test --workspace`, `cargo build -p shell-web --target wasm32-unknown-unknown`, `cargo check -p shell-mobile --target aarch64-linux-android` all green.
4. **Course creation flow (web)** — sign in as teacher → create course → add module → add rich-text lesson → schedule recurring weekly session (3 occurrences) → cancel one occurrence → reschedule another → confirm `diverged` flag in DB.
5. **Email-link invite flow (web)** — invite a real email address → receive email → click → land on accept-invite → confirm course_membership inserted.
6. **Code-redeem flow (web)** — sign in as new student → redeem code → land on course detail → confirm course + tenant memberships inserted.
7. **Permissions** — confirm a teacher in tenant A cannot read a course in tenant B (RLS).

Tag `phase-1a-complete` only after every required check passes.

---

## Open questions / risks / next steps

1. **Firebase project email-link configuration.** Phase 0 enabled Email/Password and Google. We need email-link auth enabled and `app.elementors.guru` (and `localhost:3000` for dev) on the authorized continue-URL list. Docs: Firebase Auth → Sign-in method → Email/Password → "Email link (passwordless sign-in)".
2. **Firebase email branding ceiling.** Subject + display name + sender are configurable; layout is not (without paying for Firebase Identity Platform). If branding is critical at MVP, plan to switch to Resend in Phase 2 — but the `course_invitations` table shape doesn't change.
3. **Recurrence DST behavior.** A weekly Tuesday-5pm series spanning a DST transition keeps wall-clock time (5pm local) but UTC shifts. This is the conventional behavior most users expect. Storing all timestamps as `TIMESTAMPTZ` with the user's tenant-level default timezone gets us this for free; we'll surface the timezone setting in Phase 2 alongside org-admin branding.
4. **`open` series cap of 52 weeks.** Conservative. If a tutoring center wants a year+ open-ended class, they hit the cap. The nightly extender daemon (1b) advances the cap; until then, the 52-occurrence cap is the practical limit.
5. **Concurrent code redemption.** The single-tx `SELECT FOR UPDATE` design is correct under Postgres' default isolation level. Worth a load test in Phase 3 (1000 redeemers / second class hold + immediate self-enroll).
