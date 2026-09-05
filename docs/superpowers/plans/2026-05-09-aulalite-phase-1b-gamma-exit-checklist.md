# Phase 1b-γ Exit Checklist

Run these checks in order from the repository root. Phase 1b-γ is complete only
when every required item passes.

## 1. Stack health
- [ ] `docker compose up -d`
- [ ] `curl http://localhost:8080/healthz` returns `ok`
- [ ] `docker compose exec redis redis-cli PING` returns `PONG`

## 2. Migrations
- [ ] `sqlx migrate info --source migrations` shows `20260509000013_live_room_chat` applied.
- [ ] `\d live_room_messages` shows the table with RLS policy + 2 indexes.
- [ ] `\d live_room_kicks` shows the table with RLS policy + UNIQUE constraint.
- [ ] `\d live_sessions` shows `student_publish_nonces JSONB`.

## 3. Automated verification
- [ ] `cargo test --workspace -j 2` (everything green; zero failures). Note: `-j 2` required on Windows hosts to avoid pagefile pressure.
- [ ] `cargo build -p shell-web --target wasm32-unknown-unknown`
- [ ] `cargo build -p features-courses --target wasm32-unknown-unknown`
- [ ] `dx build --platform web --package shell-web` succeeds.

## 4. Known follow-ups before tagging

### 4a. Persistent WebSocket connection in frontend
1b-γ Tasks 24-25 ship the chat / presence / hand-raise sidebars rendered in
`LiveRoomView` and `LiveRoomBroadcast`, but action closures are no-ops. The
persistent-socket pattern (one connection per session, state hydration from
WebSocket events, action sends through the same socket) is deferred:
- `LiveRoomView`: `// TODO 1b-γ exit-checklist: persistent WebSocket connection + state hydration`
- `LiveRoomBroadcast`: same TODO
- `LiveRoomView` `Promoted` event handler → `live_room_audio_publisher::publish_audio`: also gated behind the persistent-socket refactor.

Resolve this by:
- [ ] Extending `live_room_socket::conn::LiveRoomSocket` to expose a Signal-driven event stream.
- [ ] Connecting once on component mount via `use_effect`.
- [ ] Wiring `on_send` / `on_delete` / `on_raise` / `on_accept` / `on_demote` to call `socket.send_text(...)`.
- [ ] On `Promoted` event for the current user, call `live_room_audio_publisher::publish_audio` and store the publisher to `close()` on `Demoted`.

### 4b. RedisLiveRoomBroker exercised in production
The Redis impl is wired (Task 5) and the AppState injects it (Task 6), but
integration tests use `MockLiveRoomBroker`. Exercise the real Redis path:
- [ ] Restart backend with `REDIS_URL=redis://redis:6379` set.
- [ ] Open a live room socket, send a chat. Verify `KEYS aulalite:room:*` shows expected keys.
- [ ] Send `PUBSUB NUMSUB aulalite:room:*` to verify subscribers exist while sockets are connected.

## 5. Chat + presence smoke (manual, with persistent socket from 4a)
- [ ] Open the live room as teacher in Chrome. Verify the WebSocket connection in Network tab.
- [ ] Open as a student in Chrome incognito. Confirm `presence_count` increments visible to both clients.
- [ ] Send a chat message; both clients see it. Verify a row in `live_room_messages`.
- [ ] Teacher deletes the message; both clients see the deletion broadcast.

## 6. Hand-raise audio promote (manual)
- [ ] Student clicks "Raise hand". Teacher sees them in the queue.
- [ ] Teacher accepts. Browser prompts for mic permission on the student side.
- [ ] Other viewers (a third Chrome profile) hear the student's voice.
- [ ] Teacher demotes. Audio stops.
- [ ] Student raises again later in the session. New nonce minted; audio works again.

## 7. Kick (manual)
- [ ] Teacher kicks a student. Student's socket closes.
- [ ] Student tries to reload the page. Socket upgrade returns 403.
- [ ] Verify a row in `live_room_kicks`.

## 8. Rate-limit (manual)
- [ ] Send 5 chat messages back-to-back from a student client.
- [ ] Confirm at least one `rate_limited` event appears in the Network/WebSocket frames.

## 9. Cross-tenant probe
- [ ] Tenant B's user fetches `/v1/sessions/<tenant-A-session-id>/messages` → 404 (masked).
- [ ] Tenant B's user attempts socket upgrade for tenant A's session → 404 (masked).

## 10. Lifecycle
- [ ] End class. All sockets close. `live_room_messages` rows persist.
- [ ] Start a new class within the same series. Old hand-raise queue and presence are gone (broker cleared).

## 11. Daily prune (manual / DB)
- [ ] Insert a message with `created_at = now() - interval '100 days'`.
- [ ] Run `cargo run --bin backend` for ~1 day OR call `db::live_room::prune_older_than(&pool, 90)` directly via a one-shot CLI.
- [ ] Verify the old row is deleted.

## Completion tag

Only after every required check above passes (and 4a + 4b are resolved):

```bash
git tag phase-1b-gamma-complete
git push origin phase-1b-gamma-complete
```
