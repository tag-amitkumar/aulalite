# AulaLite Ops Runbook

## Local Dev

`dioxus-kinetics` is a **public** dependency now, so the build fetches it
anonymously — no `GH_TOKEN` / build-secret setup is required.

```bash
cp .env.example .env
docker compose up -d --build
# docker-compose.override.yml remaps the backend host port 8080 -> 18080
# (8080 is occupied by a local EnterpriseDB on the dev box). In-cluster stays 8080.
curl http://localhost:18080/healthz
curl http://localhost:9997/v3/paths/list
```

```powershell
Copy-Item .env.example .env
docker compose up -d --build
curl http://localhost:18080/healthz
```

Local Docker Desktop uses explicit MediaMTX ports in `docker-compose.yml`.
Production does not require host networking: advertise the node's public IP or
media DNS name with `MEDIAMTX_ADDITIONAL_HOSTS` and forward UDP `8189` to the
MediaMTX container. TLS termination handles the separate public signalling and
HLS endpoints described below.

### Build resources (Windows / WSL2)

A bare `docker compose build` builds the backend and frontend images
**concurrently**. Both are heavy Rust/WASM compiles; from a cold cache two
unbounded `rustc` fleets can exhaust the WSL2/Docker Desktop VM, and BuildKit
drops the build's gRPC stream with:

```
failed to receive status: rpc error: code = Unavailable desc = error reading from server: EOF
```

That is a **memory failure, not an auth failure**. The Dockerfiles now cap cargo
parallelism (`CARGO_BUILD_JOBS=4`, `CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1` in the
builder stage) to bound peak RSS so the concurrent build survives. If your box is
still tight on RAM you can additionally:

- build sequentially: `docker compose build backend && docker compose build frontend && docker compose up -d`
- raise the WSL2 memory ceiling in `%UserProfile%\.wslconfig`:

  ```ini
  [wsl2]
  memory=20GB
  ```

  then `wsl --shutdown` and reopen Docker Desktop.

## Dokploy Deployment

The repository contains a standalone production definition for Dokploy. Do not
select the development Compose file or try to express a two-file overlay in the
Dokploy UI.

1. Create a Dokploy **Compose / Docker Compose** application (not Docker Stack)
   and connect this repository through the installed GitHub App. Select `main`
   as the production branch, choose trigger type **On Push**, leave **Watch
   Paths** empty, disable submodules, and set **Compose Path** to
   `./docker-compose.dokploy.yml`. The GitHub App supplies its signed webhook;
   do not add a GitHub Actions deploy secret or a second manual webhook. Enable
   **Isolated Deployments** so Dokploy
   attaches every service to one app-specific ingress network while preserving
   private service-to-service connectivity. Protect and review the production
   branch, and run `pwsh -File ops/preflight.ps1` locally before every production
   push. CI and release-candidate Actions are manual-dispatch only, so ordinary
   pushes consume zero GitHub Actions minutes and deploy through Dokploy's GitHub
   webhook. Do not deploy yet; complete steps 2–8 first.
2. Paste [`dokploy.env.example`](dokploy.env.example) into Dokploy's Environment
   editor and replace every required blank or placeholder. Leave optional values
   blank only when that integration is deliberately disabled. Dokploy writes
   this outside Git beside the selected Compose file; Compose passes only
   variables explicitly referenced by `${...}`. Reuse valid provider values from the ignored local
   `.env`, but do not upload or commit that file. The legacy local keys
   `MINIO_*`, `MINIO_ENDPOINT`, and `FIREBASE_SERVICE_ACCOUNT_JSON_B64` are not
   read by the production services; use `AWS_*`, `S3_ENDPOINT_URL`, and one-line
   `FCM_SERVICE_ACCOUNT_JSON` respectively.
3. Generate independent production secrets. At minimum, use a distinct owner
   database password, runtime database password, RustFS credentials, MediaMTX
   callback secret, SSO session secret, application data key, persistent JWT RSA
   key, Stripe credentials, and email-provider key. Store the literal multiline
   PKCS#8 PEM in `JWT_RS256_PRIVATE_KEY_PEM`; the backend does not convert the two
   characters `\n` into newlines. Example generators on a trusted workstation:

   ```bash
   openssl rand -hex 32       # password / shared-secret material
   openssl rand -base64 32    # AULALITE_DATA_ENCRYPTION_KEY only
   openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:3072
   ```

   Dokploy's Environment editor uses Compose dotenv syntax. Paste the RSA key as
   one single-quoted multiline value so the quotes are removed by Compose and
   the container receives real newline characters:

   ```dotenv
   JWT_RS256_PRIVATE_KEY_PEM='-----BEGIN PRIVATE KEY-----
   ...base64 key lines...
   -----END PRIVATE KEY-----'
   ```

   Use the same single-quoted multiline form for an optional DKIM private key.
   Keep `FCM_SERVICE_ACCOUNT_JSON` as one JSON line with its private-key newlines
   escaped inside the JSON string, as issued by Google. Use Dokploy's trusted
   **Preview Compose** view to confirm the key is attached to the backend, but do
   not copy, screenshot, or attach the rendered secret-bearing output to logs or
   support tickets.

