# AulaLite Phase 3 Mobile And Native Auth Parity Design

## Context

Phase 2 completed MFA sign-in, recovery-code handling, remembered devices, and
admin recovery-code reset. The next roadmap item is Phase 3: bring web,
desktop, and mobile auth behavior into one model and remove plaintext token
storage from production native/mobile targets.

Current state:

- `shell-web::App` owns a relatively complete auth bootstrap path: it registers
  token refresh, restores a persisted token, fetches `/v1/me`, provides
  `Signal<ApiContext>` and `UserContextSignal`, and gates the router on
  `auth_ready`.
- `shell-mobile::MobileApp` duplicates part of that logic but does not gate the
  router while bootstrap is unresolved, so route guards can observe the
  transient signed-out state.
- `shell-desktop` launches `shell_web::App`, so it inherits the web/native
  bootstrap implementation but still depends on `platform-bridge::native` for
  native token persistence.
- `platform-bridge::native` persists primary Firebase tokens in
  `<config>/aulalite/auth.json` and remembered-device tokens in
  `mfa_trusted_devices.json`. The file explicitly notes that this is not
  encrypted at rest.
- Many routes wire signout inline through `platform_bridge::web::WebBridge`,
  which keeps web behavior working but leaves native/mobile parity fragile.

Current keyring ecosystem docs recommend application code use `keyring-core`
plus specific credential-store crates when the application needs platform
control. Relevant provider crates exist for Windows Credential Manager,
macOS/iOS Keychain, Android native storage, and Linux Secret Service.

## Goals

- Share auth bootstrap logic across web, desktop, and mobile shells.
- Ensure all shells provide the same core contexts before route components run:
  `ApiContext`, `UserContextSignal`, toast, locale, theme, density, and router
  guard state.
- Add an auth-ready gate to mobile so signed-in users are not bounced to login
  during bootstrap.
- Centralize signout so it clears primary credentials, remembered-device
  tokens, API context, user context, and route state consistently.
- Introduce a `TokenStore` boundary in `platform-bridge` for primary and
  remembered-device secrets.
- Store native/mobile production secrets in OS credential storage by default:
  Windows Credential Manager, macOS/iOS Keychain, Linux Secret Service, and
  Android native secure storage.
- Keep JSON file storage available only for tests, local development, and
  explicitly configured fallback deployments.
- Preserve the existing web Firebase/local-login behavior and Phase 2 MFA
  remembered-device flow.

## Non-Goals

- Replacing Firebase as the primary identity provider.
- Changing backend auth middleware, MFA enforcement, or step-up token semantics.
- Reworking every route into a new authenticated layout in this phase.
- Building a session-management console.
- Adding SSO/LTI configuration UI; later integration-admin phases own that.
- Solving push-notification device registration; Phase 4 owns push.

## Approved Approach

Use a shared shell auth bootstrap plus a configurable secure token-store
abstraction.

The recommended design has two tracks that land together:

1. Extract auth bootstrap into a reusable shell-level module used by web,
   desktop, and mobile.
2. Move native/mobile token persistence behind `TokenStore`, with secure
   platform stores as the production default and a file-backed store gated to
   dev/test or explicit fallback configuration.

This is preferred over a bootstrap-only phase because the roadmap identifies
plaintext native/mobile token storage as a Phase 3 security requirement. It is
preferred over custom per-OS APIs because the current Rust keyring ecosystem
already exposes platform-specific credential-store crates while keeping the
application in control of which stores are linked.

## Architecture

### Shared Auth Bootstrap

Add a shared module under `shell-web` or a small reusable shell-support module
that exposes an auth bootstrap component/helper. It should accept target-specific
bridge behavior but own the shared state machine:

1. initialize `Signal<ApiContext>` with the correct base URL;
2. initialize `UserContextSignal` to `None`;
3. register the `api::RefreshFn` exactly once;
4. attempt persisted local/session token restoration;
5. fetch `/v1/me` with the candidate token;
6. populate user context only after `/v1/me` succeeds;
7. clear stale local credentials where the target bridge supports clearing;
8. mark `auth_ready = true` after signed-in or anonymous bootstrap is resolved;
9. render router only after `auth_ready`.

The shared module should expose enough customization for:

- web same-origin API base URL;
- native/mobile absolute API base URL via `api::native_api_base_url()`;
- web local-login token bootstrap;
- native/mobile secure token bootstrap;
- target-specific signout bridge.

The mobile shell should use the same provider stack as the web shell. If a
route component needs a context in web, it must receive it in mobile before the
router renders.

### Signout Boundary

Add one shared signout action/helper that:

