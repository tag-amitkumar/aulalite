# Native application release status

The Android entry point mounts the same typed route graph as the web client,
uses the native Dioxus host, loads the shared visual system, authenticates
through Firebase Identity Toolkit, and persists credentials through Android
secure storage. All repository-controlled Android release integrations are
implemented. Store ownership/signing and real-device acceptance remain release
operations, not application-code gaps.

## What is release-ready in code

- Desktop and mobile use `shell_web::App`; route and role behavior no longer
  diverge between shells.
- Ordinary internal links are forwarded to the Dioxus router on native WebViews
  while modified, external, download, fragment, and targeted links retain their
  normal semantics.
- Native sign-out clears the renderer credential store and live application
  contexts, plus only that verified user's offline cache. Workspace choices
  persist outside the process; switching refreshes the authenticated user
  context and remounts workspace-scoped resources.
- The native WebViews embed the shared CSS. Both Dioxus bundle configurations
  package `shell-web/public`, including the four self-hosted fonts, AulaLite
  mark, academy imagery, and other runtime assets referenced by that CSS.
- Firebase/API responses are bounded and sanitized, requests have timeouts, and
  release configuration rejects plaintext API origins. Credentials default to
  Windows Credential Manager, Apple Keychain, Linux Secret Service, or Android
  secure storage. The plaintext development file store is rejected in release
  builds.
- Native course reads use a bounded 64 MiB, backend-verified user/workspace
  partitioned cache: 15 minutes fresh and up to seven days stale only when the
  network/server is unavailable **after identity has been verified in the same
  running session**. Cold-start offline bootstrap is not implemented, so the
  app does not claim offline sign-in after a restart. Billing, authentication,
  roles, admin data,
  grades, submissions, attendance, notifications, exports, and all mutations
  remain online-only. No mutation is queued until its backend endpoint has an
  idempotency and conflict contract.
- Android cache, preferences, development-store paths, and app-private exports
  resolve from `Context.getCacheDir()` / `Context.getFilesDir()` rather than a
  desktop-style `HOME`. Android release acceptance must still verify cold/warm
  startup, write/read, cache eviction, upgrade, and uninstall behavior on a
  physical device.
- The shared file input uses Dioxus `FileData`. Desktop and Android WebView
  file choosers stream selected bytes to presigned storage with progress rather
  than buffering the full file. Windows/macOS/Linux exports use an OS save
  dialog. Android writes a bounded app-private export and shares it through a
  checked-in host adapter backed by a read-only, non-exported content provider.
- Billing checkout/portal URLs open in the system browser and are restricted to
  configured Stripe hosts. Learning-content and SSO browser handoffs have
  separate host allowlists.
- SSO/LTI callback URLs are length-bounded, host/scheme checked, open-redirect
  checked, queued across native activation, stored in the OS credential store,
  and completed through `/v1/me`. Custom schemes and production App/Universal
  Link metadata are present.
- Android push has a checked-in Kotlin FCM service, environment-specific
  Firebase initialization, Android 13+ permission handling, foreground message
  presentation, notification channels, tap routing, secure token persistence,
  old-token revocation on rotation, backend registration, and cold-service
  token handoff. No `google-services.json` or server credential is embedded.
- Sign-out starts a best-effort device-token DELETE while the bearer is still
  valid, clears the local raw token immediately, and never blocks navigation.
  The Android host also requests deletion of the FCM installation token and
  clears any pending cold-service token. If the device is offline, the backend
  record can remain until provider/server invalidation.
- Android cold/warm HTTPS App Links, the private `aulalite://auth` scheme, and
  notification intents are forwarded through JNI into bounded Rust queues.
  Untrusted hosts, external notification URLs, control characters, and open
  redirects are rejected before router navigation.
- App metadata and multi-resolution PNG, ICO, and ICNS icons are present. CI
  checks both native host shells and the static-asset packaging contract.

## Platform configuration

Native shells load the repository `.env` for local development without
overwriting variables already supplied by the process:

```dotenv
FIREBASE_WEB_API_KEY=your-publishable-firebase-web-api-key
FIREBASE_PROJECT_ID=your-firebase-project
FIREBASE_MESSAGING_SENDER_ID=your-numeric-sender-id
FIREBASE_ANDROID_APP_ID=1:sender-id:android:app-id
AULALITE_API_BASE_URL=http://localhost:8080
AULALITE_TOKEN_STORE=auto
AULALITE_APP_LINK_HOST=aula.elementors.guru
AULALITE_BILLING_HOSTS=checkout.stripe.com,billing.stripe.com
AULALITE_CONTENT_HOSTS=aula.elementors.guru,storage.elementors.guru
```

