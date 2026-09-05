# AulaLite Phase 2 MFA UX And Auth Step-Up Design

## Context

Phase 1 production unblockers are already present in the current tree: public runtime assets exist, JIT provisioning database tests are in integration scope, MFA challenge requests are exempt from the MFA enforcement gate, and course syllabus/grading markdown is sanitized before persistence.

The next roadmap item is Phase 2: complete the MFA user experience and auth step-up flow. The backend already has self-service TOTP enrollment, verification, recovery-code consumption, and `POST /v1/auth/mfa/challenge`. The missing work is the product flow around those primitives:

- login currently treats MFA enforcement as a failed sign-in path instead of an intermediate step;
- recovery codes are shown after enrollment but lack confirmation, copy, and download handling;
- remembered devices do not exist;
- tenant admins cannot preserve MFA while issuing replacement recovery codes for locked-out users.

## Goals

- Let a user with MFA enabled sign in successfully by completing a TOTP or recovery-code challenge after primary authentication.
- Add 30-day remembered devices that are enforced by the backend and revocable by the user.
- Improve recovery-code display after enrollment so users must explicitly acknowledge that the codes were saved.
- Let org-admins and platform admins generate replacement recovery codes for a same-tenant user while keeping MFA enabled.
- Preserve the existing Firebase/native primary-token flow and the existing step-up token model.
- Keep Phase 2 focused on MFA UX and auth step-up; defer wider native auth storage hardening to Phase 3.

## Non-Goals

- Replacing Firebase Auth or changing primary sign-in semantics.
- Building a full session-management console.
- Moving native/mobile token storage to OS keychain services; Phase 3 owns that.
- Disabling MFA during admin recovery reset.
- Adding per-tenant MFA policy controls beyond the existing `AULALITE_MFA_ENFORCE` flag.
- Reworking all scattered sign-out buttons into a shared authenticated layout.

## Approved Approach

Use backend-enforced remembered-device tokens plus focused UI wiring.

The frontend may store a trusted-device token locally, but it never decides whether MFA can be skipped. The backend stores only a hash, enforces expiry and revocation, and issues a step-up token only after a valid code or valid trusted-device token.

This is preferred over frontend-only remembering because local state is tamperable and the backend still owns MFA enforcement. It is preferred over a full auth-session rewrite because Phase 2 should close the immediate lockout and login-flow gap without overlapping the Phase 3 mobile/native auth parity work.

## Architecture

### Backend

Add a `user_mfa_trusted_devices` table:

- `id UUID PRIMARY KEY`
- `user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE`
- `token_hash TEXT NOT NULL UNIQUE`
- `label TEXT NOT NULL`
- `user_agent TEXT`
- `last_used_at TIMESTAMPTZ`
- `expires_at TIMESTAMPTZ NOT NULL`
- `revoked_at TIMESTAMPTZ`
- `created_at TIMESTAMPTZ NOT NULL DEFAULT now()`
- `updated_at TIMESTAMPTZ NOT NULL DEFAULT now()`

The table is not tenant-scoped because MFA is a property of the global `users` account. It uses the same owner-scoped `app.user_id` RLS pattern as `user_mfa`, plus a system-context read path if the middleware or challenge helper needs to validate device state outside normal owner context.

Add DB helpers in `crates/backend/src/db/mfa.rs`:

- create a trusted device from a plaintext random token by storing only the token hash;
- validate a trusted device by hash, user id, expiry, and `revoked_at IS NULL`;
- update `last_used_at` after a valid use;
- list current user's non-revoked trusted devices;
- revoke one trusted device by id for the owner;
- replace recovery-code hashes while leaving `enabled = true`.

Admin recovery reset updates another user's global MFA row only after the admin handler verifies caller role and same-tenant target membership. The helper should set the owner GUC to the target user only for the `user_mfa` mutation, then emit the audit event in the caller's tenant context with the admin as actor. This keeps the target MFA write possible without making `user_mfa` tenant-scoped or logging secrets.

Extend `crates/backend/src/handlers/mfa.rs`:

- `POST /v1/auth/mfa/challenge`
  - accepts either `code` or `trusted_device_token`;
  - accepts `remember_device: bool` and an optional `device_label`;
  - verifies TOTP first, then recovery code when `code` is present;
  - validates trusted-device token when `trusted_device_token` is present;
  - returns the existing step-up token;
  - returns a new plaintext trusted-device token only when `remember_device` is true after a valid code challenge.
- `GET /v1/me/mfa/trusted-devices`
  - lists id, label, last-used time, expiry, and created time, never token material.
