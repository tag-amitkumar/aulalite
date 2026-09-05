# AulaLite Phase 4 Push Notifications Design

## Context

Phase 3 completed shared auth bootstrap and secure native token storage. The
next item in the review remediation roadmap is Phase 4: make push
notifications production-grade.

The repository already has a useful notification foundation:

- `migrations/20260529000026_notifications.sql` defines tenant-scoped
  `notifications` and `device_tokens`, plus global per-user
  `notification_preferences`.
- `crates/backend/src/handlers/notifications.rs` exposes the caller-scoped
  notification feed, unread count, read/read-all actions, device-token
  registration/removal, and preference read/update endpoints.
- `crates/backend/src/services/notifications.rs` owns the notification facade,
  email sender trait, push sender trait, mock senders, and an FCM sender stub.
- `crates/features-courses/src/notification_bell.rs` renders in-app
  notifications.
- `crates/shell-web/src/routes/notification_settings.rs` renders notification
  preferences and a browser-push enable affordance.
- `crates/shell-web/public/assets/fcm-bridge.js` and
  `crates/shell-web/public/firebase-messaging-sw.js` provide the web FCM
  registration bridge.

The gap is that push delivery is still scaffolding. `FcmPushSender` can post to
FCM only when given a pre-obtained access token through `FCM_ACCESS_TOKEN`; it
does not mint or cache OAuth tokens from a Firebase service account. Device
registration also cannot yet be listed in the UI, and delivery outcomes are
only logged through tracing rather than persisted for user or admin diagnosis.

Current Firebase guidance for HTTP v1 sends requires an OAuth Bearer token and
posts one message to
`https://fcm.googleapis.com/v1/projects/{project_id}/messages:send`. Firebase
web push setup requires VAPID credentials, notification permission, and a
service worker.

## Goals

- Replace the FCM access-token stub with service-account-backed OAuth token
  minting and caching.
- Validate push configuration at startup without making push a hard boot
  dependency in environments that intentionally leave it disabled.
- Preserve the existing best-effort notification facade: notification failures
  must not fail grading, announcements, reminders, or other primary product
  actions.
- Persist delivery attempts and provider outcomes for push sends.
- Let users list and revoke their own registered browser/mobile devices.
- Keep notification preferences as the channel-level source of truth.
- Add an admin diagnostic surface for delivery attempts and failures.
- Keep web push enrollment graceful when the VAPID key, service worker, browser
  APIs, or user permission are unavailable.
- Prepare the backend shape for future native mobile token registration without
  blocking this phase on full iOS/Android app-store push wiring.

## Non-Goals

- Replacing Firebase Cloud Messaging with a different push provider.
- Building a full operational dashboard; Phase 9 owns broader platform
  operations.
- Adding marketing campaigns, topics, or broadcast notifications.
- Reworking in-app notification feed semantics.
- Replacing the existing Resend email sender.
- Rewriting the web FCM bridge to Firebase Installation IDs in this phase.
- Making push credentials mandatory for local development, tests, or preview
  deployments.

## Approved Approach

Finish the existing notification pipeline instead of adding a parallel push
subsystem.

The recommended design keeps `services::notifications::notify` as the single
fan-out entry point. It replaces the internal FCM sender stub with a provider
implementation that can:

1. Parse and validate Firebase service-account configuration.
2. Mint short-lived OAuth access tokens with the FCM scope.
3. Cache tokens until near expiry.
4. Send one HTTP v1 request per device token.
5. Record per-token delivery outcomes.

This approach is preferred over a backend-only push patch because users and
admins need to see registered devices and failures. It is preferred over a
UI-first phase because without OAuth token minting the product still cannot
prove real delivery.

## Backend Design

### FCM Provider Configuration

Introduce a focused provider config type in
`crates/backend/src/services/notifications.rs` or a sibling
`notifications_fcm.rs` module:

- `project_id`
- `client_email`
- `private_key`
- `private_key_id`
- optional `token_uri`, defaulting to `https://oauth2.googleapis.com/token`
- optional `fcm_base_url`, defaulting to `https://fcm.googleapis.com`

Supported environment input:

- `FCM_PROJECT_ID`
- `FCM_SERVICE_ACCOUNT_JSON`
- optional `FCM_SERVICE_ACCOUNT_JSON_PATH`
- optional `FCM_TOKEN_URI`
- optional `FCM_BASE_URL`

