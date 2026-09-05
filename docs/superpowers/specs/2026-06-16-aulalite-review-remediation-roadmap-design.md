# AulaLite Review Remediation Roadmap Design

## Context

The review found production blockers, security hardening gaps, frontend/mobile integration drift, and several roadmap-sized LMS gaps. Local verification confirmed the highest-priority blockers:

- `crates/shell-web/index.html` still references tracked public assets that are currently deleted from the working tree.
- `crates/backend/src/auth/jit_provision.rs` contains `#[cfg(test)]` tests that connect to Postgres, which conflicts with the DB-free `cargo test --workspace --lib` CI job.
- `crates/backend/src/auth/middleware.rs` can reject `mfa_required` before `POST /v1/auth/mfa/challenge` can run when `AULALITE_MFA_ENFORCE` is enabled.
- Course `syllabus_md` and `grading_policy_md` are patched into storage without the existing backend markdown sanitizer used by other markdown surfaces.
- The workspace already pins `dioxus-kinetics` and exposes curated primitives through `design_system::kinetics_ui`.

This design covers the whole review as a phased program. Each phase must leave the product in a shippable state and should not depend on unfinished later phases.

## Goals

- Fix current production-breaking and lockout risks first.
- Complete security-critical auth, MFA, markdown, CORS, CSP, and mobile token-storage work.
- Bring web, mobile, and native shells back into provider and auth-flow parity.
- Build the missing admin and operational surfaces needed for a production LMS.
- Expand LMS depth with SCORM import, question banks, randomized assessments, and live-class hardening.
- Consolidate UI around the existing design system and `dioxus-kinetics` primitives.
- Make CI and tests accurately reflect DB-free unit work versus DB-backed integration work.

## Non-Goals

- Replacing the whole frontend shell in one pass.
- Rewriting the backend router or persistence layer wholesale.
- Introducing a second UI framework beside the existing design system and `dioxus-kinetics` bridge.
- Making fmt/clippy hard gates before the current baseline is cleaned and verified.
- Shipping feature UIs without backend state, tests, and error handling.

## Recommended Approach

Use a phased stabilization-to-product-complete roadmap:

1. Fix production blockers and security regressions.
2. Complete auth and mobile parity.
3. Add provider-backed product capabilities.
4. Add deeper LMS workflows.
5. Harden operations, architecture, and CI.

This approach is preferred over feature-first work because it removes active breakage before adding more surface area. It is preferred over platform-first refactoring because students and staff still need restored assets, safe markdown, and working MFA immediately.

## Phase 1: Production Unblockers

### Scope

- Restore or replace deleted web public assets referenced by `crates/shell-web/index.html`:
  - `/assets/blur-bridge.js`
  - `/assets/whiteboard-bridge.js`
  - `/assets/scorm-bridge.js`
  - `/service-worker.js`
  - MediaPipe WASM helper JS files under `/vendor/mediapipe/wasm/`
- Make backend library tests DB-free by moving JIT provisioning DB tests out of `#[cfg(test)]` library scope or gating them behind an explicit integration-test path.
- Fix MFA enforcement so the challenge route can be reached with a primary token when MFA is required.
- Sanitize course syllabus and grading-policy markdown before persistence using the existing `services::sanitize::clean_markdown` path.

### Design

Restore tracked assets from Git unless inspection shows they were intentionally replaced by new paths. If new paths exist, update `index.html` and all bridge call sites together. The default fix is restore, because the references are still active and the assets are tracked.

Move JIT provisioning DB tests to integration tests, or gate them with a helper that skips unless a real `DATABASE_URL` is intentionally present. DB-free CI must not attempt to connect to the fallback `localhost:55432` URL from library tests.

Add an explicit MFA middleware exemption for `POST /v1/auth/mfa/challenge` after primary token verification and user provisioning, but before the MFA gate. Keep all other authed routes gated. The challenge handler continues to verify TOTP or recovery code and returns the step-up token.

