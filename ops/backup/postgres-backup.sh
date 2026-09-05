#!/usr/bin/env bash
# Create a consistent, portable PostgreSQL logical backup from the running
# Compose database. The archive is staged and validated before it is published.

set -Eeuo pipefail

umask 077

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
REPO_ROOT="$(cd -- "${SCRIPT_DIR}/../.." && pwd -P)"
OUTPUT_ROOT="${BACKUP_OUTPUT_DIR:-${REPO_ROOT}/backups/postgres}"
RETENTION_DAYS="${BACKUP_RETENTION_DAYS:-0}"
POSTGRES_SERVICE="${BACKUP_POSTGRES_SERVICE:-postgres}"
UPLOAD_HOOK="${BACKUP_UPLOAD_HOOK:-}"

die() {
  printf 'backup error: %s\n' "$*" >&2
  exit 1
}

command -v docker >/dev/null 2>&1 || die "docker is required"
command -v sha256sum >/dev/null 2>&1 || die "sha256sum is required"
[[ "${RETENTION_DAYS}" =~ ^[0-9]+$ ]] || die "BACKUP_RETENTION_DAYS must be a non-negative integer"

if docker compose version >/dev/null 2>&1; then
  COMPOSE=(docker compose)
elif command -v docker-compose >/dev/null 2>&1; then
  COMPOSE=(docker-compose)
else
  die "Docker Compose is required"
fi

# Use only the base definition when addressing an already-running project. This
# avoids requiring every production interpolation variable merely to run exec.
COMPOSE+=(--project-directory "${REPO_ROOT}" -f "${REPO_ROOT}/docker-compose.yml")

mkdir -p -- "${OUTPUT_ROOT}"

# Prevent two schedulers from publishing the same point-in-time backup. flock is
# available on standard Linux deployment hosts; fail closed if it is missing.
command -v flock >/dev/null 2>&1 || die "flock is required to prevent overlapping backups"
exec 9>"${OUTPUT_ROOT}/.backup.lock"
flock -n 9 || die "another PostgreSQL backup is already running"

timestamp="$(date -u +'%Y%m%dT%H%M%SZ')"
final_dir="${OUTPUT_ROOT}/${timestamp}"
[[ ! -e "${final_dir}" ]] || die "backup destination already exists: ${final_dir}"

staging_dir="$(mktemp -d "${OUTPUT_ROOT}/.backup-${timestamp}-XXXXXX")"
cleanup() {
  if [[ -n "${staging_dir:-}" && -d "${staging_dir}" ]]; then
    rm -rf -- "${staging_dir}"
  fi
}
trap cleanup EXIT INT TERM

archive="${staging_dir}/aulalite.dump"

# pg_dump runs inside the database container, so its client major version
# always matches the server and database credentials never enter argv on the
# host. Custom format is compressed, supports parallel restore, and uses one
# transactionally consistent snapshot.
"${COMPOSE[@]}" exec -T "${POSTGRES_SERVICE}" sh -ceu '
  exec pg_dump \
    --username="$POSTGRES_USER" \
    --dbname="$POSTGRES_DB" \
    --format=custom \
    --compress=6 \
    --no-owner \
    --lock-wait-timeout=30s
' >"${archive}"

[[ -s "${archive}" ]] || die "pg_dump produced an empty archive"

# Parse the archive with the matching pg_restore before publishing it. This
# catches truncation and invalid output immediately; the isolated restore drill
# performs the deeper end-to-end verification.
"${COMPOSE[@]}" exec -T "${POSTGRES_SERVICE}" pg_restore --list <"${archive}" >/dev/null

tool_version="$("${COMPOSE[@]}" exec -T "${POSTGRES_SERVICE}" pg_dump --version | tr -d '\r\n')"
database_name="$("${COMPOSE[@]}" exec -T "${POSTGRES_SERVICE}" sh -ceu 'printf %s "$POSTGRES_DB"' | tr -d '\r\n')"
archive_bytes="$(wc -c <"${archive}" | tr -d '[:space:]')"

(
  cd -- "${staging_dir}"
  sha256sum aulalite.dump >aulalite.dump.sha256
)
archive_sha256="$(cut -d ' ' -f 1 "${staging_dir}/aulalite.dump.sha256")"

cat >"${staging_dir}/manifest.txt" <<EOF
BACKUP_FORMAT_VERSION=1
CREATED_AT_UTC=${timestamp}
DATABASE_NAME=${database_name}
ARCHIVE_FILE=aulalite.dump
ARCHIVE_BYTES=${archive_bytes}
ARCHIVE_SHA256=${archive_sha256}
POSTGRES_TOOL_VERSION=${tool_version}
EOF

chmod 0600 "${archive}" "${staging_dir}/aulalite.dump.sha256" "${staging_dir}/manifest.txt"
mv -- "${staging_dir}" "${final_dir}"
staging_dir=""

# The hook is deliberately provider-neutral. It receives paths as environment
# variables and must return success only after durable, preferably immutable and
# encrypted, off-node storage confirms the upload.
if [[ -n "${UPLOAD_HOOK}" ]]; then
  [[ -x "${UPLOAD_HOOK}" ]] || die "BACKUP_UPLOAD_HOOK is not executable: ${UPLOAD_HOOK}"
  BACKUP_DIRECTORY="${final_dir}" \
    BACKUP_ARCHIVE="${final_dir}/aulalite.dump" \
    BACKUP_CHECKSUM="${final_dir}/aulalite.dump.sha256" \
    BACKUP_MANIFEST="${final_dir}/manifest.txt" \
    "${UPLOAD_HOOK}"
fi

# Retention runs only after dump validation and a successful configured upload.
# Zero disables local pruning. The timestamp-directory constraint prevents this
# command from touching locks, operator notes, or any other files in the root.
if (( RETENTION_DAYS > 0 )) && [[ -z "${UPLOAD_HOOK}" ]]; then
  printf '%s\n' \
    "backup warning: local pruning skipped because no BACKUP_UPLOAD_HOOK confirmed offsite durability" >&2
elif (( RETENTION_DAYS > 0 )); then
  find "${OUTPUT_ROOT}" \
    -mindepth 1 -maxdepth 1 -type d \
    -name '20??????T??????Z' -mtime "+${RETENTION_DAYS}" \
    -exec rm -rf -- {} +
fi

printf 'backup complete: %s\n' "${final_dir}"
printf 'sha256: %s\n' "${archive_sha256}"
