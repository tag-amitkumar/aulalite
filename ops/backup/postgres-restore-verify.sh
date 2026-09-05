#!/usr/bin/env bash
# Restore an AulaLite logical backup into an isolated, temporary PostgreSQL
# container and run structural checks. No production database is contacted.

set -Eeuo pipefail

umask 077

VERIFY_IMAGE="${POSTGRES_VERIFY_IMAGE:-postgres:16-alpine}"
CUSTOM_SQL="${BACKUP_VERIFY_SQL_FILE:-}"

die() {
  printf 'restore verification error: %s\n' "$*" >&2
  exit 1
}

[[ $# -eq 1 ]] || die "usage: $0 <backup-directory-or-aulalite.dump>"
command -v docker >/dev/null 2>&1 || die "docker is required"
command -v sha256sum >/dev/null 2>&1 || die "sha256sum is required"

input="$1"
if [[ -d "${input}" ]]; then
  archive="${input%/}/aulalite.dump"
else
  archive="${input}"
fi
[[ -f "${archive}" ]] || die "archive not found: ${archive}"

archive_dir="$(cd -- "$(dirname -- "${archive}")" && pwd -P)"
archive="${archive_dir}/$(basename -- "${archive}")"
checksum="${archive}.sha256"
[[ -f "${checksum}" ]] || die "checksum sidecar not found: ${checksum}"

(
  cd -- "${archive_dir}"
  sha256sum --check --status "$(basename -- "${checksum}")"
) || die "archive checksum does not match"

if [[ -n "${CUSTOM_SQL}" ]]; then
  [[ -f "${CUSTOM_SQL}" ]] || die "BACKUP_VERIFY_SQL_FILE not found: ${CUSTOM_SQL}"
  custom_sql_dir="$(cd -- "$(dirname -- "${CUSTOM_SQL}")" && pwd -P)"
  CUSTOM_SQL="${custom_sql_dir}/$(basename -- "${CUSTOM_SQL}")"
fi

container="aulalite-restore-verify-$(date -u +'%Y%m%d%H%M%S')-$$"
password="verify-$(od -An -N16 -tx1 /dev/urandom | tr -d ' \n')"

cleanup() {
  if docker inspect "${container}" >/dev/null 2>&1; then
    docker rm --force --volumes "${container}" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT INT TERM

docker_args=(
  run --detach --rm
  --name "${container}"
  --network none
  --env "POSTGRES_PASSWORD=${password}"
  --env POSTGRES_DB=postgres
  --mount "type=bind,src=${archive},dst=/backup/aulalite.dump,readonly"
)
if [[ -n "${CUSTOM_SQL}" ]]; then
  docker_args+=(--mount "type=bind,src=${CUSTOM_SQL},dst=/backup/custom-verify.sql,readonly")
fi
docker_args+=("${VERIFY_IMAGE}")

docker "${docker_args[@]}" >/dev/null

ready=false
for _ in $(seq 1 60); do
  if docker exec -e PGPASSWORD="${password}" "${container}" \
    pg_isready --username=postgres --dbname=postgres >/dev/null 2>&1; then
    ready=true
    break
  fi
  sleep 1
done
[[ "${ready}" == true ]] || die "temporary PostgreSQL did not become ready"

# ACL entries in historical migrations reference these roles. They are NOLOGIN
# in the disposable verifier and do not need any production credentials.
docker exec -i -e PGPASSWORD="${password}" "${container}" \
  psql --username=postgres --dbname=postgres --set ON_ERROR_STOP=1 <<'SQL'
CREATE ROLE aulalite NOLOGIN;
CREATE ROLE aulalite_app NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS;
CREATE DATABASE aulalite_verify;
SQL

docker exec -e PGPASSWORD="${password}" "${container}" \
  pg_restore \
    --username=postgres \
    --dbname=aulalite_verify \
    --exit-on-error \
    --no-owner \
    /backup/aulalite.dump

# The archive must contain application schema and migration history, all indexes
# must be valid, and every restored FK/check constraint must remain validated.
docker exec -i -e PGPASSWORD="${password}" "${container}" \
  psql --username=postgres --dbname=aulalite_verify --set ON_ERROR_STOP=1 <<'SQL'
DO $$
DECLARE
    application_tables integer;
    migration_rows integer;
    invalid_indexes integer;
    unvalidated_constraints integer;
BEGIN
    SELECT count(*) INTO application_tables
      FROM pg_catalog.pg_tables
     WHERE schemaname = 'public';
    IF application_tables < 5 THEN
        RAISE EXCEPTION 'expected application schema, found only % public tables', application_tables;
    END IF;

    IF to_regclass('public._sqlx_migrations') IS NULL THEN
        RAISE EXCEPTION '_sqlx_migrations is missing';
    END IF;
    SELECT count(*) INTO migration_rows FROM public._sqlx_migrations;
    IF migration_rows < 1 THEN
        RAISE EXCEPTION '_sqlx_migrations is empty';
    END IF;

    SELECT count(*) INTO invalid_indexes
      FROM pg_catalog.pg_index
     WHERE NOT indisvalid OR NOT indisready;
    IF invalid_indexes <> 0 THEN
        RAISE EXCEPTION 'found % invalid indexes', invalid_indexes;
    END IF;

    SELECT count(*) INTO unvalidated_constraints
      FROM pg_catalog.pg_constraint
     WHERE contype IN ('c', 'f') AND NOT convalidated;
    IF unvalidated_constraints <> 0 THEN
        RAISE EXCEPTION 'found % unvalidated constraints', unvalidated_constraints;
    END IF;

    RAISE NOTICE 'verified % public tables and % migrations', application_tables, migration_rows;
END
$$;
SQL

if [[ -n "${CUSTOM_SQL}" ]]; then
  docker exec -e PGPASSWORD="${password}" "${container}" \
    psql --username=postgres --dbname=aulalite_verify --set ON_ERROR_STOP=1 \
    --file=/backup/custom-verify.sql
fi

printf 'restore verification complete: %s\n' "${archive}"
printf 'verified with image: %s\n' "${VERIFY_IMAGE}"