The Rust debug fallback resolves Android's emulator host as
`http://10.0.2.2:8080`, but the checked-in production Android manifest blocks
all cleartext traffic. Android device/emulator work therefore needs an HTTPS
development origin (for example, a local TLS proxy or tunnel). Every release
build must receive a bare `https://` API origin through its protected build
environment; installed release apps ignore runtime environment and `.env`
overrides. Firebase's API
key, project ID, sender ID, and Android app ID are publishable client
configuration, not server credentials, but they are still managed per
environment. Do not put Firebase service-account JSON, Stripe secret keys, or
other backend credentials in an app bundle.

Keep the Dioxus CLI exactly aligned with the workspace's Dioxus 0.7.9 pins; it
has not been downgraded. Android is the current native release target. Its
candidate is built only after the matching web/backend quality gate passes:

```bash
cargo install dioxus-cli --version 0.7.9 --locked
dx bundle --package shell-mobile --platform android \
  --target aarch64-linux-android --release \
  --package-types aab --locked
```

The checked-in metadata targets Android 16 / API 36 and a minimum SDK of 26,
with exact notification/media permissions, verified-link filters, version,
identifier, and icons. Signing identities and secrets stay in CI/store systems.

## Exact parity and verification status

Android is the current native release-priority client. The repository produces
a locally verified release AAB, but no client is described here as fully
field-verified. “Implemented” below means the repository path exists and is
covered by focused checks; it does not mean a signed artifact has passed
real-device acceptance.

| Workflow | Implemented in repository | Still gated before release |
|---|---|---|
| Core auth, workspace/role routes, courses, schedules, assignments and admin | Shared route graph, native Firebase REST auth, secure credentials and workspace refresh | Android role-matrix E2E against staging |
| File pick/upload | Renderer-neutral chooser and streaming presigned upload with validation/progress | Large-file, cancellation, backgrounding and content-provider tests on physical Android devices |
| Export/download/save/share | Authenticated native export helper; Android app-private atomic writes; secure read-only content provider and share sheet; calendar, attendance, gradebook, privacy and recovery-code paths migrated | Physical-device chooser/share-target and large-file/cancellation verification |
| Live classroom | Android camera/microphone permission, prejoin/device selection, WHIP publish, WHEP/HLS playback, authenticated reconnecting room socket, chat/presence/hands/polls/breakouts, whiteboard drawing/text/shapes/cursors/export, and connection stats implemented in code | Staging TURN/TLS/CORS and codec verification; physical-device teacher/student sessions, permissions, Bluetooth/audio routing, stylus/touch, interruption/background tests. Background effects, screen sharing, and system PiP are deliberately outside the Android v0.1 capability set and are hidden/disabled rather than presented as unfinished controls. |
| Push notifications | Kotlin FCM service, runtime permission, channels, foreground display, background/cold token persistence, secure rotation state, backend register/remove, provider-token deletion, and safe tap routes | Firebase production Android app values plus physical-device foreground/background/tap/denial/rotation tests |
| SSO/LTI/deep links | Strict parser/queue, secure callback session, route completion, exact HTTPS/custom-scheme manifest filters, cold/warm Android intent delivery, and external browser handoff | Publish `assetlinks.json`, configure IdP/LTI redirect URIs, and verify warm/cold activation on a production-domain device build |
| Offline/cache/sync | Safe course reads use a bounded stable-principal cache with fresh/stale policy during same-process network degradation; sensitive reads/mutations fail online | Secure backend-verified cold-start identity bootstrap, product decision and backend idempotency/conflict support before any mutation queue, and airplane-mode device suite |

Browser-specific implementations remain behind target guards where Android has
an equivalent native path or the capability is explicitly absent from the v0.1
Android product surface. Do not infer a gap from a target guard alone; verify
the adjacent native implementation and capability policy.

## Store and distribution gates

The repository cannot complete these external Android release gates by itself:

- Android package ownership, upload key custody, Play App Signing, Play Console
  declarations/listing, and track rollout.
- Production API/Firebase values and a Firebase Android app registered for
  `guru.elementors.aulalite`.
- `https://aula.elementors.guru/.well-known/assetlinks.json`, IdP/LTI redirect
  approval, privacy/support URLs, screenshots, and release ownership.
- Signed-artifact testing on the supported physical-device matrix.

The checked-in CI workflow builds a hashed unsigned AAB from a validated
production/staging environment. Do not call a candidate field-released until a
signed artifact has passed the physical-device acceptance matrix.
