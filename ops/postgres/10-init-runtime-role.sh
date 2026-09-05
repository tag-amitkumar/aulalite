#!/bin/sh
set -eu

# SQL migrations intentionally create this role without a password because
# credentials must never be committed to migration history. A fresh Compose
# deployment still needs the least-privileged login to exist before the backend
# migrates and switches from MIGRATION_DATABASE_URL to DATABASE_URL, so create
# it here from Dokploy's secret environment.
if [ -z "${POSTGRES_RUNTIME_PASSWORD:-}" ]; then
    echo >&2 "POSTGRES_RUNTIME_PASSWORD is required to initialize the aulalite_app database role"
    exit 1
fi

psql --username "$POSTGRES_USER" --dbname "$POSTGRES_DB" \
    --set=ON_ERROR_STOP=1 \
    --set=runtime_password="$POSTGRES_RUNTIME_PASSWORD" <<'SQL'
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'aulalite_app') THEN
        CREATE ROLE aulalite_app;
    END IF;
END
$$;

ALTER ROLE aulalite_app
    LOGIN NOSUPERUSER NOBYPASSRLS NOCREATEDB NOCREATEROLE NOREPLICATION
    PASSWORD :'runtime_password';
SQL