Normalize optional markdown patch semantics:

- Missing field means unchanged.
- `null` means clear stored field.
- Non-empty string means sanitize and store.
- Whitespace-only string should store `NULL` or `None`, matching the current staff editor behavior.

### Exit Criteria

- `cargo test -p backend --lib` no longer attempts Postgres.
- `cargo test --workspace --lib` is DB-free.
- MFA-enabled users can call the challenge route when enforcement is on.
- Unsafe HTML/script-like syllabus and grading-policy payloads do not survive storage.
- `crates/shell-web/index.html` references files that exist.

## Phase 2: Complete MFA UX And Auth Step-Up

### Scope

- Login challenge screen after primary sign-in when backend returns `mfa_required`.
- Recovery-code entry path.
- Recovery-code display and copy/download after enrollment verification.
- Admin reset flow for locked-out users.
- Optional remember-device policy if backend support is added.

### Design

Frontend login should treat `mfa_required` as an intermediate auth state, not a final failure. The login component should keep the primary token in memory, render an MFA challenge panel, submit the code to `/v1/auth/mfa/challenge`, and swap `ApiContext.id_token` to the returned step-up token on success.

The security settings page should keep the existing enrollment flow but improve recovery-code handling with clear one-time display, copy/download actions, and explicit confirmation before leaving the screen. Admin reset belongs in tenant admin user management, not in the user's self-service panel.

Backend errors must be stable enough for UI branching:

- `mfa_required`
- `invalid_code`
- `recovery_code_used` only if the backend can distinguish it safely
- `mfa_not_enabled`
- `sso_session_secret_unset`

### Exit Criteria

- Sign-in with MFA enabled reaches dashboard after valid TOTP or recovery code.
- Invalid challenge codes stay on the challenge screen with inline error.
- Recovery-code enrollment display is SSR/component tested.
- Admin reset is permission-checked and audited.

## Phase 3: Mobile And Native Auth Parity

### Scope

- Shared auth bootstrap gate for web, mobile, and desktop/native shells.
- Provider parity for `ApiContext`, `UserContextSignal`, toast, locale, theme, density, and route guards.
- Proper signout on mobile/native.
- Secure token storage for native/mobile targets.

### Design

Extract auth bootstrapping into a shared shell-level module or crate function that each shell calls with target-specific bridge implementations. The shared logic should own:

- token bootstrap
- `/v1/me` fetch
- 401 refresh behavior
- signout clearing
- auth-ready gating
- user context population

Native token storage should move behind a `TokenStore` abstraction. The first implementation can keep the current JSON store for tests and local fallback, but production desktop/mobile builds should use OS credential storage:

- Windows Credential Manager
- macOS Keychain
- Linux Secret Service when available

The API for the token store should support load, save, clear, and refresh-update without exposing storage details to route components.

### Exit Criteria

- Mobile shell no longer renders routes before auth bootstrap completes.
- Shared components do not panic from missing providers.
- Mobile/native signout clears persisted credentials.
- Production native/mobile token storage is not plaintext JSON.

## Phase 4: Push Notifications

### Scope

- Real FCM service-account OAuth token minting.
- VAPID configuration validation.
- Device/browser registration and revocation.
- User notification preference UI.
- Delivery status logging.

### Design

Backend FCM logic belongs in a provider service module with pure config validation and token-minting boundaries. Handlers should expose:

- register device/browser token
- list own devices
- revoke device
- update notification preferences
- admin delivery diagnostics where permitted

Frontend UI should use compact operational surfaces:

- registration status row
- device list table
- preference switches
- delivery diagnostic sheet for admins

Use `design_system::kinetics_ui` controls where available: `DataTable`, `Switch`, `Badge`, `Sheet`, `Alert`, `EmptyState`, and `Toast`.

### Exit Criteria

