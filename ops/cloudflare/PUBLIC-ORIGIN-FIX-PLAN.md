<!-- Generated 2026-09-02 during the aula.elementors.guru outage repair.
     Phase D (host durability) is ALREADY APPLIED. Phases A/B/C/E are NOT - they
     are gated on Firebase-console and Cloudflare-dashboard access. -->

# AULALITE PUBLIC-ORIGIN FIX PLAN — ordered, deduplicated, executable

All `.env` line numbers below were re-verified by me against the live file at
`C:\Users\admin\Downloads\elementors-aulalite-main\elementors-aulalite-main\.env` (the audit streams disagreed on several; mine are authoritative). Match by key name anyway, not by line number.

Run every `docker compose` command from the repo root
`C:\Users\admin\Downloads\elementors-aulalite-main\elementors-aulalite-main` so that `docker-compose.yml` + `docker-compose.override.yml` both apply. Never add `-f docker-compose.cloudflare.yml`.

---

## 0. STANDING RULES AND TRAPS (read before executing anything)

**T1. Never set `APP_ENV=production`.** Four independent boot guards would `bail!` and crash-loop the backend, taking the whole site (including login, which proxies through nginx to backend:8080) from working-but-insecure to hard 502: `AULALITE_ADMIN_ORIGIN` empty (.env:77), `JWT_RS256_PRIVATE_KEY_PEM` empty (.env:125), default object-store creds `aulalite`/`changeme123` (.env:119-120), non-HTTPS browser origins (.env:13), plus the DB role being `aulalite` not `aulalite_app`. `APP_ENV=local` is required for this deployment to run at all. Two audit streams independently proposed flipping it as a "verification step" — do not.

**T2. `.env` is interpolated at container CREATE time.** `docker compose restart` will NOT pick up a changed value. Every `.env` edit must be applied with `docker compose -p aulalite up -d <services>`. This matters most for `mediamtx`, whose CORS and ICE settings come from compose env injection.

**T3. After ANY `up -d backend` or `up -d mediamtx`, immediately run `docker compose -p aulalite restart frontend`.** nginx resolves `backend` once at config load (`crates/shell-web/Dockerfile:162` and `:179`, literal `proxy_pass http://backend:8080;`, no `resolver` directive). Recreating the backend can hand it a new 172.23.0.x address, leaving nginx pointed at a dead IP: the SPA still serves perfectly and every `/v1/*` call 502s. The frontend healthcheck (`wget --spider http://127.0.0.1:3000/`) only tests the static root, so this failure is invisible and does not self-heal. Verify after every recreate:
`curl -s -o /dev/null -w '%{http_code}\n' https://aula.elementors.guru/v1/me` must be **401**, not 502.
The permanent fix is F1 below.

**T4. Hard ordering constraint.** Firebase authorized-domain (A1) MUST precede the `APP_ORIGIN` flip (A5). `crates/backend/src/services/invitations.rs:52-77` sends `sendOobCode` with `requestType=EMAIL_SIGNIN` and `continueUrl={app_origin}/accept-invite`; that call returns **400 UNAUTHORIZED_DOMAIN** for `https://aula.elementors.guru` today and 200 for `http://localhost:3000`. Reversing the order turns invites from "sends a dead link" into "every invite endpoint 500s after committing the invitation row" (`member_invitations.rs:231-239`, `enrollments.rs:748-760`, `parent.rs:589`, `platform.rs:352,487`).

**T5. Hard ordering constraint.** You MUST have a working, email-verified Firebase admin account (A4) before disabling the local-login bypass (B1). The only real Firebase identity in `users` today is `shivamkumardb53@gmail.com` — an unrelated third party. Disabling the bypass first locks you out completely.

**T6. `crates/backend/src/auth/super_admin.rs` re-promotes on every boot and never demotes.** The demotion SQL (B3) must run AFTER the `.env` change to `AULALITE_SUPER_ADMIN_EMAILS` AND after the backend restart, or `main.rs:215 promote_existing()` silently re-promotes `local.teacher@example.test`.

**T7. Never publish rustfs port 9001** (the admin console, `RUSTFS_CONSOLE_ENABLE: "true"`, `RUSTFS_CONSOLE_CORS_ALLOWED_ORIGINS: *`). Only 9000.

---

## 1. ACTIONS THAT REQUIRE THE HUMAN (no credentials on this machine)

| id | where | action |
|----|-------|--------|
| **A1** | Firebase console, project `elementors-aulalite` | Authentication → Settings → Authorized domains → Add `aula.elementors.guru` |
| **A2** | Firebase console → Authentication → Users | Delete accounts with "Email verified: No" (orphans created by the broken signup path) |
| **A3** | Cloudflare dashboard, zone `elementors.guru` | SSL/TLS → Edge Certificates → **Always Use HTTPS = On** |
| **C2** | Cloudflare dashboard, tunnel `2d265905-d028-4c0c-9658-41e2d711c8e1` | Add public hostname `storage.elementors.guru` → `http://localhost:9000` (+ proxied CNAME). **Do C1 first.** |
| **E1** | same tunnel | Add `stream.elementors.guru` → `http://localhost:8889` (WebRTC/WHIP/WHEP) and `live.elementors.guru` → `http://localhost:8888` (HLS) (+ proxied CNAMEs) |
| **D2** | decision + console | Enable auto-logon for `DESKTOP-0O4MB9E\admin` (Sysinternals `Autologon64.exe`, so the password becomes an LSA secret) **or** accept "a human must sign in after every reboot" |
| **H1** | Cloudflare → Caching → Cache Rules | `http.host eq "aula.elementors.guru" and starts_with(http.request.uri.path, "/assets/")` → Eligible for cache, Edge TTL: use cache-control header |
| **H2** | Cloudflare → Analytics → Web Analytics | Disable Automatic Setup for `aula.elementors.guru` |

**Origin URL for every tunnel hostname is `http://localhost:<port>`, NOT `http://<service>:<port>`.** cloudflared runs as the Windows service `Cloudflared` on the host, outside the compose network. `ops/cloudflare/config.yml.template` says `mediamtx:8888` — that template is not in use and would be wrong here.
**Do not put Cloudflare Access in front of any of these three new hostnames** — a presigned SigV4 fetch and a WHEP POST cannot complete an Access redirect.

---

## PHASE A — a remote user can load, sign up and sign in

