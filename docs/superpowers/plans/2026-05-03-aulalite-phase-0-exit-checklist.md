# Phase 0 Exit Checklist

Run these checks in order from the repository root. Phase 0 is complete only
when every required item passes.

## 1. Stack Health

- [ ] `docker compose up -d`
- [ ] `docker compose ps` shows Postgres, Redis, MinIO, backend, and MediaMTX running; Postgres, Redis, and MinIO must be healthy.
- [ ] `curl http://localhost:8080/healthz` returns `ok`.
- [ ] `curl http://localhost:9997/v3/paths/list` returns MediaMTX JSON.
- [ ] `curl http://localhost:9000/minio/health/live` returns HTTP 200.

## 2. Database Migrations

Use the host URL when running tools outside Docker:

```bash
export DATABASE_URL=postgres://aulalite:changeme@localhost:55432/aulalite
```

- [ ] `sqlx migrate info --source migrations` shows all migrations applied.
- [ ] `psql "$DATABASE_URL" -c "\dt"` shows `tenants`, `users`, and `tenant_memberships`.

## 3. Automated Verification

- [ ] `cargo test -p backend --test rls_tenant_isolation`
- [ ] `cargo test --workspace`
- [ ] `cargo build -p shell-web --target wasm32-unknown-unknown`
- [ ] `dx build --platform web --package shell-web`
- [ ] `cargo check -p shell-mobile --target aarch64-linux-android`

The Android Rust target can be installed with:

```bash
rustup target add aarch64-linux-android
```

## 4. Web Sign-Up And JIT Provisioning

- [ ] Start the web shell with `dx serve --platform web --package shell-web --port 3000`.
- [ ] Open `http://localhost:3000` in a browser.
- [ ] Click `Create account` and register a new user with a password of at least 8 characters.
- [ ] Confirm the dashboard appears with a truncated ID token.
- [ ] Query the database and confirm the new user exists:

```bash
psql "$DATABASE_URL" -c "SELECT email, firebase_uid FROM users ORDER BY created_at DESC LIMIT 1;"
```

## 5. `/v1/me` Before Tenant Assignment

- [ ] Call `/v1/me` with the full Firebase ID token:

```bash
curl -H "Authorization: Bearer <token>" http://localhost:8080/v1/me
```

- [ ] Confirm `tenant_id` is `null`.

## 6. Tenant Provisioning

- [ ] Provision a tenant for the signed-up user:

```bash
cargo run -p aulalite-admin -- create-tenant --slug demo --name "Demo Org" --admin-email <signup-email>
```

- [ ] Refresh the ID token by signing out and signing back in.
- [ ] Call `/v1/me` again and confirm `tenant_id` is non-null and `tenant_role` is `org_admin`.

## 7. Platform Admin CLI

- [ ] Promote an existing user:

```bash
cargo run -p aulalite-admin -- promote-platform-admin --email <email>
```

- [ ] Confirm `users.is_platform_admin` is `true` for that email.

## 8. Mobile Render Checks

- [ ] `cargo check -p shell-mobile --target aarch64-linux-android` passes.
- [ ] `dx serve --platform android --package shell-mobile` launches the emulator and shows the AulaLite login screen.
- [ ] If available on a macOS host, `dx serve --platform ios --package shell-mobile` launches the iOS simulator and shows the AulaLite login screen.

Mobile native Firebase Auth is intentionally out of Phase 0. Sign-in can fail
in native mobile shells until Phase 2 native Firebase bindings are implemented.

## Known Environment Gates

- Real Firebase web sign-up requires valid Firebase config values in `crates/shell-web/index.html` or an equivalent runtime injection path.
- Android emulator/build execution requires `ANDROID_NDK_HOME` pointing to an installed Android NDK.
- iOS simulator execution requires macOS with the iOS Rust target and Xcode tooling.

## Completion Tag

Only after every required check above passes:

```bash
git tag phase-0-complete
```
