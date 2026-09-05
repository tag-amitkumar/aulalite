#!/usr/bin/env bash
# Mint Cloudflare Realtime TURN credentials and write them into .env.
#
# WHY THIS EXISTS: Cloudflare Realtime TURN issues TIME-LIMITED credentials,
# but this deployment feeds TURN to both the backend (AULALITE_TURN_*, handed
# to browsers) and MediaMTX (MTX_WEBRTCICESERVERS2_1_*, used for its own
# candidates) as STATIC environment variables. So the credentials must be
# re-minted before they expire, or every cross-network call silently loses its
# relay and students see a black video with a successful 201 on the WHEP POST.
#
# Run it manually, or on a schedule that is comfortably shorter than the TTL.
# Re-minting alone is not enough: compose interpolates ${VAR} at container
# CREATE time, so the affected containers have to be recreated (--recreate).
#
# Usage:
#   ops/cloudflare/turn-credentials.sh --key-id <TURN_TOKEN_ID> \
#       --api-token <API_TOKEN> [--ttl 86400] [--recreate]
#
# The key id and api token come from the Cloudflare dashboard:
#   Realtime -> TURN Keys -> Create.
#
# They are persisted to .env as AULALITE_TURN_KEY_ID / AULALITE_TURN_API_TOKEN
# so later refreshes need no arguments.

set -euo pipefail
cd "$(dirname "$0")/../.."   # repo root

KEY_ID=""; API_TOKEN=""; TTL=86400; RECREATE=0
while [ $# -gt 0 ]; do
  case "$1" in
    --key-id)    KEY_ID="$2"; shift 2 ;;
    --api-token) API_TOKEN="$2"; shift 2 ;;
    --ttl)       TTL="$2"; shift 2 ;;
    --recreate)  RECREATE=1; shift ;;
    *) echo "unknown arg: $1" >&2; exit 2 ;;
  esac
done

envget() { grep -m1 "^$1=" .env 2>/dev/null | cut -d= -f2- || true; }

# Fall back to the values stored by a previous run so scheduled refreshes need
# no secrets on the command line (where they would land in a task definition).
[ -n "$KEY_ID" ]    || KEY_ID="$(envget AULALITE_TURN_KEY_ID)"
[ -n "$API_TOKEN" ] || API_TOKEN="$(envget AULALITE_TURN_API_TOKEN)"
if [ -z "$KEY_ID" ] || [ -z "$API_TOKEN" ]; then
  echo "need --key-id and --api-token (or AULALITE_TURN_KEY_ID / AULALITE_TURN_API_TOKEN in .env)" >&2
  exit 2
fi

echo "minting TURN credentials (ttl=${TTL}s)…"
RESP="$(curl -sS -X POST \
  "https://rtc.live.cloudflare.com/v1/turn/keys/${KEY_ID}/credentials/generate-ice-servers" \
  -H "Authorization: Bearer ${API_TOKEN}" \
  -H "Content-Type: application/json" \
  -d "{\"ttl\":${TTL}}")"

# Parse without jq (not installed on this host). node ships with the repo's
# playwright dev-dependency toolchain and is always present here.
PARSED="$(printf '%s' "$RESP" | node -e '
let raw = "";
process.stdin.on("data", d => raw += d);
process.stdin.on("end", () => {
  let j;
  try { j = JSON.parse(raw); } catch { console.error("non-JSON response: " + raw.slice(0,300)); process.exit(1); }
  const s = (j.iceServers && (Array.isArray(j.iceServers) ? j.iceServers[0] : j.iceServers));
  if (!s || !s.username) { console.error("no credentials in response: " + raw.slice(0,300)); process.exit(1); }
  const urls = [].concat(s.urls || s.url || []);
  // Keep only relay URLs that survive a restrictive network / Cloudflare
  // Tunnel path: TLS on 5349 and plain TCP. A udp-only relay is useless to a
  // student behind a firewall that blocks outbound UDP.
  const pref = urls.filter(u => /^turns:/.test(u) || /transport=tcp/.test(u));
  const chosen = (pref.length ? pref : urls);
  // The browser list can carry several URLs (ice.rs accepts a comma list);
  // MediaMTX takes exactly ONE url per iceServers2 entry.
  const mtx = chosen.find(u => /^turns:/.test(u)) || chosen[0];
  console.log(JSON.stringify({ browser: chosen.join(","), mtx, username: s.username, credential: s.credential }));
});
')"

BROWSER_URLS="$(printf '%s' "$PARSED" | node -e 'let r="";process.stdin.on("data",d=>r+=d);process.stdin.on("end",()=>console.log(JSON.parse(r).browser))')"
MTX_URL="$(printf '%s' "$PARSED"     | node -e 'let r="";process.stdin.on("data",d=>r+=d);process.stdin.on("end",()=>console.log(JSON.parse(r).mtx))')"
USERNAME="$(printf '%s' "$PARSED"    | node -e 'let r="";process.stdin.on("data",d=>r+=d);process.stdin.on("end",()=>console.log(JSON.parse(r).username))')"
CREDENTIAL="$(printf '%s' "$PARSED"  | node -e 'let r="";process.stdin.on("data",d=>r+=d);process.stdin.on("end",()=>console.log(JSON.parse(r).credential))')"

cp .env ".env.bak.turnrefresh.$(date +%Y%m%d%H%M%S)"

setenv() {  # setenv KEY VALUE  — replace in place, or append if absent
  local key="$1" val="$2"
  if grep -q "^${key}=" .env; then
    # `|` delimiter: TURN urls contain ':' and '?' but never '|'
    sed -i "s|^${key}=.*$|${key}=${val}|" .env
  else
    printf '%s=%s\n' "$key" "$val" >> .env
  fi
}

setenv AULALITE_TURN_URL          "$BROWSER_URLS"
setenv AULALITE_TURN_USERNAME     "$USERNAME"
setenv AULALITE_TURN_CREDENTIAL   "$CREDENTIAL"
setenv AULALITE_MEDIAMTX_TURN_URL "$MTX_URL"
setenv AULALITE_TURN_KEY_ID       "$KEY_ID"
setenv AULALITE_TURN_API_TOKEN    "$API_TOKEN"
setenv AULALITE_TURN_MINTED_AT    "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
setenv AULALITE_TURN_TTL_SECONDS  "$TTL"

echo "browser urls : ${BROWSER_URLS}"
echo "mediamtx url : ${MTX_URL}"
echo "username     : ${USERNAME:0:12}…(${#USERNAME} chars)"
echo "expires      : $(date -u -d "+${TTL} seconds" +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || echo "+${TTL}s")"

if [ "$RECREATE" = "1" ]; then
  echo "recreating backend + mediamtx so the new values are interpolated…"
  docker compose -p aulalite --env-file .env up -d --no-deps backend mediamtx
  # nginx in the frontend resolves `backend` once at config load and has no
  # resolver directive, so a recreated backend must be followed by a frontend
  # reload or every /v1/* request 502s while the SPA still serves fine.
  docker compose -p aulalite restart frontend
fi
