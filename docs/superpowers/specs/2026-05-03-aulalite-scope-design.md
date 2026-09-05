# AulaLite — Scope & Architecture Design

**Date:** 2026-05-03
**Status:** Draft for review
**Operator:** Elementors (`www.elementors.guru`)
**Application URL (MVP):** `app.elementors.guru`
**Media URL (MVP):** `media.elementors.guru`

---

## Executive Summary

AulaLite is a multi-tenant lightweight Learning Management System where **live online classes are the headline feature**, supported by a full LMS surface (courses, lessons, assignments, grading, files, notifications). It targets tutoring centers, language schools, and independent tutors at MVP, with a hierarchy of Elementors (platform super admin) → Institutions → Teachers → Students → Parents (read-only analytics).

Technically, AulaLite is **Rust end-to-end**: a Dioxus client shipping to web, mobile-native (iOS + Android), and desktop from a single workspace, and an Axum backend running on Dokploy with self-hosted Postgres, Redis, MinIO, and MediaMTX. Firebase Auth handles identity only — all application data lives in Postgres. Live classes default to one-way (teacher → students) with student → teacher interaction (chat, hand-raise, opt-in stage handoff) layered on top.

The MVP ships **everything** in the feature matrix below at launch ("Approach A scope"), but is **built in vertical-slice order** ("Approach C focus") so the core teacher → student loop becomes complete and demo-able before secondary surfaces are layered on.

---

## Scoping Decisions (Captured From Brainstorming)

| # | Question | Decision |
|---|---|---|
| 1 | Core identity | Full lightweight LMS with **live class as the headline feature** |
| 2 | Tenancy | Multi-tenant SaaS; hierarchy: Elementors super admin → Institutions/Orgs → Teachers → Students (+ Parents) |
| 3 | Target customer (MVP-driving) | **B + C**: tutoring centers, language schools, independent tutors. Parent role lighter than K-12. |
| 4 | Live-class scale (MVP) | **1 to ~100 concurrent participants**; WebRTC primary, HLS fallback ready at the upper end |
| 5 | Tech stack | **Dioxus** (web + mobile native + desktop) + **Rust/Axum backend** on Dokploy + **PostgreSQL + Redis + MinIO + MediaMTX** self-hosted; **Firebase Auth** for identity only |
| 6 | Platform priority | **Web > Mobile > Desktop** (web most polished; mobile real but second priority; desktop ships if Dioxus desktop builds cleanly from the same code) |
| 7 | Pricing | Layered: flat-fee plan with included quotas (seats, class-hours, recording GB) + soft-cap upgrade prompts; **7-day free trial**; positioning *cheap and featureful* |
| 8 | Branding / whitelabel | **Single domain at MVP**, whitelabel deferred to post-MVP |
| 9 | Recording defaults | **Tenant-level default + per-class override** |

Additional constraints captured:
- Mobile native publishing (camera/mic) is the riskiest stack item; deferred from Phase 1 to Phase 2 with schedule buffer.
- Live class default mode is **one-way teacher → students**; student → teacher interaction is desirable but secondary, primarily via chat/hand-raise plus explicit teacher-initiated stage handoff.
- Parents see **grade analytics, attendance, and upcoming class schedule** for their linked children. They cannot watch live; recording watch is **off by default**, tenant-policy gated.
- MVP overage behavior is **block at cap**, with `subscriptions.overage_behavior` enum (`'block' | 'metered'`) so metered overage can be enabled per-tenant in v1.
- Backups stay within Dokploy at MVP; off-host replication added once revenue justifies it.
- Deploys are **auto-deploy to staging and prod** on tagged image push, gated by automated tests + Dokploy health-check rollback + forward-only Postgres migrations.

---

## Section 1 — Product Scope & MVP Feature Matrix

### Roles

1. **Platform super admin** (Elementors staff) — manages every tenant, break-glass access, billing reconciliation.
2. **Org admin** (institution owner) — manages teachers, students, parents, courses, branding, billing within their tenant.
3. **Teacher / Instructor** — owns courses, schedules and runs live classes, posts assignments, grades.
4. **TA** — moderates live sessions, assists with grading. Per-course assignment.
5. **Student** — joins courses, attends live classes, submits assignments, sees grades.
6. **Parent** — read-only window into their child's grade analytics, attendance, schedule. One parent can be linked to multiple students.

### MVP feature matrix (everything ships at launch; sequenced by build priority)

| Layer | MVP feature | Priority |
|---|---|:---:|
| **Core spine** | Auth (Firebase) + tenant provisioning + Postgres user mirror | P0 |
| | Course CRUD with modules + lessons | P0 |
| | Enrollment (admin invites + self-enroll via code) | P0 |
| | Live class scheduling + room | P0 |
| | Live class teaching: WebRTC publish (web), watch (web + mobile), chat, hand-raise | P0 |
| | Auto-recording (tenant default + per-class override) | P0 |
| | Recording playback (web + mobile) | P0 |
| | Assignment CRUD + student submission (text + file upload) | P0 |
| | Grading + feedback + grade visibility to student | P0 |
| **Wrap-around** | Parent role + child-link + parent dashboard (grade analytics) | P1 |
| | Mobile native live publishing (iOS/Android camera/mic) | P1 |
| | TA moderation controls (mute, kick, redact, stage handoff) | P1 |
| | Student stage / presentation handoff (teacher-initiated) | P1 |
| | Files library (per-course assets) | P1 |
| | Push notifications (FCM, transactional only) | P1 |
| | Structured search (filters, tags, instructor, status — Postgres `pg_trgm`) | P1 |
| | Stripe billing: 2 tiers + 7-day trial + soft-cap upgrade prompts | P1 |
| | Org admin: branding (logo, primary color), policy settings | P1 |
| | Attendance reports (auto-tracked from MediaMTX hooks + Redis presence) | P1 |
| | Email transactional (welcome, invites, password reset) | P1 |
| **Polish** | Desktop build (Dioxus desktop, same code) — or formally deferred to v1 | P2 |
| | HLS fallback for >50-concurrent classes | P2 |
| | Recording auto-prune by tenant retention setting | P2 |
| | Audit log surface for org admins | P2 |
| | Hard overage billing provision (`overage_behavior` setting; `'block'` only at MVP launch, `'metered'` enabled in v1) | P2 |
| | i18n scaffolding (English populated; locale switcher in place) | P2 |

### Explicitly out of MVP (v1)