### A1 [HUMAN] Add `aula.elementors.guru` to Firebase authorized domains
**Why:** signup is 100% dead on the public host today. `crates/shell-web/public/assets/firebase-bridge.js:38-43` creates the account and then calls `sendEmailVerification(user, { url: window.location.origin + '/signup' })`. Firebase validates that continue URL against `authorizedDomains` at **send** time (proven live: `sendOobCode` with a `https://aula.elementors.guru/...` continueUrl → `400 UNAUTHORIZED_DOMAIN`; identical call with `http://localhost:3000/...` → 200). The account is created, the send throws, no verification email is ever sent, and `crates/backend/src/auth/jit_provision.rs:283-285` then permanently refuses the unverified identity. There is no resend path anywhere in the app (`sendEmailVerification` appears exactly once, inside `signUp()`).
Current list, verified live: `["localhost","elementors-aulalite.firebaseapp.com","elementors-aulalite.web.app"]`.

**Verify:**
```
curl -s "https://identitytoolkit.googleapis.com/v1/projects?key=<FIREBASE_WEB_API_KEY>"
curl -s -X POST "https://identitytoolkit.googleapis.com/v1/accounts:sendOobCode?key=<FIREBASE_WEB_API_KEY>" \
  -H 'Content-Type: application/json' \
  -d '{"requestType":"EMAIL_SIGNIN","email":"probe@example.invalid","continueUrl":"https://aula.elementors.guru/accept-invite","canHandleCodeInApp":true}'
```
First must list `aula.elementors.guru`; second must stop returning `UNAUTHORIZED_DOMAIN`.
*(Note: a `PASSWORD_RESET` probe is useless for testing — email-enumeration protection returns a canned success before continueUrl validation.)*

**Correction to one audit stream:** this app has **no** Google/OAuth sign-in (`grep` for `signInWithPopup|signInWithRedirect|GoogleAuthProvider` across `crates/` returns zero hits). Email+password **sign-in** works on the public host right now and is unaffected. A1 fixes signup and invites only.