- Push can be disabled cleanly when VAPID or FCM credentials are absent.
- Configured environments can mint provider access tokens and send test notifications.
- Users can manage devices and preferences.
- Delivery attempts and failures are queryable.

## Phase 5: Integration Admin Screens

### Scope

- API key management.
- Webhook endpoints and delivery logs.
- SSO provider configuration.
- LTI platform configuration.
- Shared integration admin navigation.

### Design

Build a grouped integrations area under admin navigation to avoid crowding the main sidebar. Screens should share a consistent pattern:

- top summary metrics
- searchable/filterable table
- create/edit sheet
- detail drawer for logs or diagnostics
- destructive actions behind confirmation dialogs

Use existing backend APIs first. If a backend surface is missing audit fields, health status, or test-send support, add those in the same phase as the UI.

LTI state storage should move from process-local memory to a shared store suitable for multiple replicas, preferably Redis if already available in deployment.

### Exit Criteria

- Admins can create, rotate, revoke, and inspect API keys without manual API calls.
- Admins can configure webhooks and inspect delivery failures.
- Admins can configure SSO and LTI without code or direct DB access.
- LTI login state works across replicas.

## Phase 6: SCORM Import And Authoring UX

### Scope

- SCORM ZIP upload.
- Manifest parse and validation.
- Asset extraction and storage.
- Package versioning or replacement.
- Launch diagnostics.

### Design

Implement SCORM import as a backend lifecycle:

1. Upload ZIP.
2. Validate size, archive structure, and entry paths.
3. Parse `imsmanifest.xml`.
4. Extract allowed assets to object storage.
5. Persist package, SCO, manifest, and launch metadata.
6. Surface validation errors in a structured response.

The frontend importer should show:

- upload drop zone or picker
- validation result table
- package asset list
- launch target summary
- diagnostic panel for missing launch files or unsupported manifest structures

Security requirements:

- reject path traversal
- reject absolute paths
- reject oversized files and entries
- reject unsupported executable/server-side file types if they are not needed
- require safe launch paths inside extracted package root

### Exit Criteria

- Staff can import a valid SCORM package without manually creating manifest/assets.
- Invalid packages show actionable validation results.
- Existing SCORM player can launch imported packages.
- ZIP safety tests cover traversal and oversize cases.

## Phase 7: Question Banks And Randomized Assessments

### Scope

- Question bank CRUD.
- Reusable question pools.
- Randomized quiz sections.
- Per-student generated quiz variants.
- Attempt persistence with variant identity.

### Design

Add question banks as first-class course or tenant resources, depending on existing permission needs. The quiz editor should support fixed questions and randomized sections. Randomization must be deterministic per attempt once generated so grading, review, and audit remain stable.

Backend services should own:

- pool selection
- random seed handling
- variant generation
- validation that quizzes can be generated from available questions

Frontend should use `QuestionCard`, `Tabs`, `DataTable`, `SegmentedControl`, dialogs/sheets, and compact settings panels. The editor should prioritize scanability over decorative layout.

### Exit Criteria

- Teachers can build banks and pools.
- Quizzes can mix fixed and randomized sections.
- Student attempts receive stable variants.
- Grading and review use the exact generated variant.

## Phase 8: Live-Class Production Hardening

### Scope

- Visible stream and room health indicators.
- Improved prejoin, empty, and preview states.
- HLS/WebRTC fallback visibility.
- Recording and ingest failure surfacing.
- Load-test hooks and operational diagnostics.

### Design

Avoid turning the live room into a decorative dashboard. Add compact status surfaces near existing controls:

- prejoin checks for camera, mic, network, and permissions
- in-room health strip for transport, latency, dropped frames, and recording state
- admin/teacher diagnostic sheet for media routing and participant connection states
- clear empty states for no participants, no stream, failed recording, or waiting for host

Backend should persist enough live-session health and recording state for both the room UI and operational dashboards.

### Exit Criteria