4. Point DNS A/AAAA records for the six hosts at the Dokploy node, then
   configure these TLS domains in Dokploy; only the listed public service ports
   should receive routes:

   | Domain | Service | Container port | Purpose |
   | --- | --- | ---: | --- |
   | `aula.elementors.guru` | `frontend` | 3000 | Parent, learner, and educator web app |
   | `admin.elementors.guru` | `frontend` | 3000 | Platform administration entry point |
   | `api.aula.elementors.guru` | `backend` | 8080 | Public API and WebSocket endpoint |
   | `storage.elementors.guru` | `rustfs` | 9000 | Browser-reachable presigned S3 PUT/GET |
   | `live.elementors.guru` | `mediamtx` | 8889 | WHIP/WHEP WebRTC signalling |
   | `stream.elementors.guru` | `mediamtx` | 8888 | HLS playback |

   For each entry use path `/`, internal path `/`, **Strip Path** off, HTTPS on,
   and the Let's Encrypt certificate. Redeploy after adding or changing a
   domain so Dokploy regenerates Traefik labels.

   Keep Postgres, Redis, the RustFS console, and MediaMTX `9997/9998` private.
   The backend is public only through the API hostname and Dokploy's Traefik
   route; do not publish a fixed node TCP port.
5. Set `MEDIAMTX_ADDITIONAL_HOSTS` to the node's public IP or a DNS name that
   resolves to it (comma-separate multiple values; omit schemes and ports).
   Allow and forward public UDP `8189` to the node/container. This is the ICE
   media path; Traefik only terminates HTTPS signalling. Host networking is not
   required for this topology. Configure TURN before broad customer rollout for
   restrictive school or corporate networks.
6. On a fresh Postgres volume, the raw `POSTGRES_RUNTIME_PASSWORD` bootstraps the
   exact `aulalite_app` `NOSUPERUSER NOBYPASSRLS` login. Put the same logical
   password, URL-percent-encoded, in `DATABASE_URL`; `MIGRATION_DATABASE_URL`
   stays on the owner role.
   Changing either environment value later does not rotate an existing database
   role—use the password-rotation procedure in the RLS section below.
7. Add `aula.elementors.guru` and `admin.elementors.guru` to Firebase
   Authentication's authorized domains and register
   `https://api.aula.elementors.guru/v1/stripe/webhook` in Stripe. Keep Firebase web
   metadata and the VAPID key classified as publishable; service-account JSON,
   provider keys, signing keys, database URLs, and shared secrets are server-only.
8. Confirm the frontend root health check and keep the backend's internal
   dependency-aware check on `/readyz`. `/healthz` is process liveness and
   intentionally does not signal dependency readiness. Standard Docker Compose
   reports unhealthy containers but does not provide Swarm's automatic
   health-failure rollback; rehearse Dokploy's manual/registry rollback to a
   known-good revision before launch.
9. Start the first deployment manually. After it is green, verify the six TLS
   routes, Firebase sign-in, a presigned upload/download, Stripe webhook delivery,
   and one two-device live classroom including UDP ICE. Only then enable
   **Auto Deploy** so a push to the protected branch triggers the same build and
   rollout. A successful image build is not a substitute for these checks.

> **MediaMTX shared secret.** `MEDIAMTX_AUTH_SHARED_HEADER` is embedded in the
> MediaMTX auth-callback URL and is recoverable from MediaMTX's unauthenticated
> `/v3/config/global/get` (port 9997). The Dokploy production stack does not
> publish 9997 on the host, but if it was ever exposed, rotate the secret.

### Application data-key rotation

