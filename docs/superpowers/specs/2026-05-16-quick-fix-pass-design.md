# Quick-Fix Pass — Routing, Editable Names, Live-Session Test Runbook

Date: 2026-05-16
Author: collaboration with the assistant

This spec batches three concrete fixes that surfaced in conversation. UI polish (Next/shadcn-quality look-and-feel) is intentionally **out of scope** and will be handled in a follow-up brainstorm.

---

## 1. Routing — `/dashboard` does not resolve

### Problem
`route_enum.rs` defines `Dashboard {}` at `/`, not `/dashboard`. Several places in the app link to `/dashboard`, so clicking those links produces a "Failed to parse route" failure with the cascade shown in the bug report.

### Affected call sites
- `crates/features-courses/src/app_shell.rs` lines **27, 32, 36, 40, 46** — sidebar nav per role (OrgAdmin, Teacher, TA, Student, Parent).
- `crates/features-courses/src/accept_invite.rs:40` — "Back to dashboard" link in the invite-failure state.

### Change
Replace the literal `/dashboard` with `/` in all six sites. No structural changes elsewhere; the router already serves the `Dashboard` component at `/`.

### Verification
- Manual: load the app as each role and click the sidebar "Dashboard" link; no route-error toast/console message.
- Automated: a Playwright assertion that the rendered console contains no "Failed to parse route" lines after navigating Dashboard → Courses → Dashboard.

---

## 2. Editable names — modules, lessons, and discoverable course rename

### Problem
- New modules and lessons are seeded by the builder with placeholder titles `"Untitled module"` / `"Untitled lesson"` (`course_detail.rs:324, 341`).
- The builder renders these titles as read-only `h3` and `span` (`course_builder.rs:98, 110`); there is no rename UI anywhere.
- The course title **is** editable via the Settings tab (`course_detail.rs:CourseEditTab`, lines 216-293), but the entry point — the `/courses/:slug/edit` tab — is not obviously discoverable from the course header.

User also indicated "there can be other things" — i.e. they expect other rename gaps to be addressed in the same pass.

### Approach

**Backend** — no changes required. `PATCH /v1/courses/:cid/modules/:mid` (`backend/src/handlers/modules.rs:52`) and `PATCH /v1/courses/:cid/modules/:mid/lessons/:lid` (`backend/src/handlers/lessons.rs:76`) already exist and accept `{ title: Option<String> }` patches.

**Frontend `api.rs`** — add two helpers next to the existing `patch_course`:
- `patch_module(api, course_id, module_id, &PatchModuleBody)` → `PATCH /v1/courses/:cid/modules/:mid`.
- `patch_lesson(api, course_id, module_id, lesson_id, &PatchLessonBody)` → `PATCH /v1/courses/:cid/modules/:mid/lessons/:lid`.
- Both helpers follow the same error-mapping and `Authorization` header conventions as `patch_course`.

**`course_builder.rs` — inline rename**
- Introduce a small `InlineRename` helper local to this file (or in the design-system crate if it generalises; decide during implementation). Behaviour:
  - Default state: render the title as text inside a clickable element (hover affordance: subtle pencil icon).
  - On click: swap to a focused `<input>` pre-populated with the current value.
  - **Commit** on Enter or blur: call the supplied `on_commit(new_value)` handler, optimistically update local state, revert on error.
  - **Cancel** on Esc: discard edit and restore the original value.
  - Trim whitespace; reject empty submissions (revert to previous value and surface a brief inline error).
- Apply it to:
  - Module title (`course_builder.rs:98`) — `on_commit` calls back to `OutlineTab` which invokes `patch_module` then `outline.restart()`.
  - Lesson title in the builder list (`course_builder.rs:110`) — `on_commit` similarly invokes `patch_lesson`.
- The `EmptyState`-only path stays unchanged.

