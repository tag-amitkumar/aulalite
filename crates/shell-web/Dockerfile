# syntax=docker/dockerfile:1.7

# Pinned by digest for reproducible builds (trailing tag is informational). The
# builder uses the same -slim-bookworm base as the backend so both Rust compiles
# share one toolchain image and the frontend builder no longer silently tracks a
# newer Debian. Bump deliberately, never silently.
FROM rust:1.94-slim-bookworm@sha256:cf9dd0ec73e75f827fe59123fff9dc65af1a1c8363c3c31ee8d7f8ad0b6a5fb2 AS builder
WORKDIR /app

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev ca-certificates binaryen git \
    && rm -rf /var/lib/apt/lists/*

# design-system pulls dioxus-kinetics from a PUBLIC GitHub repo (anonymous fetch,
# no credentials). The git CLI is kept for cargo's fetch.
ENV CARGO_NET_GIT_FETCH_WITH_CLI=true

# Bound peak build memory so a concurrent backend+frontend `docker compose build`
# does not OOM the WSL2/Docker VM and drop BuildKit's gRPC stream. See the matching
# block in crates/backend/Dockerfile for the full explanation. These also apply to
# the `cargo install dioxus-cli` compile below (it inherits the env).
ENV CARGO_BUILD_JOBS=4
ENV CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1

RUN rustup target add wasm32-unknown-unknown

# The CLI changes far less often than application source. Install it before the
# workspace COPY so a normal GitHub push reuses this expensive compiled layer
# instead of rebuilding dx on every source edit.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    cargo install dioxus-cli --version 0.7.9 --locked

# Peak-memory tunable for the WORKSPACE compile below.
#
# Each concurrent rustc on this release profile (codegen-units=1) can hold well
# over a gigabyte. On a host with little free RAM the WSL2/Docker VM gets
# OOM-killed mid-compile and BuildKit's gRPC stream dies with
# "failed to receive status: rpc error: ... EOF", which reads like a Docker bug
# rather than the memory problem it is. Such a host builds with:
#   docker compose build --build-arg CARGO_BUILD_JOBS=2 frontend
#
# Declared HERE, after the dioxus-cli install, on purpose: an ARG above that
# step would invalidate its expensive cached layer for every host that
# overrides the value. dioxus-cli keeps the default job count from the ENV
# above -- it is compiled once and then cached, whereas the workspace compile
# below re-runs on every source change and is the one that actually OOMs.
ARG CARGO_BUILD_JOBS=4
ENV CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS}

COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates/ crates/
COPY tools/ tools/
COPY migrations/ migrations/

# Dioxus optimizes public .js assets with esbuild. The vendored MediaPipe
# Emscripten loaders contain Node fallback branches (`require("fs")` /
# `require("path")`) that are safe in browsers but produce noisy esbuild errors
# before Dioxus falls back to copying. Keep those files out of the optimizer and
# restore them verbatim into the final static bundle.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    mkdir -p /tmp/mediapipe-wasm-js && \
    mv crates/shell-web/public/vendor/mediapipe/wasm/*_internal.js /tmp/mediapipe-wasm-js/ && \
    rm -rf /app/target/dx/shell-web/release/web && \
    dx build --package shell-web --platform web --release && \
    mkdir -p /tmp/frontend-dist && \
    cp -r /app/target/dx/shell-web/release/web/public/* /tmp/frontend-dist/ && \
    mkdir -p /tmp/frontend-dist/vendor/mediapipe/wasm && \
    cp /tmp/mediapipe-wasm-js/*_internal.js /tmp/frontend-dist/vendor/mediapipe/wasm/

# Unprivileged nginx (runs as uid 101 `nginx`, never root). Pinned by digest.
FROM nginxinc/nginx-unprivileged:alpine@sha256:85bcbc6b2edd325462560c597d784ecee415024f1c6a004e53ac5f202b8ca561
COPY --from=builder /tmp/frontend-dist/ /usr/share/nginx/html/
COPY --chmod=0555 crates/shell-web/docker-entrypoint.d/40-aulalite-runtime-config.sh /docker-entrypoint.d/40-aulalite-runtime-config.sh

# Write the site config as root at build time, then drop back to the image's
# unprivileged `nginx` user for runtime. Only the generated runtime config and
# rendered index are writable; the rest of the static bundle stays immutable.
USER root
RUN cp /usr/share/nginx/html/index.html /usr/share/nginx/html/index.template.html && \
    touch /usr/share/nginx/html/runtime-config.js && \
    chown nginx:nginx \
      /usr/share/nginx/html/index.html \
      /usr/share/nginx/html/runtime-config.js && \
    cat <<'EOF' > /etc/nginx/conf.d/default.conf
map $http_upgrade $connection_upgrade {
    default upgrade;
    '' close;
}

# Never write query strings or bearer/invitation secrets to container logs.
# `$uri` is query-free; the map also removes the one secret-bearing path
# segment used by course invitation acceptance.
map $uri $safe_log_uri {
    ~^/v1/invitations/ /v1/invitations/[redacted]/accept;
    default $uri;
}
log_format aulalite_safe '$remote_addr - $request_method $safe_log_uri '
                         'status=$status bytes=$body_bytes_sent '
                         'request_id=$request_id duration=$request_time';
access_log /dev/stdout aulalite_safe;

# The bundled production path is Traefik -> this nginx container. Trust only
# private container-network hops to normalize Traefik's client address; direct
# public connections cannot supply a trusted forwarding header.
set_real_ip_from 10.0.0.0/8;
set_real_ip_from 172.16.0.0/12;
set_real_ip_from 192.168.0.0/16;
real_ip_header X-Forwarded-For;
real_ip_recursive on;

server {
    listen 3000;
    root /usr/share/nginx/html;
    index index.html;
    client_header_timeout 15s;
    client_body_timeout 30s;
    client_max_body_size 3m;
    keepalive_timeout 65s;
    send_timeout 60s;

    # Static Dioxus/WASM responses compress extremely well. This cuts transfer
    # size for the initial app and specialist JS without changing bundle code.
    gzip on;
    gzip_comp_level 6;
    gzip_min_length 1024;
    gzip_proxied any;
    gzip_vary on;
    gzip_types
        application/javascript
        application/json
        application/manifest+json
        application/wasm
        image/svg+xml
        text/css;

    # Security headers for the document origin. Bootstrap code is external and
    # Rust browser mutations use typed DOM APIs, so JavaScript eval and inline
    # scripts stay disabled. Production startup separately requires HTTPS media
    # endpoints; broad HTTPS/WSS connect/media sources accommodate customer
    # Firebase and MediaMTX hostnames without weakening script execution.
    #
    # static.cloudflareinsights.com is Cloudflare's Web Analytics beacon. The
    # zone has automatic injection enabled, so the EDGE rewrites every HTML
    # response to add <script src="https://static.cloudflareinsights.com/
    # beacon.min.js/...">. Our origin never emits it -- verified by diffing the
    # origin and edge HTML, and note a plain curl does NOT reproduce it:
    # Cloudflare only injects when the request looks like a browser navigation
    # (Accept: text/html + a browser UA).
    #
    # Without this entry the injected script is blocked, which cost twice: a
    # script-src-elem violation logged on EVERY page load for every visitor --
    # noise that would mask a real CSP failure -- and Web Analytics silently
    # collecting nothing while appearing to be on. Allowlisting reconciles the
    # policy with what the account is already configured to do. The alternative
    # is to turn injection off (Cloudflare dashboard -> Web Analytics), which
    # needs no code change; do that instead if the beacon is unwanted.
    add_header Content-Security-Policy "default-src 'self'; base-uri 'self'; object-src 'none'; frame-ancestors 'none'; script-src 'self' 'wasm-unsafe-eval' https://www.gstatic.com https://static.cloudflareinsights.com; script-src-attr 'none'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob: https:; font-src 'self' data:; connect-src 'self' https: wss:; media-src 'self' blob: https:; worker-src 'self' blob:; frame-src 'self' https: blob:; manifest-src 'self'; form-action 'self'" always;
    add_header X-Content-Type-Options "nosniff" always;
    add_header X-Frame-Options "DENY" always;
    add_header Referrer-Policy "strict-origin-when-cross-origin" always;
    add_header Permissions-Policy "camera=(self), microphone=(self), display-capture=(self), fullscreen=(self), geolocation=()" always;
    add_header Strict-Transport-Security "max-age=31536000; includeSubDomains" always;

    # LTI launches commonly render inside an LMS iframe. This exact landing
    # route intentionally permits HTTPS ancestors while the rest of the app is
    # non-frameable. Tenant-specific LMS allowlists remain enforced by the
    # signed LTI issuer/client/deployment checks before this route is reached.
    location = /lti/landing {
        add_header Content-Security-Policy "default-src 'self'; base-uri 'self'; object-src 'none'; frame-ancestors https:; script-src 'self' 'wasm-unsafe-eval' https://www.gstatic.com https://static.cloudflareinsights.com; script-src-attr 'none'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob: https:; font-src 'self' data:; connect-src 'self' https: wss:; media-src 'self' blob: https:; worker-src 'self' blob:; frame-src 'self' https: blob:; manifest-src 'self'; form-action 'self'" always;
        add_header X-Content-Type-Options "nosniff" always;
        add_header Referrer-Policy "strict-origin-when-cross-origin" always;
        add_header Permissions-Policy "camera=(self), microphone=(self), display-capture=(self), fullscreen=(self), geolocation=()" always;
        add_header Strict-Transport-Security "max-age=31536000; includeSubDomains" always;
        # `break` keeps this request in the LTI location while serving the SPA
        # shell. A `try_files ... /index.html` fallback would internally
        # redirect into the generic location and restore frame-ancestors 'none'
        # (plus X-Frame-Options: DENY), breaking legitimate LMS embeds.
        rewrite ^ /index.html break;
    }

    location /v1/ {
        proxy_http_version 1.1;
        proxy_connect_timeout 5s;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        # `$remote_addr` is the normalized address after the trusted Traefik
        # hop is processed above. Replace, never append, the inbound chain.
        proxy_set_header X-Forwarded-For $remote_addr;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection $connection_upgrade;
        proxy_read_timeout 75s;
        proxy_send_timeout 75s;
        proxy_pass http://backend:8080;
    }

    # Live-room WebSockets can sit idle for long stretches. Isolate their long
    # timeout from ordinary API calls so a stalled HTTP upstream cannot occupy a
    # proxy connection for an hour.
    location ~ ^/v1/sessions/[0-9a-fA-F-]+/socket$ {
        proxy_http_version 1.1;
        proxy_connect_timeout 5s;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $remote_addr;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection $connection_upgrade;
        proxy_read_timeout 3600s;
        proxy_send_timeout 3600s;
        proxy_pass http://backend:8080;
    }

    # Deployment-specific publishable config is regenerated on every container
    # start. Never let browsers or intermediary caches pin an old tenant/auth
    # configuration; index.html carries a matching content-version query too.
    location = /runtime-config.js {
        add_header Cache-Control "no-store, no-cache, must-revalidate" always;
        add_header Pragma "no-cache" always;
        expires -1;
        try_files $uri =404;
    }

    # Workers must revalidate so the browser notices each deployment. The app
    # worker itself gives fingerprinted assets a bounded offline cache.
    location = /service-worker.js {
        add_header Cache-Control "no-cache, must-revalidate" always;
        expires -1;
        try_files $uri =404;
    }

    location = /firebase-messaging-sw.js {
        add_header Cache-Control "no-cache, must-revalidate" always;
        expires -1;
        try_files $uri =404;
    }

    # Give first-time visitors useful HTTP caching before a service worker has
    # taken control. Stable-path assets revalidate daily; deploy fingerprints
    # and stale-while-revalidate prevent long-lived release pinning.
    location ~* \.(?:css|js|mjs|wasm|woff2|svg|png|webmanifest|tflite)$ {
        add_header Cache-Control "public, max-age=86400, stale-while-revalidate=604800" always;
        try_files $uri =404;
    }

    location / {
        try_files $uri $uri/ /index.html;
    }
}
EOF
USER nginx

EXPOSE 3000

# Image-level liveness probe (compose healthcheck overrides where defined). wget
# is the BusyBox built-in present in the alpine image (no curl). Use 127.0.0.1,
# not localhost: nginx binds IPv4 0.0.0.0:3000 only and BusyBox wget would try
# ::1 (IPv6) for localhost and get connection-refused.
HEALTHCHECK --interval=10s --timeout=5s --start-period=10s --retries=6 \
    CMD wget -q --spider http://127.0.0.1:3000/ || exit 1
