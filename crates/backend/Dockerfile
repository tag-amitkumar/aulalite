# syntax=docker/dockerfile:1.7

# Base images are pinned by digest for reproducible production builds. The digests
# below are the ones BuildKit resolved on a known-good build; the trailing tag is
# informational. Bump deliberately (re-resolve with `docker buildx imagetools
# inspect <tag>`), never silently.
FROM rust:1.94-slim-bookworm@sha256:cf9dd0ec73e75f827fe59123fff9dc65af1a1c8363c3c31ee8d7f8ad0b6a5fb2 AS builder
WORKDIR /app

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev ca-certificates git \
    && rm -rf /var/lib/apt/lists/*

# The design-system crate pulls dioxus-kinetics from a PUBLIC GitHub repo, pinned
# by rev for reproducibility. The fetch is anonymous (no credentials needed). The
# git CLI is used for cargo's fetch (libgit2 works for public repos too; the CLI
# is kept for parity with cargo's git handling).
ENV CARGO_NET_GIT_FETCH_WITH_CLI=true

# Bound peak build memory. A bare `docker compose build` compiles the backend and
# the frontend (WASM) images CONCURRENTLY; from a cold cache two unbounded rustc
# fleets (defaulting to ~nproc jobs × 16 codegen-units each) exhaust the WSL2/Docker
# Desktop VM and BuildKit drops the build's gRPC stream with the misleading
# "rpc error: code = Unavailable desc = error reading from server: EOF" — that is a
# memory failure, NOT an auth failure. Capping jobs + codegen-units bounds per-image
# peak RSS so the two builds survive running side by side. This lives in the builder
# stage (not Cargo.toml [profile.release]) so host / CI / desktop / mobile builds
# keep full parallelism.
ENV CARGO_BUILD_JOBS=4
ENV CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1

# Copy the workspace inputs needed to compile the backend, including embedded
# sqlx migrations.
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates/ crates/
COPY tools/ tools/
COPY migrations/ migrations/

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,id=aulalite-backend-target-bookworm,target=/app/target \
    cargo build --release -p backend && \
    cp /app/target/release/backend /usr/local/bin/backend

FROM debian:bookworm-slim@sha256:96e378d7e6531ac9a15ad505478fcc2e69f371b10f5cdf87857c4b8188404716

# curl is used by the compose/image healthcheck (`/healthz`); ffmpeg by the
# recording remux pipeline; ca-certificates for outbound TLS (Firebase JWKS,
# Stripe, etc.). Create an unprivileged `app` user so the runtime never runs as
# root.
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates ffmpeg curl \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd -r app && useradd -r -g app -s /usr/sbin/nologin app

COPY --from=builder /usr/local/bin/backend /usr/local/bin/backend
EXPOSE 8080
ENV RUST_LOG=info
USER app

# Image-level liveness probe for `docker run` / registry contexts. The compose
# healthcheck (docker-compose.yml) overrides this where it is defined.
HEALTHCHECK --interval=10s --timeout=5s --start-period=20s --retries=12 \
    CMD curl -fsS http://localhost:8080/healthz || exit 1

CMD ["/usr/local/bin/backend"]
