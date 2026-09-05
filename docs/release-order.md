# AulaLite release order and acceptance gates

A release moves through one direction only:

1. **Web SaaS** — frontend and backend OCI images
2. **Android** — unsigned AAB candidate, then signed Play candidate
3. **Windows desktop** — unsigned NSIS candidate, then signed installer
4. **iOS** — simulator candidate, then signed TestFlight candidate

`.github/workflows/release-candidate.yml` enforces this order with job
dependencies. A manual run can stop after any platform, but it cannot build a
later platform without successfully rebuilding every earlier gate from the
same commit. Candidate artifacts include SHA-256 checksums and expire after 30
days. The candidate SemVer may add an `-rc.N` or `+metadata` suffix, but its
base version must match `[workspace.package].version` so bundle metadata cannot
silently disagree with the release name.

Both GitHub workflows are manual-only. Normal pushes are handled directly by
Dokploy's GitHub App and consume zero GitHub Actions minutes. Run
`pwsh -File ops/preflight.ps1` locally before a production push; start the
manual workflows only when runner credit is intentionally available.

## Configuration contract

Local development may continue to load the ignored repository `.env`. Release
automation cannot read a developer's local file and instead uses protected
GitHub environments:

- `android-staging` / `android-production`
- `windows-staging` / `windows-production`
- `ios-staging` / `ios-production`

Each environment must define:

| Type | Name | Requirement |
|---|---|---|
| Variable | `AULALITE_API_BASE_URL` | Absolute `https://` production or staging API origin |
| Secret | `FIREBASE_WEB_API_KEY` | Publishable Firebase client key for the matching environment |

The web OCI image receives Firebase's publishable configuration at container
startup through `runtime-config.js`; it is not compiled into the WASM bundle.
Backend-only credentials (database, Stripe secret, signing keys, service
accounts) must never be placed in a frontend or native client environment.

## Gate definitions

### 1. Web SaaS

- The local release preflight passes, including the Dioxus release build; if
  runner credit is intentionally available, the equivalent manual CI workflow
  also passes.
- Frontend and backend containers build reproducibly from pinned base images.
- Migrations have been rehearsed against a restored staging backup.
- Health, readiness, authentication, tenant isolation, billing webhooks, live
  classroom publish/playback, and rollback are exercised in staging.
- Accessibility, responsive layouts, PWA install/update, cache invalidation,
  and primary role journeys pass browser E2E tests.

### 2. Android

- The same release commit passes the web gate first.
- Camera/microphone, playback, live-room socket recovery, whiteboard, file
  pick/share, notifications, deep links, and offline recovery pass on a real
  low/mid-range phone and a current emulator.
- The AAB is signed outside the candidate workflow with the protected upload
  key, then verified by Play App Signing and a closed test track.
- The bundle targets Android 16 / API 36 so it satisfies Google Play's
  August 31, 2026 new-app and update requirement.
- Permission prompts and Play data-safety declarations match actual behavior.

### 3. Windows desktop

- The same release commit passes web and Android first.
- WebView2 bootstrap, camera/microphone/screen-share permissions, downloads,
  deep links, external browser handoff, offline recovery, and upgrade/uninstall
  pass on supported Windows versions.
- The installer and executable are Authenticode-signed and timestamped before
  distribution; SmartScreen and clean-VM installation are release gates.

### 4. iOS

- The same release commit passes web, Android, and Windows first.
- Camera/microphone, ReplayKit screen sharing where enabled, playback,
  whiteboard, document picker/share sheet, APNs, universal links, and offline
  recovery pass on real devices.
- A macOS/Xcode runner builds with the production provisioning profile,
  entitlements, privacy manifest, and distribution certificate. TestFlight
  review and device smoke testing are mandatory before App Store submission.

## What “finalized” means

Compiling or producing an unsigned candidate is not a finalized application.
A platform is finalized only after its signed artifact passes the real-device,
staging, security, store-metadata, privacy, rollback, and distribution checks
above. Signing and store upload are intentionally separate, human-approved
steps; release credentials are never committed to this repository.
