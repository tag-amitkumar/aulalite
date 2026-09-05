<div align="center">

<img src="crates/shell-web/public/assets/brand/aulalite-wordmark.svg" alt="AulaLite — Live Learning Academy" width="360">

### A premium, multi-tenant Learning Management System where **live online classes are the headline feature** — built in Rust, end to end.

[![Rust](https://img.shields.io/badge/Rust-2021-CE412B?logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![Dioxus](https://img.shields.io/badge/UI-Dioxus%200.7-1A1A2E)](https://dioxuslabs.com/)
[![Axum](https://img.shields.io/badge/API-Axum-2E6E58)](https://github.com/tokio-rs/axum)
[![License](https://img.shields.io/badge/License-Proprietary-8f243d)]()
[![Status](https://img.shields.io/badge/Status-Pre--beta-b08842)]()

</div>

---

## What is AulaLite?

**AulaLite** (operated by **Elementors**, `elementors.guru`) is a lightweight, multi-tenant SaaS LMS for tutoring centres, language schools, coaching institutes, and independent tutors. *Aula* — the lecture hall — is the heart of the product: scheduling, teaching, recording, and reviewing **live online classes**, wrapped in a full LMS surface (courses, lessons, assignments, grading, files, analytics, billing, notifications).

It is **Rust from the browser to the database**: a single [Dioxus](https://dioxuslabs.com/) codebase ships to **web, desktop, and mobile**, backed by an [Axum](https://github.com/tokio-rs/axum) API over PostgreSQL, Redis, RustFS, and [MediaMTX](https://github.com/bluenviron/mediamtx), with Firebase Auth for identity only.

The UI is composed with the in-house [**dioxus-kinetics**](https://github.com/ChiranjibChaudhuri/dioxus-kinetics) component library (glass materials, motion, metric cards, command palette, data tables) layered over a bespoke warm-cream **"Elite Academy"** design system.

---

## Highlights

- 🎥 **Live classroom** — teacher WebRTC publish (WHIP) + student watch (WHEP) via MediaMTX, with real-time chat, hand-raise, presence, kick, and teacher→student stage hand-off over a self-healing WebSocket (auto-reconnect with token refresh + backoff). HLS fallback ready.
- ⏺️ **Recording pipeline** — auto-record → ffmpeg remux → RustFS → time-synced chat replay, with per-tenant retention janitors.
- 📚 **Courses & content** — modules, lessons (rich-text / video / file-bundle / live-session), drag-reorder builder, enrollment via codes **and** email invites.
- 📝 **Assignments → submissions → grading** — text + file submissions, numeric / pass-fail grading, instant or held release, course-scoped RBAC.
- 📊 **Attendance & analytics** — durable attendance (join/leave + session-end reconciliation) and org / per-course dashboards (KPIs + breakdowns).
- 👪 **Parent role** — strictly read-only dashboards of a linked child's released grades, attendance, and schedule.
- 💳 **Billing (Stripe)** — per-tenant subscriptions, usage-on-read, soft-cap prompts, signed idempotent webhooks — all on a mockable seam that runs without live keys. **Institutions hold the subscription; teachers/students are seats under it**, with seat-cap enforcement.
- 🛡️ **Admin & platform consoles** — org-admin member/role/branding management + audit log; Elementors platform super-admin for cross-tenant provisioning & suspension.
- 🔔 **Notifications** — in-app bell, transactional email (Resend), and FCM web push (service worker) on a unified, preference-aware engine.
- 🎨 **Per-tenant branding**, ⌘K **command palette**, **pg_trgm search**, and a guided **onboarding wizard**.
- 🔒 **Security posture** — Postgres Row-Level Security on every tenant table, JWT-gated media, HTML-sanitised user content, locked CORS + security headers.

---

## Architecture

```
                         ┌──────────────────────────── Clients (one Dioxus codebase) ───────────────────────────┐
                         │   shell-web (WASM)        shell-desktop (WebView)        shell-mobile (iOS/Android)    │
                         └───────────────────────────────────────┬───────────────────────────────────────────────┘
                                       HTTPS / WSS  (Firebase ID token, or native Firebase REST)
                                                                  │
            ┌─────────────────────────────────────────────────────┴───────────────────────────────────────────┐
            │                                   Rust + Axum API  (REST + WebSocket)                              │
            │  auth(JWKS+JIT) · courses · live-room realtime · recordings · assignments · attendance · billing   │
            │  notifications · admin · platform · search · branding · parent · MediaMTX auth/webhook · Stripe wh  │
            └──┬───────────────┬───────────────────┬──────────────────┬───────────────────┬────────────────────┘
               │               │                   │                  │                   │
        ┌──────┴─────┐  ┌──────┴──────┐    ┌────────┴───────┐  ┌───────┴──────┐   ┌────────┴────────┐
        │ PostgreSQL │  │   Redis     │    │    RustFS (S3) │  │   MediaMTX   │   │ External SaaS   │
        │  (RLS, all │  │ presence /  │    │ recordings /   │  │ WebRTC/WHIP/ │   │ Firebase · Stripe│
        │  app data) │  │ chat pub/sub│    │ uploads/assets │  │ WHEP · HLS   │   │ Resend · FCM    │
        └────────────┘  └─────────────┘    └────────────────┘  └──────────────┘   └─────────────────┘
```

**Boundaries that matter:** Firebase verifies identity only (JIT-provisions a Postgres `users` row on first token); MediaMTX is *only* a media router (all authz lives in the API); PostgreSQL is the source of truth with Row-Level Security per tenant.

### Tech stack

| Layer | Technology |
|---|---|
| Client UI | Dioxus 0.7.9 (WASM / WebView / native) + [dioxus-kinetics](https://github.com/ChiranjibChaudhuri/dioxus-kinetics) |
| API | Rust + Axum (REST + tungstenite WebSocket) |
| Database | PostgreSQL 16 (Row-Level Security, `sqlx`) |
| Cache / realtime | Redis 7 (presence, chat pub/sub, rate limits) — `fred` |
| Object storage | RustFS (S3-compatible) — `aws-sdk-s3` |
| Media plane | MediaMTX (WebRTC / WHIP / WHEP / HLS, RS256 viewer JWTs) |
| Identity | Firebase Auth (web JS SDK + native REST) |
| Billing / email / push | Stripe · Resend · FCM (all mockable) |
| Deploy | Docker Compose / Dokploy + Traefik |

---

## Monorepo layout

```
crates/
  core-types/        shared DTOs, errors, RBAC enums (client + server)
  api-client/        HTTP/WS client primitives
  design-system/     "Elite Academy" tokens + components + kinetics wrapper (kinetics_ui, KineticsStyles)
  platform-bridge/   PlatformBridge trait — WebBridge (wasm) + NativeBridge (desktop/mobile Firebase REST)
  features-auth/     login / signup / forgot
  features-courses/  the product UI: dashboard, courses, live room, assignments, analytics, billing, parent, …
  shell-web/         web shell (WASM) + routes + static assets (also reused by desktop)
  shell-desktop/     desktop shell (launches shell-web::App in a WebView)
  shell-mobile/      mobile shell (iOS/Android, native auth bootstrap)
  backend/           Axum API: handlers/ services/ db/ auth/ storage/ + tests
tools/
  aulalite-admin/    operator CLI
migrations/          forward-only SQL migrations (run automatically on API boot)
docs/superpowers/    specs, phase plans, and exit checklists
```

---

## Getting started

### Prerequisites

- Rust (stable, 2021 edition) + `cargo`
- The [Dioxus CLI](https://dioxuslabs.com/learn/0.7/getting_started/) — `cargo install dioxus-cli --version 0.7.9 --locked` (provides `dx`)
- Docker + Docker Compose (for Postgres / Redis / RustFS / MediaMTX) — or local equivalents
- `ffmpeg` on `PATH` (recording remux)

### 1. Bring up infrastructure

```bash
docker compose up -d postgres redis rustfs mediamtx
```

### 2. Run the API

Configure the backend via environment variables (see the table below). Local
`cargo run` also loads the ignored repository `.env` without overriding values
already supplied by the shell; production images never copy that file. Then:

```bash
cargo run -p backend        # migrations run automatically on boot; listens on $BIND_ADDR (default 0.0.0.0:8080)
```

### 3. Run a client

```bash
dx serve --package shell-web --port 3000             # web (WASM) — http://localhost:3000
dx serve --package shell-desktop --platform desktop  # desktop (WebView)
dx serve --package shell-mobile  --platform android  # or ios
```

The Dioxus framework, router, SSR helpers, and CLI are intentionally aligned at
`0.7.9`. The repository's original `0.7.4` pin was a temporary May 2026
workaround for an unpublished upstream `dioxus-fullstack` dependency; it was not
a product downgrade and that upstream condition no longer applies.

> The web container generates `/runtime-config.js` from publishable `FIREBASE_*`, `FCM_VAPID_KEY`, and the public app/admin/API origins at startup; source defaults remain intentionally empty. The page and Firebase messaging worker share that generated config. Native debug shells may read `FIREBASE_WEB_API_KEY` + `AULALITE_API_BASE_URL` from the ignored `.env`; release shells ignore runtime `.env` overrides and embed protected build-environment values. See [native application release status](docs/native-release.md) before producing distribution builds.

### Key environment variables

| Group | Variables |
|---|---|
| Core | `APP_ENV` (`production` hardens secret checks) · `APP_ORIGIN` · `API_ORIGIN` · `AULALITE_BROWSER_ORIGINS` · `AULALITE_SUPER_ADMIN_EMAILS` · `BIND_ADDR` · `DATABASE_URL` (runtime role) · `MIGRATION_DATABASE_URL` (schema owner) · `DB_*` pool bounds/timeouts · `REDIS_URL` |
| Web runtime | `AULALITE_API_BASE_URL` · `AULALITE_APP_ORIGIN` · `AULALITE_ADMIN_ORIGIN` |
| Firebase | `FIREBASE_PROJECT_ID` · `FIREBASE_TOKEN_ISSUER` · `FIREBASE_JWKS_URL` · `FIREBASE_WEB_API_KEY` · `FIREBASE_AUTH_DOMAIN` · browser `FIREBASE_*` metadata · Android `FIREBASE_MESSAGING_SENDER_ID` / `FIREBASE_ANDROID_APP_ID` · `FCM_VAPID_KEY` |
| Storage (RustFS/S3) | `S3_ENDPOINT_URL` (public presigning origin) · `S3_INTERNAL_ENDPOINT_URL` (private service origin) · `S3_REGION` · `S3_BUCKET` · `AWS_ACCESS_KEY_ID` · `AWS_SECRET_ACCESS_KEY` |
| Media (MediaMTX) | `MEDIAMTX_HTTP_URL` · `MEDIAMTX_PUBLIC_WEBRTC_URL` · `MEDIAMTX_PUBLIC_HLS_URL` · `MEDIAMTX_AUTH_SHARED_HEADER` · `JWT_RS256_PRIVATE_KEY_PEM` · `RECORDINGS_DIR` · optional `AULALITE_TURN_*` |
| Billing (Stripe) | `STRIPE_SECRET_KEY` · `STRIPE_WEBHOOK_SECRET` · `STRIPE_SUCCESS_URL` · `STRIPE_CANCEL_URL` · `STRIPE_PRICE_ID_STARTER` · `STRIPE_PRICE_ID_PRO` |
| Notifications | `RESEND_API_KEY` · `RESEND_FROM` · `FCM_PROJECT_ID` · `FCM_SERVICE_ACCOUNT_JSON` or `FCM_SERVICE_ACCOUNT_JSON_PATH` · `FCM_TOKEN_URI` · `FCM_BASE_URL` |
| Security | `SSO_SESSION_SECRET` · `AULALITE_DATA_ENCRYPTION_KEY` (optional `*_PREVIOUS` during rotation) · `AULALITE_MFA_ENFORCE` · `AULALITE_RATE_LIMIT*` · optional `AULALITE_DKIM_*` |
| Retention | `LIVE_ROOM_CHAT_RETENTION_DAYS` · `LIVE_ROOM_RECORDING_RETENTION_DAYS` |
| Native client | `AULALITE_API_BASE_URL` (desktop debug defaults to `http://localhost:8080`; Android-emulator debug to `http://10.0.2.2:8080`; release embeds an explicit bare HTTPS origin) · `AULALITE_TOKEN_STORE` (default `auto`, OS-secure) |

Outside `APP_ENV=production`, missing media/billing/notification secrets fall back to ephemeral, mock, or disabled implementations so the stack runs locally without external accounts. Production hard-fails for database isolation, application data encryption, session/media signing, object storage, MediaMTX callback auth, API rate limiting, Stripe, and transactional email; push remains explicitly degradable and emits a warning.

For live Stripe billing, enable and configure the **Customer Portal** in the same Stripe account as `STRIPE_SECRET_KEY`. Workspace billing administrators can then update payment details, retrieve invoices, change plans, or cancel from the in-app **Manage subscription** action. Portal return URLs are derived exclusively from the bare HTTP(S) `APP_ORIGIN`; no client-provided redirect is accepted.

---

## Database & migrations

All schema lives in `migrations/` as forward-only, timestamp-ordered SQL. On startup the API applies pending migrations through the owner-level `MIGRATION_DATABASE_URL`, closes that pool, then serves requests through the least-privileged `DATABASE_URL`. Local/dev may omit the migration URL and reuse the runtime connection; production refuses to do so. Every tenant-scoped table carries `tenant_id` with **FORCE ROW LEVEL SECURITY**; point the production runtime URL at the non-bypass `aulalite_app` role (see `migrations/20260517000020_app_role.sql`) so RLS is enforced at the database.

---

## Testing

```bash
cargo test --workspace                 # unit + SSR (no external services needed)
cargo check -p shell-web --target wasm32-unknown-unknown   # verify the web build
dx build --package shell-web --platform web --release       # build the deployable web/PWA bundle
cargo test -p backend --features db-tests --tests           # DB integration tests (requires DATABASE_URL)
```

- **Unit / SSR** tests run anywhere (design-system, features, backend pure logic).
- **Integration** tests (`crates/backend/tests/`) exercise real RLS / cross-tenant / billing-webhook / attendance flows. They are opt-in behind the backend `db-tests` feature and require a live PostgreSQL (`DATABASE_URL`).
- **E2E** uses Playwright against the running web app.

---

## Multi-tenant & seat model

- A **tenant = an institution / coaching centre.** Everything is tenant-scoped via RLS.
- The **organization owner holds one subscription** (`subscriptions` is keyed by `tenant_id` — *Customer = Tenant*).
- Teachers, TAs, students, and parents are **`tenant_memberships` (seats)** under that single plan; `plan.included_seats` is the cap.
- Adding a member past the cap is blocked (with an upgrade prompt) when `overage_behavior = 'block'`; pending invites count as reserved seats.
- A user may hold active memberships in multiple institutions (for example, an independent teacher's academy plus a school). Web/native clients send the chosen membership as `X-AulaLite-Tenant: <tenant UUID>` on authenticated API calls; browser live-room WebSockets use the equivalent `workspace_id` query parameter because the WebSocket API cannot set custom headers. The API validates the choice against the caller's active memberships on every request and otherwise defaults deterministically to their oldest membership.

### Roles

| Role | Capability (summary) |
|---|---|
| Platform owner (Elementors) | Cross-tenant provisioning, suspension, audited break-glass recovery |
| Organization owner | Tenant ownership, billing, administrator appointment, ownership transfer, plus all admin capabilities |
| Organization admin | Tenant settings, branding, staff/learner membership, integrations, and audit |
| Teacher | Own/assigned courses: content, live classes, assignments, grading |
| TA | Assist on assigned courses (moderation, grading) |
| Student | Enrolled courses: attend, submit, view released grades |
| Parent | Read-only: linked child's grades, attendance, schedule |

Roles map to centralized product capabilities (`PlatformManage`,
`OrganizationOwn`, `OrganizationManage`, `BillingManage`, `MembersManage`, `IntegrationsManage`,
`Teach`, `Assist`, `Grade`, `Learn`, and `ParentRead`) and are then narrowed by
resource scope such as course staff assignment, enrollment, submission owner,
or linked learner. Important invariants are enforced in both application and
database paths:

- A platform operator has global platform tools but no implicit access to a
  tenant. Tenant overrides require an active membership and explicit workspace
  selection.
- A public, verified self-service signup receives one personal academy and an
  `org_owner` membership. This is the independent-teacher model—solo teachers
  use the same tenant architecture as schools and can grow into a team later.
- Every initialized institution has exactly one active `org_owner`; it may
  have zero or more `org_admin` members. Only the owner can manage billing,
  appoint or modify administrators, or atomically transfer ownership to an
  active administrator. The owner cannot be demoted, suspended, erased, or
  replaced through generic membership/invitation paths. Platform recovery is
  a separate, audited break-glass operation.
- Teacher and TA authority is distinct: teachers create/publish course work;
  TAs assist, moderate, and grade only where assigned. Students and parents do
  not inherit staff capabilities.

---

## Deployment

A single production Compose stack (Postgres · Redis · RustFS · MediaMTX · API · web) deploys from GitHub to **Dokploy** behind **Traefik**. The app, object storage, WebRTC signalling, and HLS origins receive managed TLS; only MediaMTX ICE traffic publishes directly on UDP `8189`, with no host networking. Normal pushes use Dokploy's GitHub App and consume no GitHub Actions minutes; run the local release preflight before pushing and rehearse the documented rollback path before launch.

---

## Status & roadmap

The web/PWA client implements the broad SaaS surface: billing, workspace and platform consoles, analytics, attendance, parent access, notifications, branding, search, onboarding, and live-class workflows. Desktop and mobile share its route graph, role model, visual system, native authentication, secure credential storage, workspace lifecycle, file transfer, live-class transport, deep-link handling, and bounded same-session cache. They are implementation candidates, not signed store-ready releases: OS push/share/screen-capture adapters, a tracked browser-only workflow tail, real-device acceptance, signing, and store gates remain in the [native release status](docs/native-release.md).

---

## Credits & license

- UI library: [**dioxus-kinetics**](https://github.com/ChiranjibChaudhuri/dioxus-kinetics)
- Typefaces: **Source Serif 4** (display) + **Inter** (body) — self-hosted (SIL OFL)
- AI-assisted study is a **separate** Elementors system, intentionally not embedded here.

**Proprietary** — © Elementors. All rights reserved.
