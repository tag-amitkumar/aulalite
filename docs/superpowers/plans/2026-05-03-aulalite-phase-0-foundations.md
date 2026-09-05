# AulaLite — Phase 0 Foundations Implementation Plan

> **Historical plan:** the temporary Dioxus 0.7.4 pin below was required when
> later releases referenced an unpublished upstream package. It is superseded;
> active workspace, CLI, Docker, CI, and release configuration use 0.7.9.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stand up the AulaLite project skeleton end-to-end so a real Firebase user can sign in on web and mobile, the Rust API verifies the token, just-in-time-provisions a Postgres user row, resolves their tenant, and `/v1/me` returns their identity + role — with Postgres Row-Level Security in place from day one.

**Architecture:** Cargo workspace housing a Dioxus client (web + mobile + desktop shells, shared feature crates), a Rust + Axum backend, and Postgres + Redis + MinIO + MediaMTX running under Docker Compose on Dokploy (Traefik provided by Dokploy). Firebase Auth handles identity only; the API verifies ID tokens against Google's JWKS, JIT-creates Postgres rows, sets `app.tenant_id` per request via Postgres GUC, and uses RLS to isolate tenant data.

**Tech Stack:** Rust 1.94 (pinned via `rust-toolchain.toml`), Dioxus 0.7.4 (exact pin), Axum 0.7, sqlx 0.8 + Postgres 16, Redis 7, MinIO (RELEASE.2025+), MediaMTX latest stable, Docker Compose v2, Firebase Auth + Admin SDK (token verification via `jsonwebtoken` + cached JWKS).

**Companion spec:** `docs/superpowers/specs/2026-05-03-aulalite-scope-design.md` (full design context).

---

## Prerequisites (one-time, before Task 1)

These are environment-level setup items that aren't part of the iterative TDD loop. Do them once before starting, and verify each command succeeds.

- **Rust toolchain:** `rustup default stable` and `rustup target add wasm32-unknown-unknown`. Verify `rustc --version` ≥ 1.94. (Workspace pins `channel = "1.94"` in `rust-toolchain.toml`.)
- **Dioxus CLI:** `cargo install dioxus-cli --locked --version =0.7.4`. Verify `dx --version` reports 0.7.4. (The workspace pins `dioxus = "=0.7.4"`; 0.7.5+ pulls an unpublished `dioxus-fullstack` until upstream republishes.)
- **Docker Desktop** (or Docker Engine on Linux) ≥ 25, with Docker Compose v2. Verify `docker compose version`.
- **sqlx CLI:** `cargo install sqlx-cli --no-default-features --features rustls,postgres --locked`. Verify `sqlx --version`.
- **Postgres client (psql)** for local debugging. Verify `psql --version`.
- **Firebase project created** at console.firebase.google.com:
  - Two projects: `aulalite-dev` and `aulalite-prod` (prod will sit unused until launch).
  - In `aulalite-dev`, enable **Email/Password** and **Google** providers under Authentication → Sign-in method.
  - Generate a service account key for the dev project: Project settings → Service accounts → Generate new private key. Save the JSON to a path outside the repo and set `GOOGLE_APPLICATION_CREDENTIALS` for local dev.
  - Note the project ID, web API key, and the JWKS endpoint `https://www.googleapis.com/service_accounts/v1/jwk/securetoken@system.gserviceaccount.com`.
- **iOS toolchain** (only when starting Section H): macOS with Xcode 16+, `xcrun simctl list` works.
- **Android toolchain** (only when starting Section H): Android Studio installed, `ANDROID_HOME` set, `adb devices` works, an emulator AVD created.

---

## File Structure (target after Phase 0)

```
.
├── Cargo.toml                              # workspace root
├── rust-toolchain.toml                     # pin Rust version
├── docker-compose.yml                      # Dokploy stack
├── .env.example                            # env var template
├── .dockerignore
├── .gitignore
├── crates/
│   ├── core-types/
│   │   ├── Cargo.toml
│   │   └── src/lib.rs                     # shared DTOs, error enums, RBAC enums
│   ├── api-client/
│   │   ├── Cargo.toml
│   │   └── src/lib.rs                     # HTTP/WS client, Firebase token holder
│   ├── design-system/
│   │   ├── Cargo.toml
│   │   ├── src/lib.rs
│   │   ├── src/tokens.rs                  # color/spacing/typography tokens
│   │   ├── src/button.rs
│   │   ├── src/input.rs
│   │   ├── src/card.rs
│   │   ├── src/spinner.rs
│   │   ├── src/form_error.rs
│   │   └── assets/tokens.css              # CSS variables for the same tokens
│   ├── platform-bridge/
│   │   ├── Cargo.toml
│   │   └── src/lib.rs                     # PlatformBridge trait + per-platform impls
│   ├── features-auth/
│   │   ├── Cargo.toml
│   │   └── src/lib.rs                     # Login, Signup, ForgotPassword screens
│   ├── shell-web/
│   │   ├── Cargo.toml
│   │   ├── Dioxus.toml
│   │   ├── src/main.rs
│   │   ├── assets/
│   │   │   ├── index.html
│   │   │   └── firebase-bridge.js         # Firebase Web SDK shim
│   │   └── public/
│   ├── shell-mobile/
│   │   ├── Cargo.toml
│   │   ├── Dioxus.toml
│   │   └── src/main.rs
│   ├── shell-desktop/
│   │   ├── Cargo.toml
│   │   └── src/main.rs
│   └── backend/
│       ├── Cargo.toml
│       ├── Dockerfile
│       ├── src/main.rs
│       ├── src/config.rs
│       ├── src/db.rs                      # sqlx pool factory + tenant GUC helper
│       ├── src/auth/
│       │   ├── mod.rs
│       │   ├── jwks.rs                    # JWKS fetcher + cache
│       │   ├── verify.rs                  # ID token verification
│       │   ├── jit_provision.rs           # JIT user provisioning
│       │   └── middleware.rs              # extract → verify → context
│       ├── src/handlers/
│       │   ├── mod.rs
│       │   ├── health.rs                  # /healthz
│       │   └── me.rs                      # /v1/me
│       ├── src/context.rs                 # RequestContext struct
│       ├── src/error.rs                   # ApiError enum + IntoResponse
│       └── tests/
│           ├── rls_tenant_isolation.rs    # RLS verification integration test
│           └── me_endpoint.rs             # /v1/me integration test
├── migrations/                             # sqlx migrations directory
│   ├── 20260503000001_extensions.sql
│   ├── 20260503000002_tenants.sql
│   ├── 20260503000003_users.sql
│   └── 20260503000004_tenant_memberships.sql
├── ops/
│   ├── mediamtx/
│   │   └── mediamtx.yml                   # MediaMTX config skeleton
│   └── README.md                          # ops/runbook stub
├── tools/
│   └── aulalite-admin/                    # platform admin CLI binary
│       ├── Cargo.toml
│       └── src/main.rs
└── docs/
    ├── superpowers/
    │   ├── specs/2026-05-03-aulalite-scope-design.md   (already exists)
    │   └── plans/2026-05-03-aulalite-phase-0-foundations.md   (this file)
    └── README.md
```

---

# Section A — Cargo Workspace Skeleton

This section creates the empty workspace and all crates. Tests are minimal here; this is structural setup.

### Task 1: Initialize repo and Cargo workspace

**Files:**
- Create: `Cargo.toml`
- Create: `rust-toolchain.toml`
- Create: `.gitignore`
- Create: `.dockerignore`

- [ ] **Step 1: Initialize git repo**

```bash
git init
```

- [ ] **Step 2: Create `rust-toolchain.toml`**

```toml
[toolchain]
channel = "1.85"
components = ["rustfmt", "clippy"]
targets = ["wasm32-unknown-unknown"]
```

- [ ] **Step 3: Create workspace `Cargo.toml`**

```toml
[workspace]
resolver = "2"
members = [
    "crates/core-types",
    "crates/api-client",
    "crates/design-system",
    "crates/platform-bridge",
    "crates/features-auth",
    "crates/shell-web",
    "crates/shell-mobile",
    "crates/shell-desktop",
    "crates/backend",
    "tools/aulalite-admin",
]

[workspace.package]
version = "0.1.0"
edition = "2021"
license = "Proprietary"
publish = false

[workspace.dependencies]
anyhow = "1"
thiserror = "1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio = { version = "1", features = ["full"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }
uuid = { version = "1", features = ["v4", "serde"] }
chrono = { version = "0.4", features = ["serde"] }
sqlx = { version = "0.8", features = ["runtime-tokio", "tls-rustls", "postgres", "uuid", "chrono", "macros"] }
axum = "0.7"
tower = "0.5"
tower-http = { version = "0.5", features = ["trace", "cors"] }
reqwest = { version = "0.12", features = ["json", "rustls-tls"] }
jsonwebtoken = "9"
dioxus = "0.6"
```

- [ ] **Step 4: Create `.gitignore`**

```
/target
/dist
**/*.rs.bk
.env
.env.local
node_modules/
.DS_Store
.idea/
.vscode/
ios/build/
android/build/
*.xcuserstate
```

- [ ] **Step 5: Create `.dockerignore`**

```
target
dist
.git
.env
.env.local
node_modules
ios/build
android/build
docs
```

- [ ] **Step 6: Verify workspace parses**

