# Local UI Verification Tools

## ui-real-stack.spec.js

Single Playwright spec that drives the real backend (docker compose) and real
frontend (`dx serve`) end-to-end. Logs in as Local Teacher and Local Student
using credentials from `.env`, walks every applicable route, asserts the
rendered DOM and CSS, and screenshots each page to `target/playwright-ui/`.

### Prereqs

- Docker Desktop running
- Node 20+ with `npx playwright install chromium`
- Rust toolchain (the `dx` CLI from `cargo install dioxus-cli`)
- `.env` populated, including `LOCAL_LOGIN_*` and `LOCAL_LOGIN_*_PASSWORD`
- If backend or frontend Rust code has changed since the last image build:
  `docker compose build backend frontend && docker compose up -d` to refresh.

### Run

**Before first run after code changes:** the docker compose images may be
stale. Refresh with `docker compose build backend frontend && docker compose
up -d` before invoking the script.

From repo root:

```powershell
.\tools\run-ui-check.ps1
```

To tear down on success:

```powershell
.\tools\run-ui-check.ps1 -Clean
```

### Output

- Screenshots: `target/playwright-ui/<role>-<route>.png`
- dx serve log: `target/dx-serve.log`
- Playwright report: `playwright-report/` (default Playwright output)

### Coverage

Two tests, serial mode:

1. **teacher walks the workspace via email+password login** — dashboard,
   courses, course detail, people, schedule, assignments (list + detail),
   redeem, my schedule, live session.
2. **student walks the visible subset** — dashboard, courses, course detail,
   assignments, redeem, schedule, live session. Asserts teacher-only
   controls (Go Live, New Course) are absent.

Both tests assert a bounce-probe: after login, navigating between courses
and dashboard must not redirect to `/login`.

### Not covered

- Real WebRTC media playback (stage element + WS handshake only).
- Mobile gestures, cross-browser. Chromium only.
- CI. Local-only by design.