Quiz engine, LTI/SCORM, custom domains / advanced whitelabel, full-text search (`tsvector`), vector/semantic search, public app-store distribution polish, deep LMS analytics dashboards beyond attendance.

### Out of v1 (parking lot for v2+)

SAML/OIDC SSO, advanced quizzes, processed transcripts, AI-assisted grading.

---

## Section 2 — System Architecture

A Rust backend on Dokploy fronts a Postgres + MinIO + Redis self-hosted core, with MediaMTX as the dedicated media plane and Firebase Auth providing identity-only — everything else is yours.

### Components

| Layer | Component | Notes |
|---|---|---|
| **Client** | Dioxus shared codebase | Web (WASM), iOS native, Android native, Desktop. Platform bridges for camera/mic, FCM, file picker. |
| **Edge / TLS** | Traefik (provided by Dokploy) | Auto Let's Encrypt for `app.elementors.guru` and `media.elementors.guru`. Routes `/api`, `/ws`, static assets, HLS endpoint of MediaMTX. |
| **API** | Rust + Axum (HTTP/JSON) | REST endpoints, Stripe webhooks, MediaMTX auth callback, MediaMTX hook ingestion, Firebase token verification (cached JWKS), JIT user provisioning. |
| **Realtime gateway** | Rust + Axum + tungstenite (WebSocket) | Same binary or sibling process. Live-class chat, hand-raise, presence push, notification push. |
| **Identity** | Firebase Auth | Email/password + Google + optional TOTP MFA for admins. ID tokens verified server-side via Firebase Admin SDK or cached JWKS. |
| **Primary DB** | PostgreSQL 16 | All app data. Row-Level Security on every tenant-scoped table. |
| **Cache + ephemeral + pub/sub** | Redis 7 | Live presence, hand-raise queue, chat fan-out (pub/sub), heartbeat counters, rate limits, short-term denylist. |
| **Object store** | MinIO (S3-compatible) | Course assets, assignment uploads, recording segments, exports. Pre-signed URLs for client direct upload/download. |
| **Media plane** | MediaMTX (dedicated container) | WebRTC/WHIP/WHEP publish + read; HLS fallback (P2); per-path auth via HTTP callback to Rust API; recording to local volume. |
| **Recording uploader** | Rust sidecar | Watches MediaMTX recording dir, uploads completed segments to MinIO, posts metadata to API. |
| **Push** | Firebase Cloud Messaging | Backend mints data messages; client SDKs deliver. |
| **Email** | Resend or Postmark | Transactional only at MVP. |
| **Billing** | Stripe | Customer = Tenant. Webhook → API → Postgres usage/subscription tables. |
| **Observability** | Prometheus + (optional) Grafana | Scrapes MediaMTX `/metrics` and Axum metrics. Loki or Dokploy-native logs for stdout. |

### Dokploy network nuance

Traefik handles HTTP/HTTPS only. **WebRTC media itself** (UDP/TCP for ICE candidates) requires direct host-network exposure on the Dokploy node — Traefik can't proxy it. So in Compose, MediaMTX runs in `network_mode: host` (or with explicit `ports:` for WebRTC UDP/TCP listeners) and only its HTTP routes (HLS, API) sit behind Traefik.

### Two representative request paths

**Live class join (student):**

```
Dioxus client
  → POST /v1/live/sessions/{id}/join  (Firebase ID token)
    Axum API:
      verify Firebase token
      lookup user/tenant in Postgres
      check enrollment + session role
      respond with WebRTC URL + reader mode + (HLS fallback URL if >50 viewers)
  → WebRTC connect to MediaMTX at media.elementors.guru
    MediaMTX → POST /v1/mediamtx/auth (with token + path + action)
    Axum API: verify Firebase token, re-check enrollment, return 204 allow / 403 deny
    MediaMTX → POST /v1/mediamtx/events/read-start (hook)
    Axum API: start attendance tracking; subscribe client WS to chat/presence channel
  → WebSocket /ws/sessions/{id} (chat + hand-raise + presence)
```

**Recording lifecycle:**

```
MediaMTX records fMP4 segments to /recordings/{tenantId}/{sessionId}/...
  segment complete → runOnRecordSegmentComplete hook
    → POST /v1/mediamtx/events/record-complete
    Axum API: enqueue uploader job (Redis stream)
  Uploader sidecar:
    pulls job, copies segment to MinIO under tenant-prefixed path
    posts metadata to API → Postgres recordings table
  Session ended → API marks session "ended", reconciles attendance from Redis heartbeats
  Tenant retention policy → nightly job prunes MinIO objects + DB rows past TTL
```

### Deployment topology on Dokploy

Single Dokploy project, one Docker Compose stack:

```
- aulalite-api          (Rust/Axum, REST + WebSocket, ≥2 replicas in prod for rolling deploys)
- aulalite-web          (static Dioxus WASM bundle, served behind Traefik)
- mediamtx              (network_mode: host for WebRTC UDP/TCP; HLS+API HTTP behind Traefik)
- recording-uploader    (sidecar; sees mediamtx volume + MinIO)
- postgres
- redis
- minio
- prometheus            (scrapes API + mediamtx)
```

Stripe, Firebase, FCM, Resend are external SaaS — no local containers.

### Boundaries that matter

- **Firebase boundary** — API verifies Firebase ID tokens; on first verified token for an unknown UID, JIT-provisions a row in `users` (Postgres) keyed by `firebase_uid`. Postgres is the source of truth for everything except identity claims (email, MFA state).
- **Media boundary** — MediaMTX is *only* a media router. All authorization decisions live in the Rust API. The Rust API never streams media; MediaMTX never reads/writes Postgres.
- **WebSocket vs HTTP** — REST for everything CRUD + idempotent. WebSocket only for live-class chat, presence, hand-raise, and "your grade was posted" pushes during an active session.

---

## Section 3 — Data Model (PostgreSQL)

Schema shape; indexes, constraints, and triggers come during implementation.

### Tenant + identity