- Teachers can see whether live media and recording are healthy.
- Students get clear fallback or waiting states instead of blank room areas.
- Recording failures are visible and queryable.
- Health indicators have tests for empty/error/loading states.

## Phase 9: Operational Dashboards

### Scope

- Queue/job health.
- Webhook delivery failures.
- Recording failures.
- Email and push delivery status.
- Retry or remediation actions where safe.

### Design

Operational dashboards should be dense, table-first, and filterable:

- metric row for current failure counts and recent throughput
- tabbed sections by subsystem
- failure tables with severity, tenant, entity, timestamp, and status
- detail sheets for payload excerpts, provider responses, and retry history
- guarded retry actions for idempotent jobs

Use `MetricCard`, charts, `DataTable`, `Tabs`, `Badge`, `Sheet`, and `Dialog` from `design_system::kinetics_ui`.

### Exit Criteria

- Operators can see current platform health without reading logs.
- Failure rows link back to tenant/course/session context where permitted.
- Retry actions are audited and permission-checked.
- Dashboards degrade gracefully when optional providers are unconfigured.

## Phase 10: Architecture And CI Finish

### Scope

- Central authenticated layout or route guard.
- Split very large route/handler files along subfeature boundaries.
- CSS organization cleanup.
- Admin navigation grouping/collapsing.
- Hard CI gates for fmt and clippy after baseline cleanup.

### Design

Refactor only where it reduces drift in touched areas. The first shared boundary should be an authenticated layout/guard used by web and, where possible, mobile. It should own:

- auth-ready gating
- redirect behavior
- shared shell chrome
- common loading/error layout

Large files should be split by cohesive subfeature:

- live room: shell, media, chat, polls, whiteboard, health, recording
- course detail: tabs, syllabus, members, sessions, materials
- backend live-session handlers: room join, media callbacks, chat, whiteboard, recording, health
- API DTO helpers by domain where compile times and clarity benefit

CSS should move toward domain/component files while preserving global token and kinetics bridges.

CI hardening should happen only after the tree is clean under the pinned Rust toolchain:

- remove `continue-on-error` from fmt
- remove `continue-on-error` from clippy
- keep DB-free and DB-backed test jobs separate
- document how to run integration tests locally

### Exit Criteria

- Repeated route auth/layout code is materially reduced.
- Admin navigation can grow without crowding.
- Large files are split without changing behavior.
- fmt and clippy fail CI once verified clean.

## UI Quality Bar

All new or revised UI must follow these constraints:

- Prefer `design_system::kinetics_ui` primitives and existing design-system wrappers over one-off controls.
- Use tables, tabs, sheets, dialogs, metric cards, switches, segmented controls, sliders, and icon buttons for their standard roles.
- Keep operational screens compact, calm, and scannable.
- Do not build landing-page or marketing layouts for admin/product workflows.
- Do not nest cards inside cards.
- Use cards only for repeated items, modals, or genuinely framed tools.
- Avoid decorative gradient/orb backgrounds.
- Make text fit on mobile and desktop; table overflow must be intentional and usable.
- Major UI phases require SSR/component tests and visual checks at desktop and mobile widths.
- Navigation additions must be grouped or collapsible before the sidebar becomes crowded.

## Data Flow

Frontend route data flow:

1. Route enters through an authenticated shell or an explicit public route.
2. Shared API helpers own DTOs and request functions.
3. Screen components own resource loading and mutation state.
4. Reusable UI components render tables, forms, metrics, empty states, and overlays.
5. Recoverable errors use inline messages and toasts.
6. Missing permissions or failed initial loads use full-page states.

Backend capability data flow:

1. Migration defines schema and indexes.
2. `db::*` module owns SQL and transaction boundaries.
3. `services::*` owns provider-specific or pure domain logic.
4. `handlers::*` validates input, checks permissions, calls services/db, and emits stable errors.
5. Background jobs persist enough state for operational dashboards.
6. Audit events are emitted for sensitive admin and security actions.

