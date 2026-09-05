#!/usr/bin/env bash
# Render config.yml.template -> config.yml using ops/cloudflare/tunnel.env.
set -euo pipefail
cd "$(dirname "$0")"
[ -f tunnel.env ] || { echo "ERROR: tunnel.env missing. Copy tunnel.env.template and fill it in." >&2; exit 1; }
set -a; . ./tunnel.env; set +a
missing=""
for v in CF_TUNNEL_ID CF_APP_HOSTNAME CF_HLS_HOSTNAME CF_WEBRTC_HOSTNAME; do
  [ -n "${!v:-}" ] || missing="$missing $v"
done
[ -z "$missing" ] || { echo "ERROR: tunnel.env is missing values for:$missing" >&2; exit 1; }
[ -f "${CF_TUNNEL_ID}.json" ] || echo "WARNING: ${CF_TUNNEL_ID}.json not found here; copy the tunnel credentials file into this directory before starting." >&2
envsubst < config.yml.template > config.yml
echo "Rendered config.yml for tunnel ${CF_TUNNEL_ID}."
