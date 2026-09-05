#!/usr/bin/env bash
# Start AulaLite with the Cloudflare Tunnel attached.
#
# Loads: repo .env  ->  generated secrets  ->  tunnel hostnames, then exports
# the multi-line viewer-JWT PEM (which --env-file cannot carry) before
# handing off to docker compose.
#
# The base docker-compose.override.yml is deliberately NOT in the file list:
# it is the localhost-only port remap and has no role in a tunnel deployment.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"

[ -f "$here/config.yml" ] || { echo "ERROR: run ./render-config.sh first." >&2; exit 1; }
[ -f "$here/jwt-rs256.key" ] || { echo "ERROR: run ./generate-secrets.sh first." >&2; exit 1; }

set -a
. "$here/secrets.generated.env"
. "$here/tunnel.env"
set +a
# Real newlines survive here; they would not survive --env-file.
export JWT_RS256_PRIVATE_KEY_PEM="$(cat "$here/jwt-rs256.key")"

cd "$root"
exec docker compose \
  -f docker-compose.yml \
  -f docker-compose.cloudflare.yml \
  --env-file .env \
  --env-file "$here/secrets.generated.env" \
  --env-file "$here/tunnel.env" \
  "$@"