```sql
tenants (
  id              UUID PK,
  slug            TEXT UNIQUE NOT NULL,
  name            TEXT NOT NULL,
  status          TEXT NOT NULL,            -- 'active' | 'trialing' | 'suspended'
  plan_id         TEXT,                     -- Stripe price id
  stripe_customer_id TEXT,
  branding        JSONB,                    -- {logo_url, primary_color, ...}  (P1)
  recording_default BOOL NOT NULL DEFAULT true,
  recording_retention_days INT NOT NULL DEFAULT 90,
  trial_ends_at   TIMESTAMPTZ,
  created_at, updated_at
)

users (
  id              UUID PK,
  firebase_uid    TEXT UNIQUE NOT NULL,
  email           CITEXT UNIQUE NOT NULL,
  display_name    TEXT,
  avatar_url      TEXT,
  locale          TEXT,
  is_platform_admin BOOL NOT NULL DEFAULT false,
  created_at, last_seen_at
)

tenant_memberships (
  tenant_id       UUID FK → tenants,
  user_id         UUID FK → users,
  role            TEXT NOT NULL,            -- 'org_admin' | 'teacher' | 'ta' | 'student' | 'parent'
  status          TEXT NOT NULL,            -- 'active' | 'invited' | 'suspended'
  invited_by      UUID,
  joined_at, updated_at,
  PRIMARY KEY (tenant_id, user_id)
)

parent_links (
  parent_user_id  UUID FK → users,
  student_user_id UUID FK → users,
  tenant_id       UUID FK → tenants,
  relationship    TEXT,
  created_at,
  PRIMARY KEY (parent_user_id, student_user_id)
)
```

### Courses + content

```sql
courses (
  id, tenant_id, slug, title, description, status ('draft'|'published'|'archived'),
  cover_asset_id, owner_user_id, visibility, created_at, updated_at
)
modules (id, course_id, title, sort_order)
lessons (
  id, course_id, module_id, type ('rich_text'|'video'|'live_session'|'file_bundle'),
  title, body_md, video_asset_id, live_session_id, sort_order, published_at
)
course_memberships (
  course_id, user_id, tenant_id, role ('student'|'teacher'|'ta'),
  status, joined_at, PRIMARY KEY (course_id, user_id)
)
enrollment_codes (
  id, course_id, code UNIQUE, max_uses, uses, expires_at, created_by
)
```

### Live sessions + attendance + recordings

```sql
live_sessions (
  id, tenant_id, course_id, title,
  status ('scheduled'|'live'|'ended'|'cancelled'),
  starts_at, ends_at, actual_started_at, actual_ended_at,
  primary_teacher_id, ta_user_ids UUID[],
  mode ('lecture'|'discussion'),            -- default 'lecture' (one-way)
  recording_enabled BOOL,                   -- defaults from tenant; override here
  main_path TEXT,                           -- live/{tenantId}/{courseId}/{sessionId}/main
  hls_fallback_enabled BOOL DEFAULT false,
  created_at, updated_at
)

session_roles (
  session_id, user_id, role ('publisher'|'cohost'|'viewer'),
  granted_at, granted_by
)

attendance (
  session_id, user_id, tenant_id,
  first_joined_at, last_left_at, total_seconds INT,
  reconnect_count INT,
  PRIMARY KEY (session_id, user_id)
)

recordings (
  id, tenant_id, session_id, status ('recording'|'processing'|'available'|'failed'|'pruned'),
  storage_bucket, storage_prefix, total_bytes BIGINT, duration_seconds INT,
  starts_at, ends_at, expires_at, created_at
)

recording_segments (
  id, recording_id, sequence INT, storage_key TEXT, bytes BIGINT, duration_ms INT
)

chat_messages (
  id, tenant_id, session_id, user_id, body TEXT,
  posted_at TIMESTAMPTZ, redacted_by UUID NULL, redacted_at TIMESTAMPTZ NULL
)
```

### Assignments + submissions

```sql
assignments (
  id, tenant_id, course_id, lesson_id NULL,
  title, instructions_md, max_points INT,
  due_at, allow_late BOOL, attachments_asset_ids UUID[],
  status ('draft'|'published'), created_at
)

submissions (
  id, tenant_id, assignment_id, course_id, student_user_id,
  status ('draft'|'submitted'|'returned'|'graded'),
  text_answer TEXT, attachment_asset_ids UUID[],
  submitted_at, grade NUMERIC, graded_by_user_id, graded_at,
  student_visible_feedback TEXT,
  PRIMARY KEY (id),
  UNIQUE (assignment_id, student_user_id)
)

submission_private_notes (
  submission_id PK FK, notes TEXT, updated_by, updated_at
)
```

### Files

```sql
file_assets (
  id, tenant_id, owner_user_id,
  bucket, object_key,
  content_type, size_bytes BIGINT,
  visibility ('private'|'course'|'public'),
  linked_entity_type, linked_entity_id,
  created_at
)
```

### Billing + usage metering + audit

```sql
plans (
  id PK TEXT, name, monthly_price_cents,
  included_seats, included_class_minutes, included_recording_gb
)

subscriptions (
  tenant_id PK, plan_id, status, current_period_start, current_period_end,
  trial_ends_at, stripe_subscription_id,
  overage_behavior TEXT NOT NULL DEFAULT 'block'  -- 'block' | 'metered'
)

usage_counters (
  tenant_id, period_yyyymm,
  active_seats INT, class_minutes_used INT, recording_gb_used NUMERIC,
  PRIMARY KEY (tenant_id, period_yyyymm)
)

audit_events (
  id, tenant_id, actor_user_id, action, resource_type, resource_id,
  metadata JSONB, occurred_at
)
```

### Schema invariants

1. **Every domain table has `tenant_id`.** Row-Level Security policies on every table enforce `tenant_id = current_setting('app.tenant_id')::uuid`.
2. **No staff-only data lives in a student-readable table.** `submission_private_notes` is a separate table specifically because that's where a misconfigured permission has the highest blast radius.
3. **No large blobs in Postgres.** Anything >100 KB lives in MinIO; Postgres holds metadata.
4. **Soft deletes only where audit matters** — `submissions`, `attendance`, `recordings` keep history. CRUD entities like `lessons` can hard-delete when unreferenced.

---

## Section 4 — Live-Class Subsystem

### Path convention

```
live/{tenantId}/{courseId}/{sessionId}/main           # primary teacher stream
live/{tenantId}/{courseId}/{sessionId}/screen         # screen share
live/{tenantId}/{courseId}/{sessionId}/stage/{userId} # student presentation slot
live/{tenantId}/{courseId}/{sessionId}/backup/{userId}# TA / co-host backup
```

MediaMTX permissions are path-oriented, so every authorization decision is `(action, path)`. Tenant isolation is enforced at the path level: a student in tenant X cannot read or publish to a path containing tenant Y, by API rule.

### Default interaction model: one-way

Default mode is **lecture** — teacher publishes `main`; students watch + chat + raise hand. Stage handoff (a student or TA publishing to `stage/{userId}`) is a per-session, teacher-initiated affordance, not a default capability. `stage/{userId}` paths exist in the convention but are rarely populated.

