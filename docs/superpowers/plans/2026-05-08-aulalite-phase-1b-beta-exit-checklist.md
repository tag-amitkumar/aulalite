# Phase 1b-β Exit Checklist

Run these checks in order from the repository root. Phase 1b-β is complete only
when every required item passes.

## 1. Stack health
- [ ] `docker compose up -d`
- [ ] `curl http://localhost:8080/healthz` returns `ok`
- [ ] `curl http://localhost:8080/v1/mediamtx/jwks | jq '.keys[0].kty'` returns `"RSA"`
- [ ] MediaMTX console reachable; `curl http://localhost:9997/v3/config/global/get` returns 200

## 2. Migrations
- [ ] `sqlx migrate info --source migrations` shows `20260508000012_live_room_columns` applied.
- [ ] `\d live_session_series` shows `transport_mode` column with CHECK.
- [ ] `\d live_sessions` shows `screen_path`, `transport_mode`, `publish_nonce`, `publish_nonce_expires_at`.

## 3. Automated verification
- [ ] `cargo test --workspace -j 2` (everything green; zero failures). Note: `-j 2` required on Windows hosts to avoid pagefile pressure (established in Phase 1b-α exit).
- [ ] `cargo build -p shell-web --target wasm32-unknown-unknown`
- [ ] `cargo build -p features-courses --target wasm32-unknown-unknown`
- [ ] `dx build --platform web --package shell-web` and confirm `dist/vendor/hls.js` exists.

## 4. Known limitation: MediaMTX read auth (Pattern B follow-up)

Phase 1b-β shipped the MediaMTX config in **Pattern B** (HTTP-only auth — see `ops/mediamtx/mediamtx.yml`). MediaMTX v1.18.1 does not support per-action `authMethod`, so JWT validation cannot be performed by MediaMTX itself. The backend's `/v1/mediamtx/auth/publish` handler currently rejects `action != "publish"`.

**Manual workaround for testing**: temporarily configure MediaMTX with `authMethod: jwt` for read flows (separate config), or extend the backend handler to validate viewer JWTs for `action=read` requests. This follow-up is tracked as a tightening before tagging `phase-1b-beta-complete`.

## 5. Teacher publish flow (manual)
- [ ] Sign in as teacher in Chrome. Open a scheduled session at `/courses/<slug>/sessions/<id>`.
- [ ] Click "Go Live". Allow camera + mic permissions.
- [ ] Confirm `live_sessions.status = 'live'` in DB.
- [ ] Confirm MediaMTX HTTP API shows the path: `curl http://localhost:9997/v3/paths/get/aula/<t>/<c>/<s>`.

## 6. Student watch flow — WebRTC mode (manual, after Pattern B follow-up)
- [ ] Series with `transport_mode='webrtc'`.
- [ ] Sign in as enrolled student in a different browser/profile.
- [ ] Open the session URL during the live window.
- [ ] Confirm video plays with sub-second latency (compare wall clock teacher → student).
- [ ] Wait 14 minutes; confirm JWT refresh fires (network tab) and stream continues.

## 7. Student watch flow — HLS mode (manual, after Pattern B follow-up)
- [ ] Series with `transport_mode='hls'`.
- [ ] Sign in as enrolled student.
- [ ] Confirm HLS playback works in Chrome (via hls.js) and Safari (native).
- [ ] Latency ~3-10s expected.

## 8. Mobile watch flow (manual)
- [ ] Build shell-mobile for Android, install on device.
- [ ] Same student, HLS series, watch from the Android app.
- [ ] Confirm playback works.

## 9. Permissions / cross-tenant
- [ ] Student tries `/v1/sessions/:id/go-live` → 403.
- [ ] Tenant B fetches `/v1/sessions/<tenant-A-session-id>/join` → 404 (masked).
- [ ] Non-course-member fetches `/v1/sessions/:id/join` → 403.
- [ ] Replay attack: replay used `publish_password` → MediaMTX auth callback returns 403.

## 10. Lifecycle
- [ ] Teacher closes tab mid-class. Wait `duration + 30 min`. Confirm session auto-ends in DB.
- [ ] Per-occurrence cancel of a `live` session also runs `end-class` flow.

## 11. Failure modes
- [ ] Stop MediaMTX container. Confirm `/v1/mediamtx/healthz` returns `{healthy: false}`.
- [ ] Teacher's Go Live attempt with MediaMTX down — UI shows "media server unreachable".
- [ ] Restart MediaMTX. Confirm next Go Live succeeds.

## Completion tag

Only after every required check above passes (and the Pattern B follow-up is resolved):

```bash
git tag phase-1b-beta-complete
git push origin phase-1b-beta-complete
```
