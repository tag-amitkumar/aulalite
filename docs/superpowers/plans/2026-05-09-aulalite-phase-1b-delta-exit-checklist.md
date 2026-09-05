# Phase 1b-δ Exit Checklist

Run these checks in order from the repository root. Phase 1b-δ is complete only
when every required item passes.

## 1. Stack health
- [ ] `docker compose up -d`
- [ ] `curl http://localhost:8080/healthz` returns `ok`
- [ ] `docker compose exec backend ffmpeg -version` returns version info
- [ ] `docker compose exec backend ls /recordings` succeeds (volume mounted)

## 2. Migrations
- [ ] `sqlx migrate info --source migrations` shows `20260509000014_recordings` applied.
- [ ] `sqlx migrate info --source migrations` shows `20260509000015_file_assets_session_recording` applied.
- [ ] `\d recordings` shows the table with RLS policy + UNIQUE(session_id) + partial index.
- [ ] `\d+ file_assets` shows `linked_entity_type` CHECK includes `'session_recording'`.

## 3. Automated verification
- [ ] `cargo test --workspace -j 2` (everything green; zero failures). Note: `-j 2` required on Windows hosts to avoid pagefile pressure.
- [ ] `cargo build -p shell-web --target wasm32-unknown-unknown`
- [ ] `cargo build -p features-courses --target wasm32-unknown-unknown`

## 4. End-to-end recording (manual)
- [ ] Sign in as teacher in Chrome. Schedule a session with `recording_enabled=true`.
- [ ] Click Go Live, publish camera+mic for 2-3 minutes.
- [ ] Click End Class.
- [ ] Wait ~5 minutes for the sweep to pick it up. Watch backend logs for `recording processed`.
- [ ] Confirm `recordings.processing_status='available'` in DB.
- [ ] Confirm a `file_assets` row with `linked_entity_type='session_recording'` and an MP4 in MinIO.

## 5. Playback (manual)
- [ ] Sign in as enrolled student.
- [ ] Open `/courses/<slug>/sessions/<id>` (the same URL used during the live session).
- [ ] Confirm `LiveRoomReplay` mounts; `<video>` plays the MP4.
- [ ] Send some chat messages during step 4. Open the recording. Confirm chat side panel scrolls in sync as the video plays.

## 6. Cross-tenant probe
- [ ] Tenant B's user fetches `/v1/sessions/<tenant-A-session-id>/recording` → 404.

## 7. Failure / retry (manual)
- [ ] Force a recording into `failed` state via DB:
  ```sql
  UPDATE recordings SET processing_status='failed', processing_error='manual test'
  WHERE session_id='<id>';
  ```
- [ ] Open the recording playback URL as teacher; confirm "Retry" button appears.
- [ ] Click Retry. Confirm status flips back to `pending`.
- [ ] Wait for the next sweep tick; confirm status returns to `available` (after a successful re-process).

## 8. Retention janitor
- [ ] Insert a recordings row with `created_at = now() - interval '400 days'`.
- [ ] Manually trigger the janitor via DB or wait 24h.
- [ ] Confirm the row is deleted, the file_asset is `pruned`, and the MinIO object is gone.

## 9. Carry-overs from 1b-γ (still open)

The 1b-γ exit checklist deferred two follow-ups. Address them before tagging
`phase-1b-complete`:

- [ ] Persistent WebSocket connection in `LiveRoomView` and `LiveRoomBroadcast`
      (action closures still no-ops as of 1b-γ exit). See
      `2026-05-09-aulalite-phase-1b-gamma-exit-checklist.md` §4a.
- [ ] Exercise `RedisLiveRoomBroker` end-to-end with `REDIS_URL` set. See same
      checklist §4b.

## Completion tag

Only after every required check above passes:

```bash
git tag phase-1b-delta-complete
git push origin phase-1b-delta-complete

# After 1b-γ carry-overs (§9) are also resolved:
git tag phase-1b-complete  # closes out 1b
git push origin phase-1b-complete
```

Tagging `phase-1b-complete` unlocks Phase 1c (assignments).