- calls the active platform bridge signout;
- clears remembered-device tokens for the active target;
- resets `ApiContext.id_token` to empty while preserving the correct base URL;
- resets `UserContextSignal` to `None`;
- navigates to login;
- tolerates already-signed-out storage state.

Routes should stop constructing their own platform-specific signout closures as
they are touched. This phase should update the highest-impact shared shell
signout path and the routes needed for compile coverage, but avoid a broad route
rewrite that does not serve auth parity.

### Token Store Abstraction

Add a native-only `TokenStore` boundary in `platform-bridge`:

```rust
pub trait TokenStore {
    fn load_primary(&self) -> Result<Option<TokenFile>, BridgeError>;
    fn save_primary(&self, tokens: &TokenFile) -> Result<(), BridgeError>;
    fn clear_primary(&self) -> Result<(), BridgeError>;
    fn trusted_device_token(&self, email: &str) -> Result<Option<String>, BridgeError>;
    fn persist_trusted_device_token(&self, email: &str, token: &str) -> Result<(), BridgeError>;
    fn clear_trusted_device_token(&self, email: &str) -> Result<(), BridgeError>;
    fn clear_all_trusted_device_tokens(&self) -> Result<(), BridgeError>;
}
```

The exact trait shape can be adjusted during planning, but the boundary must
cover both primary refresh-token persistence and Phase 2 remembered-device
tokens. Route and login code should continue to call bridge-level helpers rather
than concrete store implementations.

### Store Selection Policy

Native/mobile store selection should be explicit and testable:

- `secure`: require OS credential storage; fail if unavailable.
- `dev-file`: use current JSON file storage; allowed in tests and local dev.
- `auto`: default behavior; resolves to `secure` in production-like builds and
  `dev-file` only when an explicit development/test signal is present.

Recommended config:

- `AULALITE_TOKEN_STORE=secure|dev-file|auto`
- default: `auto`;
- production default resolution: `secure`;
- dev/test fallback requires either `AULALITE_TOKEN_STORE=dev-file` or
  `cfg(test)`.

If secure storage cannot initialize in production mode, sign-in and bootstrap
should fail closed with a clear error. The user should see a concise sign-in
error rather than silently persisting tokens to plaintext JSON.

### Secure Stores

Use `keyring-core` and specific provider crates:

- Windows: `windows-native-keyring-store`
- macOS/iOS: `apple-native-keyring-store`
- Linux desktop: `zbus-secret-service-keyring-store` by default, with
  `dbus-secret-service-keyring-store` as a fallback only if the zbus provider
  proves incompatible in this workspace.
- Android: `android-native-keyring-store`

Credential naming should be stable:

- service: `aulalite`
- primary token entry: `auth.primary`
- remembered-device entry: `mfa.trusted-device.<normalized-email>`

Primary token data can be stored as JSON inside one secure secret value for
compatibility with the existing `TokenFile` shape. Remembered-device tokens can
be stored per normalized email so clearing one device token does not rewrite the
primary auth secret.

### Web Storage

Web remains browser-native:

- Firebase JS SDK owns Firebase persistence.
- Local-login and SSO/LTI session tokens continue using the current
  `localStorage` path.
- Remembered-device tokens continue using the Phase 2 web localStorage helpers.
- Web signout clears local-login and remembered-device storage before invoking
  Firebase signout.

This phase should not force web tokens into a keyring abstraction because web
has a different execution and persistence model.

## Data Flow

### Native/Mobile Bootstrap

1. Shell creates `ApiContext` and `UserContextSignal`.
2. Shared bootstrap selects a token store.
3. `NativeBridge.current_id_token()` loads the primary token from the selected
   store.
4. If the ID token is near expiry, the bridge refreshes it with Firebase and
   saves the refreshed `TokenFile` through the same store.
5. Shared bootstrap calls `/v1/me`.
6. On success, it publishes `ApiContext` and `UserContext`.
7. On stale/invalid token failure, it clears stored primary credentials and
   leaves user context empty.
8. Router renders only after `auth_ready`.

### Native/Mobile Sign-In

1. Login uses the existing `PlatformBridge` email/password path.
2. `NativeBridge` exchanges credentials with Firebase.
3. It persists the returned `TokenFile` through the selected store.
4. Login continues through the Phase 2 `/v1/me` and MFA step-up flow.
5. The final accepted token becomes the live `ApiContext.id_token`.

### Remembered Device On Native/Mobile

1. After successful MFA challenge with remember-device enabled, login calls the
   native trusted-device persistence helper.
2. The helper saves the token through the selected `TokenStore`.
3. Future sign-in retrieves by normalized email and attempts backend step-up.
4. Invalid, expired, or revoked tokens are cleared through the same store.

### Signout

