# Phase 1a Exit Checklist

Run these checks in order from the repository root. Phase 1a is complete only
when every required item passes.

## 1. Stack health (Phase 0 baseline)

- [ ] `docker compose up -d`
- [ ] `curl http://localhost:8080/healthz` returns `ok`
- [ ] Postgres / Redis / MinIO containers report healthy

## 2. Migrations

Use the host URL when running tools outside Docker:

```bash
export DATABASE_URL=postgres://aulalite:changeme@localhost:55432/aulalite
```

- [ ] `sqlx migrate info --source migrations` shows all 10 Phase 1a migrations applied
      (`courses`, `modules`, `lessons`, `course_memberships`, `enrollment_codes`,
      `course_invitations`, `live_session_series`, `live_sessions`, `file_assets`,
      `audit_events`).
- [ ] `psql "$DATABASE_URL" -c "\dt"` shows all new tables.
- [ ] `psql "$DATABASE_URL" -c "\df lookup_enrollment_code"` and
      `\df lookup_invitation_by_token` both list the SECURITY DEFINER functions.

## 3. Automated verification

- [ ] `cargo test --workspace` (everything green; zero failures)
- [ ] `cargo build -p shell-web --target wasm32-unknown-unknown`
- [ ] `cargo check -p shell-mobile --target aarch64-linux-android`
- [ ] `cargo build -p features-courses --target wasm32-unknown-unknown`
- [ ] `dx build --platform web --package shell-web`

## 4. Course creation (web, real Firebase user)

- [ ] Sign in as a real user via the web shell.
- [ ] Promote that user to `org_admin` (existing `tools/aulalite-admin` or direct DB).
- [ ] Click `+ New Course`, enter "Algebra 1", submit.
- [ ] Confirm the course appears in `My Courses` with status `draft`.
- [ ] Open the course → Build tab → add a module "Week 1" → add a rich-text
      lesson with markdown body.
- [ ] Confirm the lesson renders in the outline tab with markdown rendered.

## 5. Recurring schedule + per-occurrence cancel + reschedule

- [ ] On the course's Schedule tab, click "+ Schedule".
- [ ] Pick weekly Mon/Wed/Fri, count = 6, recording on, submit.
- [ ] Confirm preview showed 6 occurrences and they appear in Schedule.
- [ ] Click kebab on first occurrence → Cancel. Confirm row is struck-through.
- [ ] Click kebab on second occurrence → Reschedule (pick a new time).
      Confirm "edited" badge appears.
- [ ] Inspect DB:
      ```bash
      psql "$DATABASE_URL" -c "SELECT occurrence_index, status, diverged FROM live_sessions ORDER BY occurrence_index;"
      ```
      First row `status='cancelled'`, second row `diverged=true`.

## 6. Code-based enrollment

- [ ] On People tab, click "Generate code", `max_uses=1`, generate.
- [ ] Copy the code.
- [ ] Sign in as a different real user (a brand-new Firebase email).
- [ ] Visit `/redeem`, paste the code, submit.
- [ ] Confirm redirect to course detail.
- [ ] As that student, confirm `Dashboard` lists the course.
- [ ] Repeat with a third user; confirm second redemption fails with
      "invalid, expired, or fully used".

## 7. Email-link invitation

- [ ] On People tab, click "Invite by email", enter a real address you control,
      role = student, send.
- [ ] Receive the Firebase email-link sign-in. Click the link.
- [ ] Land on `/accept-invite/<token>`. Confirm "Welcome to <course>" page
      appears and the course is listed in dashboard.

## 8. RLS sanity

- [ ] Try to call `/v1/courses/:id` with a course id from a different tenant —
      the response is `404` (existence is masked).
- [ ] Repeat for `/v1/series/:sid` and `/v1/sessions/:id` with foreign tenant
      ids — same masking behavior.

## Known environment gates

- Real Firebase email-link invites require email-link sign-in to be enabled in
  the Firebase project (Authentication → Sign-in method → Email/Password →
  enable "Email link (passwordless)") and the local dev domain (`localhost`)
  whitelisted under authorized domains.
- `FIREBASE_WEB_API_KEY` and `APP_ORIGIN` env vars must be set in `.env` for
  the backend to send email-link invites.

## Completion tag

Only after every required check above passes:

```bash
git tag phase-1a-complete
git push origin phase-1a-complete
```