### Auth flow (MVP — HTTP auth)

```
client → POST /v1/live/sessions/{id}/join  (Firebase ID token)
  Axum API: verify token → check enrollment + session role → respond
            { mode, webrtc_url, whep_url, hls_url?, attendance_heartbeat_sec }

client → connects to MediaMTX (WebRTC publish/read)
  MediaMTX → POST /v1/mediamtx/auth
             { user, password, token, ip, action, path, protocol, id, query }
  Axum API: verify Firebase token (cached JWKS), re-check session role + path
            → 204 allow / 403 deny
  MediaMTX → admit or reject
```

Cutover to MediaMTX JWT mode (with `mediamtx_permissions` claims) is a v1/v2 perf optimization. By then we'll mint short-lived media JWTs from the API after Firebase verification.

### Hooks ingested by the API

```
runOnReady          → mark session live, broadcast WS presence
runOnNotReady       → mark session ended (if primary path), trigger attendance reconciliation
runOnRead           → start-of-watch event → start attendance timer
runOnUnread         → end-of-watch event → close attendance timer
runOnRecordSegmentComplete → enqueue uploader job
```

### Realtime sidecar (chat, hand-raise, presence)

Lives on the **Rust API binary**, not in MediaMTX. Each live session gets a Redis pub/sub channel `session:{sessionId}`:

| Event | Producer | Consumer |
|---|---|---|
| chat message | Client → WS → API | All session WS subscribers |
| hand-raise / lower | Client → WS → API → Redis sorted set | Teacher panel via WS |
| presence heartbeat | Client → WS every 15s | Redis ZADD with TTL; reconciler computes attendance |
| stage handoff | Teacher → WS → API → MediaMTX path permission update + WS broadcast | All clients (UI re-renders) |

Chat messages persist to Postgres so students can scroll back during and after the session.

### Recording pipeline

```
MediaMTX  →  /recordings/{tenant}/{session}/segment_NNNN.mp4 (fMP4, 1h max segment)
             → runOnRecordSegmentComplete hook
recording-uploader sidecar:
  reads job from Redis stream
  S3-compatible PUT to MinIO at recordings/{tenant}/{session}/{seg}.mp4
  POSTs metadata to API → recording_segments + recordings rows
  removes local file (configurable: keep N days for fast playback)
```

Playback URL minted by API: short-lived MinIO pre-signed GET (15 min TTL, refreshed on token refresh). Students see the recording via `/v1/recordings/{id}/playback`, which returns the pre-signed URL plus chat replay JSON.

### Reconnection ladder (client)

1. WebRTC retry with same join context.
2. Refresh Firebase ID token if stale.
3. Switch WebRTC to TCP if UDP appears blocked.
4. Add STUN (Google public is fine for MVP; self-hosted Coturn in v1 if corporate-network customers report NAT issues).
5. Fall back to **HLS read-only** if all WebRTC paths fail.
6. Surface "low-bandwidth mode" toggle as a manual escape hatch.

### Mobile-specific consideration (the risk pocket)

For Dioxus mobile, **publishing** uses native iOS/Android WebRTC SDKs through FFI — not WASM `web-sys` bindings. This is the biggest unknown in the stack and gets dedicated implementation time in Phase 2. **Watching** on mobile uses native WebRTC subscribe (or HLS fallback) — well-trodden territory. So mobile P0 (watch + chat + recordings) is low risk; mobile P1 (publish from phone) gets a generous schedule buffer and a fallback plan: if FFI publish bindings prove too rough, mobile teachers initiate class from web and use the phone only as a second camera.

### Quality / latency expectations

| Audience | Default delivery | Latency | Caveat |
|---|---|---|---|
| 1–25 viewers | WebRTC | ~200–500 ms | Default |
| 26–50 viewers | WebRTC | ~200–500 ms | Watch CPU on the MediaMTX host |
| 51–100 viewers | WebRTC primary, HLS fallback option per-session | WebRTC <500 ms / HLS 4–10 s | Tenant or session-level toggle to flip to HLS-primary if needed |
| Recording playback | HLS over MinIO pre-signed | n/a | Standard VOD |

### Out of scope (deliberate)