`FCM_SERVICE_ACCOUNT_JSON` is the preferred production input because Dokploy can
provide it as a secret without mounting files. `FCM_SERVICE_ACCOUNT_JSON_PATH`
is kept for local and container deployments that already use file-backed
service-account credentials. If both JSON and path are set, the inline JSON
wins and the startup log should mention that path is ignored.

Validation should be pure and testable:

- Missing `FCM_PROJECT_ID` means push is disabled.
- Missing service-account JSON/path with a project ID means push is disabled and
  logged as misconfigured.
- Invalid JSON, missing `client_email`, missing `private_key`, or unsupported
  key material returns a stable config error.
- Production should still boot with a warning, matching current best-effort
  notification behavior. The provider can expose an explicit disabled state for
  diagnostics.

### OAuth Token Minting

Add an `FcmAccessTokenProvider` boundary so token minting can be unit-tested and
mocked independently from message sending.

Production token minting should follow the Google service-account JWT bearer
flow:

- JWT header: `alg=RS256`, `typ=JWT`, and `kid` when available.
- JWT claims:
  - `iss`: service-account client email
  - `scope`: `https://www.googleapis.com/auth/firebase.messaging`
  - `aud`: OAuth token endpoint
  - `iat`: current Unix timestamp
  - `exp`: no more than one hour after `iat`
- Sign the header and claims using RS256 with the service-account private key.
- POST the assertion to the token endpoint using grant type
  `urn:ietf:params:oauth:grant-type:jwt-bearer`.
- Cache the returned access token until a refresh window, for example five
  minutes before expiry.

Use a maintained Rust crypto/JWT path compatible with the workspace rather than
hand-rolling RSA signing. The implementation plan should first check dependency
constraints, then select the smallest maintained crate set that supports RS256
signing and PEM service-account keys.

Tests should avoid real Google calls by injecting a mock token endpoint or
token provider. No service-account secret should appear in tests or fixtures.

### FCM Message Sending

Replace the current `FcmPushSender` constructor with a provider-backed
constructor while keeping the `PushSender` trait stable:

```rust
async fn send_push(
    &self,
    tokens: &[String],
    title: &str,
    body: &str,
    link: Option<&str>,
) -> Result<(), NotifyError>;
```

FCM HTTP v1 sends one message per registration token. The provider should
attempt every token and return the first error only after all sends are tried.

Message shape:

- `message.token`: device token.
- `message.notification.title/body`: display notification text.
- `message.data.link`: optional in-app route or absolute URL.
- `message.webpush.fcm_options.link`: optional link for web clients when safe.

Links must be constrained before being sent to a provider. Prefer relative app
paths or known app origins. Do not send arbitrary javascript-like or
cross-origin URLs in notification data.

Invalid or unregistered tokens should be detected from stable FCM error
responses where possible and marked inactive or removed. The first version may
only persist the failure, but the schema should allow token cleanup without a
future migration.

### Delivery Logging

Add a `notification_deliveries` table with one row per channel attempt. This is
separate from the in-app `notifications` table because email and push attempts
can happen even when in-app notifications are disabled.

Suggested columns:

- `id uuid primary key`
- `tenant_id uuid not null`
- `user_id uuid not null`
- `notification_id uuid null references notifications(id)`
- `channel text not null check (channel in ('email','push'))`
- `provider text not null`
- `target_hash text not null`
- `target_label text null`
- `device_token_id uuid null references device_tokens(id)`
- `kind text not null`
- `status text not null check (status in ('queued','sent','failed','skipped'))`
- `provider_message_id text null`
- `provider_status text null`
- `error_code text null`
- `error_message text null`
- `created_at timestamptz not null default now()`
- `updated_at timestamptz not null default now()`

Do not store raw push tokens in delivery logs. Store a deterministic hash and a
safe label such as platform/browser if available.

The table is tenant-scoped and must use RLS with `FORCE ROW LEVEL SECURITY`.
User self-service queries are scoped to `ctx.user_id`; admin diagnostics require
tenant staff/admin permission checks in handlers.

### Device Records

Extend the `device_tokens` read model without breaking existing registration:

