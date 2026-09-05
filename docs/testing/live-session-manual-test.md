# Live Session — Manual Test Runbook

This runbook walks through verifying the live-session (video-call) feature end to end. It is intended for someone who has never run the feature before. Expect 15–20 minutes.

## Prerequisites

- The backend, frontend, and MediaMTX are running. Default local URLs:
  - Frontend: <http://127.0.0.1:3000>
  - Backend:  <http://127.0.0.1:8080>
- You can either bring up the stack with `docker compose up -d` (the recipe in `docker-compose.yml`) or run each crate by hand (`cargo run -p backend`, `dx serve crates/shell-web`).
- Two browser windows: a normal window for the **Teacher** and an incognito/private window for the **Student** (so the two sessions hold different ID tokens).
- Both browsers must have **microphone and camera permissions** granted for the frontend origin.
- Two test accounts:
  - Teacher: `local.teacher@example.test` / `local-teacher-pass`
  - Student: `local.student@example.test` / `local-student-pass`
  - These come from `tools/aulalite-admin` seeding (or `/v1/dev/audit-seed` if you use the Playwright seed helper).

## Step 1 — Sign in

1. In the Teacher window, go to `/login` and sign in with the teacher account. You should land on `/` (Dashboard) with `Welcome back, …` and a `dashboard-hero` block.
2. In the Student window, do the same with the student account.

**Expected:** Both windows show the dashboard. No console errors about missing routes or failed token refresh.

## Step 2 — Schedule a session (Teacher)

1. Teacher: go to `/courses` and click into a course you own.
2. Click the **Schedule** tab. Use the series scheduler to create a **one-off** session starting in ~2 minutes. Pick a duration of 15 minutes and leave recording enabled.
3. After the create succeeds, the new session should appear in the Schedule tab list and in `/schedule` on the same window.

**Expected:** Session row appears in both views with title, start time, and duration. No 401s or 500s in the network panel.

## Step 3 — Student lands in the lobby

1. Student: go to `/schedule`.
2. Click the upcoming session row for the course they're enrolled in.

**Expected:** Lobby view renders. You see the course title (`live_room_lobby.rs:18`), the scheduled time, and a **Join** button that becomes enabled within the join window (~5 min before start).

## Step 4 — Teacher goes live

1. Teacher: navigate to the session URL (`/courses/:slug/sessions/:session_id`).
2. Click **Go Live** in the broadcast surface. Approve microphone + camera prompts.

**Expected:**
- Teacher sees their own camera preview, a `● LIVE` pill, and broadcast controls.
- Student lobby auto-promotes to the live room and the student starts receiving media.

## Step 5 — In-session interactions

Run each of these and confirm the result:

1. **Chat round trip** — Teacher types in chat; Student sees it. Student replies; Teacher sees it.
2. **Presence** — both names appear in the Participants panel (`live_room_presence.rs:30`).
3. **Hand-raise** — Student clicks Raise Hand. Teacher's `Hand-raise queue` (`live_room_hand_raise.rs:28`) shows the request.
4. **Grant speaking** — Teacher grants the request. Student receives WHIP credentials and can start their mic; their audio reaches the Teacher.
5. **Screenshare** — Teacher shares a screen. Student sees the screen share. Teacher revokes.
6. **Revoke hand-raise** — Teacher revokes student's speaking permission. Student returns to listen-only.

## Step 6 — End the session, verify replay

1. Teacher clicks **End session**. Both windows transition to the post-session view.
2. After ~30–60 seconds (recording transcode), the session in the Schedule list should show a Replay link.
3. Click Replay (Teacher or Student). The replay player should load and play the recorded stream.

## Common failures and how to read them

| Symptom | Likely cause |
|--|--|
| Camera/mic prompt never appears | Browser denied permissions for the origin. Reset site permissions and retry. |
| Lobby never promotes to live | Clock skew between server and client > 30s, or the broadcaster never published. Check the backend `/v1/sessions/:id` status — it should flip to `live` once WHIP is up. |
| `[ws] auth …` lines flood the console during sign-in | Expected pre-auth noise; the Playwright console gate filters it (see commit `58255ef`). Not a failure. |
| Replay never appears | The MediaMTX recording → S3 → transcode pipeline did not complete. Check backend logs for `recording_ingest` warnings and S3 credentials. |
| `Failed to parse route` on any link | A stale `/dashboard` literal slipped past Task 1's regression. Re-run `cargo test -p shell-web nav_links_do_not_point_at_unknown_dashboard_route`. |

## What "passes" looks like

A successful run produces:
- One scheduled session created by the Teacher
- One student joined and promoted to live
- At least one chat message in each direction
- One granted-then-revoked hand-raise
- One screenshare
- One replay viewable after the session ends