- `DELETE /v1/me/mfa/trusted-devices/:id`
  - revokes one trusted device for the current user.

Extend `crates/backend/src/handlers/admin.rs`:

- `POST /v1/admin/tenant/memberships/:user_id/mfa/recovery-codes`
  - requires org-admin or platform-admin;
  - requires the target user to have a membership row in the caller's tenant;
  - requires the target user to have MFA enabled;
  - replaces all unused recovery-code hashes;
  - returns plaintext replacement recovery codes once;
  - emits an audit event with target user id and action `user_mfa.recovery_codes.reset`, but never logs plaintext codes.

### Frontend

Update `features-auth::Login` into a small state machine:

1. credential form;
2. primary token received;
3. `/v1/me` check;
4. trusted-device auto-challenge if a stored trusted token exists;
5. MFA code challenge panel when a code is required;
6. final success callback with the step-up token or primary token.

The component should not persist a primary local-login token before the MFA requirement is known. It should only persist a final accepted token after `/v1/me` succeeds or after a successful step-up challenge.

Add API DTOs and helpers in the auth feature or `features-courses::api` for:

- `challenge_mfa`;
- `list_mfa_trusted_devices`;
- `revoke_mfa_trusted_device`;
- `admin_reset_mfa_recovery_codes`.

Add a platform storage helper for trusted-device tokens:

- web stores under a dedicated `localStorage` prefix keyed by normalized email;
- native/mobile stores in the existing platform-bridge config area for Phase 2;
- sign-out clears all stored AulaLite trusted-device tokens on that device.

Update `features-courses::SecuritySettings`:

- keep current enrollment flow;
- after verify, show recovery codes with copy and download actions;
- require an "I saved these codes" acknowledgement before dismissing the recovery-code panel;
- show the trusted-device list when MFA is enabled;
- allow users to revoke individual trusted devices;
- render loading, empty, error, and success states without breaking the existing notification settings page.

Update `shell-web/src/routes/admin_tenant.rs`:

- add an MFA recovery reset action beside each member row;
- show the action for loaded member rows, with backend permission as the source of truth;
- display replacement recovery codes once in a focused panel or dialog;
- require an explicit acknowledgement before hiding replacement codes.

## Data Flow

### First Sign-In With MFA

1. User submits email and password.
2. Web/native bridge returns the primary Firebase or local-login token.
3. Login checks `/v1/me` with that token.
4. If `/v1/me` succeeds, login calls the existing `on_success` callback with the primary token.
5. If `/v1/me` returns `mfa_required`, login keeps the primary token in component state and shows the MFA challenge panel.
6. User submits a TOTP or recovery code, optionally with "remember this device" selected.
7. Backend returns a step-up token and, when requested, a plaintext trusted-device token.
8. Login stores the trusted-device token locally, calls `on_success(stepup_token)`, and the shell loads `/v1/me`.

### Future Sign-In With Remembered Device

1. User submits primary credentials as usual.
2. `/v1/me` returns `mfa_required`.
3. Login looks up a stored trusted-device token for the normalized email.
4. Login calls `POST /v1/auth/mfa/challenge` with `trusted_device_token`.
5. If valid, the backend returns a step-up token and updates `last_used_at`.
6. If invalid, expired, or revoked, the client clears the local token and falls back to the code challenge panel.

### Recovery Code Consumption

1. User submits a recovery code in the same challenge field used for TOTP.
2. Backend tries TOTP verification first.
3. If TOTP fails, backend hashes the presented value and attempts atomic recovery-code consumption.
4. On success, the recovery-code hash is removed and the step-up token is returned.

### Admin Recovery Reset

1. Org-admin opens tenant members.
2. Admin triggers "Reset recovery codes" for a member.
3. Backend verifies admin role, tenant scope, and target MFA status.
4. Backend replaces unused recovery-code hashes and audits the action.
5. UI displays plaintext replacement codes once and requires acknowledgement before dismissal.

## Error Handling

Stable backend error strings used by the UI:

- `mfa_required`
- `code_required`
- `one_challenge_method_required`
- `invalid_code`
- `mfa_not_enabled`
- `trusted_device_invalid`
- `trusted_device_expired`
- `trusted_device_revoked`
- `sso_session_secret_unset`

The login UI branches only on stable strings. Unknown `401` errors remain normal sign-in failures. Unknown `400` and `500` errors stay on the current panel with a concise inline message and a toast.

Trusted-device invalid, expired, and revoked responses clear local trusted-device storage for that account, then show the code challenge panel. They do not sign the user out of the primary identity provider because a valid primary token still exists.