- List own registered devices.
- Revoke by `id`, not just by raw token.
- Keep delete-by-token for current web bridge compatibility if needed.
- Add optional fields when useful:
  - `label`
  - `user_agent`
  - `last_used_at`
  - `revoked_at`

If the current token table is migrated to soft revocation, push sends must only
load active tokens. If hard delete is kept for this phase, delivery logs still
need `device_token_id` nullable so old logs remain queryable after a token is
deleted.

### Backend Endpoints

Self-service:

- `GET /v1/me/device-tokens`
  - returns active devices for the current user and tenant.
- `DELETE /v1/me/device-tokens/:id`
  - revokes a device owned by the current user.
- existing `POST /v1/me/device-tokens`
  - continues to register/upsert a token.
- existing `DELETE /v1/me/device-tokens`
  - remains for raw-token cleanup while the web bridge still has only the token.

Admin diagnostics:

- `GET /v1/admin/notification-deliveries`
  - tenant admin and platform admin only.
  - supports filters: channel, status, user, kind, provider, before, limit.
- `GET /v1/admin/notification-deliveries/:id`
  - returns one delivery detail row in the caller's tenant.

Optional test endpoint for non-production environments:

- `POST /v1/me/device-tokens/:id/test`
  - sends a controlled "test notification" to the caller's own device.
  - disabled in production unless a specific env flag enables it.

## Frontend Design

### User Notification Settings

Keep the existing `/settings/notifications` route, but add a device-management
section beneath the browser-push enrollment row.

States:

- Loading: skeleton row/table.
- Empty: "No registered devices" with no decorative illustration.
- Loaded: compact table/list with platform, label or browser, last seen, and a
  revoke action.
- Error: inline error with retry.
- Revoking: row-level busy state and optimistic removal only after success.

The preference switches remain channel-level controls. Disabling push should not
delete device tokens; it only suppresses sends. Device revocation should stop
future sends to that browser/device regardless of the preference toggle.

Use existing design primitives where available. This is an operational settings
screen, so the UI should be compact and scan-friendly rather than card-heavy.

### Browser Push Enrollment

The existing browser-push enable button continues to call
`platform_bridge::web::fcm_request_token()`.

Improve the result states:

- unsupported browser or no service worker: disabled/unavailable message.
- missing VAPID key: configured-off message.
- permission denied: explain that the browser blocked notifications.
- token acquired: register token and refresh the device list.
- token registration failed: show an inline/toast error and keep the enable row
  visible.

The web bridge can keep using FCM registration tokens in this phase because the
existing backend and FCM sender are token-based. A future phase may migrate to
Firebase Installation IDs if the Firebase SDK deprecates registration-token
management for this use case.

### Admin Delivery Diagnostics

Add this as a grouped admin diagnostics route under existing admin navigation,
reusing the current admin shell patterns without adding a new top-level sidebar
item.

Required UI:

- filterable delivery table
- status badges for sent, failed, skipped, queued
- channel/provider/kind columns
- timestamp column
- detail sheet with provider status, sanitized provider response, and target
  label/hash

Do not show raw device tokens, service-account data, or provider access tokens.

## Data Flow

Registration:

1. User opens notification settings.
2. Web bridge registers the FCM service worker, requests permission, and gets a
   registration token when available.
3. Frontend calls `POST /v1/me/device-tokens`.
4. Backend upserts the active token for the caller's tenant/user.
5. Device list refreshes.

Delivery:

1. Product code calls `services::notifications::notify`.
2. The facade reads preferences.
3. In-app notification row is created if enabled.
4. Email is sent if enabled and configured.
5. Active device tokens are loaded if push is enabled.
6. `FcmPushSender` obtains or reuses an OAuth token.
7. One FCM HTTP v1 request is sent per token.
8. Each attempt records a delivery row with status and provider outcome.
9. Failures are logged and queryable, but they do not fail the original product
   action.

Diagnostics:

1. Admin opens delivery diagnostics.
2. Frontend calls the tenant-scoped delivery list endpoint.
3. Backend permission checks tenant admin/platform admin status.
4. Table and detail sheet render delivery status without exposing secrets.

## Error Handling

- Missing push config results in a disabled provider state, not a panic.
- Invalid service-account config is logged with a stable reason but never logs
  private key material.
- OAuth token exchange failures produce `NotifyError::Transport` or
  `NotifyError::Api` with sanitized response text.
