# AulaLite Phase 5 Integration Admin Screens Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give tenant admins web screens for integration setup and move LTI login state out of process-local memory.

**Architecture:** Add typed frontend API contracts for the existing integration admin endpoints, build one grouped `/admin/integrations` route in the web shell, and replace in-memory LTI state with a Postgres-backed single-use state table modeled on SSO login state.

**Tech Stack:** Rust 1.94 workspace, Axum, SQLx/PostgreSQL with RLS, Dioxus 0.7, existing `features-courses` API helpers, existing `design-system` primitives, and existing `shell-web` route patterns.

---

## File Structure

- Create: `migrations/20260618000066_lti_login_states.sql`
- Modify: `crates/backend/src/lib.rs`
- Modify: `crates/backend/src/main.rs`
- Modify: `crates/backend/src/services/lti.rs`
- Modify: `crates/backend/src/db/lti.rs`
- Modify: `crates/backend/src/handlers/lti.rs`
- Modify: `crates/features-courses/src/api.rs`
- Create: `crates/shell-web/src/routes/admin_integrations.rs`
- Modify: `crates/shell-web/src/routes/mod.rs`
- Modify: `crates/shell-web/src/route_enum.rs`
- Modify: `crates/features-courses/src/app_shell.rs`
- Modify: `crates/design-system/assets/components.css`
- Modify: `crates/shell-web/public/assets/components.css`
- Modify: `crates/shell-web/tests/shell_routes_smoke.rs`
- Review: `docs/superpowers/specs/2026-06-18-aulalite-phase-5-integration-admin-screens-design.md`

## Task 1: Add Frontend API Contracts

**Files:**
- Modify: `crates/features-courses/src/api.rs`

- [ ] Add DTOs and request bodies for API keys, webhooks, SSO config, and LTI platforms.
- [ ] Add helper functions for list/create/update/delete flows.
- [ ] Add serde tests for one-time key/secret responses and optional SSO client-secret behavior.
- [ ] Run `cargo test -p features-courses --lib api::tests -- --nocapture`.
- [ ] Commit with message `feat(integrations): add admin API client contracts`.

## Task 2: Add Integration Admin Route Shell

**Files:**
- Create: `crates/shell-web/src/routes/admin_integrations.rs`
- Modify: `crates/shell-web/src/routes/mod.rs`
- Modify: `crates/shell-web/src/route_enum.rs`
- Modify: `crates/features-courses/src/app_shell.rs`
- Modify: `crates/shell-web/tests/shell_routes_smoke.rs`

- [ ] Add `/admin/integrations` route and export.
- [ ] Add admin nav link.
- [ ] Add admin-gated route shell with grouped sections for API keys, webhooks, SSO, and LTI.
- [ ] Add focused SSR tests and route smoke coverage.
- [ ] Commit with message `feat(integrations): add admin integrations route`.

## Task 3: Implement API Key Management UI

**Files:**
- Modify: `crates/shell-web/src/routes/admin_integrations.rs`

- [ ] Render API key summary and table.
- [ ] Add create form with name and scope controls.
- [ ] Show plaintext API key once after minting.
- [ ] Add revoke action with confirmation.
- [ ] Add SSR tests for table and one-time key rendering helpers.
- [ ] Commit with message `feat(integrations): manage admin API keys`.

## Task 4: Implement Webhook Management UI

**Files:**
- Modify: `crates/shell-web/src/routes/admin_integrations.rs`

- [ ] Render webhook subscription summary and table.
- [ ] Add create/edit form for URL, events, and active state.
- [ ] Show signing secret once after create.
- [ ] Add delete action with confirmation.
- [ ] Render recent delivery logs and a detail view for payload/response data.
- [ ] Commit with message `feat(integrations): manage webhooks from admin`.

## Task 5: Implement SSO And LTI Management UI

**Files:**
- Modify: `crates/shell-web/src/routes/admin_integrations.rs`

- [ ] Render SSO config form with secret-preserving updates.
- [ ] Show enabled state and generated login URL.
- [ ] Render LTI platform list and registration form.
- [ ] Add LTI platform delete action with confirmation.
- [ ] Commit with message `feat(integrations): manage sso and lti from admin`.

## Task 6: Move LTI State To Shared Storage

**Files:**
- Create: `migrations/20260618000066_lti_login_states.sql`
- Modify: `crates/backend/src/lib.rs`
- Modify: `crates/backend/src/main.rs`
- Modify: `crates/backend/src/services/lti.rs`
- Modify: `crates/backend/src/db/lti.rs`
- Modify: `crates/backend/src/handlers/lti.rs`

- [ ] Add `lti_login_states` migration with system-context RLS policies.
- [ ] Add DB helpers to insert and atomically consume TTL-bounded login state.
- [ ] Replace `AppState.lti_state` handler usage with DB helpers.
- [ ] Remove the process-local `LtiStateStore` from application state.
- [ ] Adjust or add tests for single-use and expiry behavior.
- [ ] Commit with message `feat(lti): persist launch state in shared storage`.

## Task 7: Polish And Verify

**Files:**
- Modify CSS only if the new route needs missing compact admin styles.

- [ ] Run `cargo fmt --all --check`.
- [ ] Run `cargo test -p backend --lib services::lti -- --nocapture`.
- [ ] Run relevant backend DB tests if changed/available.
- [ ] Run `cargo test -p features-courses --lib -- --nocapture`.
- [ ] Run `cargo test -p shell-web --lib -- --nocapture`.
- [ ] Run `cargo test -p shell-web --test shell_routes_smoke -- --nocapture`.
- [ ] Run `cargo check -p backend --all-targets`.
- [ ] Run `cargo check -p shell-web --all-targets`.
- [ ] Run `cargo check -p platform-bridge --target wasm32-unknown-unknown`.
- [ ] Run `cargo check -p shell-web --target wasm32-unknown-unknown`.
- [ ] Run `git diff --check`.
- [ ] Merge to `main`, push `origin/main`, remove the worktree, and delete the local phase branch.