**`course_detail.rs:OutlineTab` — wire the new handlers**
- Extend `CourseBuilderProps` with `on_module_renamed: EventHandler<(String /*module_id*/, String /*new_title*/)>` and `on_lesson_renamed: EventHandler<(String /*module_id*/, String /*lesson_id*/, String /*new_title*/)>`.
- In `OutlineTab`, supply closures that:
  1. Call the new `api::patch_module` / `patch_lesson`.
  2. On success, `outline.restart()` to re-fetch.
  3. On error, log via `tracing` (until item 2/5 polish brings a global toast pattern).

**Course-title discoverability** (`course_detail.rs:57`)
- Beside `h1 { "{props.course_title}" }`, when `can_admin` is true, render a small "Edit" affordance (icon + label) that navigates to the existing Settings tab (`Route::CourseEdit { slug }`).
- Reuse existing icon set (`dioxus-free-icons` lucide pencil).
- No new edit surface is added; this is purely a discoverability fix.

**Audit-and-list step** (during implementation, not pre-planned)
- Before finishing, sweep these likely-gap surfaces:
  - Assignment titles in `assignment_editor.rs` (probably already editable — confirm).
  - Live-session titles created via `series_scheduler.rs` (probably needs a rename path on `schedule_view.rs`).
  - Course-card titles in `course_list.rs` / `dashboard.rs` — these should be display-only since rename happens in Settings.
- For each gap: if fixable in <30 lines without expanding scope, fix it; otherwise add a one-line follow-up to the implementation plan and stop.

### Verification
- Unit test in `course_builder.rs` (Dioxus SSR pattern already in use): commit Enter → handler called with trimmed new value; Esc → handler not called.
- Unit test in `course_detail.rs`: `OutlineTab` invokes `patch_module` and triggers a restart on commit.
- Manual: rename a module and a lesson; refresh; new names persist.
- Manual: as a course admin, click "Edit" next to the course title in the detail header; lands on Settings tab.

---

## 3. Live-session manual test runbook

### Problem
There is no documented click-path for verifying the live-session (video-call) feature by hand. The user does not know how to run a manual test.

### Approach
Add a single new file: `docs/testing/live-session-manual-test.md`. The runbook covers:

1. **Prerequisites** — two browsers (or one + an incognito window), test tenant with at least one Teacher + one Student, mic + camera permissions granted, the backend reachable.
2. **Scheduling a session (Teacher)** — from `/courses/:slug/schedule`, create a one-off session starting in ~2 minutes; verify it appears in both `/schedule` and the course Schedule tab.
3. **Joining the lobby (Student)** — `/schedule` → click upcoming session → lobby renders with course title, scheduled time, and Join button enabled when within join window.
4. **Going live (Teacher)** — start the broadcast; verify camera + mic preview; student lobby auto-promotes to live room.
5. **In-session interactions** — chat round-trip; presence list shows both participants; student raises hand; teacher grants speaking permission; teacher does a screenshare; teacher revokes.
6. **Ending and replay** — teacher ends session; both sides land on the post-session view; if recording was enabled in the series scheduler, recording appears in replay once processing completes.
7. **Common failures** — what to do if media permissions are denied, if the WS auth noise appears (it's filtered now per `58255ef`), if the lobby never promotes (clock skew), if the replay never appears (ingest/transcode pipeline).

### Verification
- The runbook is followable end-to-end by someone who has never run the feature before. (Self-check on first authoring.)

---

## Out of Scope

- Any visual restyling of buttons, inputs, the inline-rename affordance, or the rest of the design system. Held for the follow-up "UI polish to shadcn/Next quality" brainstorm.
- Backend rate-limiting on rename endpoints.
- Renaming via the lesson detail view (`lesson_outline_view.rs`) — covered separately if it surfaces during the audit step.
- Bulk operations (multi-select rename, etc.).

## Risks & Notes

- Optimistic UI on rename: if the PATCH fails, the user might briefly see the new value before it reverts. Acceptable for v1; if it becomes a complaint, add a saving-spinner state.
- Course-title "Edit" affordance is intentionally minimal — no new component, just a link. Polished version comes with the UI brainstorm.
- The Playwright regression for routing only covers the sidebar; we deliberately do not chase the `accept_invite.rs` link in this test (low-traffic edge path).