- FCM per-token failures are recorded independently.
- User-facing enablement errors distinguish unsupported browser, denied
  permission, missing config, and backend registration failure where the client
  can tell them apart.
- Delivery diagnostics expose sanitized provider status and failure category,
  not raw secrets.

## Security Requirements

- Never log or persist service-account private keys, OAuth access tokens, or raw
  device tokens outside `device_tokens`.
- Delivery logs store target hashes and safe labels, not raw tokens.
- FCM `link` data must be constrained to safe app routes or approved origins.
- Device list/revoke endpoints must scope to the current `ctx.user_id`.
- Admin delivery diagnostics require tenant admin or platform admin
  permissions.
- Tenant-scoped delivery logs must use RLS and `FORCE ROW LEVEL SECURITY`.
- Service-account JSON should be accepted from environment secrets and must not
  be committed to the repo.

## Testing Strategy

Unit tests:

- FCM config parsing accepts inline JSON and file-backed JSON.
- FCM config parsing rejects missing required fields.
- JWT claim construction uses the FCM scope, token endpoint audience, and
  bounded expiration.
- Token cache reuses a fresh token and refreshes near expiry.
- FCM message payload includes notification text and safe link data.
- Unsafe links are omitted or rejected.
- Provider error classification extracts stable failure categories.

Backend integration tests:

- Current user can list and revoke only their own device tokens.
- Revoked/deleted tokens are not loaded for push sends.
- Delivery logs are created for push success, push failure, and disabled push.
- Tenant admin can query delivery logs for their tenant.
- Non-admin users cannot query admin diagnostics.
- RLS blocks cross-tenant delivery-log reads under the app role.

Frontend tests:

- Notification settings renders device loading, empty, loaded, revoke, and error
  states.
- Browser push enablement shows configured-off and permission-denied copy.
- Admin delivery diagnostics renders empty, failed, and sent rows.
- Detail sheet hides raw tokens and shows sanitized provider data.

Verification commands should include the relevant backend lib/integration tests,
`shell-web` SSR tests, native `shell-web` checks, and wasm `shell-web` checks.

## Implementation Sequence

1. Add backend delivery-log schema and DB helpers.
2. Add self-service device list/revoke endpoints and API helpers.
3. Add FCM config parsing and token-provider boundary with pure tests.
4. Implement OAuth token minting and cache with mockable HTTP.
5. Replace the FCM sender stub while preserving the `PushSender` trait.
6. Record push delivery outcomes from the notification facade.
7. Add admin delivery diagnostics endpoints.
8. Upgrade notification settings device management UI.
9. Add admin diagnostics UI.
10. Run final backend/frontend/wasm verification and merge.

## Risks And Mitigations

- Risk: OAuth signing introduces security-sensitive complexity.
  - Mitigation: use maintained crates, keep signing isolated, and test claim
    construction separately from network exchange.
- Risk: Provider credentials are unavailable in local/dev.
  - Mitigation: keep disabled-provider behavior explicit and testable.
- Risk: Delivery logs leak sensitive targets.
  - Mitigation: hash targets and avoid raw tokens outside `device_tokens`.
- Risk: FCM error payloads vary.
  - Mitigation: persist raw provider status only after sanitization and keep
    coarse status/error categories stable.
- Risk: Device revocation and token upsert semantics conflict.
  - Mitigation: define active/revoked behavior in the migration and test token
    re-registration explicitly.

## Acceptance Criteria

- Push can remain disabled cleanly when FCM or VAPID configuration is absent.
- Configured environments can mint OAuth access tokens from a Firebase service
  account and send FCM HTTP v1 messages.
- Users can list and revoke their own devices.
- Push sends create queryable delivery-log rows.
- Admins can inspect tenant delivery attempts without seeing raw tokens or
  secrets.
- Existing in-app notifications, email notification behavior, and preferences
  continue to work.
- Backend, frontend, and wasm checks pass with push configured off.

## References

- Firebase Cloud Messaging HTTP v1 send API:
  https://firebase.google.com/docs/cloud-messaging/send/v1-api
- Firebase Cloud Messaging for web apps:
  https://firebase.google.com/docs/cloud-messaging/web/get-started
- Google OAuth 2.0 service-account flow:
  https://developers.google.com/identity/protocols/oauth2/service-account
