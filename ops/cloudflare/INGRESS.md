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
| `stream.elementors.guru` | `http://localhost:8888` | MediaMTX HLS (watch-only fallback) |
| `storage.elementors.guru` | `http://localhost:9000` | **RustFS S3** — presigned recording playback, uploads, attachments |
| *(catch-all)* | `http_status:404` | refuse rather than silently serve |

The `*.aula.` variants described in earlier revisions of this file are no longer in the
live config; the table above is the applied ingress as of 2026-09-06.

MediaMTX's API (9997) and metrics (9998) are deliberately **not** exposed — they are
unauthenticated control surfaces. Neither is the RustFS **console on 9001**: only the S3
API on 9000 is routed. (9001 is still published on `0.0.0.0` by compose, so it remains
reachable from the LAN even though it is not on the internet.)

## `storage.elementors.guru` — why it exists and what it must not have

Added 2026-09-06 (fix-plan phase C). Recording playback was dead in the browser: the
backend signs presigned URLs with `S3_ENDPOINT_URL`, that was `http://localhost:9000`, and
the app's CSP is `media-src 'self' blob: https:`. Chrome rejected the URL before issuing a
single request — `securitypolicyviolation` on `media-src`, `MEDIA_ELEMENT_ERROR: Media load
rejected by URL safety check`. The host is inside the SigV4 signature, so it cannot be
rewritten client-side; the origin itself had to become https.

**No Cloudflare Access on this hostname.** The browser fetches these URLs unauthenticated —
SigV4 in the query string *is* the authentication. An Access policy would block playback.

**It cannot be a path prefix under `aula.`** — `force_path_style(true)` signs the full path,
so any nginx prefix rewrite breaks the SigV4 canonical URI and returns 403.

**Rotate the RustFS root credentials before ever exposing 9000** (done 2026-09-06). The key
id travels in plaintext in every presigned URL as `X-Amz-Credential`, so shipping the
`aulalite` / `changeme123` defaults would hand an attacker half the pair and, with it, full
read/write/delete over every tenant's recordings, submissions and attachments.

Verified after the change: presigned GET returns 200 with byte-identical content through the
edge, `Range` requests return 206 (seeking works), unsigned GET and bucket listing return
403, and presigned PUT uploads succeed. Note Cloudflare caps request bodies at **100 MB** on
free/pro, while the app permits 500 MB for `video` and `scorm` — uploads above the cap will
fail at the edge with 413 even though the app would accept them.

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

## The edge rewrites HTML: Web Analytics beacon vs CSP

Verified end-to-end 2026-09-06 through the public hostnames (not localhost).

The zone has Web Analytics **automatic injection** enabled, so Cloudflare
rewrites every HTML response to add
`<script src="https://static.cloudflareinsights.com/beacon.min.js/...">`. Our
origin never emits it -- confirmed by diffing origin and edge HTML.

Two traps:

* **A plain `curl` does not reproduce the injection.** Cloudflare only rewrites
  when the request looks like a browser navigation (`Accept: text/html` plus a
  browser `User-Agent`). `curl https://aula.elementors.guru/` returns the
  unmodified 4598-byte shell, so the beacon looks absent until you check with a
  real browser.
* **It was blocked by our own CSP**, which cost twice over: a
  `script-src-elem` violation logged on every page load for every visitor --
  noise that would mask a genuine CSP failure -- while Web Analytics silently
  collected nothing despite being switched on. `script-src` now allows
  `https://static.cloudflareinsights.com` (both CSP variants in
  `crates/shell-web/Dockerfile`). If the beacon is unwanted, turn injection off
  in the dashboard instead and revert that entry.

What was checked, all green: DNS and edge reachability for all four hostnames,
with every edge status matching its localhost origin; `/v1/me` identical through
the edge; the live-room WebSocket upgrading with `101 Switching Protocols` at
both edge and origin; a teacher publishing WHIP to `media.` (201, publisher
connected, MediaMTX path `ready`); a student decoding real WHEP video (640x480,
63 frames) ; and HLS through `stream.` serving master playlist, media playlist
and a 99,979-byte `video/mp4` segment.

Two things that look like faults and are not: `/v1/healthz` 404s at the edge
**and** at the origin (it simply is not a route -- the backend serves
`/healthz`), and `gap.mp4` in a media playlist returns 401. The latter is the
placeholder MediaMTX lists against an `#EXT-X-GAP` entry; players skip those
rather than fetch them, so fetching one proves nothing.

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
