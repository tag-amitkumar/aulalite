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
- [x] `cargo test --workspace -j 2` — **248 passed, 0 failed across 47 binaries** (verified at HEAD `41fc071`).
- [x] `cargo build -p shell-web --target wasm32-unknown-unknown` — clean.
- [x] `cargo build -p features-courses --target wasm32-unknown-unknown` — clean.
- [ ] `dx build --platform web --package shell-web` succeeds (manual).

## 4. End-to-end teacher flow (manual)
- [ ] Sign in as teacher. Open a course → Assignments tab → "New assignment".
- [ ] Fill out form (title, instructions, grading_mode=numeric, max_points=100, accepts_text+files, release_mode=instant). Save draft.
- [ ] Click Publish. Confirm assignment status flips to `published` in DB.

## 5. End-to-end student flow (manual)
- [ ] Sign in as enrolled student. Open the same course → Assignments tab.
- [ ] Open the published assignment. Confirm `SubmissionForm` renders.
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
- [x] RLS sweep tests in `crates/backend/tests/rls_tenant_isolation.rs` cover this:
      `cross_tenant_assignments_masked` and `cross_tenant_submissions_masked` both pass.
- [ ] Manual smoke: tenant B's user fetches `/v1/courses/<tenant-A-course-id>/assignments` → 404.
- [ ] Manual smoke: tenant B's user fetches `/v1/assignments/<tenant-A-assignment-id>` → 404.
- [ ] Manual smoke: tenant B's user fetches `/v1/submissions/<tenant-A-submission-id>` → 404.

## 11. Open follow-ups carried into next phase

From 1b-γ exit checklist (still open):
- [ ] Persistent WebSocket polish: store WhipPublisher on Promoted, close on Demoted, surface
      RateLimited/Error events as toasts. Partial work landed in 1b-γ; remainder is light polish.
- [ ] `RedisLiveRoomBroker` end-to-end production exercise (manual).

From 1c (deliberate scope cuts, deferred to 1d):
- [ ] `shell-web` route arms pass placeholder `course_id`/`is_teacher`/`current_user_id` —
      need real course-by-slug + role resolution before the routes are user-facing.
      See `crates/shell-web/src/main.rs` `Route::AssignmentList`/`Route::AssignmentNew`/etc.
- [ ] `Route::AssignmentGrade` renders a placeholder; needs `use_resource(get_assignment)`
      to fetch the AssignmentDto before rendering `SubmissionsGradingTable`.
- [ ] File upload UI in `SubmissionForm` — currently a placeholder `<p>` directing to
      `file_picker.rs`. Wire a real picker with linked_entity_type='submission_attachment'.

These do NOT block tagging `phase-1c-complete`.

## Completion tag

```bash
git tag phase-1c-complete
git push origin phase-1c-complete
```

Tagging `phase-1c-complete` unlocks Phase 1d (notifications, role polish, parent role).
