# Cloudflare Tunnel ingress — how it is ACTUALLY configured

Written 2026-09-03 while putting MediaMTX on a public hostname.

## There is no live config.yml. Do not create one.

`config.yml.template` in this directory describes a **locally-managed** tunnel with
a cloudflared container inside the compose network (`http://frontend:3000` origins).
That is **not** this deployment. Editing it changes nothing.

The real connector is the Windows service **`Cloudflared`**, running as LocalSystem:

```
"C:\Program Files (x86)\cloudflared\cloudflared.exe" tunnel run --token-file C:\ProgramData\cloudflared\token
```

Because it runs with `--token-file`, the tunnel is **remotely managed**: ingress rules
live in Cloudflare, not on disk. Dropping a `config.yml` next to the token does nothing
— the service never reads it. (A previous attempt to convert the service to a local
config also fails without an elevated shell: `sc.exe config` returns `Access is denied`.)

Two consequences that cost real debugging time:

* Because the connector is a **host** process, it reaches the stack only over Docker's
  **published loopback ports**. Every origin must be `http://localhost:<port>`. A Docker
  service name like `http://mediamtx:8889` does not resolve there and 502s at the edge.
* Config changes apply **live**, within seconds, with no service restart.

## Live ingress (as applied)

| hostname | origin | what it is |
|---|---|---|
| `aula.elementors.guru` | `http://localhost:3000` | app + API (nginx proxies `/v1/*` and the live-room WebSocket) |
| `media.elementors.guru` | `http://localhost:8889` | **MediaMTX WebRTC signalling** (WHIP/WHEP) |
| `media.aula.elementors.guru` | `http://localhost:8889` | same, pre-routed but TLS-blocked — see below |
| `stream.elementors.guru` | `http://localhost:8888` | MediaMTX HLS (watch-only fallback) |
| `stream.aula.elementors.guru` | `http://localhost:8888` | same, TLS-blocked |
| *(catch-all)* | `http_status:404` | refuse rather than silently serve |

MediaMTX's API (9997) and metrics (9998) are deliberately **not** exposed — they are
unauthenticated control surfaces.

`elementors.guru` / `www.elementors.guru` are deliberately absent: the marketing site is
a **separate PHP origin** (`X-Powered-By: PHP/7.4.33`) reached by its own proxied DNS
records. It must never be routed through this tunnel.

## Why the media hostname is single-level

Cloudflare **Universal SSL** issues a certificate for exactly `elementors.guru` and
`*.elementors.guru`. A wildcard matches **one** label, so `media.aula.elementors.guru`
has **no certificate** and its TLS handshake fails before any HTTP request happens:

```
$ echo | openssl s_client -connect aula.elementors.guru:443 -servername aula.elementors.guru \
    | openssl x509 -noout -text | grep -A2 "Subject Alternative Name"
    DNS:elementors.guru, DNS:*.elementors.guru
```

That is why `aula.elementors.guru` works but a `media.aula.` name cannot on this plan.
The working media hostnames are therefore single-level, which also matches the convention
this repo started with in `tunnel.env` (`stream.` / `live.`).

The `*.aula.` variants are already routed and already have DNS, so they start working the
moment **Advanced Certificate Manager / Total TLS** is enabled for `*.aula.elementors.guru`
— then only `MEDIAMTX_PUBLIC_WEBRTC_URL` / `MEDIAMTX_PUBLIC_HLS_URL` in `.env` need
changing, plus a `docker compose up -d backend`.

## How to change ingress

Ingress is edited in the dashboard (Zero Trust → Networks → Tunnels), or via the API.
`cloudflared tunnel login` writes `~/.cloudflared/cert.pem`, which is an
`ARGO TUNNEL TOKEN` — base64 JSON carrying `zoneID`, `accountID` and an `apiToken` scoped
to tunnel + DNS management. That token is what the commands below use. It does **not**
grant Realtime/TURN or other account APIs (those return `code 10000 Authentication error`).

```bash
ACC=abc2e53c1385d55c68f5ffd4c74644cd
TID=2d265905-d028-4c0c-9658-41e2d711c8e1
TOK=$(grep -v '^-----' ~/.cloudflared/cert.pem | tr -d '\n' \
      | base64 -d | node -e 'let r="";process.stdin.on("data",d=>r+=d);process.stdin.on("end",()=>console.log(JSON.parse(r).apiToken))')

# READ the current config FIRST — a PUT replaces the whole ingress array, so
# omitting a hostname silently deletes its route.
curl -s -H "Authorization: Bearer $TOK" \
  "https://api.cloudflare.com/client/v4/accounts/$ACC/cfd_tunnel/$TID/configurations"

# WRITE (body shape: {"config": {"ingress": [...], "warp-routing": {...}}})
curl -s -X PUT -H "Authorization: Bearer $TOK" -H 'Content-Type: application/json' \
  --data @new-config.json \
  "https://api.cloudflare.com/client/v4/accounts/$ACC/cfd_tunnel/$TID/configurations"
```

The catch-all `http_status:404` rule must stay **last** — rules match in order.

DNS records are separate from ingress. A hostname needs both: a proxied CNAME to
`<TID>.cfargotunnel.com` **and** an ingress rule. Create the CNAME with:

```bash
cloudflared tunnel route dns 2d265905-d028-4c0c-9658-41e2d711c8e1 media.elementors.guru
```

Missing DNS looks like `NXDOMAIN`; missing ingress looks like a 404 from the catch-all —
distinguish the catch-all's empty body from MediaMTX's own 18-byte `404 page not found`.
