# System Audit Remediation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the broad system audit actionable by fixing reproducible defects found during baseline verification and recording remaining risks.

**Architecture:** Keep the default developer verification path service-free. DB-backed integration tests stay in the backend crate, but are gated behind an explicit Cargo feature so `cargo test --workspace` matches the README contract and a live Postgres run remains intentional.

**Tech Stack:** Rust workspace, Cargo integration test targets, Axum backend, Dioxus web shell, Playwright local E2E tooling.

---

### Task 1: Gate DB-Backed Backend Integration Tests

**Files:**
- Modify: `crates/backend/Cargo.toml`
- Modify: `README.md`

- [ ] **Step 1: Reproduce the current failure**

Run: `cargo test --workspace`
Expected: FAIL in `backend` test target `analytics` with `PoolTimedOut` when no live Postgres is available.

- [ ] **Step 2: Add backend feature and explicit test target list**

In `crates/backend/Cargo.toml`, add:

```toml
autotests = false

[features]
db-tests = []
```

Then add `[[test]]` entries so `health` remains a default test and DB-backed integration tests include:

```toml
required-features = ["db-tests"]
```

- [ ] **Step 3: Update verification docs**

In `README.md`, keep `cargo test --workspace` as the service-free command and add the explicit DB command:

```bash
cargo test -p backend --features db-tests --tests
```

- [ ] **Step 4: Verify the default path**

Run: `cargo fmt --check`, `cargo test --workspace`, and `cargo check -p shell-web --target wasm32-unknown-unknown`.
Expected: all commands exit 0 without requiring Docker/Postgres.

### Task 2: Continue Static Audit

**Files:**
- Review only first; modify only when a reproducible defect is found.

- [ ] **Step 1: Run static quality and dependency checks**

Run: `cargo clippy --workspace --all-targets -- -D warnings`, `npm audit --omit=dev`, and source searches for unsafe HTML, SQL string construction, secret leaks, and permissive CORS/header defaults.

- [ ] **Step 2: For any confirmed defect, add or identify the failing verification**

Use the smallest relevant Rust unit/integration test, existing Playwright smoke test, or cargo check command that fails before the fix.

- [ ] **Step 3: Implement focused fixes**

Patch only the root cause. Avoid unrelated refactors and avoid bundling unrelated warning cleanup unless it gates verification.

- [ ] **Step 4: Verify final state**

Run the commands from Task 1 plus any targeted checks for newly fixed defects.