Run: `cargo metadata --no-deps --format-version 1 > /dev/null`
Expected: command succeeds with no output (workspace is parseable; member crates not yet present is fine because `cargo metadata` will only error if a referenced manifest is missing — we'll create them next).

If it fails because the listed members don't exist, that's expected; the next tasks create them.

- [ ] **Step 7: Initial commit**

```bash
git add Cargo.toml rust-toolchain.toml .gitignore .dockerignore
git commit -m "chore: initialize Cargo workspace and toolchain pin"
```

---

### Task 2: Add `core-types` crate

**Files:**
- Create: `crates/core-types/Cargo.toml`
- Create: `crates/core-types/src/lib.rs`

- [ ] **Step 1: Create crate manifest**

```toml
# crates/core-types/Cargo.toml
[package]
name = "core-types"
version.workspace = true
edition.workspace = true
license.workspace = true
publish.workspace = true

[dependencies]
serde = { workspace = true }
uuid = { workspace = true }
chrono = { workspace = true }
thiserror = { workspace = true }
```

- [ ] **Step 2: Create initial `lib.rs` with role enum (used in later tasks)**

```rust
// crates/core-types/src/lib.rs
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TenantRole {
    OrgAdmin,
    Teacher,
    Ta,
    Student,
    Parent,
}

impl TenantRole {
    pub fn as_str(self) -> &'static str {
        match self {
            TenantRole::OrgAdmin => "org_admin",
            TenantRole::Teacher => "teacher",
            TenantRole::Ta => "ta",
            TenantRole::Student => "student",
            TenantRole::Parent => "parent",
        }
    }
}
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo build -p core-types`
Expected: `Finished ...` with no errors.

- [ ] **Step 4: Commit**

```bash
git add crates/core-types
git commit -m "feat(core-types): scaffold crate with TenantRole enum"
```

---

### Task 3: Add `api-client` crate (skeleton only)

**Files:**
- Create: `crates/api-client/Cargo.toml`
- Create: `crates/api-client/src/lib.rs`

- [ ] **Step 1: Create manifest**

```toml
# crates/api-client/Cargo.toml
[package]
name = "api-client"
version.workspace = true
edition.workspace = true
license.workspace = true
publish.workspace = true

[dependencies]
core-types = { path = "../core-types" }
serde = { workspace = true }
serde_json = { workspace = true }
thiserror = { workspace = true }
```

- [ ] **Step 2: Create stub `lib.rs`**

```rust
// crates/api-client/src/lib.rs
//! HTTP/WS client bindings for AulaLite. Real implementation lands in Phase 1.

#[derive(Debug, Clone)]
pub struct ApiBaseUrl(pub String);
```

- [ ] **Step 3: Build + commit**

```bash
cargo build -p api-client
git add crates/api-client
git commit -m "feat(api-client): scaffold crate"
```

---

### Task 4: Add `design-system` crate (skeleton only)

**Files:**
- Create: `crates/design-system/Cargo.toml`
- Create: `crates/design-system/src/lib.rs`

- [ ] **Step 1: Manifest**

```toml
# crates/design-system/Cargo.toml
[package]
name = "design-system"
version.workspace = true
edition.workspace = true
license.workspace = true
publish.workspace = true

[dependencies]
dioxus = { workspace = true }
```

- [ ] **Step 2: Stub `lib.rs`**

```rust
// crates/design-system/src/lib.rs
//! Design system primitives. Real components added in Section F.
```

- [ ] **Step 3: Build + commit**

```bash
cargo build -p design-system
git add crates/design-system
git commit -m "feat(design-system): scaffold crate"
```

---

### Task 5: Add `platform-bridge` crate (trait + stubs)

**Files:**
- Create: `crates/platform-bridge/Cargo.toml`
- Create: `crates/platform-bridge/src/lib.rs`

- [ ] **Step 1: Manifest**

```toml
# crates/platform-bridge/Cargo.toml
[package]
name = "platform-bridge"
version.workspace = true
edition.workspace = true
license.workspace = true
publish.workspace = true

[dependencies]
async-trait = "0.1"
thiserror = { workspace = true }
```

- [ ] **Step 2: Trait definition**

```rust
// crates/platform-bridge/src/lib.rs
use async_trait::async_trait;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum BridgeError {
    #[error("permission denied")]
    PermissionDenied,
    #[error("not implemented on this platform")]
    NotImplemented,
    #[error("io: {0}")]
    Io(String),
}

#[async_trait]
pub trait PlatformBridge: Send + Sync {
    async fn current_id_token(&self) -> Result<String, BridgeError>;
    async fn sign_in_email_password(&self, email: &str, password: &str) -> Result<String, BridgeError>;
    async fn sign_up_email_password(&self, email: &str, password: &str) -> Result<String, BridgeError>;
    async fn sign_out(&self) -> Result<(), BridgeError>;
    async fn send_password_reset(&self, email: &str) -> Result<(), BridgeError>;
}
```

- [ ] **Step 3: Build + commit**

```bash
cargo build -p platform-bridge
git add crates/platform-bridge
git commit -m "feat(platform-bridge): define PlatformBridge trait"
```

---

### Task 6: Add `features-auth` crate (skeleton only)

**Files:**
- Create: `crates/features-auth/Cargo.toml`
- Create: `crates/features-auth/src/lib.rs`

- [ ] **Step 1: Manifest**

```toml
# crates/features-auth/Cargo.toml
[package]
name = "features-auth"
version.workspace = true
edition.workspace = true
license.workspace = true
publish.workspace = true

[dependencies]
dioxus = { workspace = true }
core-types = { path = "../core-types" }
design-system = { path = "../design-system" }
platform-bridge = { path = "../platform-bridge" }
```

- [ ] **Step 2: Stub `lib.rs`**

```rust
// crates/features-auth/src/lib.rs
//! Auth screens. Real components in Section G.
```

- [ ] **Step 3: Build + commit**

```bash
cargo build -p features-auth
git add crates/features-auth
git commit -m "feat(features-auth): scaffold crate"
```

---

### Task 7: Add `shell-web` crate (Dioxus web hello world)

**Files:**
- Create: `crates/shell-web/Cargo.toml`
- Create: `crates/shell-web/Dioxus.toml`
- Create: `crates/shell-web/src/main.rs`
- Create: `crates/shell-web/assets/index.html` (minimal — Dioxus generates the rest)

- [ ] **Step 1: Manifest**

```toml
# crates/shell-web/Cargo.toml
[package]
name = "shell-web"
version.workspace = true
edition.workspace = true
license.workspace = true
publish.workspace = true

[dependencies]
dioxus = { workspace = true, features = ["web"] }
features-auth = { path = "../features-auth" }
design-system = { path = "../design-system" }
```

- [ ] **Step 2: `Dioxus.toml`**

```toml
[application]
name = "aulalite-web"
default_platform = "web"

[web.app]
title = "AulaLite"

[web.watcher]
reload_html = true

[web.resource]
style = []
script = []
```

- [ ] **Step 3: `src/main.rs`**

```rust
use dioxus::prelude::*;

fn main() {
    dioxus::launch(App);
}

#[component]
fn App() -> Element {
    rsx! {
        div { class: "app-shell",
            h1 { "AulaLite" }
            p { "Phase 0 — hello world" }
        }
    }
}
```

- [ ] **Step 4: Build (web target)**

Run: `cd crates/shell-web && dx build --platform web && cd ../..`
Expected: build succeeds and emits assets in `crates/shell-web/dist/`.

- [ ] **Step 5: Smoke-test by serving locally**

Run: `cd crates/shell-web && dx serve --platform web --port 3000`
Expected: open `http://localhost:3000` in a browser; you see "AulaLite — Phase 0 — hello world". Stop with Ctrl-C.

- [ ] **Step 6: Commit**

```bash
git add crates/shell-web
git commit -m "feat(shell-web): scaffold Dioxus web shell with hello world"
```

---

### Task 8: Add `shell-desktop` crate (Dioxus desktop hello world)

**Files:**
- Create: `crates/shell-desktop/Cargo.toml`
- Create: `crates/shell-desktop/src/main.rs`

- [ ] **Step 1: Manifest**

```toml
# crates/shell-desktop/Cargo.toml
[package]
name = "shell-desktop"
version.workspace = true
edition.workspace = true
license.workspace = true
publish.workspace = true

[dependencies]
dioxus = { workspace = true, features = ["desktop"] }
features-auth = { path = "../features-auth" }
design-system = { path = "../design-system" }
```

- [ ] **Step 2: `src/main.rs` (identical content shape to web)**

```rust
use dioxus::prelude::*;

fn main() {
    dioxus::launch(App);
}

#[component]
fn App() -> Element {
    rsx! {
        div { class: "app-shell",
            h1 { "AulaLite (Desktop)" }
            p { "Phase 0 — hello world" }
        }
    }
}
```

- [ ] **Step 3: Build**

Run: `cargo build -p shell-desktop`
Expected: build succeeds. (Some platforms need `webkit2gtk` on Linux; install via system package manager if it errors with linker complaints.)

- [ ] **Step 4: Smoke-run**

Run: `cargo run -p shell-desktop`
Expected: a native window opens showing "AulaLite (Desktop) — Phase 0 — hello world". Close it.

- [ ] **Step 5: Commit**

```bash
git add crates/shell-desktop
git commit -m "feat(shell-desktop): scaffold Dioxus desktop shell with hello world"
```

---

### Task 9: Add `shell-mobile` crate (Dioxus mobile scaffold)

**Files:**
- Create: `crates/shell-mobile/Cargo.toml`
- Create: `crates/shell-mobile/Dioxus.toml`
- Create: `crates/shell-mobile/src/main.rs`

> Note: Running on a real device or simulator is verified in Section H, after the auth screens exist. This task creates the crate and confirms it *compiles* against the mobile target.

- [ ] **Step 1: Manifest**

```toml
# crates/shell-mobile/Cargo.toml
[package]
name = "shell-mobile"
version.workspace = true
edition.workspace = true
license.workspace = true
publish.workspace = true

[lib]
crate-type = ["staticlib", "cdylib", "rlib"]

[dependencies]
dioxus = { workspace = true, features = ["mobile"] }
features-auth = { path = "../features-auth" }
design-system = { path = "../design-system" }
```

- [ ] **Step 2: `Dioxus.toml` (mobile bundle metadata)**

```toml
[application]
name = "aulalite-mobile"
default_platform = "mobile"

[bundle]
identifier = "guru.elementors.aulalite"
publisher = "Elementors"
icon = []
resources = []
```

- [ ] **Step 3: `src/main.rs`**

```rust
use dioxus::prelude::*;

fn main() {
    dioxus::launch(App);
}

#[component]
fn App() -> Element {
    rsx! {
        div { class: "app-shell",
            h1 { "AulaLite (Mobile)" }
            p { "Phase 0 — hello world" }
        }
    }
}
```

- [ ] **Step 4: Compile-check (no run yet)**

Run: `cargo check -p shell-mobile`
Expected: compiles. (Running on simulator/device is deferred to Section H.)

- [ ] **Step 5: Commit**

```bash
git add crates/shell-mobile
git commit -m "feat(shell-mobile): scaffold Dioxus mobile shell"
```

---

### Task 10: Add `backend` crate (Axum hello + /healthz)

**Files:**
- Create: `crates/backend/Cargo.toml`
- Create: `crates/backend/src/main.rs`
- Create: `crates/backend/src/handlers/mod.rs`
- Create: `crates/backend/src/handlers/health.rs`
- Create: `crates/backend/tests/health.rs`

- [ ] **Step 1: Manifest**

```toml
# crates/backend/Cargo.toml
[package]
name = "backend"
version.workspace = true
edition.workspace = true
license.workspace = true
publish.workspace = true

[dependencies]
core-types = { path = "../core-types" }
anyhow = { workspace = true }
thiserror = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
tokio = { workspace = true }
tracing = { workspace = true }
tracing-subscriber = { workspace = true }
axum = { workspace = true }
tower = { workspace = true }
tower-http = { workspace = true }
sqlx = { workspace = true }
reqwest = { workspace = true }
jsonwebtoken = { workspace = true }
uuid = { workspace = true }
chrono = { workspace = true }

[dev-dependencies]
tokio = { workspace = true, features = ["test-util", "macros"] }
http-body-util = "0.1"
tower = { workspace = true }
```

- [ ] **Step 2: Write the failing test for `/healthz`**

```rust
// crates/backend/tests/health.rs
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

#[tokio::test]
async fn healthz_returns_ok() {
    let app = backend::router_for_tests();
    let response = app
        .oneshot(Request::builder().uri("/healthz").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&body[..], b"ok");
}
```

- [ ] **Step 3: Run test — expect failure**

Run: `cargo test -p backend --test health`
Expected: FAIL — `backend` does not yet expose a library or `router_for_tests` symbol.

- [ ] **Step 4: Make backend a `lib.rs` + `main.rs` crate so tests can call into it**

Update `crates/backend/Cargo.toml` to add a `[lib]` section:

```toml
[lib]
name = "backend"
path = "src/lib.rs"

[[bin]]
name = "backend"
path = "src/main.rs"
```

- [ ] **Step 5: Create `src/lib.rs`**

```rust
// crates/backend/src/lib.rs
pub mod handlers;

use axum::{routing::get, Router};

pub fn router() -> Router {
    Router::new().route("/healthz", get(handlers::health::healthz))
}

pub fn router_for_tests() -> Router {
    router()
}
```

- [ ] **Step 6: Create `src/handlers/mod.rs` and `src/handlers/health.rs`**

```rust
// crates/backend/src/handlers/mod.rs
pub mod health;
```

```rust
// crates/backend/src/handlers/health.rs
pub async fn healthz() -> &'static str {
    "ok"
}
```

- [ ] **Step 7: Create `src/main.rs`**

```rust
// crates/backend/src/main.rs
use std::net::SocketAddr;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .json()
        .init();

    let app = backend::router();
    let addr: SocketAddr = std::env::var("BIND_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:8080".to_string())
        .parse()?;

    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, "backend listening");
    axum::serve(listener, app).await?;
    Ok(())
}
```

- [ ] **Step 8: Run test — expect pass**

Run: `cargo test -p backend --test health`
Expected: PASS.

- [ ] **Step 9: Smoke-run the binary**

Run: `cargo run -p backend` and in another terminal `curl http://localhost:8080/healthz`
Expected: `ok`. Stop the server with Ctrl-C.

- [ ] **Step 10: Commit**

```bash
git add crates/backend
git commit -m "feat(backend): scaffold Axum binary with /healthz endpoint"
```

---

# Section B — Docker + Dokploy Stack

### Task 11: Backend Dockerfile (multi-stage Rust build)

**Files:**
- Create: `crates/backend/Dockerfile`

- [ ] **Step 1: Write the Dockerfile**

```dockerfile
# crates/backend/Dockerfile
# syntax=docker/dockerfile:1.7

FROM rust:1.85-slim AS builder
WORKDIR /app

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Cache deps: copy manifests first
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
COPY tools/ tools/
COPY migrations/ migrations/

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo build --release -p backend && \
    cp /app/target/release/backend /usr/local/bin/backend

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /usr/local/bin/backend /usr/local/bin/backend
EXPOSE 8080
ENV RUST_LOG=info
CMD ["/usr/local/bin/backend"]
```

- [ ] **Step 2: Build the image**

Run: `docker build -f crates/backend/Dockerfile -t aulalite-backend:dev .`
Expected: image built successfully.

- [ ] **Step 3: Smoke-run the container**

Run: `docker run --rm -p 8080:8080 aulalite-backend:dev` and in another terminal `curl http://localhost:8080/healthz`
Expected: `ok`. Stop the container with Ctrl-C.

- [ ] **Step 4: Commit**

```bash
git add crates/backend/Dockerfile
git commit -m "feat(backend): add multi-stage Dockerfile"
```

---

### Task 12: docker-compose.yml with Postgres, Redis, MinIO

**Files:**
- Create: `docker-compose.yml`
- Create: `.env.example`

- [ ] **Step 1: Write `.env.example`**

```bash
# .env.example — copy to .env and fill in real values for local dev
POSTGRES_USER=aulalite
POSTGRES_PASSWORD=changeme
POSTGRES_DB=aulalite

DATABASE_URL=postgres://aulalite:changeme@postgres:5432/aulalite
DATABASE_URL_LOCAL=postgres://aulalite:changeme@localhost:55432/aulalite

REDIS_URL=redis://redis:6379

MINIO_ROOT_USER=aulalite
MINIO_ROOT_PASSWORD=changeme123
MINIO_ENDPOINT=http://minio:9000

FIREBASE_PROJECT_ID=aulalite-dev
FIREBASE_JWKS_URL=https://www.googleapis.com/service_accounts/v1/jwk/securetoken@system.gserviceaccount.com
FIREBASE_TOKEN_ISSUER=https://securetoken.google.com/aulalite-dev

BIND_ADDR=0.0.0.0:8080
RUST_LOG=info,backend=debug,sqlx=warn
```

- [ ] **Step 2: Write `docker-compose.yml`**

```yaml
# docker-compose.yml
name: aulalite
services:
  postgres:
    image: postgres:16-alpine
    restart: unless-stopped
    environment:
      POSTGRES_USER: ${POSTGRES_USER}
      POSTGRES_PASSWORD: ${POSTGRES_PASSWORD}
      POSTGRES_DB: ${POSTGRES_DB}
    ports:
      - "55432:5432"
    volumes:
      - pg_data:/var/lib/postgresql/data
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U $$POSTGRES_USER -d $$POSTGRES_DB"]
      interval: 5s
      timeout: 5s
      retries: 10

  redis:
    image: redis:7-alpine
    restart: unless-stopped
    command: ["redis-server", "--appendonly", "yes"]
    ports:
      - "6379:6379"
    volumes:
      - redis_data:/data
    healthcheck:
      test: ["CMD", "redis-cli", "ping"]
      interval: 5s
      timeout: 3s
      retries: 10

  minio:
    image: minio/minio:RELEASE.2025-01-20T14-49-07Z
    restart: unless-stopped
    command: ["server", "/data", "--console-address", ":9001"]
    environment:
      MINIO_ROOT_USER: ${MINIO_ROOT_USER}
      MINIO_ROOT_PASSWORD: ${MINIO_ROOT_PASSWORD}
    ports:
      - "9000:9000"
      - "9001:9001"
    volumes:
      - minio_data:/data
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:9000/minio/health/live"]
      interval: 10s
      timeout: 5s
      retries: 10

  backend:
    build:
      context: .
      dockerfile: crates/backend/Dockerfile
    restart: unless-stopped
    depends_on:
      postgres:
        condition: service_healthy
      redis:
        condition: service_healthy
      minio:
        condition: service_healthy
    environment:
      DATABASE_URL: ${DATABASE_URL}
      REDIS_URL: ${REDIS_URL}
      MINIO_ENDPOINT: ${MINIO_ENDPOINT}
      MINIO_ROOT_USER: ${MINIO_ROOT_USER}
      MINIO_ROOT_PASSWORD: ${MINIO_ROOT_PASSWORD}
      FIREBASE_PROJECT_ID: ${FIREBASE_PROJECT_ID}
      FIREBASE_JWKS_URL: ${FIREBASE_JWKS_URL}
      FIREBASE_TOKEN_ISSUER: ${FIREBASE_TOKEN_ISSUER}
      BIND_ADDR: ${BIND_ADDR}
      RUST_LOG: ${RUST_LOG}
    ports:
      - "8080:8080"

volumes:
  pg_data:
  redis_data:
  minio_data:
```

- [ ] **Step 3: Bring the stack up**

```bash
cp .env.example .env
docker compose up -d postgres redis minio
docker compose ps
```

Expected: `postgres`, `redis`, `minio` all show `healthy`.

- [ ] **Step 4: Bring up backend last**

```bash
docker compose up -d backend
sleep 3
curl http://localhost:8080/healthz
```

Expected: `ok`.

- [ ] **Step 5: Tear down**

```bash
docker compose down
```

- [ ] **Step 6: Commit**

```bash
git add docker-compose.yml .env.example
git commit -m "feat(ops): docker-compose stack with postgres, redis, minio, backend"
```

---

### Task 13: MediaMTX skeleton config + add to compose

**Files:**
- Create: `ops/mediamtx/mediamtx.yml`
- Modify: `docker-compose.yml`

- [ ] **Step 1: Write skeleton `mediamtx.yml`**

```yaml
# ops/mediamtx/mediamtx.yml — Phase 0 minimal config; auth + recording wired in Phase 1.
logLevel: info

api: yes
apiAddress: :9997

metrics: yes
metricsAddress: :9998

webrtc: yes
webrtcAddress: :8889
webrtcEncryption: no       # local dev only; prod sits behind Traefik with TLS
webrtcAdditionalHosts: []

hls: yes
hlsAddress: :8888
hlsEncryption: no
hlsVariant: lowLatency

paths:
  all_others:
```

- [ ] **Step 2: Append `mediamtx` service to `docker-compose.yml`**

Add to the `services:` block:

```yaml
  mediamtx:
    image: bluenviron/mediamtx:latest
    restart: unless-stopped
    network_mode: host          # required for WebRTC UDP/TCP exposure
    volumes:
      - ./ops/mediamtx/mediamtx.yml:/mediamtx.yml:ro
      - mediamtx_recordings:/recordings
```

And add to the `volumes:` block:

```yaml
  mediamtx_recordings:
```

> Note: `network_mode: host` is incompatible with the `ports:` form on Linux. On macOS/Windows Docker Desktop, host networking has limited support; for local dev on those platforms, replace `network_mode: host` with explicit `ports: ["8889:8889/udp", "8889:8889/tcp", "8888:8888", "9997:9997", "9998:9998"]`. Production Dokploy is Linux, so host networking applies there.

- [ ] **Step 3: Verify MediaMTX comes up**

```bash
docker compose up -d mediamtx
sleep 2
curl http://localhost:9997/v3/paths/list
```

Expected: JSON with empty `items` array (no paths active yet).

- [ ] **Step 4: Tear down**

```bash
docker compose down
```

- [ ] **Step 5: Commit**

```bash
git add ops/mediamtx/mediamtx.yml docker-compose.yml
git commit -m "feat(ops): add MediaMTX skeleton config and compose service"
```

---

### Task 14: ops README — Dokploy deployment notes

**Files:**
- Create: `ops/README.md`

- [ ] **Step 1: Write the runbook stub**

```markdown
# AulaLite Ops Runbook

## Local dev

```bash
cp .env.example .env
docker compose up -d
curl http://localhost:8080/healthz
```

## Dokploy deployment (staging + prod)

1. In Dokploy, create a project named `aulalite-staging` (or `aulalite-prod`).
2. Add this repo via Git source.
3. Set environment variables in Dokploy UI from `.env.example`. Replace placeholder values with real per-environment secrets — never commit `.env`.
4. Configure domains:
   - `staging.elementors.guru` → `backend` service port 8080
   - `media-staging.elementors.guru` → `mediamtx` service port 8888 (HLS + 9997 API only)
5. WebRTC UDP/TCP listeners (8889) are exposed via host networking on the Dokploy node — not via Traefik. Open the relevant ports in the node's firewall.
6. Health check: `/healthz` on `backend`. Configure Dokploy to auto-rollback on health-check failure.
7. Deploy: tag a release in Git; Dokploy auto-deploys on tagged image push.

## Backups

Dokploy volume backup is configured to run nightly against the `pg_data`, `minio_data`, and `mediamtx_recordings` volumes. Target: a Dokploy-supported destination per the spec's Open Questions section. Update this file once chosen.
```

- [ ] **Step 2: Commit**

```bash
git add ops/README.md
git commit -m "docs(ops): add runbook stub with Dokploy deployment notes"
```

---

### Task 15: Verify full stack starts cleanly

This is a sanity checkpoint. No code changes; just verify everything we have so far still works.

- [ ] **Step 1: Bring everything up**

```bash
docker compose up -d
sleep 5
docker compose ps
```

Expected: every service shows `running` (and `healthy` where a healthcheck is defined).

- [ ] **Step 2: Hit the endpoints**

```bash
curl http://localhost:8080/healthz
# → ok
curl http://localhost:9997/v3/paths/list
# → JSON with empty items
curl http://localhost:9000/minio/health/live
# → 200 OK (no body)
```

- [ ] **Step 3: Tear down**

```bash
docker compose down
```

- [ ] **Step 4: Commit a checkpoint marker**

(No file changes to commit; create a checkpoint tag instead.)

```bash
git tag phase-0-checkpoint-1-stack-up
```

---

# Section C — Database Migrations + RLS

### Task 16: Set up sqlx migrations directory

**Files:**
- Create: `migrations/.gitkeep`
- Modify: `crates/backend/src/lib.rs` (add `db` module placeholder)

- [ ] **Step 1: Create migrations dir**

```bash
mkdir -p migrations
touch migrations/.gitkeep
```

- [ ] **Step 2: Verify `sqlx migrate info` runs against the dev database**

```bash
docker compose up -d postgres
export DATABASE_URL=postgres://aulalite:changeme@localhost:55432/aulalite
sqlx migrate info --source migrations
```

Expected: empty output (no migrations yet, but command succeeds).

- [ ] **Step 3: Add `db` module placeholder to backend**

```rust
// crates/backend/src/db.rs
use sqlx::postgres::{PgPool, PgPoolOptions};

pub async fn pool_from_env() -> anyhow::Result<PgPool> {
    let url = std::env::var("DATABASE_URL")?;
    let pool = PgPoolOptions::new()
        .max_connections(20)
        .connect(&url)
        .await?;
    Ok(pool)
}

pub async fn run_migrations(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::migrate!("../../migrations").run(pool).await?;
    Ok(())
}
```

Update `crates/backend/src/lib.rs`:

```rust
pub mod db;
pub mod handlers;

use axum::{routing::get, Router};

pub fn router() -> Router {
    Router::new().route("/healthz", get(handlers::health::healthz))
}

pub fn router_for_tests() -> Router {
    router()
}
```

- [ ] **Step 4: Build to confirm**

```bash
cargo build -p backend
```

Expected: builds (the `migrate!` macro is happy with an empty dir).

- [ ] **Step 5: Commit**

```bash
git add migrations crates/backend/src/db.rs crates/backend/src/lib.rs
git commit -m "feat(db): wire sqlx pool factory and migrate! macro"
```

---

### Task 17: Migration 0001 — extensions

**Files:**
- Create: `migrations/20260503000001_extensions.sql`

- [ ] **Step 1: Write the migration**

```sql
-- migrations/20260503000001_extensions.sql
CREATE EXTENSION IF NOT EXISTS "uuid-ossp";
CREATE EXTENSION IF NOT EXISTS citext;
CREATE EXTENSION IF NOT EXISTS pg_trgm;
```

- [ ] **Step 2: Apply it**

```bash
export DATABASE_URL=postgres://aulalite:changeme@localhost:55432/aulalite
sqlx migrate run --source migrations
```

Expected: `Applied 20260503000001/migrate extensions ...`

- [ ] **Step 3: Verify in Postgres**

```bash
psql "$DATABASE_URL" -c "SELECT extname FROM pg_extension WHERE extname IN ('uuid-ossp','citext','pg_trgm');"
```

Expected: three rows printed.

- [ ] **Step 4: Commit**

```bash
git add migrations/20260503000001_extensions.sql
git commit -m "feat(db): migration 0001 — enable uuid-ossp, citext, pg_trgm"
```

---

### Task 18: Migration 0002 — `tenants` table + RLS policy

**Files:**
- Create: `migrations/20260503000002_tenants.sql`

- [ ] **Step 1: Write the migration**

```sql
-- migrations/20260503000002_tenants.sql
CREATE TABLE tenants (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    slug TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'trialing'
        CHECK (status IN ('active','trialing','suspended')),
    plan_id TEXT,
    stripe_customer_id TEXT,
    branding JSONB,
    recording_default BOOLEAN NOT NULL DEFAULT TRUE,
    recording_retention_days INTEGER NOT NULL DEFAULT 90
        CHECK (recording_retention_days BETWEEN 1 AND 3650),
    trial_ends_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX tenants_status_idx ON tenants(status);

ALTER TABLE tenants ENABLE ROW LEVEL SECURITY;

CREATE POLICY tenants_self_access ON tenants
    USING (id = current_setting('app.tenant_id', true)::uuid);
```

- [ ] **Step 2: Apply**

```bash
sqlx migrate run --source migrations
```

Expected: `Applied 20260503000002/migrate tenants ...`

- [ ] **Step 3: Verify**

```bash
psql "$DATABASE_URL" -c "\d tenants"
psql "$DATABASE_URL" -c "SELECT polname FROM pg_policy WHERE polrelid = 'tenants'::regclass;"
```

Expected: table description shows; one policy row `tenants_self_access`.

- [ ] **Step 4: Commit**

```bash
git add migrations/20260503000002_tenants.sql
git commit -m "feat(db): migration 0002 — tenants table with RLS"
```

---

### Task 19: Migration 0003 — `users` table

**Files:**
- Create: `migrations/20260503000003_users.sql`

> `users` is identity, not tenant data — a single user can be a member of multiple tenants. Therefore RLS does not apply at the `users` level; tenancy is enforced via `tenant_memberships`.

- [ ] **Step 1: Write the migration**

```sql
-- migrations/20260503000003_users.sql
CREATE TABLE users (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    firebase_uid TEXT NOT NULL UNIQUE,
    email CITEXT NOT NULL UNIQUE,
    display_name TEXT,
    avatar_url TEXT,
    locale TEXT,
    is_platform_admin BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen_at TIMESTAMPTZ
);

CREATE INDEX users_firebase_uid_idx ON users(firebase_uid);
CREATE INDEX users_email_idx ON users(email);
```

- [ ] **Step 2: Apply**

```bash
sqlx migrate run --source migrations
```

Expected: applied successfully.

- [ ] **Step 3: Commit**

```bash
git add migrations/20260503000003_users.sql
git commit -m "feat(db): migration 0003 — users table"
```

---

### Task 20: Migration 0004 — `tenant_memberships` table + RLS policy

**Files:**
- Create: `migrations/20260503000004_tenant_memberships.sql`

- [ ] **Step 1: Write the migration**

```sql
-- migrations/20260503000004_tenant_memberships.sql
CREATE TABLE tenant_memberships (
    tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role TEXT NOT NULL
        CHECK (role IN ('org_admin','teacher','ta','student','parent')),
    status TEXT NOT NULL DEFAULT 'active'
        CHECK (status IN ('active','invited','suspended')),
    invited_by UUID REFERENCES users(id),
    joined_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, user_id)
);

CREATE INDEX tenant_memberships_user_idx ON tenant_memberships(user_id);
CREATE INDEX tenant_memberships_tenant_idx ON tenant_memberships(tenant_id);

ALTER TABLE tenant_memberships ENABLE ROW LEVEL SECURITY;

-- Allow access in two cases:
--   (a) the request is operating in tenant scope (app.tenant_id matches the row), OR
--   (b) the request is bootstrapping (app.user_id matches and we haven't resolved tenant yet).
-- The bootstrap clause lets the auth middleware look up the user's memberships
-- *before* it knows which tenant to scope to.
CREATE POLICY tenant_memberships_self_access ON tenant_memberships
    USING (
        tenant_id::text = current_setting('app.tenant_id', true)
        OR user_id::text = current_setting('app.user_id', true)
    );
```

- [ ] **Step 2: Apply**

```bash
sqlx migrate run --source migrations
```

Expected: applied.

- [ ] **Step 3: Verify all four migrations are recorded**

```bash
psql "$DATABASE_URL" -c "SELECT version, description FROM _sqlx_migrations ORDER BY version;"
```

Expected: 4 rows, in order: extensions, tenants, users, tenant_memberships.

- [ ] **Step 4: Commit**

```bash
git add migrations/20260503000004_tenant_memberships.sql
git commit -m "feat(db): migration 0004 — tenant_memberships table with RLS"
```

---

### Task 21: Integration test — RLS cross-tenant denial

**Files:**
- Create: `crates/backend/tests/rls_tenant_isolation.rs`
- Modify: `crates/backend/Cargo.toml` (add `[dev-dependencies]` for test deps if not already)

- [ ] **Step 1: Write the failing test**

```rust
// crates/backend/tests/rls_tenant_isolation.rs
//! Verifies Postgres RLS prevents cross-tenant reads when app.tenant_id is set.

use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

async fn pool() -> sqlx::PgPool {
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://aulalite:changeme@localhost:55432/aulalite".into());
    PgPoolOptions::new()
        .max_connections(4)
        .connect(&url)
        .await
        .expect("connect to test DB")
}

#[tokio::test]
async fn rls_blocks_cross_tenant_reads_on_tenant_memberships() {
    let pool = pool().await;

    // Insert two tenants and one user, plus a membership in tenant A.
    // Use the `bypassrls`-style fact that a default Postgres role is the table owner here,
    // which historically bypasses RLS — to make this test deterministic we explicitly disable it.
    let tenant_a = Uuid::new_v4();
    let tenant_b = Uuid::new_v4();
    let user_id = Uuid::new_v4();
    let firebase_uid = format!("fbuid_{}", Uuid::new_v4());
    let email = format!("user_{}@example.test", Uuid::new_v4());

    sqlx::query("INSERT INTO tenants (id, slug, name, status) VALUES ($1, $2, $3, 'active')")
        .bind(tenant_a)
        .bind(format!("ten_a_{}", tenant_a))
        .bind("Tenant A")
        .execute(&pool)
        .await
        .unwrap();

    sqlx::query("INSERT INTO tenants (id, slug, name, status) VALUES ($1, $2, $3, 'active')")
        .bind(tenant_b)
        .bind(format!("ten_b_{}", tenant_b))
        .bind("Tenant B")
        .execute(&pool)
        .await
        .unwrap();

    sqlx::query("INSERT INTO users (id, firebase_uid, email) VALUES ($1, $2, $3)")
        .bind(user_id)
        .bind(&firebase_uid)
        .bind(&email)
        .execute(&pool)
        .await
        .unwrap();

    sqlx::query(
        "INSERT INTO tenant_memberships (tenant_id, user_id, role) VALUES ($1, $2, 'org_admin')",
    )
    .bind(tenant_a)
    .bind(user_id)
    .execute(&pool)
    .await
    .unwrap();

    // Force RLS even for table owner role; the `aulalite` role created by Postgres image
    // owns tables it created, so we set FORCE ROW LEVEL SECURITY on these tables.
    sqlx::query("ALTER TABLE tenants FORCE ROW LEVEL SECURITY")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("ALTER TABLE tenant_memberships FORCE ROW LEVEL SECURITY")
        .execute(&pool)
        .await
        .unwrap();

    // Acquire a single connection so SET LOCAL persists for the txn.
    let mut conn = pool.acquire().await.unwrap();
    sqlx::query("BEGIN").execute(&mut *conn).await.unwrap();
    sqlx::query("SET LOCAL app.tenant_id = $1")
        .bind(tenant_b.to_string()) // we are tenant B
        .execute(&mut *conn)
        .await
        .unwrap();

    let visible: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM tenant_memberships WHERE tenant_id = $1",
    )
    .bind(tenant_a)
    .fetch_one(&mut *conn)
    .await
    .unwrap();

    sqlx::query("COMMIT").execute(&mut *conn).await.unwrap();

    assert_eq!(
        visible.0, 0,
        "tenant B must not see tenant A's memberships"
    );
}

#[tokio::test]
async fn rls_allows_same_tenant_reads_on_tenant_memberships() {
    let pool = pool().await;

    let tenant = Uuid::new_v4();
    let user_id = Uuid::new_v4();
    let firebase_uid = format!("fbuid_{}", Uuid::new_v4());
    let email = format!("user_{}@example.test", Uuid::new_v4());

    sqlx::query("INSERT INTO tenants (id, slug, name, status) VALUES ($1, $2, $3, 'active')")
        .bind(tenant)
        .bind(format!("ten_{}", tenant))
        .bind("Tenant")
        .execute(&pool)
        .await
        .unwrap();

    sqlx::query("INSERT INTO users (id, firebase_uid, email) VALUES ($1, $2, $3)")
        .bind(user_id)
        .bind(&firebase_uid)
        .bind(&email)
        .execute(&pool)
        .await
        .unwrap();

    sqlx::query(
        "INSERT INTO tenant_memberships (tenant_id, user_id, role) VALUES ($1, $2, 'org_admin')",
    )
    .bind(tenant)
    .bind(user_id)
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query("ALTER TABLE tenant_memberships FORCE ROW LEVEL SECURITY")
        .execute(&pool)
        .await
        .unwrap();

    let mut conn = pool.acquire().await.unwrap();
    sqlx::query("BEGIN").execute(&mut *conn).await.unwrap();
    sqlx::query("SET LOCAL app.tenant_id = $1")
        .bind(tenant.to_string())
        .execute(&mut *conn)
        .await
        .unwrap();

    let visible: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM tenant_memberships WHERE tenant_id = $1",
    )
    .bind(tenant)
    .fetch_one(&mut *conn)
    .await
    .unwrap();

    sqlx::query("COMMIT").execute(&mut *conn).await.unwrap();

    assert!(visible.0 >= 1, "same tenant must see its own memberships");
}
```

- [ ] **Step 2: Run — expect pass (no implementation needed; the test is verifying the migration)**

```bash
docker compose up -d postgres
export DATABASE_URL=postgres://aulalite:changeme@localhost:55432/aulalite
sqlx migrate run --source migrations
cargo test -p backend --test rls_tenant_isolation
```

Expected: both tests PASS.

- [ ] **Step 3: Persist the FORCE RLS as a migration step (so it isn't only set during tests)**

Create `migrations/20260503000005_force_rls.sql`:

```sql
-- migrations/20260503000005_force_rls.sql
ALTER TABLE tenants FORCE ROW LEVEL SECURITY;
ALTER TABLE tenant_memberships FORCE ROW LEVEL SECURITY;
```

Apply:

```bash
sqlx migrate run --source migrations
```

- [ ] **Step 4: Re-run tests to confirm they still pass without per-test FORCE statements**

Edit the test file to remove the `ALTER TABLE ... FORCE ROW LEVEL SECURITY` lines (since they're now in migrations). Re-run:

```bash
cargo test -p backend --test rls_tenant_isolation
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/backend/tests/rls_tenant_isolation.rs migrations/20260503000005_force_rls.sql
git commit -m "test(db): RLS cross-tenant isolation integration tests"
```

---

# Section D — Firebase Auth Server-Side Verification

### Task 22: JWKS fetcher with TTL cache

**Files:**
- Create: `crates/backend/src/auth/mod.rs`
- Create: `crates/backend/src/auth/jwks.rs`
- Modify: `crates/backend/src/lib.rs` (add `pub mod auth`)

- [ ] **Step 1: Write the failing test**

```rust
// crates/backend/src/auth/jwks.rs
// (test will live at the bottom of this file as a #[cfg(test)] module)
```

Add the test at the bottom of `jwks.rs` after step 2 below; we'll write the file together.

- [ ] **Step 2: Write the JWKS module with implementation + test**

```rust
// crates/backend/src/auth/jwks.rs
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use jsonwebtoken::DecodingKey;
use tokio::sync::RwLock;

#[derive(Debug, thiserror::Error)]
pub enum JwksError {
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    #[error("invalid key data: {0}")]
    KeyData(String),
    #[error("kid not found")]
    KidNotFound,
}

#[derive(Clone)]
pub struct JwksCache {
    inner: Arc<RwLock<Inner>>,
    url: String,
    ttl: Duration,
    http: reqwest::Client,
}

struct Inner {
    keys: HashMap<String, DecodingKey>,
    fetched_at: Option<Instant>,
}

impl JwksCache {
    pub fn new(url: impl Into<String>, ttl: Duration) -> Self {
        Self {
            inner: Arc::new(RwLock::new(Inner {
                keys: HashMap::new(),
                fetched_at: None,
            })),
            url: url.into(),
            ttl,
            http: reqwest::Client::new(),
        }
    }

    pub async fn key_for_kid(&self, kid: &str) -> Result<DecodingKey, JwksError> {
        if let Some(key) = self.cached(kid).await {
            return Ok(key);
        }
        self.refresh().await?;
        self.cached(kid).await.ok_or(JwksError::KidNotFound)
    }

    async fn cached(&self, kid: &str) -> Option<DecodingKey> {
        let r = self.inner.read().await;
        if r.fetched_at.map_or(true, |t| t.elapsed() > self.ttl) {
            return None;
        }
        r.keys.get(kid).cloned()
    }

    async fn refresh(&self) -> Result<(), JwksError> {
        // Firebase secure-token JWKS endpoint returns a flat JSON map { kid: x509-pem }
        // in the form: { "kid1": "-----BEGIN CERTIFICATE-----\n...", "kid2": "..." }.
        // We translate each PEM cert to an RSA DecodingKey via from_rsa_pem after
        // extracting the public key, but jsonwebtoken supports `from_rsa_pem` on the cert directly
        // via its `from_rsa_pem` (which accepts SubjectPublicKeyInfo PEM). For Firebase x509 certs,
        // we use `from_rsa_components` after parsing the cert OR use `from_rsa_pem` if the
        // cert is convertible. Simpler: use the alternative endpoint:
        //   https://www.googleapis.com/service_accounts/v1/jwk/securetoken@system.gserviceaccount.com
        // which returns standard JWK set. We assume this endpoint here.
        let resp: JwkSet = self.http.get(&self.url).send().await?.error_for_status()?.json().await?;
        let mut keys = HashMap::new();
        for jwk in resp.keys {
            if jwk.kty != "RSA" {
                continue;
            }
            let n = jwk.n.ok_or_else(|| JwksError::KeyData("missing n".into()))?;
            let e = jwk.e.ok_or_else(|| JwksError::KeyData("missing e".into()))?;
            let key = DecodingKey::from_rsa_components(&n, &e)
                .map_err(|err| JwksError::KeyData(err.to_string()))?;
            keys.insert(jwk.kid, key);
        }
        let mut w = self.inner.write().await;
        w.keys = keys;
        w.fetched_at = Some(Instant::now());
        Ok(())
    }
}

#[derive(serde::Deserialize)]
struct JwkSet {
    keys: Vec<Jwk>,
}

#[derive(serde::Deserialize)]
struct Jwk {
    kid: String,
    kty: String,
    n: Option<String>,
    e: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn empty_cache_returns_none_until_refresh() {
        // Use a non-existent URL to force refresh failure; cache should still
        // distinguish "never fetched" from "fetched but kid missing".
        let cache = JwksCache::new("http://127.0.0.1:1/jwks", Duration::from_secs(60));
        let r = cache.key_for_kid("nope").await;
        assert!(r.is_err(), "expected error from unreachable JWKS endpoint");
    }
}
```

- [ ] **Step 3: Create `mod.rs`**

```rust
// crates/backend/src/auth/mod.rs
pub mod jwks;
```

- [ ] **Step 4: Wire into `lib.rs`**

Add `pub mod auth;` to `crates/backend/src/lib.rs`.

- [ ] **Step 5: Run the test**

```bash
cargo test -p backend --lib auth::jwks
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/backend/src/auth crates/backend/src/lib.rs
git commit -m "feat(auth): JWKS fetcher with TTL cache and unit test"
```

---

### Task 23: ID token verifier

**Files:**
- Create: `crates/backend/src/auth/verify.rs`
- Modify: `crates/backend/src/auth/mod.rs`

- [ ] **Step 1: Write the failing test (against a self-issued token to avoid Firebase round-trip)**

```rust
// crates/backend/src/auth/verify.rs
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use serde::Deserialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum VerifyError {
    #[error("invalid token: {0}")]
    Invalid(String),
    #[error("jwks: {0}")]
    Jwks(#[from] super::jwks::JwksError),
    #[error("missing kid header")]
    MissingKid,
}

#[derive(Debug, Deserialize, Clone)]
pub struct FirebaseClaims {
    pub sub: String,                   // Firebase UID
    pub email: Option<String>,
    pub email_verified: Option<bool>,
    pub name: Option<String>,
    pub picture: Option<String>,
    pub aud: String,                   // Firebase project ID
    pub iss: String,                   // https://securetoken.google.com/<project>
    pub exp: i64,
    pub iat: i64,
    pub auth_time: Option<i64>,
}

pub struct Verifier {
    jwks: super::jwks::JwksCache,
    project_id: String,
    issuer: String,
}

impl Verifier {
    pub fn new(jwks: super::jwks::JwksCache, project_id: impl Into<String>, issuer: impl Into<String>) -> Self {
        Self { jwks, project_id: project_id.into(), issuer: issuer.into() }
    }

    pub async fn verify(&self, token: &str) -> Result<FirebaseClaims, VerifyError> {
        let header = jsonwebtoken::decode_header(token).map_err(|e| VerifyError::Invalid(e.to_string()))?;
        let kid = header.kid.ok_or(VerifyError::MissingKid)?;
        let key = self.jwks.key_for_kid(&kid).await?;
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_audience(&[&self.project_id]);
        validation.set_issuer(&[&self.issuer]);
        let data = decode::<FirebaseClaims>(token, &key, &validation)
            .map_err(|e| VerifyError::Invalid(e.to_string()))?;
        Ok(data.claims)
    }

    /// Test-only verifier that takes a raw `DecodingKey` and skips JWKS — used in unit tests.
    #[cfg(test)]
    pub fn verify_with_key(token: &str, key: &DecodingKey, project_id: &str, issuer: &str) -> Result<FirebaseClaims, VerifyError> {
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_audience(&[project_id]);
        validation.set_issuer(&[issuer]);
        let data = decode::<FirebaseClaims>(token, key, &validation)
            .map_err(|e| VerifyError::Invalid(e.to_string()))?;
        Ok(data.claims)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{encode, EncodingKey, Header};

    fn pem_keypair() -> (EncodingKey, DecodingKey) {
        // Use a fixed test RSA keypair (PKCS#8 PEM). Generate once with:
        //   openssl genpkey -algorithm RSA -out priv.pem -pkeyopt rsa_keygen_bits:2048
        //   openssl rsa -in priv.pem -pubout -out pub.pem
        // and paste below. To avoid embedding huge keys here, we generate at test time
        // using the `rsa` crate would add a dependency; instead use a small fixed pair
        // committed under `crates/backend/tests/fixtures/`. Phase 0 keeps test deps light:
        // we read the keys from disk if present, otherwise skip the test.
        let priv_pem = std::fs::read("tests/fixtures/test_priv.pem").expect(
            "tests/fixtures/test_priv.pem must exist; generate with `openssl genpkey -algorithm RSA -out crates/backend/tests/fixtures/test_priv.pem -pkeyopt rsa_keygen_bits:2048` and `openssl rsa -in priv.pem -pubout -out crates/backend/tests/fixtures/test_pub.pem`",
        );
        let pub_pem = std::fs::read("tests/fixtures/test_pub.pem").expect("test_pub.pem missing");
        (
            EncodingKey::from_rsa_pem(&priv_pem).unwrap(),
            DecodingKey::from_rsa_pem(&pub_pem).unwrap(),
        )
    }

    #[test]
    fn verify_with_key_accepts_well_formed_token() {
        let (enc, dec) = pem_keypair();
        let claims = serde_json::json!({
            "sub": "fbuid_test",
            "email": "test@example.com",
            "aud": "aulalite-dev",
            "iss": "https://securetoken.google.com/aulalite-dev",
            "exp": chrono::Utc::now().timestamp() + 600,
            "iat": chrono::Utc::now().timestamp(),
        });
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("test-kid".into());
        let token = encode(&header, &claims, &enc).unwrap();

        let result = Verifier::verify_with_key(
            &token,
            &dec,
            "aulalite-dev",
            "https://securetoken.google.com/aulalite-dev",
        )
        .expect("should verify");
        assert_eq!(result.sub, "fbuid_test");
        assert_eq!(result.email.as_deref(), Some("test@example.com"));
    }

    #[test]
    fn verify_with_key_rejects_wrong_audience() {
        let (enc, dec) = pem_keypair();
        let claims = serde_json::json!({
            "sub": "fbuid_test",
            "aud": "wrong-project",
            "iss": "https://securetoken.google.com/aulalite-dev",
            "exp": chrono::Utc::now().timestamp() + 600,
            "iat": chrono::Utc::now().timestamp(),
        });
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("test-kid".into());
        let token = encode(&header, &claims, &enc).unwrap();

        let r = Verifier::verify_with_key(
            &token,
            &dec,
            "aulalite-dev",
            "https://securetoken.google.com/aulalite-dev",
        );
        assert!(r.is_err());
    }

    #[test]
    fn verify_with_key_rejects_expired_token() {
        let (enc, dec) = pem_keypair();
        let claims = serde_json::json!({
            "sub": "fbuid_test",
            "aud": "aulalite-dev",
            "iss": "https://securetoken.google.com/aulalite-dev",
            "exp": chrono::Utc::now().timestamp() - 60,
            "iat": chrono::Utc::now().timestamp() - 600,
        });
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("test-kid".into());
        let token = encode(&header, &claims, &enc).unwrap();

        let r = Verifier::verify_with_key(
            &token,
            &dec,
            "aulalite-dev",
            "https://securetoken.google.com/aulalite-dev",
        );
        assert!(r.is_err());
    }
}
```

- [ ] **Step 2: Generate the test fixtures**

```bash
mkdir -p crates/backend/tests/fixtures
openssl genpkey -algorithm RSA -out crates/backend/tests/fixtures/test_priv.pem -pkeyopt rsa_keygen_bits:2048
openssl rsa -in crates/backend/tests/fixtures/test_priv.pem -pubout -out crates/backend/tests/fixtures/test_pub.pem
```

- [ ] **Step 3: Add fixtures to `.gitignore`?**

No — these are test keys, not real credentials. Commit them. Add a comment in the file saying they are test-only.

- [ ] **Step 4: Update `auth/mod.rs`**

```rust
// crates/backend/src/auth/mod.rs
pub mod jwks;
pub mod verify;
```

- [ ] **Step 5: Run tests**

```bash
cargo test -p backend --lib auth::verify
```

Expected: 3 tests PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/backend/src/auth/verify.rs crates/backend/src/auth/mod.rs crates/backend/tests/fixtures
git commit -m "feat(auth): Firebase ID token verifier with audience/issuer/exp checks"
```

---

# Section E — RequestContext + Auth Middleware

### Task 24: Define `RequestContext` struct

**Files:**
- Create: `crates/backend/src/context.rs`
- Modify: `crates/backend/src/lib.rs`

- [ ] **Step 1: Write the struct**

```rust
// crates/backend/src/context.rs
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct RequestContext {
    pub user_id: Uuid,
    pub firebase_uid: String,
    pub email: String,
    pub display_name: Option<String>,
    pub tenant_id: Option<Uuid>,
    pub tenant_role: Option<core_types::TenantRole>,
    pub is_platform_admin: bool,
}
```

- [ ] **Step 2: Wire**

Add `pub mod context;` to `crates/backend/src/lib.rs`. Update `crates/backend/Cargo.toml` to depend on `core-types = { path = "../core-types" }` if not already.

- [ ] **Step 3: Build to confirm**

```bash
cargo build -p backend
```

- [ ] **Step 4: Commit**

```bash
git add crates/backend/src/context.rs crates/backend/src/lib.rs crates/backend/Cargo.toml
git commit -m "feat(backend): define RequestContext struct"
```

---

### Task 25: JIT user provisioning function

**Files:**
- Create: `crates/backend/src/auth/jit_provision.rs`
- Modify: `crates/backend/src/auth/mod.rs`

- [ ] **Step 1: Write the failing test**

```rust
// crates/backend/src/auth/jit_provision.rs
use sqlx::PgPool;
use uuid::Uuid;

use crate::auth::verify::FirebaseClaims;

#[derive(Debug, thiserror::Error)]
pub enum ProvisionError {
    #[error("db: {0}")]
    Db(#[from] sqlx::Error),
    #[error("missing email in claims")]
    MissingEmail,
}

#[derive(Debug, Clone)]
pub struct ProvisionedUser {
    pub user_id: Uuid,
    pub firebase_uid: String,
    pub email: String,
    pub display_name: Option<String>,
}

pub async fn ensure_user(pool: &PgPool, claims: &FirebaseClaims) -> Result<ProvisionedUser, ProvisionError> {
    let email = claims.email.clone().ok_or(ProvisionError::MissingEmail)?;
    let row: (Uuid, String, String, Option<String>) = sqlx::query_as(
        r#"
        INSERT INTO users (firebase_uid, email, display_name, last_seen_at)
        VALUES ($1, $2, $3, now())
        ON CONFLICT (firebase_uid) DO UPDATE
            SET last_seen_at = EXCLUDED.last_seen_at,
                email = EXCLUDED.email,
                display_name = COALESCE(EXCLUDED.display_name, users.display_name)
        RETURNING id, firebase_uid, email::text, display_name
        "#,
    )
    .bind(&claims.sub)
    .bind(&email)
    .bind(claims.name.as_deref())
    .fetch_one(pool)
    .await?;

    Ok(ProvisionedUser {
        user_id: row.0,
        firebase_uid: row.1,
        email: row.2,
        display_name: row.3,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use sqlx::postgres::PgPoolOptions;

    async fn pool() -> PgPool {
        let url = std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://aulalite:changeme@localhost:5432/aulalite".into());
        PgPoolOptions::new().max_connections(2).connect(&url).await.unwrap()
    }

    fn claims(uid: &str, email: &str, name: Option<&str>) -> FirebaseClaims {
        FirebaseClaims {
            sub: uid.into(),
            email: Some(email.into()),
            email_verified: Some(true),
            name: name.map(String::from),
            picture: None,
            aud: "aulalite-dev".into(),
            iss: "https://securetoken.google.com/aulalite-dev".into(),
            exp: Utc::now().timestamp() + 600,
            iat: Utc::now().timestamp(),
            auth_time: None,
        }
    }

    #[tokio::test]
    async fn first_call_creates_user() {
        let pool = pool().await;
        let uid = format!("fbuid_{}", uuid::Uuid::new_v4());
        let email = format!("u_{}@example.test", uuid::Uuid::new_v4());
        let p = ensure_user(&pool, &claims(&uid, &email, Some("U"))).await.unwrap();
        assert_eq!(p.firebase_uid, uid);
        assert_eq!(p.email.to_lowercase(), email.to_lowercase());
    }

    #[tokio::test]
    async fn second_call_is_idempotent_and_updates_last_seen() {
        let pool = pool().await;
        let uid = format!("fbuid_{}", uuid::Uuid::new_v4());
        let email = format!("u_{}@example.test", uuid::Uuid::new_v4());
        let c = claims(&uid, &email, Some("U"));
        let first = ensure_user(&pool, &c).await.unwrap();
        let second = ensure_user(&pool, &c).await.unwrap();
        assert_eq!(first.user_id, second.user_id);
    }
}
```

- [ ] **Step 2: Add to `auth/mod.rs`**

```rust
// crates/backend/src/auth/mod.rs
pub mod jit_provision;
pub mod jwks;
pub mod verify;
```

- [ ] **Step 3: Run tests**

```bash
docker compose up -d postgres
export DATABASE_URL=postgres://aulalite:changeme@localhost:5432/aulalite
sqlx migrate run --source migrations
cargo test -p backend --lib auth::jit_provision
```

Expected: 2 tests PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/backend/src/auth/jit_provision.rs crates/backend/src/auth/mod.rs
git commit -m "feat(auth): JIT user provisioning with idempotent upsert"
```

---

### Task 26: Auth middleware

**Files:**
- Create: `crates/backend/src/auth/middleware.rs`
- Create: `crates/backend/src/error.rs`
- Modify: `crates/backend/src/auth/mod.rs`
- Modify: `crates/backend/src/lib.rs`

- [ ] **Step 1: Define `ApiError`**

```rust
// crates/backend/src/error.rs
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("unauthorized: {0}")]
    Unauthorized(String),
    #[error("forbidden")]
    Forbidden,
    #[error("not found")]
    NotFound,
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("internal: {0}")]
    Internal(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, msg) = match &self {
            ApiError::Unauthorized(m) => (StatusCode::UNAUTHORIZED, m.clone()),
            ApiError::Forbidden => (StatusCode::FORBIDDEN, "forbidden".into()),
            ApiError::NotFound => (StatusCode::NOT_FOUND, "not found".into()),
            ApiError::BadRequest(m) => (StatusCode::BAD_REQUEST, m.clone()),
            ApiError::Internal(m) => (StatusCode::INTERNAL_SERVER_ERROR, m.clone()),
        };
        (status, axum::Json(json!({ "error": msg }))).into_response()
    }
}
```

- [ ] **Step 2: Write the middleware**

```rust
// crates/backend/src/auth/middleware.rs
use axum::extract::{Request, State};
use axum::http::header::AUTHORIZATION;
use axum::middleware::Next;
use axum::response::Response;
use sqlx::PgPool;

use crate::auth::{jit_provision::ensure_user, verify::Verifier};
use crate::context::RequestContext;
use crate::error::ApiError;

#[derive(Clone)]
pub struct AuthState {
    pub pool: PgPool,
    pub verifier: std::sync::Arc<Verifier>,
}

pub async fn require_auth(
    State(state): State<AuthState>,
    mut req: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let token = req
        .headers()
        .get(AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .ok_or_else(|| ApiError::Unauthorized("missing bearer token".into()))?
        .to_string();

    let claims = state
        .verifier
        .verify(&token)
        .await
        .map_err(|e| ApiError::Unauthorized(format!("token verification failed: {e}")))?;

    let user = ensure_user(&state.pool, &claims)
        .await
        .map_err(|e| ApiError::Internal(format!("user provisioning failed: {e}")))?;

    // Bootstrap: set app.user_id so the tenant_memberships RLS policy permits
    // the user-scoped lookup below. This is a transaction-scoped GUC so the
    // setting only lives until the COMMIT.
    let mut conn = state.pool.acquire().await
        .map_err(|e| ApiError::Internal(format!("pool acquire failed: {e}")))?;
    sqlx::query("BEGIN").execute(&mut *conn).await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    sqlx::query("SELECT set_config('app.user_id', $1, true)")
        .bind(user.user_id.to_string())
        .execute(&mut *conn).await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    // Look up tenant membership; for Phase 0 we resolve the user's first active membership.
    // (Multi-tenant routing — explicit tenant header — is added in Phase 1.)
    let membership: Option<(uuid::Uuid, String)> = sqlx::query_as(
        r#"
        SELECT tm.tenant_id, tm.role
        FROM tenant_memberships tm
        WHERE tm.user_id = $1 AND tm.status = 'active'
        ORDER BY tm.joined_at ASC
        LIMIT 1
        "#,
    )
    .bind(user.user_id)
    .fetch_optional(&mut *conn)
    .await
    .map_err(|e| ApiError::Internal(format!("membership lookup failed: {e}")))?;

    sqlx::query("COMMIT").execute(&mut *conn).await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    drop(conn);

    let is_platform_admin: bool = sqlx::query_scalar("SELECT is_platform_admin FROM users WHERE id = $1")
        .bind(user.user_id)
        .fetch_one(&state.pool)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let (tenant_id, tenant_role) = match membership {
        Some((tid, role_str)) => (Some(tid), Some(parse_role(&role_str)?)),
        None => (None, None),
    };

    let ctx = RequestContext {
        user_id: user.user_id,
        firebase_uid: user.firebase_uid,
        email: user.email,
        display_name: user.display_name,
        tenant_id,
        tenant_role,
        is_platform_admin,
    };

    req.extensions_mut().insert(ctx);
    Ok(next.run(req).await)
}

fn parse_role(s: &str) -> Result<core_types::TenantRole, ApiError> {
    Ok(match s {
        "org_admin" => core_types::TenantRole::OrgAdmin,
        "teacher" => core_types::TenantRole::Teacher,
        "ta" => core_types::TenantRole::Ta,
        "student" => core_types::TenantRole::Student,
        "parent" => core_types::TenantRole::Parent,
        other => return Err(ApiError::Internal(format!("unknown role: {other}"))),
    })
}
```

- [ ] **Step 3: Wire the modules**

Update `crates/backend/src/auth/mod.rs`:

```rust
pub mod jit_provision;
pub mod jwks;
pub mod middleware;
pub mod verify;
```

Update `crates/backend/src/lib.rs`:

```rust
pub mod auth;
pub mod context;
pub mod db;
pub mod error;
pub mod handlers;

use axum::{routing::get, Router};

pub fn router() -> Router {
    Router::new().route("/healthz", get(handlers::health::healthz))
}

pub fn router_for_tests() -> Router {
    router()
}
```

- [ ] **Step 4: Build to confirm**

```bash
cargo build -p backend
```

Expected: builds.

- [ ] **Step 5: Commit**

```bash
git add crates/backend/src/auth crates/backend/src/error.rs crates/backend/src/lib.rs
git commit -m "feat(auth): Bearer-token middleware that resolves RequestContext"
```

---

### Task 27: `/v1/me` endpoint + integration test

**Files:**
- Create: `crates/backend/src/handlers/me.rs`
- Modify: `crates/backend/src/handlers/mod.rs`
- Modify: `crates/backend/src/lib.rs`
- Create: `crates/backend/tests/me_endpoint.rs`

- [ ] **Step 1: Write the handler**

```rust
// crates/backend/src/handlers/me.rs
use axum::Extension;
use axum::Json;
use serde::Serialize;

use crate::context::RequestContext;
use crate::error::ApiError;

#[derive(Serialize)]
pub struct MeResponse {
    pub user_id: uuid::Uuid,
    pub firebase_uid: String,
    pub email: String,
    pub display_name: Option<String>,
    pub tenant_id: Option<uuid::Uuid>,
    pub tenant_role: Option<core_types::TenantRole>,
    pub is_platform_admin: bool,
}

pub async fn me(Extension(ctx): Extension<RequestContext>) -> Result<Json<MeResponse>, ApiError> {
    Ok(Json(MeResponse {
        user_id: ctx.user_id,
        firebase_uid: ctx.firebase_uid,
        email: ctx.email,
        display_name: ctx.display_name,
        tenant_id: ctx.tenant_id,
        tenant_role: ctx.tenant_role,
        is_platform_admin: ctx.is_platform_admin,
    }))
}
```

- [ ] **Step 2: Update handlers `mod.rs`**

```rust
// crates/backend/src/handlers/mod.rs
pub mod health;
pub mod me;
```

- [ ] **Step 3: Update `lib.rs` to mount the route behind auth middleware**

```rust
// crates/backend/src/lib.rs
pub mod auth;
pub mod context;
pub mod db;
pub mod error;
pub mod handlers;

use std::sync::Arc;

use axum::{middleware, routing::get, Router};
use sqlx::PgPool;

use crate::auth::middleware::{require_auth, AuthState};
use crate::auth::verify::Verifier;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub verifier: Arc<Verifier>,
}

pub fn router(state: AppState) -> Router {
    let auth_state = AuthState {
        pool: state.pool.clone(),
        verifier: state.verifier.clone(),
    };

    let public = Router::new().route("/healthz", get(handlers::health::healthz));

    let authed = Router::new()
        .route("/v1/me", get(handlers::me::me))
        .layer(middleware::from_fn_with_state(auth_state, require_auth));

    Router::new().merge(public).merge(authed)
}

/// Test-only router builder that does not require a live Firebase project.
pub fn router_for_tests() -> Router {
    Router::new().route("/healthz", get(handlers::health::healthz))
}
```

- [ ] **Step 4: Update `main.rs` to construct `AppState`**

```rust
// crates/backend/src/main.rs
use std::sync::Arc;
use std::time::Duration;
use std::net::SocketAddr;

use backend::auth::jwks::JwksCache;
use backend::auth::verify::Verifier;
use backend::AppState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .json()
        .init();

    let pool = backend::db::pool_from_env().await?;
    backend::db::run_migrations(&pool).await?;

    let project_id = std::env::var("FIREBASE_PROJECT_ID")?;
    let issuer = std::env::var("FIREBASE_TOKEN_ISSUER")?;
    let jwks_url = std::env::var("FIREBASE_JWKS_URL")?;

    let jwks = JwksCache::new(jwks_url, Duration::from_secs(3600));
    let verifier = Arc::new(Verifier::new(jwks, project_id, issuer));

    let app = backend::router(AppState { pool, verifier });

    let addr: SocketAddr = std::env::var("BIND_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:8080".to_string())
        .parse()?;

    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, "backend listening");
    axum::serve(listener, app).await?;
    Ok(())
}
```

- [ ] **Step 5: Write the integration test**

```rust
// crates/backend/tests/me_endpoint.rs
//! Integration test for /v1/me. We don't run the full Firebase verifier here;
//! instead we build a custom router that uses a stubbed claim injector so we can
//! exercise the JIT-provisioning + tenant-resolution + serialization path.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::middleware;
use axum::routing::get;
use axum::Router;
use http_body_util::BodyExt;
use sqlx::postgres::PgPoolOptions;
use tower::ServiceExt;
use uuid::Uuid;

async fn pool() -> sqlx::PgPool {
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://aulalite:changeme@localhost:5432/aulalite".into());
    PgPoolOptions::new().max_connections(2).connect(&url).await.unwrap()
}

#[derive(Clone)]
struct StubAuth {
    pool: sqlx::PgPool,
    firebase_uid: String,
    email: String,
}

async fn stub_middleware(
    axum::extract::State(state): axum::extract::State<StubAuth>,
    mut req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Result<axum::response::Response, backend::error::ApiError> {
    let claims = backend::auth::verify::FirebaseClaims {
        sub: state.firebase_uid.clone(),
        email: Some(state.email.clone()),
        email_verified: Some(true),
        name: None,
        picture: None,
        aud: "aulalite-dev".into(),
        iss: "https://securetoken.google.com/aulalite-dev".into(),
        exp: chrono::Utc::now().timestamp() + 600,
        iat: chrono::Utc::now().timestamp(),
        auth_time: None,
    };
    let user = backend::auth::jit_provision::ensure_user(&state.pool, &claims)
        .await
        .map_err(|e| backend::error::ApiError::Internal(e.to_string()))?;
    let is_admin: bool = sqlx::query_scalar("SELECT is_platform_admin FROM users WHERE id = $1")
        .bind(user.user_id)
        .fetch_one(&state.pool)
        .await
        .map_err(|e| backend::error::ApiError::Internal(e.to_string()))?;

    let ctx = backend::context::RequestContext {
        user_id: user.user_id,
        firebase_uid: user.firebase_uid,
        email: user.email,
        display_name: user.display_name,
        tenant_id: None,
        tenant_role: None,
        is_platform_admin: is_admin,
    };
    req.extensions_mut().insert(ctx);
    Ok(next.run(req).await)
}

#[tokio::test]
async fn me_returns_user_after_jit_provision() {
    let pool = pool().await;
    let firebase_uid = format!("fbuid_{}", Uuid::new_v4());
    let email = format!("u_{}@example.test", Uuid::new_v4());

    let stub = StubAuth { pool: pool.clone(), firebase_uid: firebase_uid.clone(), email: email.clone() };

    let app = Router::new()
        .route("/v1/me", get(backend::handlers::me::me))
        .layer(middleware::from_fn_with_state(stub, stub_middleware));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/me")
                .header("authorization", "Bearer ignored-by-stub")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["firebase_uid"].as_str().unwrap(), firebase_uid);
    assert_eq!(json["email"].as_str().unwrap().to_lowercase(), email.to_lowercase());
    assert_eq!(json["tenant_id"], serde_json::Value::Null);
}
```

- [ ] **Step 6: Run the test**

```bash
docker compose up -d postgres
export DATABASE_URL=postgres://aulalite:changeme@localhost:5432/aulalite
sqlx migrate run --source migrations
cargo test -p backend --test me_endpoint
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/backend
git commit -m "feat(backend): /v1/me endpoint behind auth middleware + integration test"
```

---

# Section F — Design System Primitives

These tasks build only the primitives needed for auth screens. Other primitives (Modal, Tabs, Drawer, etc.) come during Phase 1 as the relevant features are built.

### Task 28: Design tokens (Rust + CSS variables)

**Files:**
- Create: `crates/design-system/src/tokens.rs`
- Create: `crates/design-system/assets/tokens.css`
- Modify: `crates/design-system/src/lib.rs`

- [ ] **Step 1: Write the Rust tokens**

```rust
// crates/design-system/src/tokens.rs
//! Design tokens. Mirrored 1:1 in `assets/tokens.css`.

pub mod color {
    pub const BG: &str = "var(--color-bg)";
    pub const SURFACE: &str = "var(--color-surface)";
    pub const TEXT: &str = "var(--color-text)";
    pub const TEXT_MUTED: &str = "var(--color-text-muted)";
    pub const PRIMARY: &str = "var(--color-primary)";
    pub const PRIMARY_HOVER: &str = "var(--color-primary-hover)";
    pub const SUCCESS: &str = "var(--color-success)";
    pub const WARNING: &str = "var(--color-warning)";
    pub const DANGER: &str = "var(--color-danger)";
    pub const LIVE: &str = "var(--color-live)";
}

pub mod space {
    pub const S1: &str = "var(--space-1)";
    pub const S2: &str = "var(--space-2)";
    pub const S3: &str = "var(--space-3)";
    pub const S4: &str = "var(--space-4)";
    pub const S5: &str = "var(--space-5)";
    pub const S6: &str = "var(--space-6)";
}

pub mod radius {
    pub const SM: &str = "var(--radius-sm)";
    pub const MD: &str = "var(--radius-md)";
    pub const LG: &str = "var(--radius-lg)";
}
```

- [ ] **Step 2: Write the CSS**

```css
/* crates/design-system/assets/tokens.css */
:root {
  /* Colors */
  --color-bg: #f7f8fa;
  --color-surface: #ffffff;
  --color-text: #1a1c1f;
  --color-text-muted: #6b7280;
  --color-primary: #2f6feb;
  --color-primary-hover: #1f5ed4;
  --color-success: #1f9d55;
  --color-warning: #d68a1c;
  --color-danger: #c23a3a;
  --color-live: #e0235a;

  /* Spacing */
  --space-1: 4px;
  --space-2: 8px;
  --space-3: 12px;
  --space-4: 16px;
  --space-5: 24px;
  --space-6: 32px;

  /* Radius */
  --radius-sm: 4px;
  --radius-md: 8px;
  --radius-lg: 16px;

  /* Type */
  --font-display: 'Inter', system-ui, -apple-system, sans-serif;
  --font-body: 'Inter', system-ui, -apple-system, sans-serif;
  --font-mono: 'JetBrains Mono', ui-monospace, monospace;

  /* Shadows */
  --shadow-sm: 0 1px 2px rgba(0,0,0,0.04);
  --shadow-md: 0 2px 8px rgba(0,0,0,0.08);
  --shadow-lg: 0 4px 16px rgba(0,0,0,0.12);
}
```

- [ ] **Step 3: Update `lib.rs`**

```rust
// crates/design-system/src/lib.rs
pub mod tokens;
```

- [ ] **Step 4: Build + commit**

```bash
cargo build -p design-system
git add crates/design-system
git commit -m "feat(design-system): tokens module + tokens.css"
```

---

### Task 29: `Button` primitive

**Files:**
- Create: `crates/design-system/src/button.rs`
- Modify: `crates/design-system/src/lib.rs`

- [ ] **Step 1: Write the failing test**

We test rendered output via Dioxus's `dioxus_ssr` for unit tests. Add `dioxus-ssr` to `[dev-dependencies]` of `crates/design-system/Cargo.toml`:

```toml
[dev-dependencies]
dioxus-ssr = "0.6"
```

Then:

```rust
// crates/design-system/src/button.rs (test at bottom)
use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct ButtonProps {
    pub label: String,
    #[props(default)]
    pub disabled: bool,
    #[props(default)]
    pub variant: ButtonVariant,
    pub on_click: EventHandler<MouseEvent>,
}

#[derive(Clone, PartialEq, Default)]
pub enum ButtonVariant {
    #[default]
    Primary,
    Secondary,
    Danger,
}

#[component]
pub fn Button(props: ButtonProps) -> Element {
    let class = match props.variant {
        ButtonVariant::Primary => "ds-button ds-button--primary",
        ButtonVariant::Secondary => "ds-button ds-button--secondary",
        ButtonVariant::Danger => "ds-button ds-button--danger",
    };
    rsx! {
        button {
            class: "{class}",
            r#type: "button",
            disabled: props.disabled,
            onclick: move |evt| props.on_click.call(evt),
            "{props.label}"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_primary_label() {
        fn _app() -> Element {
            rsx! {
                Button {
                    label: "Sign in".to_string(),
                    on_click: |_| {},
                }
            }
        }
        let mut vdom = VirtualDom::new(_app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("Sign in"));
        assert!(html.contains("ds-button--primary"));
    }

    #[test]
    fn renders_disabled() {
        fn _app() -> Element {
            rsx! {
                Button {
                    label: "X".to_string(),
                    disabled: true,
                    on_click: |_| {},
                }
            }
        }
        let mut vdom = VirtualDom::new(_app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("disabled"));
    }
}
```

- [ ] **Step 2: Add to `lib.rs`**

```rust
// crates/design-system/src/lib.rs
pub mod button;
pub mod tokens;

pub use button::{Button, ButtonProps, ButtonVariant};
```

- [ ] **Step 3: Run tests**

```bash
cargo test -p design-system
```

Expected: 2 tests PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/design-system
git commit -m "feat(design-system): Button primitive with primary/secondary/danger variants"
```

---

### Task 30: `Input` primitive

**Files:**
- Create: `crates/design-system/src/input.rs`
- Modify: `crates/design-system/src/lib.rs`

- [ ] **Step 1: Write the test + implementation**

```rust
// crates/design-system/src/input.rs
use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct InputProps {
    pub value: String,
    #[props(default)]
    pub placeholder: String,
    #[props(default = "text".to_string())]
    pub input_type: String,
    #[props(default)]
    pub disabled: bool,
    pub on_input: EventHandler<String>,
}

#[component]
pub fn Input(props: InputProps) -> Element {
    rsx! {
        input {
            class: "ds-input",
            r#type: "{props.input_type}",
            value: "{props.value}",
            placeholder: "{props.placeholder}",
            disabled: props.disabled,
            oninput: move |evt| props.on_input.call(evt.value()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_with_placeholder() {
        fn _app() -> Element {
            rsx! {
                Input {
                    value: "".to_string(),
                    placeholder: "Email".to_string(),
                    on_input: |_| {},
                }
            }
        }
        let mut vdom = VirtualDom::new(_app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("placeholder=\"Email\""));
        assert!(html.contains("ds-input"));
    }
}
```

- [ ] **Step 2: Update `lib.rs`**

```rust
pub mod button;
pub mod input;
pub mod tokens;

pub use button::{Button, ButtonProps, ButtonVariant};
pub use input::{Input, InputProps};
```

- [ ] **Step 3: Run tests**

```bash
cargo test -p design-system
```

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/design-system
git commit -m "feat(design-system): Input primitive"
```

---

### Task 31: `Card`, `FormError`, `Spinner` primitives

**Files:**
- Create: `crates/design-system/src/card.rs`
- Create: `crates/design-system/src/form_error.rs`
- Create: `crates/design-system/src/spinner.rs`
- Modify: `crates/design-system/src/lib.rs`

- [ ] **Step 1: Card**

```rust
// crates/design-system/src/card.rs
use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct CardProps {
    pub children: Element,
}

#[component]
pub fn Card(props: CardProps) -> Element {
    rsx! { div { class: "ds-card", {props.children} } }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_children_inside_card_div() {
        fn _app() -> Element {
            rsx! { Card { p { "hello" } } }
        }
        let mut vdom = VirtualDom::new(_app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("ds-card"));
        assert!(html.contains("hello"));
    }
}
```

- [ ] **Step 2: FormError**

```rust
// crates/design-system/src/form_error.rs
use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct FormErrorProps {
    pub message: Option<String>,
}

#[component]
pub fn FormError(props: FormErrorProps) -> Element {
    match &props.message {
        Some(m) => rsx! { div { class: "ds-form-error", role: "alert", "{m}" } },
        None => rsx! {},
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_when_message_present() {
        fn _app() -> Element {
            rsx! { FormError { message: Some("bad email".to_string()) } }
        }
        let mut vdom = VirtualDom::new(_app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("bad email"));
    }

    #[test]
    fn renders_nothing_when_message_absent() {
        fn _app() -> Element {
            rsx! { FormError { message: None } }
        }
        let mut vdom = VirtualDom::new(_app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(!html.contains("ds-form-error"));
    }
}
```

- [ ] **Step 3: Spinner**

```rust
// crates/design-system/src/spinner.rs
use dioxus::prelude::*;

#[component]
pub fn Spinner() -> Element {
    rsx! {
        div { class: "ds-spinner", role: "status", aria_label: "Loading" }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_with_role_status() {
        fn _app() -> Element { rsx! { Spinner {} } }
        let mut vdom = VirtualDom::new(_app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("role=\"status\""));
    }
}
```

- [ ] **Step 4: Update `lib.rs`**

```rust
pub mod button;
pub mod card;
pub mod form_error;
pub mod input;
pub mod spinner;
pub mod tokens;

pub use button::{Button, ButtonProps, ButtonVariant};
pub use card::{Card, CardProps};
pub use form_error::{FormError, FormErrorProps};
pub use input::{Input, InputProps};
pub use spinner::Spinner;
```

- [ ] **Step 5: Run all design-system tests**

```bash
cargo test -p design-system
```

Expected: all PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/design-system
git commit -m "feat(design-system): Card, FormError, Spinner primitives"
```

---

### Task 32: Add component CSS

**Files:**
- Modify: `crates/design-system/assets/tokens.css` (or add a sibling file)
- Create: `crates/design-system/assets/components.css`

- [ ] **Step 1: Write component styles**

```css
/* crates/design-system/assets/components.css */
.ds-button {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  height: 40px;
  padding: 0 var(--space-4);
  border: none;
  border-radius: var(--radius-md);
  font-family: var(--font-body);
  font-weight: 500;
  cursor: pointer;
  transition: background-color 120ms ease;
}
.ds-button:disabled { opacity: 0.5; cursor: not-allowed; }

.ds-button--primary {
  background: var(--color-primary);
  color: #fff;
}
.ds-button--primary:hover:not(:disabled) { background: var(--color-primary-hover); }

.ds-button--secondary {
  background: var(--color-surface);
  color: var(--color-text);
  box-shadow: inset 0 0 0 1px var(--color-text-muted);
}

.ds-button--danger {
  background: var(--color-danger);
  color: #fff;
}

.ds-input {
  display: block;
  width: 100%;
  height: 40px;
  padding: 0 var(--space-3);
  border: 1px solid var(--color-text-muted);
  border-radius: var(--radius-md);
  font-family: var(--font-body);
  font-size: 14px;
  background: var(--color-surface);
}
.ds-input:focus { outline: 2px solid var(--color-primary); outline-offset: -1px; }

.ds-card {
  background: var(--color-surface);
  border-radius: var(--radius-lg);
  box-shadow: var(--shadow-md);
  padding: var(--space-5);
}

.ds-form-error {
  color: var(--color-danger);
  font-size: 13px;
  margin-top: var(--space-1);
}

.ds-spinner {
  width: 24px;
  height: 24px;
  border: 3px solid rgba(0,0,0,0.1);
  border-top-color: var(--color-primary);
  border-radius: 50%;
  animation: ds-spin 1s linear infinite;
}

@keyframes ds-spin { to { transform: rotate(360deg); } }
```

- [ ] **Step 2: Commit**

```bash
git add crates/design-system/assets/components.css
git commit -m "feat(design-system): component styles for primitives"
```

---

# Section G — Auth Screens (Web)

### Task 33: Firebase Web SDK JS bridge

**Files:**
- Create: `crates/shell-web/assets/firebase-bridge.js`
- Modify: `crates/shell-web/assets/index.html`

- [ ] **Step 1: Write the JS bridge**

```javascript
// crates/shell-web/assets/firebase-bridge.js
// Loaded as a classic script to avoid bundler complexity in Phase 0.
// Wraps Firebase Web SDK (modular API) and exposes a small global `window.aula.fb`
// that Dioxus calls via wasm-bindgen.

import { initializeApp } from "https://www.gstatic.com/firebasejs/10.13.0/firebase-app.js";
import {
  getAuth,
  signInWithEmailAndPassword,
  createUserWithEmailAndPassword,
  sendPasswordResetEmail,
  signOut,
  onIdTokenChanged,
} from "https://www.gstatic.com/firebasejs/10.13.0/firebase-auth.js";

const firebaseConfig = {
  apiKey: window.__AULALITE_FIREBASE_API_KEY__,
  authDomain: window.__AULALITE_FIREBASE_AUTH_DOMAIN__,
  projectId: window.__AULALITE_FIREBASE_PROJECT_ID__,
};

const app = initializeApp(firebaseConfig);
const auth = getAuth(app);

window.aula = window.aula || {};
window.aula.fb = {
  async signIn(email, password) {
    const cred = await signInWithEmailAndPassword(auth, email, password);
    return cred.user.getIdToken();
  },
  async signUp(email, password) {
    const cred = await createUserWithEmailAndPassword(auth, email, password);
    return cred.user.getIdToken();
  },
  async signOut() {
    await signOut(auth);
  },
  async forgot(email) {
    await sendPasswordResetEmail(auth, email);
  },
  async currentIdToken() {
    if (!auth.currentUser) return null;
    return auth.currentUser.getIdToken();
  },
  onIdTokenChanged(cb) {
    onIdTokenChanged(auth, async (user) => {
      const token = user ? await user.getIdToken() : null;
      cb(token);
    });
  },
};
```

- [ ] **Step 2: Update `index.html` (placeholders for env-injected Firebase config)**

```html
<!-- crates/shell-web/assets/index.html -->
<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>AulaLite</title>
  <link rel="stylesheet" href="/assets/tokens.css" />
  <link rel="stylesheet" href="/assets/components.css" />
  <script>
    // These globals are injected by the build process from env vars
    // (or, for local dev, hardcoded here temporarily).
    window.__AULALITE_FIREBASE_API_KEY__ = "REPLACE_WITH_DEV_API_KEY";
    window.__AULALITE_FIREBASE_AUTH_DOMAIN__ = "aulalite-dev.firebaseapp.com";
    window.__AULALITE_FIREBASE_PROJECT_ID__ = "aulalite-dev";
  </script>
  <script type="module" src="/assets/firebase-bridge.js"></script>
</head>
<body>
  <div id="main"></div>
</body>
</html>
```

- [ ] **Step 3: Update `Dioxus.toml` to copy assets**

Edit `crates/shell-web/Dioxus.toml`:

```toml
[application]
name = "aulalite-web"
default_platform = "web"

[web.app]
title = "AulaLite"

[web.resource]
style = ["/assets/tokens.css", "/assets/components.css"]
script = []

[web.watcher]
reload_html = true
```

Move the `tokens.css` and `components.css` into `crates/shell-web/assets/` (or symlink); for simplicity, copy:

```bash
cp crates/design-system/assets/*.css crates/shell-web/assets/
```

- [ ] **Step 4: Build and serve**

```bash
cd crates/shell-web && dx serve --platform web --port 3000
```

Open `http://localhost:3000` — you should see the hello world plus the bridge script loading. Open browser devtools console and run `window.aula.fb` — should be defined.

- [ ] **Step 5: Commit**

```bash
git add crates/shell-web
git commit -m "feat(shell-web): Firebase Web SDK bridge + asset copy"
```

---

### Task 34: Wasm-bindgen bindings to the Firebase bridge

**Files:**
- Create: `crates/platform-bridge/src/web.rs`
- Modify: `crates/platform-bridge/Cargo.toml`
- Modify: `crates/platform-bridge/src/lib.rs`

- [ ] **Step 1: Add wasm deps to platform-bridge**

```toml
# crates/platform-bridge/Cargo.toml
[package]
name = "platform-bridge"
version.workspace = true
edition.workspace = true
license.workspace = true
publish.workspace = true

[dependencies]
async-trait = "0.1"
thiserror = { workspace = true }

[target.'cfg(target_arch = "wasm32")'.dependencies]
wasm-bindgen = "0.2"
wasm-bindgen-futures = "0.4"
js-sys = "0.3"
web-sys = { version = "0.3", features = ["Window"] }
```

- [ ] **Step 2: Write `web.rs`**

```rust
// crates/platform-bridge/src/web.rs
#![cfg(target_arch = "wasm32")]

use async_trait::async_trait;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;

use crate::{BridgeError, PlatformBridge};

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["aula", "fb"], js_name = signIn, catch)]
    async fn js_sign_in(email: &str, password: &str) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(js_namespace = ["aula", "fb"], js_name = signUp, catch)]
    async fn js_sign_up(email: &str, password: &str) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(js_namespace = ["aula", "fb"], js_name = signOut, catch)]
    async fn js_sign_out() -> Result<JsValue, JsValue>;

    #[wasm_bindgen(js_namespace = ["aula", "fb"], js_name = forgot, catch)]
    async fn js_forgot(email: &str) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(js_namespace = ["aula", "fb"], js_name = currentIdToken, catch)]
    async fn js_current_id_token() -> Result<JsValue, JsValue>;
}

pub struct WebBridge;

#[async_trait(?Send)]
impl PlatformBridge for WebBridge {
    async fn current_id_token(&self) -> Result<String, BridgeError> {
        let v = js_current_id_token().await.map_err(|e| BridgeError::Io(format!("{e:?}")))?;
        v.as_string().ok_or_else(|| BridgeError::Io("no token".into()))
    }

    async fn sign_in_email_password(&self, email: &str, password: &str) -> Result<String, BridgeError> {
        let v = js_sign_in(email, password).await.map_err(|e| BridgeError::Io(format!("{e:?}")))?;
        v.as_string().ok_or_else(|| BridgeError::Io("no token".into()))
    }

    async fn sign_up_email_password(&self, email: &str, password: &str) -> Result<String, BridgeError> {
        let v = js_sign_up(email, password).await.map_err(|e| BridgeError::Io(format!("{e:?}")))?;
        v.as_string().ok_or_else(|| BridgeError::Io("no token".into()))
    }

    async fn sign_out(&self) -> Result<(), BridgeError> {
        js_sign_out().await.map_err(|e| BridgeError::Io(format!("{e:?}")))?;
        Ok(())
    }

    async fn send_password_reset(&self, email: &str) -> Result<(), BridgeError> {
        js_forgot(email).await.map_err(|e| BridgeError::Io(format!("{e:?}")))?;
        Ok(())
    }
}
```

> Note: `async_trait(?Send)` is used because `wasm-bindgen` futures aren't `Send`. The `lib.rs` trait must be re-declared without the `Send` bound on wasm. Apply the cfg accordingly:

- [ ] **Step 3: Update `lib.rs` to feature-gate Send**

```rust
// crates/platform-bridge/src/lib.rs
use async_trait::async_trait;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum BridgeError {
    #[error("permission denied")]
    PermissionDenied,
    #[error("not implemented on this platform")]
    NotImplemented,
    #[error("io: {0}")]
    Io(String),
}

#[cfg(not(target_arch = "wasm32"))]
#[async_trait]
pub trait PlatformBridge: Send + Sync {
    async fn current_id_token(&self) -> Result<String, BridgeError>;
    async fn sign_in_email_password(&self, email: &str, password: &str) -> Result<String, BridgeError>;
    async fn sign_up_email_password(&self, email: &str, password: &str) -> Result<String, BridgeError>;
    async fn sign_out(&self) -> Result<(), BridgeError>;
    async fn send_password_reset(&self, email: &str) -> Result<(), BridgeError>;
}

#[cfg(target_arch = "wasm32")]
#[async_trait(?Send)]
pub trait PlatformBridge {
    async fn current_id_token(&self) -> Result<String, BridgeError>;
    async fn sign_in_email_password(&self, email: &str, password: &str) -> Result<String, BridgeError>;
    async fn sign_up_email_password(&self, email: &str, password: &str) -> Result<String, BridgeError>;
    async fn sign_out(&self) -> Result<(), BridgeError>;
    async fn send_password_reset(&self, email: &str) -> Result<(), BridgeError>;
}

#[cfg(target_arch = "wasm32")]
pub mod web;
```

- [ ] **Step 4: Build for wasm**

```bash
cargo build -p platform-bridge --target wasm32-unknown-unknown
```

Expected: builds.

- [ ] **Step 5: Commit**

```bash
git add crates/platform-bridge
git commit -m "feat(platform-bridge): web Firebase JS bridge bindings"
```

---

### Task 35: Login screen

**Files:**
- Create: `crates/features-auth/src/login.rs`
- Modify: `crates/features-auth/src/lib.rs`
- Modify: `crates/features-auth/Cargo.toml` (add wasm-bindgen-futures for the spawn)

- [ ] **Step 1: Add deps**

```toml
# crates/features-auth/Cargo.toml
[package]
name = "features-auth"
version.workspace = true
edition.workspace = true
license.workspace = true
publish.workspace = true

[dependencies]
dioxus = { workspace = true }
core-types = { path = "../core-types" }
design-system = { path = "../design-system" }
platform-bridge = { path = "../platform-bridge" }

[target.'cfg(target_arch = "wasm32")'.dependencies]
wasm-bindgen-futures = "0.4"
```

- [ ] **Step 2: Write the Login component**

```rust
// crates/features-auth/src/login.rs
use dioxus::prelude::*;
use design_system::{Button, ButtonVariant, Card, FormError, Input, Spinner};

#[derive(Props, Clone, PartialEq)]
pub struct LoginProps {
    pub on_success: EventHandler<String>,
}

#[component]
pub fn Login(props: LoginProps) -> Element {
    let mut email = use_signal(String::new);
    let mut password = use_signal(String::new);
    let mut error = use_signal(|| None::<String>);
    let mut submitting = use_signal(|| false);

    let handle_submit = {
        let on_success = props.on_success.clone();
        move |_| {
            let email_val = email.read().clone();
            let password_val = password.read().clone();
            let on_success = on_success.clone();
            submitting.set(true);
            error.set(None);

            #[cfg(target_arch = "wasm32")]
            {
                use platform_bridge::PlatformBridge;
                wasm_bindgen_futures::spawn_local(async move {
                    let bridge = platform_bridge::web::WebBridge;
                    match bridge.sign_in_email_password(&email_val, &password_val).await {
                        Ok(token) => {
                            on_success.call(token);
                        }
                        Err(e) => error.set(Some(format!("Sign in failed: {e}"))),
                    }
                    submitting.set(false);
                });
            }

            #[cfg(not(target_arch = "wasm32"))]
            {
                let _ = (email_val, password_val);
                error.set(Some("Sign-in only available on web in Phase 0".into()));
                submitting.set(false);
            }
        }
    };

    rsx! {
        div { class: "auth-screen",
            Card {
                h1 { "Sign in to AulaLite" }
                form { onsubmit: handle_submit.clone(), prevent_default: "onsubmit",
                    div { class: "field",
                        label { "Email" }
                        Input {
                            value: email.read().clone(),
                            placeholder: "you@example.com".to_string(),
                            input_type: "email".to_string(),
                            disabled: *submitting.read(),
                            on_input: move |v| email.set(v),
                        }
                    }
                    div { class: "field",
                        label { "Password" }
                        Input {
                            value: password.read().clone(),
                            placeholder: "Your password".to_string(),
                            input_type: "password".to_string(),
                            disabled: *submitting.read(),
                            on_input: move |v| password.set(v),
                        }
                    }
                    FormError { message: error.read().clone() }
                    div { class: "actions",
                        if *submitting.read() {
                            Spinner {}
                        } else {
                            Button {
                                label: "Sign in".to_string(),
                                variant: ButtonVariant::Primary,
                                on_click: move |_| handle_submit(()),
                            }
                        }
                    }
                }
            }
        }
    }
}
```

- [ ] **Step 3: Wire to `lib.rs`**

```rust
// crates/features-auth/src/lib.rs
pub mod login;
pub use login::Login;
```

- [ ] **Step 4: Compile-check (wasm + native)**

```bash
cargo build -p features-auth --target wasm32-unknown-unknown
cargo build -p features-auth
```

Expected: both succeed.

- [ ] **Step 5: Commit**

```bash
git add crates/features-auth
git commit -m "feat(features-auth): Login screen wired to web Firebase bridge"
```

---

### Task 36: Signup screen

**Files:**
- Create: `crates/features-auth/src/signup.rs`
- Modify: `crates/features-auth/src/lib.rs`

- [ ] **Step 1: Write the Signup component (parallels Login)**

```rust
// crates/features-auth/src/signup.rs
use dioxus::prelude::*;
use design_system::{Button, ButtonVariant, Card, FormError, Input, Spinner};

#[derive(Props, Clone, PartialEq)]
pub struct SignupProps {
    pub on_success: EventHandler<String>,
}

#[component]
pub fn Signup(props: SignupProps) -> Element {
    let mut email = use_signal(String::new);
    let mut password = use_signal(String::new);
    let mut confirm = use_signal(String::new);
    let mut error = use_signal(|| None::<String>);
    let mut submitting = use_signal(|| false);

    let handle_submit = {
        let on_success = props.on_success.clone();
        move |_| {
            let email_val = email.read().clone();
            let password_val = password.read().clone();
            let confirm_val = confirm.read().clone();
            if password_val != confirm_val {
                error.set(Some("Passwords do not match".into()));
                return;
            }
            if password_val.len() < 8 {
                error.set(Some("Password must be at least 8 characters".into()));
                return;
            }
            let on_success = on_success.clone();
            submitting.set(true);
            error.set(None);

            #[cfg(target_arch = "wasm32")]
            {
                use platform_bridge::PlatformBridge;
                wasm_bindgen_futures::spawn_local(async move {
                    let bridge = platform_bridge::web::WebBridge;
                    match bridge.sign_up_email_password(&email_val, &password_val).await {
                        Ok(token) => on_success.call(token),
                        Err(e) => error.set(Some(format!("Sign up failed: {e}"))),
                    }
                    submitting.set(false);
                });
            }

            #[cfg(not(target_arch = "wasm32"))]
            {
                let _ = (email_val, password_val);
                error.set(Some("Sign-up only available on web in Phase 0".into()));
                submitting.set(false);
            }
        }
    };

    rsx! {
        div { class: "auth-screen",
            Card {
                h1 { "Create your AulaLite account" }
                form { onsubmit: handle_submit.clone(), prevent_default: "onsubmit",
                    div { class: "field",
                        label { "Email" }
                        Input {
                            value: email.read().clone(),
                            placeholder: "you@example.com".to_string(),
                            input_type: "email".to_string(),
                            disabled: *submitting.read(),
                            on_input: move |v| email.set(v),
                        }
                    }
                    div { class: "field",
                        label { "Password" }
                        Input {
                            value: password.read().clone(),
                            placeholder: "At least 8 characters".to_string(),
                            input_type: "password".to_string(),
                            disabled: *submitting.read(),
                            on_input: move |v| password.set(v),
                        }
                    }
                    div { class: "field",
                        label { "Confirm password" }
                        Input {
                            value: confirm.read().clone(),
                            placeholder: "Repeat your password".to_string(),
                            input_type: "password".to_string(),
                            disabled: *submitting.read(),
                            on_input: move |v| confirm.set(v),
                        }
                    }
                    FormError { message: error.read().clone() }
                    div { class: "actions",
                        if *submitting.read() {
                            Spinner {}
                        } else {
                            Button {
                                label: "Create account".to_string(),
                                variant: ButtonVariant::Primary,
                                on_click: move |_| handle_submit(()),
                            }
                        }
                    }
                }
            }
        }
    }
}
```

- [ ] **Step 2: Update `lib.rs`**

```rust
// crates/features-auth/src/lib.rs
pub mod login;
pub mod signup;
pub use login::Login;
pub use signup::Signup;
```

- [ ] **Step 3: Build**

```bash
cargo build -p features-auth --target wasm32-unknown-unknown
```

- [ ] **Step 4: Commit**

```bash
git add crates/features-auth
git commit -m "feat(features-auth): Signup screen with password confirmation"
```

---

### Task 37: Forgot-password screen + dashboard placeholder + router

**Files:**
- Create: `crates/features-auth/src/forgot.rs`
- Modify: `crates/features-auth/src/lib.rs`
- Modify: `crates/shell-web/src/main.rs`

- [ ] **Step 1: Write `forgot.rs`**

```rust
// crates/features-auth/src/forgot.rs
use dioxus::prelude::*;
use design_system::{Button, ButtonVariant, Card, FormError, Input, Spinner};

#[component]
pub fn ForgotPassword() -> Element {
    let mut email = use_signal(String::new);
    let mut sent = use_signal(|| false);
    let mut error = use_signal(|| None::<String>);
    let mut submitting = use_signal(|| false);

    let handle_submit = move |_| {
        let email_val = email.read().clone();
        submitting.set(true);
        error.set(None);

        #[cfg(target_arch = "wasm32")]
        {
            use platform_bridge::PlatformBridge;
            wasm_bindgen_futures::spawn_local(async move {
                let bridge = platform_bridge::web::WebBridge;
                match bridge.send_password_reset(&email_val).await {
                    Ok(()) => sent.set(true),
                    Err(e) => error.set(Some(format!("Could not send reset: {e}"))),
                }
                submitting.set(false);
            });
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = email_val;
            error.set(Some("Password reset only available on web in Phase 0".into()));
            submitting.set(false);
        }
    };

    rsx! {
        div { class: "auth-screen",
            Card {
                h1 { "Reset your password" }
                if *sent.read() {
                    p { "If an account exists for that email, a reset link has been sent." }
                } else {
                    form { onsubmit: handle_submit.clone(), prevent_default: "onsubmit",
                        div { class: "field",
                            label { "Email" }
                            Input {
                                value: email.read().clone(),
                                placeholder: "you@example.com".to_string(),
                                input_type: "email".to_string(),
                                disabled: *submitting.read(),
                                on_input: move |v| email.set(v),
                            }
                        }
                        FormError { message: error.read().clone() }
                        div { class: "actions",
                            if *submitting.read() {
                                Spinner {}
                            } else {
                                Button {
                                    label: "Send reset link".to_string(),
                                    variant: ButtonVariant::Primary,
                                    on_click: move |_| handle_submit(()),
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
```

- [ ] **Step 2: Update `lib.rs`**

```rust
// crates/features-auth/src/lib.rs
pub mod forgot;
pub mod login;
pub mod signup;
pub use forgot::ForgotPassword;
pub use login::Login;
pub use signup::Signup;
```

- [ ] **Step 3: Add a tiny in-app router to `shell-web`**

```rust
// crates/shell-web/src/main.rs
use dioxus::prelude::*;
use features_auth::{ForgotPassword, Login, Signup};

#[derive(Clone, PartialEq)]
enum Route {
    Login,
    Signup,
    Forgot,
    Dashboard { token: String },
}

fn main() {
    dioxus::launch(App);
}

#[component]
fn App() -> Element {
    let mut route = use_signal(|| Route::Login);

    let on_authed = move |token: String| route.set(Route::Dashboard { token });

    rsx! {
        div { class: "app-shell",
            header { class: "app-header",
                strong { "AulaLite" }
                nav {
                    if !matches!(*route.read(), Route::Login) {
                        a { href: "#", onclick: move |_| route.set(Route::Login), "Sign in" }
                    }
                    if !matches!(*route.read(), Route::Signup) {
                        a { href: "#", onclick: move |_| route.set(Route::Signup), "Create account" }
                    }
                    if !matches!(*route.read(), Route::Forgot) {
                        a { href: "#", onclick: move |_| route.set(Route::Forgot), "Forgot?" }
                    }
                }
            }
            main {
                match &*route.read() {
                    Route::Login => rsx! { Login { on_success: on_authed.clone() } },
                    Route::Signup => rsx! { Signup { on_success: on_authed.clone() } },
                    Route::Forgot => rsx! { ForgotPassword {} },
                    Route::Dashboard { token } => rsx! {
                        div { class: "dashboard",
                            h1 { "Welcome to AulaLite" }
                            p { "You are signed in." }
                            details {
                                summary { "Debug: ID token (truncated)" }
                                code { "{token[..token.len().min(40)]}…" }
                            }
                        }
                    },
                }
            }
        }
    }
}
```

- [ ] **Step 4: Run end-to-end manually**

```bash
docker compose up -d
cd crates/shell-web
dx serve --platform web --port 3000
```

In a separate terminal, get a Firebase ID token from the dev project. Or, use the UI: click "Create account", supply email/password (≥8 chars). On success, the dashboard should appear with a truncated token.

Then verify the backend handled the JIT provisioning:

```bash
curl -H "Authorization: Bearer <paste-the-full-token>" http://localhost:8080/v1/me
```

Expected: JSON with `firebase_uid`, `email`, `tenant_id: null`. Check the `users` table in Postgres:

```bash
psql "$DATABASE_URL" -c "SELECT id, firebase_uid, email FROM users ORDER BY created_at DESC LIMIT 1;"
```

Expected: a row matching the user that just signed up.

- [ ] **Step 5: Commit**

```bash
git add crates/features-auth crates/shell-web
git commit -m "feat(shell-web): in-app router with login/signup/forgot/dashboard"
```

---

# Section H — Mobile Auth (Phase 0 minimal: compile + simulator hello)

Mobile in Phase 0 is intentionally narrow: prove the Dioxus mobile shell **compiles, runs in a simulator, and shows the same auth screens** as web. The mobile-specific Firebase bridge (native iOS/Android SDKs via FFI) is parked for Phase 2 — Phase 0 mobile uses a **WebView-style bridge**: the same JS Firebase bridge loaded in a WKWebView/Android WebView. This is acceptable for Phase 0 because the goal here is "user can sign in on mobile and `/v1/me` works," not "native UI parity."

> If running this section is blocked by toolchain issues (missing Xcode, Android SDK, etc.), defer it to a separate session — Phase 0 exit can still pass on web alone provided this section is explicitly tracked as a known follow-up.

### Task 38: Wire `shell-mobile` to render the same auth screens

**Files:**
- Modify: `crates/shell-mobile/src/main.rs`

- [ ] **Step 1: Update `main.rs` to mirror `shell-web`**

Copy `crates/shell-web/src/main.rs` into `crates/shell-mobile/src/main.rs` verbatim. The component code is platform-agnostic; only the platform feature flag in `Cargo.toml` differs.

- [ ] **Step 2: Compile-check**

```bash
cargo check -p shell-mobile --target aarch64-apple-ios-sim
# or for Android:
cargo check -p shell-mobile --target aarch64-linux-android
```

Expected: compiles. (You may need to install the target via `rustup target add aarch64-apple-ios-sim` or `aarch64-linux-android`.)

- [ ] **Step 3: iOS simulator run**

```bash
cd crates/shell-mobile
dx serve --platform ios
```

Expected: an iOS simulator boots and shows the AulaLite UI with Login screen. Tap through; sign-in will fail because the JS Firebase bridge isn't available in the native renderer yet — that's expected for Phase 0.

- [ ] **Step 4: Android emulator run**

```bash
dx serve --platform android
```

Expected: emulator boots, app launches.

- [ ] **Step 5: Commit**

```bash
git add crates/shell-mobile
git commit -m "feat(shell-mobile): mirror web auth screens; minimal Phase 0 shell"
```

> Known follow-up filed for Phase 2: mobile native Firebase Auth bindings (so sign-in works on iOS/Android). For now, Phase 0 ships mobile as compile-passing with UI rendering; *real* mobile auth is a Phase 2 item per the spec.

---

# Section I — Platform Admin Tenant Provisioning CLI

### Task 39: `aulalite-admin` CLI binary

**Files:**
- Create: `tools/aulalite-admin/Cargo.toml`
- Create: `tools/aulalite-admin/src/main.rs`
- Modify: workspace `Cargo.toml` to register the member (already done in Task 1)

- [ ] **Step 1: Manifest**

```toml
# tools/aulalite-admin/Cargo.toml
[package]
name = "aulalite-admin"
version.workspace = true
edition.workspace = true
license.workspace = true
publish.workspace = true

[[bin]]
name = "aulalite-admin"
path = "src/main.rs"

[dependencies]
anyhow = { workspace = true }
clap = { version = "4", features = ["derive"] }
sqlx = { workspace = true }
tokio = { workspace = true }
uuid = { workspace = true }
```

- [ ] **Step 2: Write the CLI**

```rust
// tools/aulalite-admin/src/main.rs
use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand};
use sqlx::postgres::PgPoolOptions;

#[derive(Parser)]
#[command(name = "aulalite-admin", version)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Create a new tenant and assign an existing user as org_admin
    CreateTenant {
        #[arg(long)]
        slug: String,
        #[arg(long)]
        name: String,
        #[arg(long)]
        admin_email: String,
    },
    /// Promote a user to platform super admin (Elementors staff)
    PromotePlatformAdmin {
        #[arg(long)]
        email: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let url = std::env::var("DATABASE_URL").context("DATABASE_URL must be set")?;
    let pool = PgPoolOptions::new().max_connections(2).connect(&url).await?;

    match cli.cmd {
        Cmd::CreateTenant { slug, name, admin_email } => {
            let user_id: Option<uuid::Uuid> = sqlx::query_scalar(
                "SELECT id FROM users WHERE email = $1::citext",
            )
            .bind(&admin_email)
            .fetch_optional(&pool)
            .await?;

            let user_id = user_id.ok_or_else(|| anyhow!(
                "no user with email {admin_email}; have them sign up first, then re-run"
            ))?;

            let tenant_id: uuid::Uuid = sqlx::query_scalar(
                "INSERT INTO tenants (slug, name, status) VALUES ($1, $2, 'active') RETURNING id",
            )
            .bind(&slug)
            .bind(&name)
            .fetch_one(&pool)
            .await?;

            sqlx::query(
                "INSERT INTO tenant_memberships (tenant_id, user_id, role, status) VALUES ($1, $2, 'org_admin', 'active')",
            )
            .bind(tenant_id)
            .bind(user_id)
            .execute(&pool)
            .await?;

            println!("Created tenant {slug} ({tenant_id}) with org_admin {admin_email}");
        }
        Cmd::PromotePlatformAdmin { email } => {
            let updated = sqlx::query(
                "UPDATE users SET is_platform_admin = TRUE WHERE email = $1::citext",
            )
            .bind(&email)
            .execute(&pool)
            .await?;
            if updated.rows_affected() == 0 {
                return Err(anyhow!("no user with email {email}"));
            }
            println!("Promoted {email} to platform admin");
        }
    }
    Ok(())
}
```

- [ ] **Step 3: Build**

```bash
cargo build -p aulalite-admin
```

- [ ] **Step 4: Use it end-to-end**

After signing up via the web UI in Task 37 (so a `users` row exists), run:

```bash
export DATABASE_URL=postgres://aulalite:changeme@localhost:5432/aulalite
cargo run -p aulalite-admin -- create-tenant --slug acme-academy --name "Acme Academy" --admin-email <your-signup-email>
```

Expected: prints "Created tenant acme-academy (<uuid>) with org_admin <email>".

- [ ] **Step 5: Verify `/v1/me` now reports the tenant**

```bash
curl -H "Authorization: Bearer <fresh ID token>" http://localhost:8080/v1/me
```

Expected: `tenant_id` is non-null and `tenant_role` is `org_admin`.

> If the cached token from earlier predates the membership, refresh the token in the web UI (sign out + sign in) and try again — Firebase ID tokens last ~1 hour and don't carry tenant info, so the backend always re-resolves on each request, but signing back in just makes the test simpler.

- [ ] **Step 6: Commit**

```bash
git add tools/aulalite-admin
git commit -m "feat(tools): aulalite-admin CLI for tenant + platform admin provisioning"
```

---

# Section J — Phase 0 Exit Acceptance

### Task 40: End-to-end smoke runbook + tag the milestone

**Files:**
- Create: `docs/superpowers/plans/2026-05-03-aulalite-phase-0-exit-checklist.md`

- [ ] **Step 1: Write the exit checklist**

```markdown
# Phase 0 Exit Checklist

Run these in order. All must succeed for Phase 0 to be complete.

## 1. Stack health
- [ ] `docker compose up -d`
- [ ] `docker compose ps` shows all services healthy
- [ ] `curl http://localhost:8080/healthz` → `ok`
- [ ] `curl http://localhost:9997/v3/paths/list` → JSON
- [ ] `curl http://localhost:9000/minio/health/live` → 200

## 2. Migrations
- [ ] `sqlx migrate info --source migrations` shows all 5 migrations applied
- [ ] `psql "$DATABASE_URL" -c "\dt"` shows tenants, users, tenant_memberships

## 3. RLS verification
- [ ] `cargo test -p backend --test rls_tenant_isolation` passes

## 4. Backend tests
- [ ] `cargo test --workspace` — all green

## 5. Web sign-up + sign-in
- [ ] Open `http://localhost:3000` in a browser
- [ ] Click "Create account" — register a new user (≥8 char password)
- [ ] Dashboard appears with truncated ID token visible

## 6. JIT provisioning
- [ ] `psql "$DATABASE_URL" -c "SELECT email, firebase_uid FROM users ORDER BY created_at DESC LIMIT 1;"` shows the new user

## 7. /v1/me without tenant
- [ ] `curl -H "Authorization: Bearer <token>" http://localhost:8080/v1/me` → `tenant_id: null`

## 8. Tenant provisioning
- [ ] `cargo run -p aulalite-admin -- create-tenant --slug demo --name "Demo Org" --admin-email <signup-email>`
- [ ] Refresh ID token (sign out + sign in on web)
- [ ] `curl -H "Authorization: Bearer <new token>" http://localhost:8080/v1/me` → `tenant_id` non-null, `tenant_role: "org_admin"`

## 9. Mobile compile-check
- [ ] `cargo check -p shell-mobile --target aarch64-apple-ios-sim` (or equivalent Android target)

## 10. Optional but recommended
- [ ] iOS simulator boots and shows AulaLite Login screen (sign-in won't work natively yet — known Phase 2 follow-up)
- [ ] Android emulator boots and shows AulaLite Login screen

If all checked: tag the repo and open the Phase 1 plan kickoff.
```

- [ ] **Step 2: Run the checklist**

Execute each item. Fix anything that doesn't pass. Re-run.

- [ ] **Step 3: Tag the milestone**

```bash
git add docs/superpowers/plans/2026-05-03-aulalite-phase-0-exit-checklist.md
git commit -m "docs(plan): Phase 0 exit checklist"
git tag phase-0-complete
```

- [ ] **Step 4: Decide on Phase 1 plan**

Open a new planning session with the brainstorming or writing-plans skill to scope Phase 1 (P0 Core Spine vertical slice). The Phase 1 plan inherits the workspace, schema, auth, and design system built here, and adds: courses, lessons, enrollment, live sessions, MediaMTX integration, recordings, assignments, submissions, grading.

---

## Phase 0 Implementation Risks

These are tracked here so the Phase 1 plan can react to whichever pan out.

1. **Dioxus mobile WebView Firebase bridge** — Phase 0 uses a JS bridge that may not load identically in iOS WKWebView vs Android WebView. If sign-in works on web but breaks in the simulator, the workaround for Phase 0 is to ship mobile UI without working auth (compile + render only) and treat native Firebase Auth bindings as the first Phase 2 task.
2. **`SET LOCAL app.tenant_id` connection-pool leakage** — sqlx pools reuse connections; if a session-scoped GUC isn't reset between checkouts, a request could see another tenant's data on a recycled connection. Mitigation: always wrap with `BEGIN ... COMMIT` (which makes `SET LOCAL` truly transaction-scoped) or use `RESET app.tenant_id` on connection return. The Phase 1 plan must set this up rigorously before any tenant-scoped read.
3. **Firebase JWKS endpoint format** — the comment in `jwks.rs` notes Firebase has both an x509-cert endpoint and a JWK endpoint. Phase 0 uses the JWK endpoint (`https://www.googleapis.com/service_accounts/v1/jwk/securetoken@system.gserviceaccount.com`). If Google deprecates that endpoint, the verifier needs to fall back to parsing the x509 cert response.

---

*End of Phase 0 plan.*