TOTP seeds and tenant OIDC client secrets are AES-256-GCM encrypted before
storage. Generate `AULALITE_DATA_ENCRYPTION_KEY` as base64 of 32 random bytes;
keep it in the deployment secret store, separate from database credentials.
For example, generate it on a trusted operator workstation with
`openssl rand -base64 32`, store it immediately, and do not paste it into tickets
or deployment logs.
For a rolling deployment, do not switch the current key in one wave. Rewrapping
starts before the new replica serves traffic: an old replica that knows only the
old key would immediately lose the ability to read those rows, and it could
still create old-key ciphertext after the new replica's scan. Use this staged
procedure instead:

1. Generate and escrow the new key. Deploy **all** replicas with the old key
   still in `AULALITE_DATA_ENCRYPTION_KEY` and the new key temporarily in
   `AULALITE_DATA_ENCRYPTION_KEY_PREVIOUS`. Despite the variable name, this
   first compatibility wave only teaches every old-code replica to decrypt both
   keys; writes remain on the old current key. Wait until no replica from before
   this wave remains.
2. Deploy all replicas with the new key in `AULALITE_DATA_ENCRYPTION_KEY` and the
   old key in `AULALITE_DATA_ENCRYPTION_KEY_PREVIOUS`. Startup first
   authenticates every existing ciphertext without writing, then
   compare-and-swap rewraps legacy/old-key rows before each new replica serves.
   This preflight avoids a partially-rotated database when a key is mistyped.
   Old replicas from step 1 can read the new ciphertext during the rollout.
3. After every replica uses the step-2 configuration, perform one convergence
   restart with that same pair. This catches any old-key write made by a step-1
   replica after an earlier startup scan. Confirm the restart log reports the
   expected scanned and rewrapped row counts and that all replicas become
   ready. Startup fails before the rewrap pass on any pre-existing corrupt or
   unreadable row; keep the rollout stopped and restore the exact prior key pair
   before diagnosis.
4. Only then remove `AULALITE_DATA_ENCRYPTION_KEY_PREVIOUS` in a final deploy.

For a single-replica installation, stop/drain the process before step 2 and keep
the previous key through one clean restart before removing it. Never rotate both
keys at once, and never remove the old key merely because one replica logged a
successful scan.

Back up every active/rotation data key in the disaster-recovery secret store,
labelled with its activation and retirement dates. A database backup without a
matching key cannot restore MFA or enterprise SSO secrets. Removing the old key
from the application after step 4 is not authorization to destroy it: retain it
until every database backup that may contain old-key ciphertext has expired (and
any compliance retention window has passed). When restoring such a backup, boot
with its key as current or previous, let startup rewrap, and only then resume
traffic.

### Row Level Security (required for real tenant isolation)

Postgres RLS only enforces when the connection role does **not** bypass it. The
`aulalite` schema owner bypasses RLS, so production request traffic must use the
exact `aulalite_app` role.

On a fresh Dokploy named volume, the custom Postgres image automatically creates
that login from raw `POSTGRES_RUNTIME_PASSWORD` before the backend starts. The
backend then runs migrations through owner-only `MIGRATION_DATABASE_URL`, closes
that pool, and opens `DATABASE_URL` as `aulalite_app`; migrations grant the
runtime role its DML privileges. No manual first-deploy `ALTER ROLE` is needed.

Postgres initialization scripts do not rerun on an existing volume. For a
password mismatch or planned rotation, stop/drain the backend, open Dokploy's
terminal for the `postgres` service, and use the interactive password command so
the new secret does not enter shell history or process arguments:

```text
psql --username "$POSTGRES_USER" --dbname "$POSTGRES_DB"
\password aulalite_app
```

Then update raw `POSTGRES_RUNTIME_PASSWORD` for future fresh-volume restores and
put the same logical password, URL-percent-encoded, in `DATABASE_URL`. Redeploy,
confirm `/readyz`, and verify the role remains `LOGIN NOSUPERUSER NOBYPASSRLS`.
Never grant schema ownership, `SUPERUSER`, or `BYPASSRLS` to `aulalite_app`.
The background sweeps use transaction-local system context policies, so they
continue working through the restricted runtime role.

Production has no RLS-bypass escape hatch: startup requires the exact
`aulalite_app` runtime role and verifies that it remains `NOBYPASSRLS` and
`NOSUPERUSER`. Use the owner URL only for migrations.

## Backups

### PostgreSQL logical backup

[`backup/postgres-backup.sh`](backup/postgres-backup.sh) runs `pg_dump` inside
the active Postgres 16 container, using the same client major as the server. It
creates a compressed custom-format archive from one consistent snapshot,
validates the archive with `pg_restore --list`, writes a SHA-256 sidecar and
manifest, and atomically publishes the timestamped directory. Backup files are
mode `0600`; they still contain customer data and must be treated as secrets.