### A2 [HUMAN] Delete unverified orphan Firebase users
Every prior signup attempt over the tunnel left a real, unverifiable account. After A1 they cannot self-recover (retry → `EMAIL_EXISTS`; sign-in → blocked by the backend's verification gate). Delete them so those people can sign up again.

### A3 [HUMAN] Cloudflare "Always Use HTTPS"
**Why:** `curl -D - http://aula.elementors.guru/` returns 200 with the full SPA in cleartext, and `http://aula.elementors.guru/v1/me` returns a real backend 401 in cleartext — no redirect. In a non-secure context `getUserMedia` and service-worker registration do not exist, so live class silently cannot start. Narrow window in practice (modern browsers auto-upgrade typed navigations, and HSTS pins repeat visitors), but it is a one-click, zero-risk fix.
**Verify:** `curl -s -o /dev/null -w '%{http_code} %{redirect_url}\n' http://aula.elementors.guru/` → `301 https://aula.elementors.guru/`

### A4 [ORCH + HUMAN] Create the operator's real admin account — **gate for Phase B**
After A1 is verified, sign up at `https://aula.elementors.guru/signup` with the operator's own email and complete email verification.
**Verify the row landed:**
```
docker exec aulalite-postgres-1 psql -U aulalite -d aulalite -tAc "select email, firebase_uid, created_at from users order by created_at desc limit 3"
```
**Fallback if signup still fails:** sign up, then use "Forgot password" on the same address — `firebase-bridge.js:69-72` calls `sendPasswordResetEmail` with no ActionCodeSettings, so it uses the always-allowlisted `elementors-aulalite.firebaseapp.com` handler, and completing a reset marks the email verified. This path demonstrably worked once already.

### A5 [ORCH] `.env` edit round 1 — origin sweep

| line | old | new |
|------|-----|-----|
| 11 | `APP_ORIGIN=http://localhost:3000` | `APP_ORIGIN=https://aula.elementors.guru` |
| 12 | `API_ORIGIN=http://localhost:8080` | `API_ORIGIN=https://aula.elementors.guru` |
| 13 | `AULALITE_BROWSER_ORIGINS=http://localhost:3000` | `AULALITE_BROWSER_ORIGINS=https://aula.elementors.guru,http://localhost:3000` |
| 16 | `AULALITE_SUPER_ADMIN_EMAILS=local.teacher@example.test` | `AULALITE_SUPER_ADMIN_EMAILS=<the A4 email>` |
| 76 | `AULALITE_APP_ORIGIN=http://localhost:3000` | `AULALITE_APP_ORIGIN=https://aula.elementors.guru` |
| 147 | `STRIPE_SUCCESS_URL=http://localhost:3000/admin/billing?status=success` | `STRIPE_SUCCESS_URL=https://aula.elementors.guru/admin/billing?status=success` |
| 148 | `STRIPE_CANCEL_URL=http://localhost:3000/admin/billing?status=cancel` | `STRIPE_CANCEL_URL=https://aula.elementors.guru/admin/billing?status=cancel` |

Leave **`.env:75 AULALITE_API_BASE_URL=` empty** and **`.env:77 AULALITE_ADMIN_ORIGIN=` empty**. Empty API base is what keeps every SPA call same-origin and preflight-free (`crates/features-courses/src/api.rs:142-145,186` + nginx `location /v1/` at `Dockerfile:149-163`).
Leave `.env:84 AULALITE_CONTENT_HOSTS=aula.elementors.guru,storage.elementors.guru` **as-is** — it already names the hostname C2 creates. (One stream flagged it as wrong; it is correct once C2 exists.)

**Why:** `main.rs:160` reads `APP_ORIGIN` and it is formatted into every generated link — `member_invitations.rs:238`, `enrollments.rs:752-756` (the single-use invite token rides in the dead URL), `parent.rs:589`, `platform.rs:352,487`, `announcements.rs:234-238`, `discussions.rs:429-433`, `certificates.rs:289`, `bulk.rs:465-469`, `calendar.rs:189-190` (baked into the served `.ics`), `sso.rs:69-70` (OAuth `redirect_uri`), `lti.rs:283`. `billing.rs:280-284` prefers the explicit `STRIPE_*_URL` env over the derived default, so .env:147-148 must be changed too.

**Conflict resolved (comma list vs single value):** two streams said replace with the single public origin, two said keep localhost. Use the **comma list**. `crates/backend/src/lib.rs:312-341` splits on `,` and de-dupes; `docker-compose.yml:257-258` documents list-valued overrides. Keeping `http://localhost:3000` preserves local dev browser uploads and local live rooms at zero cost, and is harmless while `APP_ENV=local` (the https-only assertion at `main.rs:197-205` is production-gated).

### A6 [ORCH] Apply and verify
```
docker compose -p aulalite up -d backend frontend mediamtx
docker compose -p aulalite restart frontend          # T3
```
`frontend` is required because `crates/shell-web/docker-entrypoint.d/40-aulalite-runtime-config.sh:10,26` regenerates `/runtime-config.js` at container start. `mediamtx` is required because `docker-compose.yml:259-260` feed `AULALITE_BROWSER_ORIGINS` into `MTX_WEBRTCALLOWORIGINS`/`MTX_HLSALLOWORIGINS`.

**Verify all four:**
```
curl -s -o /dev/null -w '%{http_code}\n' https://aula.elementors.guru/v1/me                       # 401, NOT 502
curl -s https://aula.elementors.guru/runtime-config.js | grep APP_ORIGIN                           # base64-decode -> https://aula.elementors.guru
curl -s -D- -o /dev/null https://aula.elementors.guru/v1/me -H 'Origin: https://aula.elementors.guru' | grep -i access-control-allow-origin
docker inspect aulalite-mediamtx-1 --format '{{range .Config.Env}}{{println .}}{{end}}' | grep ALLOWORIGINS
```
Then sign in with the A4 account and confirm `/v1/me` returns `"is_platform_admin":true`. **Do not proceed to Phase B until this passes.**

---

## PHASE B — close the public authentication bypass (do this immediately after Phase A; do not defer)

This is categorised as "security hardening" by the requested ordering, but it is a **live, exercised, full remote platform takeover** and its only dependency is Phase A. Run it as soon as A6 verifies.

**What is live right now, proven end-to-end by two independent streams:**
`POST https://aula.elementors.guru/v1/auth/local-login` → 401 `{"error":"invalid credentials"}` (route registered, not 404).
`GET /v1/me` with `Bearer <LOCAL_LOGIN_TEACHER_TOKEN>` → 200 `{"email":"local.teacher@example.test","tenant_role":"org_owner","is_platform_admin":true}`.
`GET /v1/platform/tenants` with that token → 200, listing both `aulalite-demo` and the real third-party tenant `shivamkumardb53's academy`.
The public login form calls this endpoint first for every credential typed (`crates/features-auth/src/login.rs:300,386`). `local_login.rs:186-189` mints `email_verified: Some(true)`, bypassing the verification gate. The only brute-force control is a 250 ms delay (`dev_login.rs:60`).
**Both tokens have now been transmitted over the public internet during this audit. Treat them as burned.**

### B1 [ORCH] `.env` edit round 2

| line | old | new |
|------|-----|-----|
| 95 | `LOCAL_LOGIN_BYPASS_ENABLED=true` | `LOCAL_LOGIN_BYPASS_ENABLED=false` |
| 97 | `LOCAL_LOGIN_TEACHER_TOKEN=24fb97ba79...` | `LOCAL_LOGIN_TEACHER_TOKEN=` |
| 101 | `LOCAL_LOGIN_STUDENT_TOKEN=ad31e3a5e5...` | `LOCAL_LOGIN_STUDENT_TOKEN=` |
| 105 | `LOCAL_LOGIN_TEACHER_PASSWORD=<LOCAL_LOGIN_TEACHER_PASSWORD>` | `LOCAL_LOGIN_TEACHER_PASSWORD=` |
| 106 | `LOCAL_LOGIN_STUDENT_PASSWORD=<LOCAL_LOGIN_STUDENT_PASSWORD>` | `LOCAL_LOGIN_STUDENT_PASSWORD=` |
| 198 | `LOCAL_LOGIN_EXTRA_PROFILES=[{...3 profiles with plaintext passwords...}]` | `LOCAL_LOGIN_EXTRA_PROFILES=` |

Line 95 alone is technically sufficient — `local_login.rs:87-88 is_enabled()` short-circuits, and `profile_for_token()` at `:124-127` also short-circuits, so the already-issued static tokens stop working too. The blanking is a separate, mandatory credential-burn step, not belt-and-braces.

### B2 [ORCH] Apply
```
docker compose -p aulalite up -d backend
docker compose -p aulalite restart frontend
```

### B3 [ORCH] Revoke the existing bypass identities — **only after B2 completes** (see T6)
```
docker exec aulalite-postgres-1 psql -U aulalite -d aulalite -c "UPDATE users SET tokens_valid_after = now(), is_platform_admin = false WHERE email IN ('local.teacher@example.test','local.student@example.test','teacher2@example.test','student2@example.test','parent1@example.test');"
```
(Both columns verified to exist via `information_schema`.)

### B4 Verify
```
curl -s -o /dev/null -w '%{http_code}\n' -X POST https://aula.elementors.guru/v1/auth/local-login -H 'Content-Type: application/json' -d '{"email":"x@y.z","password":"y"}'    # must be 404
curl -s -o /dev/null -w '%{http_code}\n' https://aula.elementors.guru/v1/me -H 'Authorization: Bearer <LOCAL_LOGIN_TEACHER_TOKEN>'                        # must be 401
curl -s -o /dev/null -w '%{http_code}\n' https://aula.elementors.guru/v1/me                                                                                                    # must be 401, not 502
```
Then re-confirm the A4 account can still sign in and still shows `is_platform_admin: true`.

*(Already checked and clean — do not re-litigate: `/v1/dev/audit-seed` returns 404, gated by `LOCAL_AUDIT_SEED_ENABLED=false` at .env:107. `/metrics` is not proxied by nginx and returns the SPA shell. The backend sets no cookies at all, so there is no SameSite/Secure/Domain issue; auth is bearer-header only, which is also why the absence of CSRF/Origin checks is correct-by-construction.)*

---

## PHASE C — files, recordings, uploads and SCORM work for a remote user

**What is broken:** every presigned object-store URL is signed for `http://localhost:9000` — the *viewer's* machine. Confirmed with a live response body through the tunnel: `playback_url":"http://localhost:9000/aulalite/.../recording.mp4?X-Amz-Algorithm=AWS4-HMAC-SHA256&..."`. The host is inside the SigV4 signature (`object_store.rs:64-70` builds a separate presign client on `public_endpoint_url`; `main.rs:274` passes `S3_ENDPOINT_URL` there), so it cannot be rewritten client-side. Uploads fail **deterministically before a single byte leaves the browser**: `crates/features-courses/src/file_picker.rs:347-365 validate_presigned_put_url()` rejects non-HTTPS unless `cfg!(debug_assertions)`, and the shipped WASM is a release build — proven from the artifact itself (`strings` on the deployed `.wasm` contains `"The upload destination must use HTTPS"` and zero occurrences of the `10.0.2.2` debug-only escape-hatch literal). Reads are dead too: the served CSP has `img-src 'self' data: blob: https:` and `media-src 'self' blob: https:` — no `http:`.
Good news: `https:` is already permitted in `connect-src`/`media-src`/`img-src`, so **no CSP change is needed**.
A path prefix under `aula.elementors.guru` **cannot** substitute for a separate hostname: `force_path_style(true)` signs the full path, and any nginx prefix rewrite breaks the SigV4 canonical URI → 403.

### C1 [ORCH] Rotate the object-store root credentials — **BEFORE C2, non-negotiable**
`.env:119 AWS_ACCESS_KEY_ID=aulalite`, `.env:120 AWS_SECRET_ACCESS_KEY=changeme123` are the shipped defaults, and `docker-compose.yml:55-56` feed them straight into `RUSTFS_ACCESS_KEY`/`RUSTFS_SECRET_KEY` — these are RustFS's **root** credentials, not a scoped app user. The key id already leaks in plaintext in every presigned URL (`X-Amz-Credential=aulalite%2F...`), so an attacker starts with half the pair. Publishing port 9000 without rotating hands full read/write/delete of every tenant's recordings, submissions and attachments to the internet. (`main.rs:236-251` refuses to boot on exactly these strings in production; that guard is dormant at `APP_ENV=local`.)

New values, both changed together:
```
AWS_ACCESS_KEY_ID=$(openssl rand -hex 16)
AWS_SECRET_ACCESS_KEY=$(openssl rand -base64 32)
```
Apply: `docker compose -p aulalite up -d rustfs backend && docker compose -p aulalite restart frontend`
(The backend retries `ensure_bucket` for ~60 s, `main.rs:286-303`, so a brief rustfs restart is tolerated. Outstanding presigned URLs are invalidated immediately — `X-Amz-Expires=900`, so a 15-minute blast radius.)

**RISK — the one action in this plan with a data-access failure mode, and the streams disagree.** One stream asserts a straight recreate is fine; another warns that RustFS may hold the original root credentials in its persisted volume metadata, in which case rotating leaves the bucket unreadable. **Verify immediately after the recreate, before proceeding:**
```
docker logs --tail 50 aulalite-backend-1 | grep -iE 'bucket|s3|ensure_bucket'
curl -s http://localhost:18080/readyz          # {"status":"ready","dependencies":{...,"media":"ok"}}
```
If `media` is not `ok`, revert .env:119-120 to `aulalite`/`changeme123`, recreate, and escalate — do **not** proceed to C2 with default creds.

Independent of the tunnel work, these defaults should be rotated anyway: ports 9000/9001 are bound to `0.0.0.0` and reachable from the whole LAN today.

### C2 [HUMAN] Tunnel hostname `storage.elementors.guru` → `http://localhost:9000`
Only 9000. Never 9001. No Cloudflare Access.
**Verify:** `curl -sI https://storage.elementors.guru/aulalite` returns a RustFS S3 response (403/404 with an S3 error body), not a Cloudflare 502.

### C3 [ORCH] `.env` edit round 3
| line | old | new |
|------|-----|-----|
| 115 | `S3_ENDPOINT_URL=http://localhost:9000` | `S3_ENDPOINT_URL=https://storage.elementors.guru` |

Leave `.env:116 S3_INTERNAL_ENDPOINT_URL=http://rustfs:9000` **untouched** — it is already correct and already in use (`main.rs:229-230` → `object_store.rs:59`). One stream proposed "adding" it; it exists.
No trailing slash and no path (`main.rs:257` requires `path == "/"` if production is ever enabled).
`AULALITE_BROWSER_ORIGINS` was already fixed in A5 — that is what drives the RustFS **bucket** CORS rule, re-applied on every backend boot (`object_store.rs:208-240 ensure_bucket` → `put_bucket_cors`). Without A5 already applied, a browser PUT from `https://aula.elementors.guru` to `https://storage.elementors.guru` would be CORS-rejected by the bucket even with a correct HTTPS URL.

### C4 [ORCH] Apply and verify
```
docker compose -p aulalite up -d backend
docker compose -p aulalite restart frontend
curl -s https://aula.elementors.guru/v1/sessions/<a real session uuid>/recording -H 'Authorization: Bearer <A4 account token>' | grep -o 'playback_url":"https[^"]*'
curl -sI "<that url>"     # expect 200
```
Then perform one real file upload from a remote browser end-to-end.
**Caveat to communicate:** Cloudflare's proxy caps request bodies at 100 MB, so uploads above that will fail through the tunnel regardless. `client_max_body_size 3m` at `Dockerfile:101` does not apply — presigned PUTs bypass nginx entirely.

---

## PHASE D — survive a reboot, sleep or sign-out unattended

**What actually happened on 9/1 (settled, do not re-investigate):** the host has not rebooted since 2026-08-19. The 45-minute outage was a **full user-session teardown** (Winlogon 7002 "User Logoff Notification", then a brand-new interactive logon at 12:04:46 AM — `quser` shows `admin console 4 Active LOGON TIME 9/2/2026 12:04 AM`, explorer.exe SessionId 4 StartTime 12:04:50 AM). Docker Desktop's `exit status 0x40010004` = `DBG_TERMINATE_PROCESS`, i.e. session teardown. The ~50-second S3 sleep resumed cleanly and was not the killer. Because a fresh logon demonstrably occurred, **D1 alone would have reduced the 45-minute outage to about 60 seconds.**

There is worse precedent: after the 8/19 reboot, Docker's backend died at 14:52:46Z and did not relaunch until 08:59:01Z the next day — **18h06m** of hard 502 while the Cloudflared service came up at boot as normal.

### D1 [ORCH] Enable Docker Desktop AutoStart — highest-value durability fix
**Preferred:** Docker Desktop → Settings → General → tick "Start Docker Desktop when you sign in to your computer" → Apply & restart. The checkbox writes both artefacts consistently.
**If scripting instead**, quit Docker Desktop completely first (currently six `Docker Desktop` processes plus `com.docker.backend` and `com.docker.build` — all must be gone or the exit handler overwrites the JSON), then:
```powershell
$f="$env:APPDATA\Docker\settings-store.json"; $j=Get-Content $f -Raw | ConvertFrom-Json; $j.AutoStart=$true; $j | ConvertTo-Json -Depth 10 | Set-Content $f -Encoding utf8
Set-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run' -Name 'Docker Desktop' -Value ([byte[]](2,0,0,0,0,0,0,0,0,0,0,0))
```
Current state: `settings-store.json` has `"AutoStart": false`; the StartupApproved byte is `01 00 00...` (disabled). `02` is byte-for-byte what `MicrosoftEdgeAutoLaunch`, the one enabled entry on this box, holds.
**Verify:** sign out and back in; `docker ps` shows all six aulalite containers within ~60 s of logon.

### D2 [HUMAN, decision] Auto-logon, or accept the manual dependency
There is **no** `com.docker.service` on this host (`sc query` → error 1060), so there is no system-level path to start the engine without an interactive session. `AutoAdminLogon`/`DefaultUserName`/`ForceAutoLogon` are all empty. Without auto-logon (or moving the origin to an always-on host with the repo's `docker-compose.prod.yml`), **"a human must sign in after a reboot" remains a standing manual dependency** and must be stated plainly rather than papered over. If enabling: Sysinternals `Autologon64.exe` for `DESKTOP-0O4MB9E\admin` so the password becomes an LSA secret rather than a plaintext registry value, paired with `ScreenSaverIsSecure=1`.

### D3 [ORCH, elevated] Stop unattended Windows Update restarts
Windows Update is actively servicing this box (WindowsUpdateClient event 19 "Installation Successful" as recently as 9/2 5:00 AM; `UsoSvc` Running/Automatic), `ActiveHoursStart=8`/`ActiveHoursEnd=17` permits auto-restart any time 17:00-08:00, and there is **no** restraining policy (`HKLM\SOFTWARE\Policies\Microsoft\Windows\WindowsUpdate\AU` does not exist).
```powershell
New-Item 'HKLM:\SOFTWARE\Policies\Microsoft\Windows\WindowsUpdate\AU' -Force | Out-Null
Set-ItemProperty 'HKLM:\SOFTWARE\Policies\Microsoft\Windows\WindowsUpdate\AU' -Name NoAutoUpdate -Type DWord -Value 0
Set-ItemProperty 'HKLM:\SOFTWARE\Policies\Microsoft\Windows\WindowsUpdate\AU' -Name AUOptions -Type DWord -Value 2
Set-ItemProperty 'HKLM:\SOFTWARE\Policies\Microsoft\Windows\WindowsUpdate\AU' -Name NoAutoRebootWithLoggedOnUsers -Type DWord -Value 1
gpupdate /force
```
`AUOptions=2` = notify before download: nothing installs or reboots without a human. **Tradeoff: this defers patching. Accept only if someone applies updates manually on a schedule.** If that is not acceptable, use the weaker variant — widen active hours to the 18-hour maximum (`ActiveHoursStart=6`, `ActiveHoursEnd=23`) under `HKLM:\SOFTWARE\Microsoft\WindowsUpdate\UX\Settings`.

### D4 [ORCH, elevated] Harden the power flyout — cosmetic, apply only after D1
```powershell
Set-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\FlyoutMenuSettings' -Name ShowSleepOption -Type DWord -Value 0
Set-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\FlyoutMenuSettings' -Name ShowHibernateOption -Type DWord -Value 0
```
**Two commands that were proposed and MUST NOT be run:** `powercfg /hibernate off` (hibernate played no part; it only removes hiberfil.sys and Fast Startup) and `powercfg /setacvalueindex ... UIBUTTON_ACTION 2` (a Windows-7-era setting the Win10 Start flyout does not consult). Also do **not** run `New-Item ... FlyoutMenuSettings -Force` — the key already exists with `ShowSleepOption=1, ShowHibernateOption=1, ShowLockOption=1`, and `-Force` risks clearing `ShowLockOption`.
Note this is partial: the actual killer was a **sign-out**, and "Sign out" cannot be removed from the flyout. D1 is the real remedy.

### D5 [ORCH, elevated] Harden the Cloudflared service
Current: `sc qfailure Cloudflared` → `RESET_PERIOD 86400`, exactly **one** action (`RESTART` after 20 s) — and that single action has already been consumed for real (System 7031 at 8/19 8:23:24 PM: "terminated unexpectedly... 1 time(s)", 15 s after boot). A second crash inside 24 h gets nothing.
```
sc.exe failure Cloudflared reset= 86400 actions= restart/20000/restart/60000/restart/120000
sc.exe failureflag Cloudflared 1
sc.exe config Cloudflared start= delayed-auto
```
(Note the space after each `=`.)
**REJECT the proposed "move cloudflared into the compose project" step.** `docker-compose.cloudflare.yml:28` runs `--config /etc/cloudflared/config.yml` and mounts `./ops/cloudflare`, but `ops/cloudflare/config.yml` does not exist (only `config.yml.template`, 3798 bytes) and there is no credentials JSON — the live tunnel is token/remotely-managed. It would additionally require rewriting the command to `run --token`, repointing every dashboard ingress from `localhost` to `frontend:3000`, and it makes the tunnel die whenever Docker dies, trading a 502 for a connection failure with no uptime gain. Do not do it on this host.

### D6 [ORCH] Origin watchdog scheduled task — defence in depth, apply after D1
Nothing on the host watches the origin (`Get-ScheduledTask` filtered on docker|aula|cloud returns nothing). Container restart policies are correct and self-heal (`restart: unless-stopped` on all six; they came back 18 s after the engine relaunched) — the gap is engine death, not container death.
Five corrections to the version that was proposed:
1. Drop `-RepetitionDuration ([TimeSpan]::MaxValue)` (throws on some builds) — omit it, or use `(New-TimeSpan -Days 3650)`.
2. `-LogonType Interactive` means it runs **only while admin is signed in**. State plainly: it does **not** cover the unattended-reboot case. It covers engine crash, manual quit, and WSL hang (SCM 7011 WSLService timeout already fired once).
3. Make it *act*, not just log: `if ($r -ne 200) { docker restart aulalite-frontend-1 *> $null }`.
4. Add a maintenance sentinel as the first line: `if (Test-Path 'C:\ProgramData\aulalite\maintenance') { exit 0 }`.
5. Use `docker compose ... start` before `up -d` — bare `up -d` will **recreate** containers whenever `.env` has drifted, turning the watchdog into an unwanted redeploy:
```powershell
docker compose -f docker-compose.yml -f docker-compose.override.yml start
if (-not (docker ps -q --filter 'name=aulalite-frontend-1' --filter 'status=running')) {
  docker compose -f docker-compose.yml -f docker-compose.override.yml up -d
}
```

---

## PHASE E — live video for remote users

**Honest framing up front:** the URL/DNS/CORS half of this is straightforwardly fixable. The **media transport half is not proven achievable on this topology**, and applying only the URL fix produces a room that negotiates and then stays black — worse than the current honest failure, because it looks like it should work. Four independent blockers stack here; three streams each found a different subset.

**The four blockers, all confirmed:**
1. **URLs.** `MEDIAMTX_PUBLIC_WEBRTC_URL=http://localhost:8889` / `..._HLS_URL=http://localhost:8888` are formatted verbatim into client-facing URLs at `live_sessions.rs:1201-1202` (teacher WHIP), `:1789-1791` and `:1803-1807` (student WHEP + `index.m3u8?jwt=`), `:2422-2426` (WHEP over the live-room socket), `:3543`, and `breakout.rs:69-75`. The frontend does not rewrite them (`live_room_broadcast.rs:2337-2338` passes `main_publish_url` straight through). nginx has no media location. `stream.`/`live.` are NXDOMAIN.
2. **MediaMTX's own CORS.** `webrtcAllowOrigins:["http://localhost:3000"]`, `hlsAllowOrigins:["http://localhost:3000"]` (read live from `http://localhost:9997/v3/config/global/get`). Proven at the port: `OPTIONS http://localhost:8889/aula/whep` with `Origin: https://aula.elementors.guru` → 204 with **no** ACAO; with `Origin: http://localhost:3000` → 204 **with** ACAO. **Already fixed by A5** — but only if `mediamtx` was recreated, not restarted.
3. **ICE.** `webrtcICEServers2: []`, `webrtcAdditionalHosts: ["127.0.0.1"]`, `webrtcLocalTCPAddress: ""`. MediaMTX offers only `127.0.0.1` and its `172.23.0.x` container address, gathers no srflx and no relay candidate of its own, and has no TCP-ICE fallback. The browser's TURN relay candidate has nothing to pair with. `AULALITE_TURN_URL` is handed to **browsers only** (`services/ice.rs:19-30` → `live_sessions.rs:1205`); nothing feeds TURN to MediaMTX.
4. **The TURN URL points at a transport the relay does not answer.** `.env:138 turn:openrelay.metered.ca:443?transport=tcp` = plain TCP on 443. An independently-written raw STUN prober (20-byte header, magic cookie `0x2112A442`, Allocate `0x0003` with REQUESTED-TRANSPORT) got: plain TCP/443 → connected, **no STUN response in 12 s**; TLS TCP/443 → `401 realm="metered.ca"`; plain TCP/80 → `401`; UDP/443 → `401`. Port 443 answers TLS and UDP but is dead to plain TCP. `ice.rs:103-108` validates only the scheme, so this fails **silently**.

### E1 [HUMAN] Two tunnel hostnames (see table in §1)
**Verify DNS resolves to Cloudflare IPs BEFORE E2**, or the backend hands out URLs that NXDOMAIN: `nslookup stream.elementors.guru` and `nslookup live.elementors.guru`.
*(Streams used inconsistent names — one had `live.`=8889, another `hls.`=8888. The port→purpose mapping is what matters: **8889 = WebRTC/WHIP/WHEP, 8888 = HLS.** I standardise on `stream.`=8889 and `live.`=8888; whatever names you pick, keep .env:127/128 consistent with the ingress.)*

### E2 [ORCH] `.env` edit round 4
| line | old | new |
|------|-----|-----|
| 127 | `MEDIAMTX_PUBLIC_WEBRTC_URL=http://localhost:8889` | `MEDIAMTX_PUBLIC_WEBRTC_URL=https://stream.elementors.guru` |
| 128 | `MEDIAMTX_PUBLIC_HLS_URL=http://localhost:8888` | `MEDIAMTX_PUBLIC_HLS_URL=https://live.elementors.guru` |
| 138 | `AULALITE_TURN_URL=turn:openrelay.metered.ca:443?transport=tcp` | `AULALITE_TURN_URL=turns:openrelay.metered.ca:443?transport=tcp,turn:openrelay.metered.ca:80?transport=tcp` |
| new | — | `AULALITE_MEDIAMTX_TURN_URL=turns:openrelay.metered.ca:443?transport=tcp` |

**No trailing slash** on 127/128 — the `format!` strings insert their own `/`.
`.env:139-140` (`openrelayproject`/`openrelayproject`) stay as-is; both are pre-auth-verified only.

**`MEDIAMTX_ADDITIONAL_HOSTS` (.env:131) — conflict resolved: LEAVE IT AT `127.0.0.1`.** Two streams said set it to a public IP/hostname; two said do not. The correct answer is do not: with `webrtcLocalTCPAddress` empty, `additionalHosts` only produces a **UDP/8189 host candidate**, and a Cloudflare Tunnel HTTP ingress forwards no UDP — pointing it at the Cloudflare edge IP creates a dead candidate. This host is `192.168.1.2` behind NAT with no public IP and no port-forward. Changing it only helps if a genuinely reachable public UDP endpoint exists, which it does not today.

### E3 [ORCH] `docker-compose.yml` — give MediaMTX its own relay
Append to the `mediamtx:` environment block after line 260, **preserving the indirection verbatim**:
```yaml
      MTX_WEBRTCICESERVERS2_0_URL: ${AULALITE_MEDIAMTX_TURN_URL:-${AULALITE_TURN_URL}}
      MTX_WEBRTCICESERVERS2_0_USERNAME: ${AULALITE_TURN_USERNAME}
      MTX_WEBRTCICESERVERS2_0_PASSWORD: ${AULALITE_TURN_CREDENTIAL}
      MTX_WEBRTCLOCALTCPADDRESS: ":8189"
```
**CONFLICT — these two actions must be done together or MediaMTX fails to start.** One stream's fix makes `AULALITE_TURN_URL` a comma-separated pair; another's wires `MTX_WEBRTCICESERVERS2_0_URL: ${AULALITE_TURN_URL}`. `_0_URL` is index 0's **single** url string, not a list — it would receive the literal comma-joined string and fail pion's URL parse. Hence the separate `AULALITE_MEDIAMTX_TURN_URL` and the `:-` fallback copied from `docker-compose.cloudflare.yml:85`. Do **not** simplify it to the bare variable. There is no `MTX_WEBRTCICESERVERS2_1_URL` anywhere in the repo, so a second MediaMTX relay would have to be added to compose first.
Prefer these env vars over editing `ops/mediamtx/mediamtx.yml` — the image is digest-pinned and a yml edit would require `docker compose -p aulalite build mediamtx`.
Reachability confirmed: from inside the mediamtx network namespace, TCP 80 and 443 to `openrelay.metered.ca` are both OPEN.

### E4 [ORCH] Code fix — the viewer's ICE servers are silently discarded
`crates/shell-web/src/routes/live_session.rs:11-23`: `struct JoinResp` has no `ice_servers` field, so serde throws away the list the backend sends (`live_sessions.rs:1414-1417` declares it, `:1847` populates it via `ice::ice_servers()`). Every **viewer** WHEP peer therefore falls back to hard-coded Google STUN (`live_room_whip.rs:113-124`). TURN is inert for viewers no matter how correct `AULALITE_TURN_URL` becomes — and E3's server-side relay cannot help a browser with no relay candidate to pair with. (The teacher path is unaffected; `set_ice_servers` is called at `live_room_broadcast.rs:2336,2426`, both publish paths.)

Add to `JoinResp`:
```rust
#[serde(default)]
ice_servers: Vec<features_courses::live_room_whip::IceServerConfig>,
```
Verified viable: `crates/features-courses/src/lib.rs:107-108` make both modules public, and `IceServerConfig` derives `Deserialize` (`live_room_whip.rs:10-17`). `JoinResp` derives `PartialEq` and `Default`; `IceServerConfig` does **not** derive `Default`, but `Vec<T>: Default` regardless, so this compiles — do not add a non-`Vec` field here. Then wire it to `set_ice_servers` on the viewer path the same way `live_room_broadcast.rs` does.
*(Checked and clean: the other `ice_servers: Vec::new()` sites — `live_room_session.rs:347,380,455`, `live_room_view.rs:1336,1399` — are all under `#[cfg(not(target_arch = "wasm32"))]`, i.e. the native/Tauri bridge, and do not affect the shipped web app.)*

**This requires a frontend WASM release rebuild (several minutes). Batch it with Phase F — one rebuild, not two.**

### E5 [ORCH] Apply and verify
```
docker compose -p aulalite up -d backend mediamtx
docker compose -p aulalite restart frontend
docker logs --tail 50 aulalite-mediamtx-1
curl -s http://127.0.0.1:9997/v3/config/global/get | tr ',' '\n' | grep -E 'AllowOrigins|ICEServers2|LocalTCP'
curl -i -X OPTIONS http://localhost:8889/aula/whep -H 'Origin: https://aula.elementors.guru' -H 'Access-Control-Request-Method: POST' | grep -i access-control-allow-origin
```
Then a genuine two-browser, two-network go-live test.

**Do not declare live video fixed on config evidence alone.** A free public TURN relay carrying every participant's media, with the origin a Windows desktop behind NAT and a tunnel that forwards no UDP, is not a working live-class topology. If live class is genuinely required, MediaMTX needs a directly reachable public UDP endpoint (port-forward 8189/udp + 8889, or move media to a host with a public IP) — no dashboard setting substitutes. **The reliable path on this topology is HLS viewing via `live.elementors.guru`** (`transport_mode='hls'` per session; note the DB default is `'webrtc'` — `migrations/20260508000012_live_room_columns.sql:6,11` and `live_sessions.rs:59-61,551`), with the teacher publishing from the Docker host itself. If neither is achievable, disable live rooms rather than half-shipping them.
*(Ruled out and clean — the WebSocket signalling path itself works end-to-end through the tunnel: an upgrade request with a valid `?access_token=` reaches the backend and returns genuine 401/404/409 responses with `Upgrade: websocket` echoed back. MediaMTX auth is fine. The CSP needs no widening — it already permits `https:` and `wss:`.)*

---

## PHASE F — ONE frontend image rebuild, batching every nginx + code change

Do E4 and all of F together, then build once:
```
docker compose -p aulalite build frontend && docker compose -p aulalite up -d frontend
```
All F items live in the heredoc-generated nginx conf inside `crates/shell-web/Dockerfile` (starts line 67) — there is no separate conf file to include.

**F1 [durability, highest value here] Fix the cached-DNS foot-gun from T3.** After line 101 add:
```
resolver 127.0.0.11 valid=10s ipv6=off;
resolver_timeout 5s;
```
and convert BOTH `proxy_pass` sites (`:162` and `:179`) to use a variable so nginx re-resolves per request:
```
set $aulalite_backend "http://backend:8080";
proxy_pass $aulalite_backend$request_uri;
```
The `$request_uri` suffix is **required** — once `proxy_pass` contains a variable, nginx stops passing the location-matched URI, and `$request_uri` carries the query string that both the API and the WebSocket route need. This permanently removes the "restart frontend after every backend recreate" rule.

**F2 Public health routes** — add alongside `location /v1/`:
```
location = /healthz { proxy_pass http://backend:8080/healthz; }
location = /readyz  { proxy_pass http://backend:8080/readyz;  }
```
Today `/healthz`, `/readyz` and `/metrics` on the public host all return the SPA shell (200 text/html) via the catch-all at `:214-216`, which is exactly why the Docker-stopped outage was invisible to any path-based monitor.

**F3 Restore security headers on subresources.** The six `add_header` lines at `:125-130` are silently replaced (not merged) inside the locations at `:185-190` (`/runtime-config.js`), `:194-198`, `:200-204` and `:209-212` (static assets) — verified live: the deployed `.wasm` and `.js` return **zero** of CSP / nosniff / HSTS / XFO / Referrer-Policy / Permissions-Policy. Either repeat all six lines verbatim in each of the four blocks (as `/lti/landing` at `:136-147` already correctly does), or drop the `add_header Cache-Control` from `:209-212` in favour of `expires 1d;` so the block has no `add_header` at all and inherits. **Do not use `more_set_headers`** — `ngx_headers_more` is not compiled into the digest-pinned `nginxinc/nginx-unprivileged:alpine` image.

**F4 Manifest MIME.** `manifest.webmanifest` is served as `application/octet-stream`. Add `location = /manifest.webmanifest { types { } default_type application/manifest+json; ... }` (an exact-match location outranks the regex at `:209`) — and include the six headers from F3, or use the `expires` approach. It is 698 bytes, under `gzip_min_length 1024`, so only the MIME type actually changes behaviour.

**F5 [cosmetic] Stop the doubled headers on `/v1/*`.** `location /v1/` declares no `add_header`, so nginx appends its server-level set to the backend's own (`lib.rs:235-257`), producing two each of CSP, HSTS, Referrer-Policy, XFO and nosniff. Harmless — multiple CSPs are enforced as an intersection and `default-src 'none'` is correct for JSON — but it looks like a misconfiguration during triage. Add `proxy_hide_header` for those five. **Do NOT add `proxy_hide_header Permissions-Policy;`** — only nginx sets that one; hiding it would strip it entirely.

**Optional:** make the healthcheck actually cover the proxy —
`docker-compose.yml:229` → `test: ["CMD-SHELL", "wget -q --spider http://127.0.0.1:3000/ && wget -q -S --spider http://127.0.0.1:3000/v1/me 2>&1 | grep -q ' 401' || exit 1"]` — so a wedged upstream marks the container unhealthy instead of silently serving a broken app.

---

## PHASE G — remaining security hardening

**G1 Enable rate limiting — apply LAST of the config changes.** `.env:187 AULALITE_RATE_LIMIT=false` → `on`. `services/rate_limit.rs:139-150` returns `None` unless the value is `1/true/yes/on`, so no middleware is installed at all today: `/v1/auth/local-login`, the Stripe webhook, certificate verification, the api-key public API, LTI login/launch and the MediaMTX auth callback are all unthrottled.
**Risk:** `main.rs:771-777` returns `Err` (process exit) if it opts in and cannot reach Redis. Confirm first, apply, then check immediately, and revert on failure rather than leaving the backend crash-looping:
```
docker exec aulalite-redis-1 redis-cli ping          # PONG
docker compose -p aulalite up -d backend && docker compose -p aulalite restart frontend
docker logs --tail 50 aulalite-backend-1 | grep -E 'API rate limiting ENABLED|rate limiter connect failed'
```
`.env:188-194` bucket limits need no change.

**G2 Decide on self-service signup — a real decision, not a default.** `.env:22 SELF_SERVICE_SIGNUP_ENABLED=true` + `auth/middleware.rs:72-78` means any verified Firebase identity on project `elementors-aulalite` with no invitation auto-provisions **its own tenant** with `org_owner` (`jit_provision.rs:265-273`, named `"<owner>'s academy"`). This has already happened unprompted on the live host: tenant `258850ab-0266-4c33-bc96-bdc42dc938da`, slug `academy-298ba85cb2974ba5bb58ba7ca2f058b7`, `"shivamkumardb53's academy"`, status trialing, created 2026-09-01T16:04:42Z, matching user `shivamkumardb53@gmail.com`.
- If this is meant to be a single-tenant instance: set `SELF_SERVICE_SIGNUP_ENABLED=false` and `docker compose -p aulalite up -d backend`. **Only after A4** — with it off, a new Firebase identity with no invitation is rejected by `jit_provision` and you would have no admin.
- If open signup is intended: leave it on, and **G1 becomes mandatory rather than last**.
- **The existing third-party tenant is the operator's call.** Inspect it (`docker exec aulalite-postgres-1 psql -U aulalite -d aulalite -c "select * from tenants where slug='academy-298ba85cb2974ba5bb58ba7ca2f058b7'"`); do not delete it without an explicit decision.

**G3** See T1. `APP_ENV=production` is a separate future project requiring all of: `AULALITE_ADMIN_ORIGIN`, a real RS256 PEM, non-default object-store creds (already done in C1), all-https origins, plus `SSO_SESSION_SECRET`, `AULALITE_DATA_ENCRYPTION_KEY`, `RESEND_API_KEY`, `AULALITE_MFA_ENFORCE`. Sequence it behind everything above, never in front.

---

## PHASE H — nice-to-have

**H1 [HUMAN]** Cloudflare cache rule for `/assets/` (see §1). The 2.2 MB WASM returns `cf-cache-status: DYNAMIC` on every fetch (~1.4 s each, pulled through the tunnel from this desktop), while `/assets/*.js` and `*.css` return HIT with Age >20000 — so the rule is narrowly about the wasm. Safe: filenames are content-fingerprinted. **Scope it to `/assets/` exactly and do not let it catch `/runtime-config.js`**, which correctly returns `cf-cache-status: BYPASS` with `no-store` and must stay that way or the edge pins a stale Firebase/tenant config.

**H2 [HUMAN]** Disable Cloudflare Web Analytics automatic injection (see §1). The beacon is UA-gated (invisible to curl's default UA, present with a Chrome UA) and is CSP-blocked by `script-src 'self' 'wasm-unsafe-eval' https://www.gstatic.com`, putting a red error in every visitor's console while collecting nothing. **Do not widen `script-src` for it.**
Verify: `curl -s --compressed -H 'User-Agent: Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/126.0.0.0' https://aula.elementors.guru/ | grep -c cloudflareinsights` → 0

**H3** Point an external uptime monitor at `https://aula.elementors.guru/readyz` **after F2 ships**, asserting on the **body substring `"ready"`, not the status code**. `/readyz` exercises database, redis and media; `/healthz` is a static `"ok"`. Asserting on status alone reproduces the exact bug F2 fixes — with any nginx-only failure mode, a 200 with SPA HTML is indistinguishable from health.

---

## SUMMARY OF `.env` EDIT ROUNDS (each needs `up -d`, never `restart`)

| round | phase | lines | services to recreate |
|-------|-------|-------|----------------------|
| R1 | A5 | 11, 12, 13, 16, 76, 147, 148 | `backend frontend mediamtx` + restart frontend |
| R2 | B1 | 95, 97, 101, 105, 106, 198 | `backend` + restart frontend |
| R3 | C1, C3 | 119, 120, then 115 | `rustfs backend` (C1), `backend` (C3) + restart frontend |
| R4 | E2 | 127, 128, 138, + new `AULALITE_MEDIAMTX_TURN_URL` | `backend mediamtx` + restart frontend |
| R5 | G1, G2 | 187, (22) | `backend` + restart frontend |

Non-`.env` file edits: `docker-compose.yml` (E3, optional healthcheck), `crates/shell-web/Dockerfile` (F1-F5), `crates/shell-web/src/routes/live_session.rs` (E4). The last two share **one** frontend image rebuild.