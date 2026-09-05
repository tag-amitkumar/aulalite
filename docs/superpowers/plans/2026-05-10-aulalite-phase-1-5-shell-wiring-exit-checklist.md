# Phase 1.5 Exit Checklist

Run these checks in order from the repository root. Phase 1.5 is complete only
when every required item passes.

## 1. Stack health
- [ ] `docker compose up -d`
- [ ] `curl http://localhost:8080/healthz` returns `ok`
- [x] Local bypass HTTP smoke on alternate port:
      temporary database + `.env` local overrides + `BIND_ADDR=127.0.0.1:18080`;
      `/healthz`, `/v1/dev/login/config`, `POST /v1/dev/login`, and authenticated
      `/v1/me` all passed. This does not replace the required Docker/Dokploy
      stack check above.

Local note from 2026-05-10: `docker` daemon calls timed out on this machine, and
port `8080` is currently owned by Apache `httpd`, returning an HTML 404 for
`/healthz`. Leave the two Docker stack-health items unchecked until Docker is
responsive and the backend owns `8080`.

Local note from 2026-05-11: `docker compose up -d` timed out after 304s.
`docker version --format '{{json .Server.Version}}'` timed out, `com.docker.service`
could not be started from this session, and Apache `httpd` still owns port `8080`.
Stopping that `httpd` process failed with `Access is denied`. Docker/Dokploy
stack health remains unchecked. Fallback audit backend health passed on
`127.0.0.1:18080` against fresh database `aulalite_audit_20260511`.

## 2. Automated verification
- [x] `cargo test --workspace -j 2` — **251 passed, 0 failed across 51 binaries** (verified at HEAD `06ea802`).
- [x] `cargo build -p shell-web --target wasm32-unknown-unknown` — clean.
- [x] `cargo build -p features-courses --target wasm32-unknown-unknown` — clean.
- [x] `cargo build -p shell-desktop` — clean (native).
- [x] `cargo build -p shell-mobile` — clean (workspace native target; full
      mobile-target build requires Android/iOS toolchain).
- [x] `dx build --platform web --package shell-web` succeeds — clean build with existing warnings.
- [x] `cargo test -p backend --test local_login_bypass -- --nocapture` — 3 passed,
      0 failed. Verifies production guard, dev login token response, and `/v1/me`
      authentication through real middleware.
- [x] Phase 1d-a focused verification:
      `cargo test -p backend --test course_detail_tabs --test audit_seed --test local_login_bypass -- --nocapture`,
      `cargo check -p backend --lib -j 1`,
      `cargo test -p shell-web --test shell_routes_smoke`,
      `dx build --platform web --package shell-web`, and
      `docker compose config --quiet` passed on 2026-05-11.
- [x] Final tenant-scope regression verification:
      `cargo test -j 1 -p backend --test course_tenant_scope -- --nocapture`,
      `cargo test -j 1 -p backend --test course_tenant_scope --test course_detail_tabs --test permissions_matrix -- --nocapture`,
      and broader course/module/lesson/enrollment/upload/live backend integration
      selection passed on 2026-05-11 after scoping course admin helpers to the
      caller's active tenant.

## 3. End-to-end browser flow (manual, the headline acceptance)
- [ ] Sign in as teacher in Chrome.
- [ ] **URL bar shows `/`**, the dashboard renders **the actual courses owned by that teacher** (not `vec![]`).
- [ ] Header shows the **real display_name + email** (not "User <user@example.com>").
- [ ] Hard-refresh the page. **Stay signed in**, dashboard re-renders the same data.
- [ ] Wait an hour (or manually expire the token via `firebase.auth.currentUser.getIdTokenResult(true)` in devtools) and refresh.
       **Page does not kick to /login** — the 401-retry refreshes silently.
- [ ] Click a course → **/courses/:slug renders, lessons load**, browser back button returns to `/`.
- [ ] Click Assignments tab → URL bar updates to `/courses/:slug/assignments`, real assignment list loads.
- [ ] Click an assignment → URL `/courses/:slug/assignments/:id` opens detail with role-aware UI.
- [ ] Sign out from header. URL goes to `/login`. Reload → stays on `/login`.