Run one backup from the repository root:

```bash
COMPOSE_PROJECT_NAME=<dokploy-app-name> \
BACKUP_OUTPUT_DIR=/srv/aulalite-backups/postgres \
BACKUP_RETENTION_DAYS=0 \
bash ops/backup/postgres-backup.sh
```

For Dokploy, obtain the application name from **Preview Compose** / the named
volume prefix and pass it to this host-side backup process as shown. Do not add
or override `COMPOSE_PROJECT_NAME` in Dokploy's application Environment editor;
Dokploy owns it for deployments, schedules, and volume backups. Auto-deploy
fresh-clones the repository, so schedule from a root-owned stable checkout or
operations bundle that preserves the repository layout (including
`docker-compose.yml` and `ops/backup/`), not a path inside Dokploy's disposable
Git checkout.

`BACKUP_RETENTION_DAYS=0` disables local pruning. A lock prevents overlapping
runs. After a working offsite hook is configured, set a tested local retention
window such as 14 days; the script refuses to prune when no upload hook has
confirmed durability. Schedule the command nightly with the deployment host's
scheduler and alert on any non-zero exit. For example, a host cron entry can
call a small root-owned wrapper that changes into the release directory,
exports the three settings above, and runs the script. Do not put provider
credentials directly in a crontab.

The archive intentionally omits ownership commands for portability. PostgreSQL
roles are cluster-global and are not contained in `pg_dump`; after a disaster,
recreate the owner from `POSTGRES_USER` and the `aulalite_app` runtime role from
the secret store before restoring. Database ACLs remain in the archive.

### Offsite upload hook

Set `BACKUP_UPLOAD_HOOK` to a root-owned executable to upload each completed
directory to the chosen target. The executable receives:

- `BACKUP_DIRECTORY`
- `BACKUP_ARCHIVE`
- `BACKUP_CHECKSUM`
- `BACKUP_MANIFEST`

The hook must be idempotent and return success only after the remote service has
confirmed durable storage and checksum integrity. Prefer client-side encryption,
object lock/immutability, separate credentials with write-only access, and a
different failure domain/account. Start from
[`backup/upload-hook.example.sh`](backup/upload-hook.example.sh); the example
fails closed until an actual uploader is supplied. Local retention runs only
after a configured hook succeeds, so a remote outage cannot silently age out
the last good local copies.

The storage vendor and bucket are an external launch decision. Until they are
selected, the system has verified local backups but no off-node durability.

### Restore drill

Every successful backup should be followed by the isolated verifier (nightly at
first; at minimum weekly once the job is stable):

```bash
bash ops/backup/postgres-restore-verify.sh \
  /srv/aulalite-backups/postgres/20260716T020000Z
```

The verifier checks the SHA-256, starts a network-isolated temporary Postgres 16
container, restores the entire archive with `--exit-on-error`, and checks schema,
migration history, index validity, and constraint validity. It never connects
to or creates a database in production. Set `POSTGRES_VERIFY_IMAGE` to the exact
Postgres 16 image digest approved by operations for fully pinned drills. A
deployment-specific read-only smoke query can be added with
`BACKUP_VERIFY_SQL_FILE=/secure/path/verify.sql`.

Keep dated drill logs with the archive manifest. Alert if no backup and passing
restore verification has completed inside the agreed recovery-point window.
Before launch, set and test explicit recovery objectives; a sensible starting
point for a small academy SaaS is a 24-hour RPO and a 4-hour RTO.

### Disaster restore procedure

Run a real recovery only into a new, empty Postgres 16 cluster while application
writes are stopped. The isolated verifier above is the routine rehearsal; this
procedure is intentionally operator-driven because it replaces durable state.

1. Select the newest backup with a passing restore-drill log, retrieve it from
   offsite storage, and verify `sha256sum --check aulalite.dump.sha256`.
2. Provision a fresh Postgres 16 cluster. Recreate the `aulalite` schema-owner
   role and the `aulalite_app` `NOSUPERUSER NOBYPASSRLS` runtime role, loading
   their passwords from the secret store. Do not recover passwords from shell
   history or place them in the archive directory.
3. Create an empty `aulalite` database owned by `aulalite`, then restore as a
   cluster administrator while setting the object-creation role:

   ```bash
   createdb --host "$PGHOST" --username "$PGADMIN" \
     --owner aulalite aulalite
   pg_restore --host "$PGHOST" --username "$PGADMIN" \
     --role aulalite --dbname aulalite \
     --exit-on-error --no-owner aulalite.dump
   ```

