# AulaLite — Cloudflare Tunnel deployment

## Why a tunnel and not a proxied A record

The Docker host is `192.168.1.2` — a private address behind NAT on Wi-Fi,
with no public IP. There is no origin address a proxied Cloudflare A record
could point at, and no inbound port can be opened. A named tunnel has the
origin dial *out* to Cloudflare, so it needs no static IP and no firewall
change. Dokploy/Traefik (`docker-compose.dokploy.yml`) is the other supported
edge, but it assumes a public origin host and is explicitly out of scope here.

## Routing

| Public hostname | Origin (inside the compose network) | Carries |
|---|---|---|
| `aula.elementors.guru` | `frontend:3000` | SPA **and** `/v1/*` API + live-room WebSocket |
| `stream.elementors.guru` | `mediamtx:8888` | HLS playback |
| `live.elementors.guru` | `mediamtx:8889` | WHIP/WHEP signalling only (see caveat) |

App and API deliberately share one hostname: the frontend nginx already
reverse-proxies `/v1/*` (including the WebSocket upgrade) to `backend:8080`,
so every API call is same-origin and no CORS preflight can fail at the edge.
That is why `AULALITE_API_BASE_URL` is empty.

TLS terminates at the Cloudflare edge. The tunnel leg is an encrypted QUIC
connection, so plain HTTP inside the compose network is correct and is not a
mixed-content source — the browser only ever speaks HTTPS to the edge.

## Live video: what actually works through a tunnel

**HLS playback works. WebRTC publishing does not, without extra setup.**

Only the WHIP/WHEP signalling handshake (an HTTP POST carrying the SDP) is
HTTP. The negotiated media is ICE/UDP straight to the address MediaMTX
advertises in `webrtcAdditionalHosts`, on UDP 8189. A Cloudflare Tunnel
carries HTTP/WebSocket over TCP and does not forward arbitrary UDP, so the
SDP exchange succeeds and then no RTP ever arrives. To publish live you need
one of:

- a **TURN relay** reachable over TCP/TLS 443 — set `AULALITE_TURN_URL`,
  `AULALITE_TURN_USERNAME`, `AULALITE_TURN_CREDENTIAL` (the backend already
  forwards these to clients as `RTCIceServer` entries, see
  `crates/backend/src/services/ice.rs`);
- a routable public IP with UDP 8189 forwarded, listed in
  `MEDIAMTX_ADDITIONAL_HOSTS`;
- Cloudflare Spectrum (Enterprise) for UDP.

Until then, use the HLS route for viewing.

## Setup

1. **Authenticate (you must do this — it opens a browser):**
   ```
   cloudflared tunnel login
   ```
   Pick the `elementors.guru` zone. This writes `~/.cloudflared/cert.pem`.

2. **Create the tunnel, DNS records and ingress config:**
   ```
   ./generate-secrets.sh          # once; self-contained secrets
   ./post-login-setup.sh aulalite # tunnel + CNAMEs + config.yml
   ```

3. **Start it:**
   ```
   ./deploy.sh up -d
   ```

`deploy.sh` uses an explicit compose file list that excludes
`docker-compose.override.yml`, so the localhost dev setup is untouched.

## Cloudflare Access (required before this is internet-reachable)

The stack runs `APP_ENV=local` with `LOCAL_LOGIN_BYPASS_ENABLED=true`, so a
known email/password pair is platform super-admin. Do **not** expose
`aula.elementors.guru` without a gate. In the Zero Trust dashboard:

Access → Applications → Add → Self-hosted
- Application domain: `aula.elementors.guru`
- Policy: Allow → Emails → *your address*
- Session duration: to taste

Apply Access to the **app hostname only**. Gating `stream.` / `live.` would
break media playback, because the player cannot complete an Access login
redirect. Those endpoints stay protected by the app's own viewer JWTs —
MediaMTX authenticates every read and publish against
`/v1/mediamtx/auth/publish` (`authHTTPExclude` covers only api/metrics/pprof).

## Files

| File | Purpose | Committed? |
|---|---|---|
| `config.yml.template` | ingress rules | yes |
| `tunnel.env` | hostnames + tunnel UUID | no (gitignored) |
| `config.yml` | rendered ingress | no |
| `secrets.generated.env` | generated secrets | no |
| `jwt-rs256.key` | viewer-JWT signing key | no |
| `<uuid>.json` | tunnel credentials | no |