## Error Handling

- Use stable machine-readable errors where UI branching is required.
- Distinguish misconfiguration, invalid input, permission failure, and transient provider failure.
- Keep raw debug/provider payloads out of main page layouts; show controlled details in admin sheets.
- Persist failure state for background jobs that operators must act on.
- Keep optional-provider-off states graceful and explicit.

## Security Requirements

- Markdown stored from staff-editable fields must be sanitized before persistence.
- MFA enforcement must not block the MFA challenge route.
- Recovery codes are shown once, stored only as hashes, and consumed atomically.
- Production CORS should fail closed when configured origins are invalid.
- Production frontend should add a CSP compatible with Dioxus and required static assets.
- Native/mobile production token storage must use OS credential storage rather than plaintext JSON.
- LTI login state must not be process-local in multi-replica deployments.
- SCORM ZIP import must reject traversal, absolute paths, oversize entries, and unsafe launch paths.

## Testing Strategy

Unit tests:

- sanitizer behavior
- TOTP and MFA token/session helpers
- FCM token minting and provider config validation
- SCORM manifest parsing and archive safety
- question randomization and variant generation
- pure UI helper rendering where feasible

Backend integration tests:

- MFA challenge under enforcement
- course syllabus/grading markdown persistence
- admin integration CRUD
- webhook delivery logs
- SCORM package lifecycle
- question-bank quiz generation
- operational dashboard queries

Frontend tests:

- login challenge flow
- security settings recovery-code flow
- admin integration tables and sheets
- SCORM importer empty/error/success states
- question-bank editor and randomized section controls
- live health indicators
- operational dashboard filters and detail sheets

Visual checks:

- desktop and mobile widths for every major new admin or learning workflow
- table overflow and sticky/action columns
- modal and sheet sizing
- text fit in buttons, badges, cards, and table cells
- sidebar grouping and collapsed admin navigation

CI:

- keep `cargo test --workspace --lib` DB-free
- run DB integration tests only in DB-backed jobs
- keep wasm/mobile/native checks explicit
- make fmt and clippy hard gates only after baseline cleanup is complete

## Implementation Sequencing

The implementation plan should create one checklist per phase and stop at verification checkpoints. Phase 1 must be implemented before any roadmap feature work. Phases 2 and 3 should follow before provider-heavy features because MFA and auth parity affect all later admin screens. Phases 4 through 9 can be implemented in separate branches if needed, but each must include backend, frontend, tests, and visual verification. Phase 10 can run incrementally alongside later phases only when refactors are directly adjacent to touched code.

## Risks And Mitigations

- Risk: The whole review becomes too large to finish safely in one branch.
  - Mitigation: Treat each phase as a shippable checkpoint with its own tests.
- Risk: UI additions become inconsistent.
  - Mitigation: Use `design_system::kinetics_ui` primitives and add visual acceptance checks.
- Risk: Provider features are built without real credentials in local/dev.
  - Mitigation: Add explicit unconfigured states and pure config-validation tests.
- Risk: CI hardening blocks work before warning/fmt baseline is clean.
  - Mitigation: clean baseline first, then remove `continue-on-error`.
- Risk: Large-file refactors introduce behavior changes.
  - Mitigation: split by cohesive subfeature and keep behavior-preserving tests before refactors.

## Acceptance Summary

The review is fully addressed when:

- Current production blockers are fixed and verified.
- MFA can be enforced without lockout.
- Markdown storage paths are sanitized consistently.
- Mobile/native auth and token storage are production-ready.
- Push, integrations, SCORM import, question banks, live health, and operational dashboards are usable through professional admin/student/staff UI.
- The UI uses the pinned `dioxus-kinetics` bridge consistently where it fits.
- CI accurately separates DB-free and DB-backed checks and eventually hard-fails fmt/clippy.