2026-05-11 fallback evidence, not a Chrome pass: local seed returned
`local-audit` / `audit-course` / `AUDIT123`; local teacher and student login
profiles worked over HTTP; teacher and student dashboards returned Audit Course;
course outline returned Audit Module / Audit Live Lesson; people returned Local
Student and Local Teacher; schedule returned Audit Live Class; teacher course
description patch saved through the API; Audit Assignment loaded with
`accepts_files=true`.

## 4. Student flow
- [ ] Sign in as student. Dashboard shows enrolled courses.
- [ ] Open an assignment with `accepts_files=true`.
       The **file picker renders** (not the placeholder `<p>`).
       Select a PDF → upload completes → asset_id appears in `attachment_asset_ids`.
- [ ] Submit the assignment. Status flips to `submitted`. Receipt visible.

2026-05-11 fallback evidence, not a Chrome pass: student created an Audit
Assignment submission, uploaded `audit-submission.txt` through
`/v1/uploads/begin` + presigned PUT + `/complete`, patched the submission with
the returned `asset_id`, submitted it, and teacher submission listing returned
the submitted row.

## 5. Invite + redeem
- [ ] Trigger an invite email from teacher → student.
- [ ] Click the email link. URL `/accept-invite/:token` opens.
       **Accept actually works** (POST /v1/invitations/:token/accept). Browser redirects to the course detail.
       (Pre-1.5 this hardcoded "Invite flow not yet wired".)
- [ ] As a student with an enrollment code, navigate to `/redeem`. Enter the code.
       **Redeem actually enrolls** and routes to the course.

## 6. Live class polish (1b-γ §4a follow-ups)
- [ ] Teacher promotes a student. Student's mic activates.
- [ ] Teacher demotes the same student. **Mic stops** (WhipPublisher closed). Verify no audio bleed
       in a third Chrome profile that was watching.
- [ ] Trigger rate-limit (5 chats back-to-back as student). **Inline banner** appears at top of live room
       reading "Rate limited — try again in N ms"; auto-dismisses after 5s.
- [ ] Server emits an Error event (e.g. send malformed JSON). **Inline banner** shows code + message; auto-dismisses.

## 7. Redis broker production smoke (1b-γ §4b follow-up)
- [ ] `REDIS_URL=redis://localhost:56379 cargo run -p backend --bin redis_broker_smoke`
       returns exit 0 with "OK — round-trip successful".

## 8. Cross-tenant probe
- [ ] Tenant B's user fetches `/v1/me` → returns tenant B's user only.
- [ ] Tenant B navigates to `/courses/<tenant-A-course-slug>` → 404 inline error rendered.
- [ ] Existing `cross_tenant_assignments_masked` and `cross_tenant_submissions_masked` tests still pass
      (verified — included in the 251 passing tests).

## 9. shell-desktop sanity
- [ ] `cargo run -p shell-desktop` opens a desktop window with the same UI.
- [ ] On desktop, login is non-functional (deliberate — desktop bridge is a follow-up); the window
      mounts on `/login` and the form renders without panic.

## 10. shell-mobile sanity
- [x] `cargo build -p shell-mobile` against the workspace default target completes.
       (Full mobile-target build requires Android/iOS toolchain — out of scope.)

## 11. Open follow-ups carried into next phase

These are NEW deferrals after 1.5; they were not in scope:
- [ ] Desktop platform-bridge implementation (file-based token persistence, e.g. via `directories`
      crate + Firebase REST).
- [ ] Mobile platform-bridge implementation (Android Keystore / iOS Keychain + Firebase REST).
- [x] AssignmentEditor reference-attachment file picker (teacher attaches PDFs to assignments).
      Completed in Phase 1d-a; focused tests and `dx build --platform web --package shell-web`
      passed on 2026-05-11.
- [x] CourseDetail outline/people/edit/schedule tab content.
      Completed in Phase 1d-a; SSR smokes and HTTP fallback verified outline,
      people, schedule, and edit data paths on 2026-05-11.

These do NOT block tagging `phase-1-5-complete`.

## Completion tag

```bash
git tag phase-1-5-complete
git push origin phase-1-5-complete
```

Tagging `phase-1-5-complete` clears the audit gap: the app actually works end-to-end
in a browser. The original Phase 1d (notifications, parent role, TA scoping) becomes
the next natural phase.