- Server-side compositing / SFU multiplexed grids (too far out of MediaMTX's documented strengths).
- A "video meeting" UI with 25 simultaneous webcam tiles. The model is **one-stage-at-a-time** with explicit handoff.

---

## Section 5 — Multi-Tenant Model & RBAC

Two layers, ported to Postgres.

### Layer 1 — Coarse-grained, tenant-wide role

Pulled from `tenant_memberships.role` for the `(tenant_id, user_id)` of the current request. Resolved once per request by an Axum middleware and pinned into a `RequestContext`:

```
RequestContext {
  user_id, firebase_uid, email,
  tenant_id, tenant_role,    // 'org_admin' | 'teacher' | 'ta' | 'student' | 'parent'
  is_platform_admin: bool,   // Elementors super admin
  jwt_iat, jwt_exp,
}
```

The middleware also runs `SET LOCAL app.tenant_id = $1` on the Postgres connection so RLS enforces isolation at the database level.

### Layer 2 — Fine-grained, resource-scoped role

| Question | Source of truth |
|---|---|
| Can this user see this course? | `course_memberships(course_id, user_id)` |
| Can this user teach this session? | `live_sessions.primary_teacher_id` ∪ `live_sessions.ta_user_ids[]` ∪ `session_roles` |
| Can this parent see this student's grades? | `parent_links(parent_user_id, student_user_id)` |

Decisions are made in handler code (Rust), not in DB rules — DB only enforces tenancy. This keeps complex authorization debuggable in one place.

### Permission matrix (MVP)

| Action | Platform admin | Org admin | Teacher | TA | Student | Parent |
|---|:-:|:-:|:-:|:-:|:-:|:-:|
| Manage tenants (create, suspend, billing) | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ |
| Manage tenant settings, branding, plan | tenant-scoped | ✅ | ❌ | ❌ | ❌ | ❌ |
| Invite/remove org members | tenant-scoped | ✅ | ❌ | ❌ | ❌ | ❌ |
| Create/edit course | ✅ | ✅ | own + assigned | ❌ | ❌ | ❌ |
| Publish lessons | ✅ | ✅ | own course | ❌ | ❌ | ❌ |
| Enroll students | ✅ | ✅ | own course | ❌ | self via code | ❌ |
| Schedule live class | ✅ | ✅ | own course | ❌ | ❌ | ❌ |
| Publish to live `main` | ✅ | ❌ | own session | ❌ | ❌ | ❌ |
| Publish to `stage/{uid}` (after invite) | ✅ | ❌ | ❌ | invited | invited | ❌ |
| Read live stream | ✅ | ✅ | own course | own course | enrolled | ❌ |
| Moderate (mute, kick, redact chat) | ✅ | ✅ | own session | own session | ❌ | ❌ |
| Post assignment | ✅ | ✅ | own course | ❌ | ❌ | ❌ |
| Submit assignment | ❌ | ❌ | ❌ | ❌ | own | ❌ |
| Grade submission | ✅ | ✅ | own course | own course | ❌ | ❌ |
| Read student-visible feedback | ✅ | ✅ | own course | own course | own | linked-child's |
| Read staff-private notes | ✅ | ✅ | own course | own course | ❌ | ❌ |
| View grade analytics + attendance | ✅ | ✅ | own course | own course | own (limited) | linked-child's |
| Watch recording | ✅ | ✅ | own course | own course | enrolled | tenant-policy gated, off by default |
| Access audit log | ✅ | tenant-scoped | ❌ | ❌ | ❌ | ❌ |

### Parent role specifics

The parent role is **strictly read-only**. A parent's RequestContext resolves to a `'parent'` tenant role; their permitted resource queries are filtered through `parent_links` to the set of `student_user_id`s they're linked to.

Parent surface = grade analytics, attendance summaries, performance trends, schedule of upcoming classes for linked children. Parents cannot:

- Submit assignments, post chat, raise hand, or join live sessions.
- Watch live (never).
- Watch recordings unless the tenant policy explicitly enables it (default off).
- See other students' submissions or grades, even within the same course.
- See staff-private notes ever.

Parent invitation flow: org admin or teacher generates an invite link tied to a specific student; parent signs up via Firebase Auth; first verified token creates the `parent_links` row + `tenant_memberships(role='parent')`.

### Database-level guardrails (RLS)

Every table with `tenant_id` has a row-level security policy of the form:

```sql
CREATE POLICY tenant_isolation ON courses
  USING (tenant_id = current_setting('app.tenant_id')::uuid);
```

`app.tenant_id` is set by the middleware. Connection pool is RLS-bypassed only for **platform admin** routes and **maintenance jobs** (recording pruner, billing reconciler), which use a separate Postgres role with `BYPASSRLS`.

### Token + session lifecycle

| Token | Issuer | TTL | Refresh path |
|---|---|---|---|
| Firebase ID token | Firebase | ~1 hr | Client SDK auto-refresh via refresh token |
| API session cookie (web, optional) | Backend | 8 hrs | HttpOnly + SameSite=Strict |
| Media auth (MVP) | Firebase ID token re-used | tied to ID token | Re-sent on each MediaMTX auth callback |
| Media JWT (v1+) | Backend | publish: 2–5 min, read: 15–30 min | Re-mint via `/v1/live/sessions/{id}/publish-token` |
| Stripe webhook signing key | Stripe | static | Rotated via Stripe dashboard; HMAC verified per webhook |

### Security posture

- App Check on the API for Firebase calls (P1; doesn't apply to web in the same way as native).
- MFA optional in MVP; **required for org_admin and platform_admin in v1** (Firebase TOTP via Identity Platform).
- All chat messages pass an emoji-tolerant sanitizer server-side; HTML stripped, links rewritten through a redirect endpoint.
- Recording consent banner shown to all participants on join when `live_sessions.recording_enabled = true`. Tenant policy decides between acknowledgment-required (blocks join) or notification-only.
- Audit log is append-only (`audit_events`); RLS makes it readable per-tenant; deletion is via expiry/retention only, never by user action.

---

## Section 6 — Frontend (Dioxus)

### Workspace layout

```
aulalite/
├── crates/
│   ├── core-types/         # shared API DTOs, errors, RBAC enums (also used by backend)
│   ├── api-client/         # HTTP + WS client, Firebase token holder, retry logic
│   ├── design-system/      # tokens, primitive components (Button, Input, Card, ...)
│   ├── features-auth/
│   ├── features-courses/
│   ├── features-live/      # live class room (publisher + viewer + chat + handraise)
│   ├── features-assignments/
│   ├── features-grading/
│   ├── features-parent/
│   ├── features-org-admin/
│   ├── features-platform-admin/
│   ├── features-billing/
│   ├── platform-bridge/    # trait + per-platform impls (camera, mic, file picker, push, deep links)
│   ├── shell-web/
│   ├── shell-mobile/
│   └── shell-desktop/
```

`features-*` crates depend only on `core-types`, `api-client`, `design-system`, and `platform-bridge`. They don't know what platform they're rendering on. Platform-specific code lives behind the `platform-bridge` trait.

### Design system

Tokens as Rust constants + CSS variables (so the same names work for inline `style` and stylesheets):

```
--color-bg, --color-surface, --color-text, --color-text-muted
--color-primary, --color-primary-hover
--color-success, --color-warning, --color-danger
--color-live (a distinct hue for "this is a live class right now")
--font-display, --font-body, --font-mono
--space-1..8, --radius-sm/md/lg, --shadow-sm/md/lg
--breakpoint-sm/md/lg
```

Component primitives (in `design-system`): `Button`, `IconButton`, `Input`, `Textarea`, `Select`, `Checkbox`, `Toggle`, `Card`, `Tabs`, `Modal`, `Toast`, `Toolbar`, `Avatar`, `Badge`, `EmptyState`, `Spinner`, `Skeleton`, `Table`, `Pagination`, `Drawer`, `Tooltip`, `Menu`, `DatePicker`, `TimePicker`, `FileDrop`. Built once with vanilla CSS; **Tailwind layered on top** for high-velocity composition in `features-*`.

**Visual tone:** calm, modern academic SaaS. Neutral grays + one primary accent + a distinct "live" hue. Clear typographic hierarchy. Comfortable density on learning surfaces; denser tables for admin/grading.

**Accessibility minimums:** WCAG AA contrast, visible focus rings, full keyboard navigation for admin/grading workflows, screen-reader labels on every interactive control.

### Routing

```
/                                  (marketing redirect → /app)
/login, /signup, /forgot
/app                               (authenticated shell)
  /app/dashboard
  /app/courses
  /app/courses/:slug
  /app/courses/:slug/lessons/:lessonId
  /app/courses/:slug/assignments/:id
  /app/sessions/:id/live
  /app/sessions/:id/recording
  /app/parent
  /app/admin
    /app/admin/users
    /app/admin/courses
    /app/admin/billing
    /app/admin/branding
    /app/admin/audit
  /app/platform                    (Elementors super admin, role-gated)
```

Role-gated routes redirect to `/app/dashboard` with a toast if accessed without permission.

### State

- `api-client` holds the Firebase token and refreshes on 401.
- Signals for local component state.
- A small global store (Dioxus context) for current user, tenant, role, feature flags, and active live-session presence map.
- WebSocket subscriptions are scoped to the live room: opened on `LiveRoom` mount, closed on unmount. No global WS connection.

### Live room layout

```
┌────────────────────────────────────────────────────────────┐
│ Course title  ·  Session title  ·  ● LIVE  ·  recording on │
├──────────────────────────────────┬─────────────────────────┤
│                                  │ Roster (counts)         │
│       MAIN STAGE (video)         ├─────────────────────────┤
│                                  │ Chat                    │
│  controls: cam, mic, share,      │   - timestamps          │
│  layout, leave (teacher only)    │   - reply / @mention    │
│                                  │   - moderation (delete) │
│                                  ├─────────────────────────┤
│                                  │ Hand raise queue        │
├──────────────────────────────────┴─────────────────────────┤
│  Bottom: connection quality · "low-bandwidth mode" toggle  │
└────────────────────────────────────────────────────────────┘
```

- Default view = teacher on stage; student tiles do not exist by default (one-way mode).
- Hand-raise queue is teacher-visible only; students see their own position.
- "Invite to stage" (teacher action) prompts the student; on accept, MediaMTX path permission for `stage/{uid}` is granted via API and the student's local UI flips to publisher mode.
- Visible recording badge + privacy notice on first join of the session.
- A single "low-bandwidth" toggle triggers HLS fallback for the current viewer without disturbing other viewers.
- Mobile layout collapses to: stage on top half, tabbed (chat / roster / hands) on bottom half.

### Platform bridge contract

```rust
pub trait PlatformBridge {
    async fn request_camera_mic(&self) -> Result<MediaPermissions>;
    fn supported_codecs(&self) -> CodecSet;
    async fn open_file_picker(&self, accept: &[&str]) -> Result<Vec<PickedFile>>;
    async fn save_to_downloads(&self, bytes: Bytes, name: &str) -> Result<()>;
    async fn register_for_push(&self) -> Result<PushToken>;
    fn handle_deep_link(&self, url: &str);
    fn open_external_url(&self, url: &str);
}
```

Implementations: `web` (browser APIs via wasm-bindgen), `ios` (native APIs via dioxus-mobile FFI), `android` (JNI via dioxus-mobile), `desktop` (host APIs via dioxus-desktop).

### Mobile-only specifics

- **Push:** FCM; APNs token forwarded through FCM on iOS. Notification taps deep-link to `aulalite://session/{id}` → mapped to in-app route.
- **Camera/mic publish (P1 build):** native WebRTC SDKs through FFI. If FFI work is too rough, fallback is WKWebView + JS bridge for publish only.
- **File upload:** native pickers; large files chunked through MinIO multipart pre-signed URLs (no whole-file load into memory).
- **Background:** audio-only continuation when app backgrounds during live class (iOS background audio mode + Android foreground service).

### PWA (web shell)

Service worker for: app-shell caching, offline-readable course materials, queued offline actions for assignment drafts. Push permission is a "tell me before next class" affordance, separate from FCM on mobile.

### Desktop shell

Dioxus desktop with system WebView. Same code path as web for ~95%; differences are file system access (write recordings/downloads to chosen folder), system tray for "class starts in 5 min" reminders, and persistent window state.

---

## Section 7 — Build Sequence ("A but C focused")

Full MVP feature set ships at launch; build order follows the vertical slice spine first, wrap-around second, polish third. Sized in milestones.

### Phase 0 — Foundations (one milestone)

- Cargo workspace, three shells building "hello world" on web + iOS + Android + desktop.
- Dokploy stack up: Traefik (Dokploy-managed), Postgres, Redis, MinIO, MediaMTX, API skeleton.
- Postgres migrations framework + RLS scaffolding + a single `tenants` + `users` + `tenant_memberships` slice.
- Firebase Auth wired end-to-end: client login → ID token → API verifies → JIT-provision row in `users`.
- Design system primitives + auth screens (login, signup, forgot password).
- Platform admin can manually create a tenant via a one-off CLI / script.

**Exit criterion:** a real Firebase user signs in on web and mobile, lands on an empty dashboard, and the API knows who they are and which tenant they're in.

### Phase 1 — P0 Core Spine (vertical slice)

In strict sequence so the loop is closeable end-to-end at each step:

1. Courses + modules + lessons CRUD
2. Enrollment (admin invite + self-enroll via code)
3. Live session scheduling (metadata only)
4. MediaMTX integration + HTTP auth callback (a teacher publishes, a student watches; no chat yet)
5. Live room UI: chat, hand-raise queue, recording badge, presence
6. Auto-recording → uploader → playback (tenant default + per-session override)
7. Recording playback page (web + mobile read)
8. Assignment CRUD
9. Submission flow (text + file upload via MinIO multipart pre-signed)
10. Grading flow (teacher grades; student sees grade + feedback)

**Exit criterion:** a teacher can run a full course end-to-end on web — schedule a class, teach it, recording is saved, students rewatch, an assignment is posted, students submit, teacher grades, students see grade. Mobile is consumer-only at this exit (watch + chat + assignments + grades).

### Phase 2 — P1 Wrap-around (parallel-friendly)

- Parent role (link flow, parent dashboard with grade analytics + attendance trend charts)
- Mobile native live publishing (FFI-heavy WebRTC binding work)
- TA moderation + stage handoff (mute/kick/redact, "invite to stage" flow)
- Files library
- Push notifications (FCM client + backend notification engine)
- Structured search (Postgres `pg_trgm`)
- Stripe billing (2 tiers, 7-day trial, soft caps, upgrade prompts; webhook → Postgres)
- Org admin surface (user mgmt, branding, policies, plan)
- Attendance reports
- Email transactional (Resend integration, templates)

**Exit criterion:** every MVP feature in Section 1's matrix is functional. Internal QA passes the full role matrix.

### Phase 3 — P2 Polish (last milestone before launch)

- Desktop shell ship-quality (or formally deferred to v1)
- HLS fallback path verified for the 50–100-concurrent case
- Recording auto-prune by tenant retention (default 90 days)
- Audit log surface for org admins
- `overage_behavior` setting wired (`'block'` only at MVP launch; UI shows `'metered'` as "available in v1")
- i18n scaffolding (English populated; locale switcher in place)
- Accessibility pass + browser/device matrix QA
- Load test: 100-concurrent class on a real Dokploy node; HLS-fallback flip tested

**Exit criterion:** ready for closed beta with 3–5 paying tutors / 1–2 paying institutions.

### Hard gates

1. **End of Phase 0** — RLS demonstrated to block cross-tenant reads. Firebase token revoked = API rejects on next request.
2. **End of Phase 1** — vertical-slice demo runs flawlessly for one teacher and three students live.
3. **End of Phase 2** — Stripe full subscription lifecycle tested in test mode (signup → trial → first invoice → upgrade → cancel). Mobile publish proven on at least one iOS and one Android device.
4. **End of Phase 3** — 100-viewer load test on production Dokploy node passes; HLS fallback flips correctly when triggered.

---

## Section 8 — Operations / DevOps on Dokploy

### Environments

| Env | Purpose | Domains | Notes |
|---|---|---|---|
| `dev` | Local dev | `localhost:3000` | Local Compose; Firebase test project |
| `staging` | Internal QA, demos | `staging.elementors.guru`, `media-staging.elementors.guru` | Same shape as prod, smaller node, Stripe test mode |
| `prod` | Real users | `app.elementors.guru`, `media.elementors.guru` | Stripe live, FCM live, Firebase prod |

### Compose stack (per environment)

```yaml
services:
  api:                # Rust/Axum, REST + WebSocket, ≥2 replicas in prod for rolling deploy
  recording-uploader: # sidecar, sees mediamtx volume + MinIO
  mediamtx:           # network_mode: host (WebRTC UDP/TCP); HLS+API HTTP behind Traefik
  postgres:           # pinned major; volume-backed; daily backup via Dokploy volume backup
  redis:              # AOF persistence
  minio:              # 4-disk single-node erasure coding minimum; bucket policies tenant-prefixed
  prometheus:         # scrapes api + mediamtx
  # Traefik comes from Dokploy, not declared here
```

### Secrets & config

- Dokploy environment variables for service config (no secrets in Compose YAML).
- Critical secrets managed in Dokploy: Firebase Admin service account JSON, Stripe secret + webhook signing secret, FCM server key, Resend API key, MediaMTX shared secret, MinIO root credentials, Postgres password.
- Application reads secrets from env at startup; rotations require restart.

### Database operations

- **Migrations** via `sqlx-cli` or `refinery`, forward-only, applied automatically on API boot (dev + staging + prod, given auto-deploy gating).
- **Backups:** daily Postgres volume backup via Dokploy (target: a Dokploy-supported destination — sftp / S3-bucket on a separate provider / attached object storage). 30-day retention.
- **Honest flag:** same-host backups don't protect against catastrophic node loss. **Off-host replication is added once revenue justifies it** (rough trigger: 10+ paying customers or first signed contract requiring it).
- **PITR:** out of MVP; revisit when first paying customer crosses ~50 active users.
- **Restore drill:** rehearsed once during Phase 3 before launch. Documented runbook.

### MinIO operations

- Bucket layout:
  - `assets/` — course materials, lesson attachments
  - `submissions/` — assignment uploads
  - `recordings/` — fMP4 segments
  - `backups/` — Dokploy volume backups
  - `exports/` — attendance reports, CSV exports
- Lifecycle rules per bucket: `recordings` honors per-tenant `recording_retention_days`; `submissions` and `assets` no expiry; `backups` 30 days; `exports` 7 days.

### MediaMTX operations

- Pinned image version; no auto-pulls.
- Recording dir is a named Docker volume with monitored disk space (Prometheus alert at 80%).
- Restart policy: `unless-stopped`. Graceful restart drops live sessions, so deploys for `mediamtx` are scheduled in low-usage windows and announced via "system status" in org-admin UI.
- Health probe: HTTP `GET /v3/paths/list` returns 200; Prometheus `/metrics` for richer signals.
- TURN: not in MVP. Coturn added as a sibling Compose service in v1 if customers report NAT issues. Stopgap = swap to a public TURN provider (documented in runbook).

### Observability

| Signal | Source |
|---|---|
| API request rate, latency, error rate | Axum `tower-http` metrics |
| MediaMTX `mediamtx_paths_inbound_frames_in_error` | `/metrics` |
| MediaMTX active publishers / readers | `/metrics` |
| Auth-deny rate at `/v1/mediamtx/auth` | Custom counter |
| Recording uploader lag | Custom gauge |
| Postgres connection pool saturation | sqlx metrics |
| Redis pub/sub message rate per session | Custom counter |
| MinIO bucket size + write/read rate | MinIO `/minio/v2/metrics/cluster` |
| Stripe webhook delivery success | Custom counter |

**Alerts (PagerDuty or email at MVP):**

- Backend 5xx > 1% for 5 min
- Auth deny rate > 10% over 5 min
- Recording uploader lag > 5 min
- MediaMTX has no publisher 5 min after a session's `starts_at`
- Disk usage on MediaMTX node > 80%
- Postgres connection saturation > 80%
- Stripe webhook failure
- Daily backup job failure

Logs: structured JSON to stdout; Dokploy collects and rotates. Loki + Grafana added in v1 if log volume justifies it.

### CI / CD

- **Repo:** monorepo (frontend + backend in one Cargo workspace; deployed as separate Docker images).
- **CI:** `cargo fmt --check` → `cargo clippy -D warnings` → `cargo test` (workspace) → build Docker images → push to registry.
- **CD:** **auto-deploy to staging *and* prod** on tagged image push, gated by:
  1. CI green
  2. Dokploy `/healthz` health-check gate (auto-rollback on failure)
  3. Forward-only Postgres migrations (never destructive)
- **Pre-launch:** Phase 3 includes a "deploy + restore" drill where staging is wiped and rebuilt from scratch using only the runbook.

### Disaster posture (honest, MVP-realistic)

- **Postgres lost** — restore from last daily backup; up to 24 hours of submissions/grades may be lost. Acceptable for closed beta. PITR added when stakes justify it.
- **MinIO lost** — course materials and submissions need restore from backups. Recordings older than retention are gone forever — by design.
- **MediaMTX lost mid-class** — clients reconnect after restart; the class may be cut short. Recordings of the cut portion are lost.
- **Firebase Auth outage** — existing logged-in users continue (cached ID tokens valid up to 1 hour); new logins blocked. Status page communicates.

---

## Section 9 — Testing Strategy

### 1. Unit / domain tests (Rust, in-crate)

- RBAC evaluator — full permission matrix from Section 5, fuzzed across (role, resource, action) tuples.
- Enrollment state machine — invited → active → suspended; idempotent self-enroll-via-code.
- Grading math — points, percentages, late-submission rules, regrade flows.
- Attendance reconciler — given hook events + heartbeats, computes correct `total_seconds` and `reconnect_count`.
- Recording-uploader job state machine — partial uploads, retries, missing segments.
- Stripe webhook idempotency — same `event.id` handled twice produces no duplicate effect.

Run on every commit; complete in under 30 seconds.

### 2. Integration tests (Rust, against real Postgres + Redis + MinIO via testcontainers)

Each test gets its own transaction that rolls back at the end.

- **RLS verification** — for every table with `tenant_id`, a test inserts as tenant A and confirms tenant B cannot see, update, or delete.
- **Cross-tenant attack tests** — try to access another tenant's resource by guessed UUID, by referer manipulation, by direct API path; all must 404 or 403.
- MediaMTX auth callback — happy path (allow), wrong tenant (deny), expired token (deny), wrong path-format (deny), valid student trying to publish to `main` (deny).
- MediaMTX hook ingestion — record-complete, read-start/stop, ready/not-ready all produce correct DB state.
- Submission flow — draft → submit → grade → student-visible feedback round-trip.
- Recording lifecycle — segment arrives → metadata persisted → playback URL minted → retention prune deletes both row and MinIO object.
- Stripe webhook → subscription state changes — trial start, trial end, payment success, payment failed, plan change, cancellation.

### 3. Security tests

- App Check verification — backend rejects unsigned requests once App Check is enforced (P1).
- Revoked-token handling — Firebase `revokeRefreshTokens` → next API call returns 401 within one ID-token cycle.
- Rate limits — `/v1/live/sessions/{id}/join` enforces per-user and per-tenant ceilings.
- Chat sanitizer — XSS payloads stripped; link rewriting works; allowed unicode/emoji preserved.
- Pre-signed URL TTL — MinIO playback URLs expire and refuse after expiry.
- CSP and CORS — set to deny by default, allowlisted to `app.elementors.guru` and known asset origins.
- Dependency audit — `cargo audit` in CI; advisories fail the build.

### 4. E2E / browser & device matrix

- Tooling: Playwright (against the WASM web app); mobile via Maestro or device-farm runs.
- Critical E2E flows: login → join scheduled live class → watch + chat → recording appears → rewatch; teacher schedule → publish → end → recording downloadable; assignment post → submit → grade → student sees grade; parent invite → see linked child's grades + attendance trend chart; org admin invite teacher and student → assign teacher to course → see audit log entries.
- Browser matrix: latest Chrome, Firefox, Safari on macOS + Windows; Chrome on Android; Safari on iOS.
- Mobile native: 1 iOS device (iPhone 13+) and 1 Android device (Pixel 6+ or equivalent) per release.
- Codec smoke: H.264 baseline + Opus on all browsers.

### 5. Load / interop tests (Phase 3, pre-launch)

- 100-concurrent viewer load test on production-equivalent Dokploy node. Verify WebRTC bitrate stable, MediaMTX CPU < 70%, API + DB stay healthy.
- HLS fallback flip — manually trigger mid-class; verify flipped viewers continue with at most one buffering event.
- Reconnection ladder — kill UDP, verify TCP fallback; kill all WebRTC, verify HLS fallback; kill Firebase momentarily, verify token refresh resumes class.
- Recording pipeline under load — verify segments upload to MinIO faster than they're produced (uploader lag < 30s steady state).
- OBS WHIP publish smoke — instructor publishes from OBS → MediaMTX → students see it on web.

### CI gates

| Gate | Scope | Where |
|---|---|---|
| `cargo fmt --check` | All crates | PR |
| `cargo clippy -D warnings` | All crates | PR |
| Unit tests | All crates | PR |
| Integration tests | API + db / redis / minio | PR |
| Security tests | API + auth | PR |
| Build Docker images | api, recording-uploader | PR |
| E2E (Playwright) | Web app against staging | nightly + pre-tag |
| Mobile device farm | iOS + Android | pre-tag |
| Load test | 100-viewer | manual, Phase 3 + before any major release |

### Out of MVP testing scope (deliberate)

- Internationalization / RTL layouts — English only at launch.
- Automated a11y audits beyond `axe-core` smoke; manual screen-reader pass for auth + live room only.
- Chaos / fault-injection tests — added in v1.
- Penetration test — recommended; budget for v1 launch.

---

## Open Questions / Risks / Next Steps

### Open questions to resolve before implementation planning

1. **Team size and timeline.** Phase sizing in Section 7 is in milestones, not weeks. Concrete duration depends on team headcount (1 senior generalist? 3-person team? 5-person team?) and weekly capacity.
2. **Stripe price IDs and tier shapes.** Section 1 says "2 tiers" but exact monthly prices, included quotas, and Starter-vs-Pro feature differences need a pricing sheet.
3. **Email provider choice.** Resend vs Postmark — pick one before Phase 2.
4. **Dokploy backup target.** Final decision on which Dokploy-supported destination receives daily backups (sftp endpoint? attached object storage? small VPS?).
5. **Identity Platform upgrade.** Required for TOTP MFA in v1; affects Firebase pricing. Decide before Phase 2 if MFA is needed at MVP launch instead of v1.

### Top three implementation risks

1. **Dioxus mobile native WebRTC publish** — biggest unknown in the stack. Mitigated by Phase-2 placement, generous schedule buffer, and a "WKWebView publish bridge" fallback documented in advance.
2. **Self-hosted MediaMTX scaling at 100 concurrent** — single Dokploy node has finite NIC + CPU. Mitigated by HLS-fallback toggle and a documented upgrade path to a beefier media node + a separate origin.
3. **Backup-on-same-host risk** — node loss = data loss until off-host replication ships. Mitigated by frequent restore drills and a clear trigger (10 paying customers) for moving to off-host.

### Next step

Move to **implementation planning** via the `superpowers:writing-plans` skill. The plan will translate Sections 7 (build sequence) and the per-section detail above into a concrete, ordered, step-by-step implementation plan with verifiable exit criteria per step.

---

*End of design document.*
