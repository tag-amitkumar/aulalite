# AulaLite Phase 1d-a - Audit Closure Design

**Date:** 2026-05-10
**Status:** Approved for spec write-up
**Predecessor:** Phase 1.5 shell wiring
**Primary goal:** Close the web audit blockers before starting the original Phase 1d feature set.

## Overview

Phase 1d-a is an audit-closure slice. It is not the full original Phase 1d.
The work focuses on making the web product credible end-to-end: real course
detail tabs, real assignment reference attachments, local audit seed data, and
verified checklist evidence. Only after the app flow is green do we address the
local Docker/8080 stack-health blocker.

The original Phase 1d items - notifications, parent role, TA scoping, and
desktop/mobile platform bridges - remain out of scope for this slice.

## Decisions

| # | Decision | Choice |
|---|---|---|
| 1 | Phase shape | Split Phase 1d into `1d-a Audit Closure` first, then original 1d features later. |
| 2 | Priority | App/browser blockers first; Docker/8080 after app flow is proven. |
| 3 | CourseDetail tabs | Replace placeholders with real data using existing feature components. |
| 4 | Backend API policy | Add small missing read endpoints only where the existing UI cannot be wired otherwise. |
| 5 | Local audit data | Add guarded local/dev seed support rather than relying on manual DB row crafting. |
| 6 | Scope control | No notifications, parent dashboard, TA moderation/scoping changes, or platform bridge auth in this slice. |

## Scope

### In scope

- CourseDetail tabs stop showing placeholder content.
- Outline tab renders real modules and lessons.
- People tab renders real course members, pending invites, and active enrollment codes.
- Edit tab supports course metadata and cover editing through existing course patch/upload paths.
- Schedule tab renders real course sessions and supports scheduling/cancel/reschedule where backend support already exists.
- AssignmentEditor supports teacher reference attachments through the existing file picker and `assignment_attachment` file asset path.
- Local audit seed support creates enough tenant/user/course data to run the teacher and student browser checklist with local bypass profiles.
- The Phase 1.5 exit checklist is updated with verified pass/fail evidence.
- Docker/8080 stack health is addressed after app blockers are resolved.

### Out of scope

- Notifications.
- Parent role, parent links, or parent dashboard.
- TA moderation or new TA scoping behavior beyond displaying existing `ta` roles.
- Desktop/mobile platform bridge auth implementation.
- Mobile native publishing.
- Broad design-system redesign.
- Destructive database reset or migration history edits unless explicitly approved.

## Architecture

### Frontend structure

`shell-web/src/routes/course_detail.rs` remains the route-level coordinator.
It resolves the course slug, owns navigation between tabs, and passes typed
props into feature components from `features-courses`.

The tab bodies should be kept small and isolated:

- `outline`: fetches course outline and renders `CourseBuilder` for teachers or read-only lesson outline for students.
- `people`: fetches members, pending invites, and active enrollment codes, then renders `CoursePeople`.
- `edit`: fetches the course row and renders metadata/cover editing controls.
- `schedule`: fetches course sessions and renders `ScheduleView`; teachers also get `SeriesScheduler`.
- `assignments`: continues to route to the existing assignments list route.

`features-courses` remains the presentational/component layer. Its `api.rs`
module gains typed DTOs and functions for the new backend read endpoints and
for existing mutation endpoints that are not yet exposed to the shell.

### Backend API additions

The backend already has many mutation endpoints. The missing pieces are read
endpoints needed by the CourseDetail tabs.

Add:

- `GET /v1/courses/:cid/modules-with-lessons`
  - Returns modules ordered by `sort_order`, each with lessons ordered by `sort_order`.
  - Readable by course members and org admins.
  - Masks unauthorized access as not found where existing course APIs do so.

- `GET /v1/courses/:cid/members`
  - Returns active course members with `user_id`, `display_name`, `email`, `role`, and `status`.
  - Admin-visible in the people tab.
  - Student visibility can be minimal or disabled; teacher/admin remains the audit requirement.

- `GET /v1/courses/:cid/sessions`
  - Returns course live-session occurrences ordered by `starts_at`.
  - Readable by course members and org admins.
  - Includes fields needed by `ScheduleView`: session id, course id/title/slug, title, starts_at, duration, status, diverged/edit flag.

Reuse existing endpoints:

- `PATCH /v1/courses/:id` for course edit.
- `POST /v1/courses/:cid/modules`, `POST /v1/courses/:cid/modules/reorder`, `PATCH/DELETE /v1/courses/:cid/modules/:mid`.
- `POST /v1/courses/:cid/modules/:mid/lessons`, lesson reorder, patch, delete.
- Course invitation and enrollment-code endpoints.
- `POST /v1/courses/:cid/sessions` and `PATCH /v1/sessions/:id`.
- Assignment create/patch/publish endpoints.

### Local audit seed support

Add local-only seed support guarded by the same safety posture as local login:

- Enabled only when `APP_ENV` is not `production` or `prod`.
- Disabled by default unless a specific local seed flag is enabled.
- Exposed as `POST /v1/dev/audit-seed`, mounted only as a public dev route that still refuses production-like environments.
- Creates or upserts deterministic local audit records:
  - tenant
  - teacher user
  - student user
  - course
  - teacher and student course memberships
  - modules and lessons
  - at least one scheduled live session
  - at least one published assignment accepting files
  - one active enrollment code
- Returns a concise summary for browser checklist use without exposing secrets.

The local login bypass should support explicit local profiles for this audit:
`teacher` and `student`. The login page can present separate local buttons when
the dev login config reports both profiles. These profiles are available only in
non-production environments and are backed by deterministic Firebase UID/email
claims so the audit seed endpoint can make the matching database records.

## User Flows

### Teacher audit flow

1. Teacher signs in with the local bypass.
2. Dashboard shows seeded real courses.
3. Teacher opens `/courses/:slug`.
4. Outline tab shows real modules and lessons.
5. Teacher can add a module, add/edit a lesson, and reorder where backend support exists.
6. People tab shows members, pending invites, and active codes.
7. Teacher can invite by email, revoke invites, generate codes, and revoke codes.
8. Edit tab can patch title, description, status, and cover asset.
9. Schedule tab shows real sessions and can schedule/cancel/reschedule.
10. Assignment editor can attach reference materials.

### Student audit flow

1. Student signs in through the local `student` bypass profile.
2. Dashboard shows enrolled courses.
3. Course detail outline renders read-only modules and lessons.
4. Assignments tab/list/detail/submission flow works with real assignment data.
5. Student can use the file picker on an assignment that accepts files.
6. Redeem flow uses an active enrollment code and routes to the course.

## Error Handling

- Loading states stay inline inside each tab.
- Course, lesson, session, and assignment 404s render inline errors, not panics.
- Forbidden actions are hidden for roles that should not see them.
- If the backend rejects a mutation anyway, show an inline error in that tab.
- Mutations refresh their relevant resource after success.
- Local seed/dev paths return explicit disabled errors in production-like environments.
- The existing 401 refresh/signout behavior remains the global auth failure path.

## Testing And Verification

### Automated tests

- Backend integration tests for each new read endpoint:
  - happy path
  - unauthorized/cross-tenant masking
  - role-specific visibility where relevant
- Backend test for local seed guard:
  - disabled in production even if flag is true
  - enabled in local env
- Frontend SSR/component smokes for CourseDetail tab rendering with fake contexts.
- Focused tests for `features-courses::api` DTO compatibility where feasible.

### Build checks

- `cargo test -p backend` or focused backend tests during implementation.
- `cargo check -p backend --lib -j 1`.
- `dx build --platform web --package shell-web`.
- `git diff --check`.
- Existing workspace/native build checks as time allows.

### Manual checklist

The existing Phase 1.5 exit checklist remains the source of truth for audit
evidence. This slice should update it with exact commands, dates, and outcomes.

Required manual evidence:

- Teacher browser flow passes with real course data.
- Student browser flow passes with real enrolled course data.
- Assignment file upload and submit path passes.
- Invite and redeem paths pass.
- Cross-tenant probes remain masked.
- Docker stack health passes after app flow is green, or remains explicitly blocked with current evidence.

## Environment Lane

The local machine currently has two known environment blockers:

- Docker daemon calls timed out during prior verification.
- Port `8080` was owned by Apache `httpd`, returning an HTML 404 for `/healthz`.

The app audit lane should complete first. Then the environment lane should:

1. Identify whether Apache can be stopped or whether Compose should use a different host port locally.
2. Restore Docker CLI responsiveness.
3. Run `docker compose up -d`.
4. Verify `curl http://localhost:8080/healthz` returns `ok`.

An alternate-port backend smoke is useful evidence, but it does not replace the
required Docker/Dokploy-style stack-health check.

## Implementation Sequence

1. Add missing backend read endpoints for outline, members, and course sessions.
2. Extend `features-courses::api` with typed DTOs and calls.
3. Wire CourseDetail outline, people, edit, and schedule tabs.
4. Wire AssignmentEditor reference attachments.
5. Add guarded local audit seed support.
6. Run focused backend and frontend verification.
7. Run the browser audit checklist and update evidence.
8. Address Docker/8080 stack health if app flow is green.

## Risks

- The existing local database has migration checksum drift. Implementation should avoid destructive fixes unless explicitly approved.
- The CourseDetail UI may expose backend gaps beyond the three known read endpoints. Keep any additions minimal and audit-driven.
- Local bypass users may need deterministic tenant/course membership. Seed support must make that repeatable.
- Docker/8080 issues are machine-state issues and may require user approval if stopping Apache affects other local work.

## Completion Criteria

Phase 1d-a is complete when:

- CourseDetail tabs render real data instead of placeholders.
- AssignmentEditor reference attachments work.
- Local audit data can be created repeatably in a non-production environment.
- The teacher and student browser checklist items have recorded evidence.
- Required automated checks pass.
- Docker stack health either passes or has a precise remaining blocker documented separately from app correctness.