1. User triggers signout from shell UI.
2. Shared signout calls the active bridge signout.
3. Bridge clears primary and remembered-device credentials.
4. Shell clears live auth/user contexts.
5. Shell routes to login.

## Error Handling

- Missing production secure storage returns a stable `BridgeError` message such
  as `secure token store unavailable`.
- Corrupt dev-file JSON clears or rejects the file only in dev-file mode; secure
  mode never reads those files.
- Failed refresh clears primary credentials only when Firebase rejects the
  refresh token or returns an unrecoverable auth error.
- Transient network refresh errors should surface as sign-in/bootstrap errors
  without deleting the refresh token.
- Routes should not see missing providers during bootstrap; they either render
  after `auth_ready` or render a shell-level auth splash.

## Security Requirements

- Production native/mobile builds must not silently write primary refresh tokens
  to plaintext JSON.
- Dev/test file storage must require explicit configuration outside `cfg(test)`.
- Remembered-device tokens are treated as secrets and stored in the same secure
  store policy as primary tokens on native/mobile.
- Signout must clear both primary and remembered-device secrets.
- Secure-store entry names must not include plaintext auth tokens or recovery
  codes.
- Logs and errors must not include ID tokens, refresh tokens, remembered-device
  tokens, or recovery codes.
- The store abstraction must keep web localStorage and native secure storage
  separated so target-specific behavior is auditable.

## UI Requirements

- Mobile must show the same auth splash/gate concept as web while bootstrap is
  resolving.
- Sign-in errors caused by secure-store initialization should be short and
  actionable.
- Auth surfaces should stay compact and consistent with existing
  `features-auth` components.
- This phase does not add marketing layouts, onboarding heroes, or new admin
  navigation.

## Testing Strategy

Unit tests:

- token-store selection for `secure`, `dev-file`, and `auto`;
- dev-file store roundtrip for primary tokens;
- dev-file store roundtrip and removal for remembered-device tokens;
- secure-store unavailable maps to stable bridge errors;
- bootstrap decision helpers distinguish success, stale token, no token, and
  failed `/v1/me`;
- signout helper clears live contexts and calls storage cleanup.

SSR/component tests:

- mobile shell renders auth splash before `auth_ready`;
- mobile shell provides the same context types as web before route rendering;
- login still renders with the shared providers;
- signout wiring composes without target-specific panics.

Compile checks:

- `cargo test -p platform-bridge --lib`
- `cargo test -p shell-web --lib`
- `cargo test -p shell-mobile --lib`
- `cargo check -p shell-web --target wasm32-unknown-unknown`
- `cargo check -p shell-mobile --target wasm32-unknown-unknown`
- native desktop/mobile checks for the host target

Manual/device checks:

- Windows Credential Manager stores and clears primary auth entries.
- macOS/iOS Keychain stores and clears primary auth entries.
- Android secure storage stores and clears primary auth entries.
- Linux Secret Service stores and clears primary auth entries when a session
  keyring is available.

## Exit Criteria

- Web, desktop, and mobile shells share one auth bootstrap state machine.
- Mobile no longer renders protected routes before auth bootstrap resolves.
- All shells provide the auth/user/toast/theme/locale provider stack expected by
  shared route components.
- Native/mobile signout clears primary credentials and remembered-device tokens.
- Production native/mobile token storage uses OS credential storage by default.
- Plaintext JSON token storage remains available only for tests, local dev, or
  explicit fallback configuration.
- Phase 2 MFA remembered-device behavior still works on web and native/mobile.
- Focused tests and compile checks pass.

## Risks And Mitigations

- Risk: keyring provider crates have platform-specific build requirements.
  - Mitigation: gate dependencies by target and keep dev-file tests platform
    independent.
- Risk: mobile native secure storage requires additional platform glue.
  - Mitigation: isolate the store behind `TokenStore` and implement provider
    setup as target-specific modules.
- Risk: shared bootstrap grows too generic and hard to reason about.
  - Mitigation: keep the bootstrap state machine small, with explicit bridge
    hooks instead of an abstract framework.
- Risk: broad route signout rewrites create churn.
  - Mitigation: centralize the helper and migrate only the necessary call sites
    in this phase.
- Risk: secure-store failures strand local developers.
  - Mitigation: allow `AULALITE_TOKEN_STORE=dev-file` for local development and
    make the error message point to that override.

## Dependency Notes

Verified on 2026-06-17:

- `keyring` 4.x documentation says applications that need platform control
  should use `keyring-core` plus selected credential-store crates rather than
  depending on the all-in-one `keyring` crate with the CLI glue.
- `keyring-core` provides the cross-platform API layer and testing stores.
- Provider crates are available for Apple native stores, Windows native stores,
  Android native storage, and Linux Secret Service.
