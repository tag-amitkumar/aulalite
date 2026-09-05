# AulaLite Phase 5 Integration Admin Screens Design

## Context

Phase 4 made notification delivery observable from the admin console. The next
roadmap item is Phase 5: give tenant admins first-class screens for external
integration setup instead of requiring direct API calls or database changes.

The backend already has most of the integration management surface:

- `/v1/admin/api-keys` for listing, minting, and revoking tenant API keys.
- `/v1/admin/webhooks` and `/v1/admin/webhooks/deliveries` for outbound webhook
  subscriptions and delivery inspection.
- `/v1/admin/sso` for tenant OIDC SSO configuration.
- `/v1/admin/lti/platforms` for LTI 1.3 platform registration.
- `/v1/lti/login` and `/v1/lti/launch` for the unauthenticated LTI launch flow.

The main gaps are in the web shell. `features-courses` has no API client
contracts for these integration endpoints, and `shell-web` has no grouped
admin area for the screens. The roadmap also calls out that LTI login state
must not remain process-local in multi-replica deployments.

## Goals

- Add a grouped admin integrations area under the existing admin navigation.
- Let org admins and platform admins manage tenant API keys from the web UI.
- Let admins create, inspect, update, and remove outbound webhook subscriptions.
- Let admins inspect recent webhook delivery failures from the web UI.
- Let admins configure tenant SSO without direct database changes.
- Let admins register and delete LTI platforms without direct database changes.
- Move LTI login state to shared storage suitable for multiple backend replicas.
- Preserve existing MFA and auth-step-up behavior.

## Non-Goals

- Adding new public API resources or write scopes.
- Adding webhook replay/test-send controls unless the backend already exposes
  them during this phase.
- Implementing SAML SSO.
- Implementing LTI Advantage services beyond launch.
- Building a platform-wide operations dashboard; that remains Phase 9.

## Approved Approach

Build a single `/admin/integrations` route with sections for API keys,
webhooks, SSO, and LTI. Use the existing backend APIs first. If a backend
contract is missing only to satisfy the roadmap exit criteria, add the smallest
server-side change in the same phase.

The route should use the existing `AppShell`, admin-gating pattern, Dioxus
signals/resources, and design-system primitives. It should favor dense admin
tables, inline summary metrics, sheets/forms for create/edit actions, and
confirmation prompts for destructive actions.

For LTI state, prefer the established Postgres pattern already used by SSO
login state. Redis is available in the backend for live rooms and rate limiting,
but production boot does not make Redis a hard dependency for all features. A
Postgres-backed LTI state table gives shared, single-use, TTL-bounded state
without introducing a new always-on Redis requirement for login correctness.

## Backend Design

### LTI Login State

Add a migration for `lti_login_states`:

- `state text primary key`
- `nonce text not null`
- `target_link_uri text`
- `created_at timestamptz not null default now()`

Enable RLS and system-context select/insert/delete policies, matching
`sso_login_states`. The unauthenticated login and launch handlers can then
persist and consume state by using system context through `db::lti`.

Replace `AppState.lti_state` and `services::lti::LtiStateStore` usage in the
handler with:

- `db::lti::insert_login_state(pool, state, nonce, target_link_uri)`
- `db::lti::take_login_state(pool, state)`

The consume operation must be single-use and TTL-bound. Use a 15 minute TTL,
consistent with SSO state.

Keep pure unit tests for `services::lti` claim projection and move state-store
tests to DB helper tests where practical.

### Existing Admin APIs

Reuse current contracts:

- API keys return plaintext only on mint.
- Webhook signing secret returns only on create.
- SSO never returns the client secret, only `has_client_secret`.
- LTI platform secrets are not present in the current backend contract.

Do not expose raw signing secrets, client secrets, or API key material after the
one-time creation response.

## Frontend Design

### Client Contracts

Add typed DTOs and helpers in `crates/features-courses/src/api.rs` for:

- API keys: list, mint, revoke.
- Webhooks: list, create, update, delete, list deliveries.
- SSO: get, upsert.
- LTI platforms: list, register, delete.

Add serialization/deserialization tests for payloads that have one-time secret
fields or optional secret-preserving behavior.

### Admin Route

Create `crates/shell-web/src/routes/admin_integrations.rs` and add:

- `/admin/integrations` route in `route_enum`.
- Export in `routes/mod.rs`.
- Sidebar nav entry under Admin.
- Command palette entry if admin links are centralized there.

The first version can keep all four integration sections on one page with tabs
or segmented navigation. Each section should include:

- Summary counters.
- Search/filter where useful.
- Table/list of current resources.
- Create or edit affordance.
- Confirmation for destructive actions.
- One-time secret display after create.

### Section Details

API keys:

- List key name, prefix, scopes, created time, last-used time, and revoked
  state.
- Mint a key with name and scopes.
- Display plaintext once after mint.
- Revoke active keys behind confirmation.

Webhooks:

- List URL, events, active state, created time.
- Create and edit URL/events/active.
- Display signing secret once after create.
- Show recent deliveries with status, event, attempts, response code, and
  payload preview in a detail sheet.

SSO:

- Show current issuer/client/endpoints/enabled state.
- Upsert config with client secret optional when an existing secret is present.
- Show generated login URL after save.
- Clearly indicate whether a secret is already stored without revealing it.

LTI:

- List platforms with issuer, client id, deployment id, and created time.
- Register platform with required OIDC/LTI fields.
- Delete platforms behind confirmation.
- Show launch/login endpoint guidance using existing public endpoints without
  adding long instructional copy to the UI.

## Verification

- `cargo fmt --all --check`
- `cargo test -p backend --lib services::lti -- --nocapture`
- DB-backed backend tests covering LTI state insertion/consumption if existing
  test harness supports it.
- `cargo test -p features-courses --lib -- --nocapture`
- `cargo test -p shell-web --lib -- --nocapture`
- `cargo test -p shell-web --test shell_routes_smoke -- --nocapture`
- `cargo check -p backend --all-targets`
- `cargo check -p shell-web --all-targets`
- `cargo check -p platform-bridge --target wasm32-unknown-unknown`
- `cargo check -p shell-web --target wasm32-unknown-unknown`
- `git diff --check`