Admin reset errors:

- `403` means the caller cannot reset codes for the target;
- `404` means the target user is not in this tenant;
- `400 mfa_not_enabled` means no replacement codes can be generated because MFA is not currently active.

## Security Requirements

- Never store plaintext recovery codes or trusted-device tokens on the server.
- Generate trusted-device tokens with at least 32 bytes of CSPRNG entropy, encoded for transport.
- Trusted-device tokens are valid only with a fresh primary-authenticated token for the same user.
- Trusted-device records expire after 30 days and can be revoked before expiry.
- The trusted-device list never returns token hashes or plaintext token material.
- Admin reset returns plaintext replacement recovery codes once and does not log them.
- Audit events identify the actor and target user but omit secrets.
- Sign-out clears locally stored trusted-device tokens on the current device.
- Recovery-code panels require explicit acknowledgement before hiding secrets.
- The existing MFA challenge route remains reachable with a primary token under `AULALITE_MFA_ENFORCE`.

## UI Quality Bar

- Use existing design-system primitives and `design_system::kinetics_ui` where practical.
- Keep auth and admin surfaces compact and task-focused.
- Do not add marketing copy or decorative layout to admin/product workflows.
- Avoid nested cards.
- Ensure recovery-code and trusted-device text wraps cleanly on mobile and desktop widths.
- Buttons use direct action labels, and destructive/revocation actions require clear confirmation.

## Testing Strategy

Unit tests:

- trusted-device token hashing and expiry classification;
- challenge request validation for code versus trusted-device token;
- recovery-code replacement helper behavior;
- login error classifier for `mfa_required`;
- local trusted-device storage key normalization.

Backend integration tests:

- TOTP challenge returns a usable step-up token;
- recovery code is consumed once;
- remembered device creation stores only a hash and returns plaintext once;
- trusted-device challenge succeeds before expiry;
- revoked and expired trusted devices are rejected;
- trusted-device list and revoke are owner-scoped;
- admin recovery reset requires admin role and same-tenant target membership;
- admin recovery reset keeps MFA enabled and replaces recovery-code hashes.

Frontend SSR/component tests:

- login renders MFA challenge after an `mfa_required` outcome;
- trusted-device failure falls back to code entry;
- recovery-code panel renders copy/download/acknowledgement controls;
- trusted-device list renders empty, populated, and error states;
- admin tenant member reset action renders and the replacement-code panel requires acknowledgement.

Verification commands:

- `cargo test -p backend --lib`
- `cargo test -p backend --test mfa`
- `cargo test -p features-auth --lib`
- `cargo test -p features-courses --lib`
- `cargo test -p shell-web --lib`
- `cargo check -p backend --all-targets`
- `cargo check -p shell-web --target wasm32-unknown-unknown`

DB-backed tests require a migrated PostgreSQL database. DB-free library tests must continue to pass without PostgreSQL.

## Exit Criteria

- A user with MFA enabled can sign in with TOTP and reach the dashboard.
- A user with MFA enabled can sign in with a recovery code and reach the dashboard.
- A user can remember a device for 30 days and sign in again without typing a second factor after primary auth.
- Revoking a remembered device forces code challenge on the next sign-in from that device.
- Expired remembered devices are rejected and cleared from local storage.
- Recovery codes displayed after enrollment cannot be dismissed without acknowledgement.
- Org-admins can issue replacement recovery codes for same-tenant MFA users without disabling MFA.
- Admin recovery reset is permission-checked and audited.
- Phase 2 tests and compile checks pass, with DB-free tests remaining DB-free.

## Risks And Mitigations

- Risk: remembered-device tokens become a second long-lived bearer credential.
  - Mitigation: require primary auth, hash tokens server-side, expire after 30 days, support revocation, and clear local storage on sign-out.
- Risk: login flow persists a primary token before MFA is complete.
  - Mitigation: persist only after `/v1/me` succeeds or after step-up challenge succeeds.
- Risk: admin reset leaks replacement codes through audit metadata or logs.
  - Mitigation: audit only action and target identifiers; never include plaintext codes.
- Risk: native/mobile storage remains plaintext in Phase 2.
  - Mitigation: keep token lifetime short, clear on sign-out, and leave OS keychain migration explicitly in Phase 3.

## Follow-On Boundaries

Phase 3 remains responsible for shared shell auth bootstrap, provider parity, proper native/mobile signout coverage, and OS credential storage. Phase 2 should only touch those areas where MFA login, remembered-device storage, and sign-out cleanup require it.
