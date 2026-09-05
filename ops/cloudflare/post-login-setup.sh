#!/usr/bin/env bash
# Runs everything that becomes possible once `cloudflared tunnel login` has
# written ~/.cloudflared/cert.pem. Idempotent: re-running reuses an existing
# tunnel and DNS records rather than duplicating them.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
cd "$here"

cert="${HOME}/.cloudflared/cert.pem"
[ -f "$cert" ] || { echo "ERROR: $cert not found. Run: cloudflared tunnel login" >&2; exit 1; }

name="${1:-aulalite}"

# 1. Create the tunnel (or reuse it if the name is already taken).
if cloudflared tunnel list --output json 2>/dev/null | grep -q "\"name\":\"${name}\""; then
  echo "Tunnel '${name}' already exists; reusing it."
else
  cloudflared tunnel create "$name"
fi

id=$(cloudflared tunnel list --output json | tr ',' '\n' | grep -B2 "\"name\":\"${name}\"" | grep '"id"' | head -1 | sed 's/.*"id":"\([^"]*\)".*/\1/')
[ -n "$id" ] || { echo "ERROR: could not resolve the tunnel UUID for '${name}'." >&2; exit 1; }
echo "Tunnel UUID: $id"

# 2. Put the credentials where the container's read-only mount can see them.
cp "${HOME}/.cloudflared/${id}.json" "./${id}.json"
chmod 600 "./${id}.json"

# 3. Record the UUID so render-config.sh and deploy.sh pick it up.
sed -i "s|^CF_TUNNEL_ID=.*|CF_TUNNEL_ID=${id}|" tunnel.env
set -a; . ./tunnel.env; set +a

# 4. DNS: one proxied CNAME per hostname -> <uuid>.cfargotunnel.com.
#    `route dns` creates it, and is a no-op error if it already points here.
for host in "$CF_APP_HOSTNAME" "$CF_HLS_HOSTNAME" "$CF_WEBRTC_HOSTNAME"; do
  echo "Routing $host ..."
  cloudflared tunnel route dns "$name" "$host" || \
    echo "  (already routed, or the record exists — verify in the dashboard)"
done

# 5. Render the ingress config.
./render-config.sh
echo
echo "Done. Next: ./deploy.sh up -d"