4. Run the same structural queries used by `postgres-restore-verify.sh`, then
   connect with the runtime `aulalite_app` credentials and verify `/readyz` from
   the backend at the same release revision that produced the backup.
5. Restore `rustfs_data` and any required raw recordings, reconcile the newest
   database `file_assets`/`recordings` rows against object presence, and perform
   signed upload/download plus one live-class smoke test.
6. Only after those checks pass, switch ingress to the recovered stack. Record
   the measured recovery time, data cutoff, archive checksum, and approver.
   Upgrade the application and run newer migrations as a separate change.

### Non-database state

Also back up these volumes with the platform's snapshot/export facility and
copy them off-node:

- `rustfs_data` — uploaded course files and processed recordings.
- `mediamtx_recordings` — in-flight/raw recordings not yet copied to RustFS.

These are named volumes in `docker-compose.dokploy.yml`, so configure Dokploy
Volume Backups to an S3 destination for both. A Dokploy `pg_data` volume backup
is useful as a supplemental recovery point, but a live filesystem snapshot is
not a replacement for the consistent logical Postgres archive and passing
restore drill above.

`redis_data` is recoverable transient state and is not part of the durable
recovery set. Never use a raw snapshot of a running `pg_data` volume as the only
database backup; the logical archive and a passing restore drill are the source
of truth.

> **Log verbosity in prod.** `.env.example` ships `RUST_LOG=info,backend=debug,sqlx=warn`
> for local debugging. In production set `RUST_LOG=info,sqlx=warn` (the backend
> image already defaults to `info`); the Dokploy stack rotates container logs
> (`json-file`, 10m × 3) regardless.

## Request correlation and logs

Every backend request now has an `X-Request-ID`. A safe upstream value (8–64
ASCII letters, digits, `-`, `_`, `.`, or `:`) is preserved; missing or malformed
values are replaced with a UUID. The id is returned in the response, exposed to
browser JavaScript through CORS, and recorded as `request_id` on the structured
`http_request` tracing span. Ask support users for this response id and search
the JSON container logs by the same value to follow the request across handler
events. The bundled frontend proxy passes the header through to the backend.

If another load balancer is placed in front, configure it to preserve a single
bounded `X-Request-ID` value or replace it with its own id; do not append a
comma-separated chain. Forward logs to durable centralized storage before
launch—the Compose log rotation is a local safety cap, not a searchable audit
archive or alerting system.

## Object storage (RustFS)

RustFS is S3-compatible and replaces MinIO. The backend creates the `aulalite`
bucket automatically on boot (`ensure_bucket`, retried with backoff), so no `mc`
or console step is needed. The console is on `:9001` (login with
`RUSTFS_ACCESS_KEY` / `RUSTFS_SECRET_KEY`, which are sourced from
`AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY`).

**Browser-direct uploads (presigned PUT/GET).** The `/v1/uploads/*` flow mints
presigned URLs the browser uses to PUT/GET bytes directly against the storage
endpoint. `S3_ENDPOINT_URL` is therefore the public signing origin the browser
reaches (`https://storage.elementors.guru` in production). The SigV4 signature
covers that host. `S3_INTERNAL_ENDPOINT_URL` is the backend-only control/data
endpoint (`http://rustfs:9000`) used for bucket bootstrap and object operations;
the private backend does not need to hairpin through public ingress.

No manual bucket-CORS setup is required. Before binding its HTTP listener, the
backend creates the bucket if necessary and reconciles one exact rule for
`APP_ORIGIN`: `GET`/`PUT`/`HEAD`, any request header, exposed `ETag`,
`Content-Length`, and `Content-Type`, with a 3600-second max age. This runs for
existing buckets too. Bootstrap retries every two seconds for roughly one minute
and then exits if bucket or CORS reconciliation still fails, so `/readyz` and
the dependent frontend remain unavailable instead of serving broken uploads.

For diagnostics only, an operator with the scoped S3 credentials can inspect
the effective policy after startup:

```bash
aws --endpoint-url https://storage.elementors.guru s3api get-bucket-cors \
  --bucket "$S3_BUCKET"
```

Local Compose uses `S3_ENDPOINT_URL=http://localhost:9000` for browser-facing
presigned URLs and `S3_INTERNAL_ENDPOINT_URL=http://rustfs:9000` for backend
operations, so direct upload/download flows work without leaking a Compose DNS
name into the browser.
