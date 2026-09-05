# AulaLite Editorial UI Refresh Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Refresh the main AulaLite web UI from plain browser-default rendering into a polished Modern Academy editorial interface while preserving current routes and backend behavior.

**Architecture:** Use a CSS-first redesign carried by shared token and component assets, with small Dioxus markup changes only where a page needs better structure or current raw classes block consistent styling. Keep existing component boundaries across auth, shell, dashboard, courses, schedule, and live room.

**Tech Stack:** Dioxus 0.7, Rust workspace tests, static CSS assets copied by Dioxus into the web build, Docker Compose with nginx frontend and backend API proxy.

---

## Scope Check

The design covers one subsystem: the existing web UI presentation layer. It does not require API, database, routing, auth, Docker topology, desktop shell, or mobile shell changes. The implementation can ship as one plan because every task contributes to the same visible UI refresh and remains testable through existing route smokes plus new CSS/SSR contracts.

## File Structure

- `crates/shell-web/public/assets/tokens.css`: runtime CSS tokens loaded by the web shell.
- `crates/shell-web/public/assets/components.css`: runtime component/page CSS loaded by the web shell.
- `crates/design-system/assets/tokens.css`: source-aligned copy for design-system package consumers.
- `crates/design-system/assets/components.css`: source-aligned copy for design-system package consumers.
- `crates/shell-web/tests/editorial_assets.rs`: CSS contract tests proving both asset copies stay synced and include editorial selectors.
- `crates/features-auth/tests/login.rs`: SSR contract for auth markup classes.
- `crates/shell-web/tests/dashboard_smoke.rs`: SSR contract for shell and dashboard structure.
- `crates/features-auth/src/login.rs`: small markup class additions for the auth form.
- `crates/shell-web/src/routes/login.rs`: local testing panel structure and button classes.
- `crates/features-courses/src/app_shell.rs`: shell structure classes and topbar identity grouping.
- `crates/features-courses/src/dashboard.rs`: editorial page header, stat cards, and course index markup.
- `crates/features-courses/src/course_list.rs`: rename the empty course-cover class to match new CSS.
- `crates/features-courses/src/live_room_broadcast.rs`: replace the live-state internal roadmap sentence with production-facing text.

## Working Tree Rule

Before implementation, run `git status --short`. This repository may already contain unrelated uncommitted Docker/frontend fixes. Do not revert them. Use path-limited `git add -- <paths>` in every commit step.

---

### Task 1: Add Failing Editorial UI Contracts

**Files:**
- Create: `crates/shell-web/tests/editorial_assets.rs`
- Modify: `crates/features-auth/tests/login.rs`
- Modify: `crates/shell-web/tests/dashboard_smoke.rs`

- [ ] **Step 1: Create CSS asset contract tests**

Create `crates/shell-web/tests/editorial_assets.rs` with this content:

```rust
use std::fs;
use std::path::PathBuf;

fn shell_asset(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("public")
        .join("assets")
        .join(name);
    fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!("failed to read {}: {err}", path.display())
    })
}

fn design_asset(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("design-system")
        .join("assets")
        .join(name);
    fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!("failed to read {}: {err}", path.display())
    })
}

#[test]
fn shell_and_design_system_assets_stay_in_sync() {
    assert_eq!(shell_asset("tokens.css"), design_asset("tokens.css"));
    assert_eq!(shell_asset("components.css"), design_asset("components.css"));
}

#[test]
fn tokens_define_modern_academy_theme_contract() {
    let css = shell_asset("tokens.css");

    for expected in [
        "--color-paper",
        "--color-ink",
        "--color-accent",
        "--color-accent-strong",
        "--color-live",
        "--font-display",
        "--font-body",
        "--radius-md: 8px",
    ] {
        assert!(
            css.contains(expected),
            "tokens.css missing expected token `{expected}`"
        );
    }
}

#[test]
fn components_cover_editorial_web_surface() {
    let css = shell_asset("components.css");

    for selector in [
        ".auth-composite",
        ".auth-local-panel",
        ".auth-title",
        ".app-shell-layout",
        ".app-side",
        ".app-topbar",
        ".page-header",
        ".dashboard-grid",
        ".dashboard-stat",
        ".course-cards",
        ".course-card-cover-empty",
        ".course-detail-header",
        ".schedule-list",
        ".live-room-view",
        ".live-room-broadcast",
        ".live-room-sidebars",
    ] {
        assert!(
            css.contains(selector),
            "components.css missing expected selector `{selector}`"
        );
    }
}
```

- [ ] **Step 2: Strengthen the auth SSR test**

Replace the assertions at the end of `login_renders_email_password_and_submit_controls` in `crates/features-auth/tests/login.rs` with:

```rust
    assert!(html.contains("Sign in to AulaLite"));
    assert!(html.contains("placeholder=\"you@example.com\""));
    assert!(html.contains("type=\"password\""));
    assert!(html.contains("Sign in"));
    assert!(html.contains("auth-login-card"));
    assert!(html.contains("auth-title"));
    assert!(html.contains("auth-form"));
    assert!(html.contains("field-label"));
```

- [ ] **Step 3: Strengthen the dashboard SSR test**

In `crates/shell-web/tests/dashboard_smoke.rs`, add these assertions to `renders_role_aware_dashboard_for_teacher` after the existing `Welcome back, Teach` assertion:

```rust
    assert!(
        html.contains("app-shell-layout"),
        "expected app shell wrapper in SSR output: {html}"
    );
    assert!(
        html.contains("page-header"),
        "expected editorial page header in SSR output: {html}"
    );
    assert!(
        html.contains("dashboard-stat"),
        "expected dashboard stat cards in SSR output: {html}"
    );
```

- [ ] **Step 4: Run tests and confirm they fail for the right reasons**

Run:

```powershell
cargo test -p shell-web --test editorial_assets
cargo test -p features-auth --test login
cargo test -p shell-web --test dashboard_smoke
```

Expected:

- `editorial_assets` fails because `--color-paper` and the new selectors are absent.
- `features-auth --test login` fails because `auth-login-card`, `auth-title`, `auth-form`, or `field-label` are absent.
- `dashboard_smoke` fails because `page-header` or `dashboard-stat` is absent.

- [ ] **Step 5: Commit failing contracts**

```powershell
git add -- crates/shell-web/tests/editorial_assets.rs crates/features-auth/tests/login.rs crates/shell-web/tests/dashboard_smoke.rs
git commit -m "test: add editorial ui contracts"
```

---

### Task 2: Replace Theme Tokens In Both Asset Copies

**Files:**
- Modify: `crates/shell-web/public/assets/tokens.css`
- Modify: `crates/design-system/assets/tokens.css`

- [ ] **Step 1: Replace `tokens.css` in both locations**

Replace both files with the exact content in Appendix A.

- [ ] **Step 2: Verify token tests advance**

Run:

```powershell
cargo test -p shell-web --test editorial_assets tokens_define_modern_academy_theme_contract
```

Expected: PASS.

- [ ] **Step 3: Verify asset sync remains intact**

Run:

```powershell
cargo test -p shell-web --test editorial_assets shell_and_design_system_assets_stay_in_sync
```

Expected: PASS.

- [ ] **Step 4: Commit token update**

```powershell
git add -- crates/shell-web/public/assets/tokens.css crates/design-system/assets/tokens.css
git commit -m "style: add modern academy theme tokens"
```

---

### Task 3: Replace Shared Component And Page CSS

**Files:**
- Modify: `crates/shell-web/public/assets/components.css`
- Modify: `crates/design-system/assets/components.css`

- [ ] **Step 1: Replace `components.css` in both locations**

Replace both files with the exact content in Appendix B.

- [ ] **Step 2: Verify CSS selector coverage**

Run:

```powershell
cargo test -p shell-web --test editorial_assets components_cover_editorial_web_surface
```

Expected: PASS.

- [ ] **Step 3: Verify asset sync**

Run:

```powershell
cargo test -p shell-web --test editorial_assets shell_and_design_system_assets_stay_in_sync
```

Expected: PASS.

- [ ] **Step 4: Commit component CSS update**

```powershell
git add -- crates/shell-web/public/assets/components.css crates/design-system/assets/components.css
git commit -m "style: add editorial component system"
```

---

### Task 4: Polish Auth And Local Testing Markup

**Files:**
- Modify: `crates/features-auth/src/login.rs`
- Modify: `crates/shell-web/src/routes/login.rs`

- [ ] **Step 1: Replace the `features_auth::Login` RSX body**

In `crates/features-auth/src/login.rs`, replace the current `rsx! { ... }` block inside `Login` with:

```rust
    rsx! {
        div { class: "auth-screen auth-login-card",
            Card {
                div { class: "auth-heading-block",
                    p { class: "auth-eyebrow", "AulaLite" }
                    h1 { class: "auth-title", "Sign in to AulaLite" }
                }
                form {
                    class: "auth-form",
                    onsubmit: move |event| {
                        event.prevent_default();
                        submit_login(email, password, error, submitting, form_success.clone());
                    },
                    div { class: "field",
                        label { class: "field-label", "Email" }
                        Input {
                            value: email.read().clone(),
                            placeholder: "you@example.com".to_string(),
                            input_type: "email".to_string(),
                            disabled: *submitting.read(),
                            on_input: move |value| email.set(value),
                        }
                    }
                    div { class: "field",
                        label { class: "field-label", "Password" }
                        Input {
                            value: password.read().clone(),
                            placeholder: "Your password".to_string(),
                            input_type: "password".to_string(),
                            disabled: *submitting.read(),
                            on_input: move |value| password.set(value),
                        }
                    }
                    FormError {
                        message: error.read().clone(),
                    }
                    div { class: "actions",
                        if *submitting.read() {
                            Spinner {}
                        } else {
                            Button {
                                label: "Sign in".to_string(),
                                variant: ButtonVariant::Primary,
                                on_click: move |_| {
                                    submit_login(email, password, error, submitting, button_success.clone());
                                },
                            }
                        }
                    }
                }
            }
        }
    }
```

- [ ] **Step 2: Replace the login route wrapper**

In `crates/shell-web/src/routes/login.rs`, replace the current final `rsx! { ... }` block with:

```rust
    rsx! {
        div { class: "auth-composite",
            if let Some(Ok(config)) = dev_login_config.read().as_ref() {
                if config.enabled && !config.profiles.is_empty() {
                    section { class: "auth-local-panel ds-card",
                        p { class: "auth-eyebrow", "Local testing" }
                        h2 { "Choose a test profile" }
                        div { class: "auth-account-grid",
                            for profile in &config.profiles {
                                button {
                                    key: "{profile.name}",
                                    class: "ds-button ds-button--secondary auth-account-button",
                                    onclick: {
                                        let profile_name = profile.name.clone();
                                        let dev_on_success = on_success.clone();
                                        move |_| {
                                            let profile_name = profile_name.clone();
                                            let mut dev_on_success = dev_on_success.clone();
                                            spawn(async move {
                                                let api_ctx = ApiContext {
                                                    base_url: String::new(),
                                                    id_token: String::new(),
                                                };
                                                if let Ok(response) = api::dev_login(&api_ctx, &profile_name).await {
                                                    dev_on_success(response.id_token);
                                                }
                                            });
                                        }
                                    },
                                    "{profile.display_name}"
                                }
                            }
                        }
                    }
                }
            }
            features_auth::Login { on_success: on_success }
        }
    }
```

- [ ] **Step 3: Run auth tests**

Run:

```powershell
cargo test -p features-auth --test login
cargo test -p shell-web --test shell_routes_smoke login_route_renders
```

Expected: PASS.

- [ ] **Step 4: Commit auth markup**

```powershell
git add -- crates/features-auth/src/login.rs crates/shell-web/src/routes/login.rs
git commit -m "style: polish auth entry screen"
```

---

### Task 5: Polish Shell, Dashboard, And Course Structure

**Files:**
- Modify: `crates/features-courses/src/app_shell.rs`
- Modify: `crates/features-courses/src/dashboard.rs`
- Modify: `crates/features-courses/src/course_list.rs`

- [ ] **Step 1: Replace `AppShell` RSX body**

In `crates/features-courses/src/app_shell.rs`, replace the current `rsx! { ... }` block inside `AppShell` with:

```rust
    rsx! {
        div { class: "app-shell-layout",
            aside { class: "app-side",
                div { class: "app-brand",
                    span { class: "app-brand-mark", "A" }
                    span { class: "app-brand-name", "AulaLite" }
                }
                nav { class: "app-nav", aria_label: "Primary navigation",
                    for (label, href) in &menu {
                        a { class: "nav-link", href: "{href}", "{label}" }
                    }
                }
            }
            main { class: "app-main",
                header { class: "app-topbar",
                    div { class: "app-topbar-title",
                        span { class: "topbar-kicker", "Workspace" }
                    }
                    div { class: "user-menu",
                        div { class: "user-meta",
                            span { class: "user-name", "{props.user.display_name}" }
                            span { class: "user-email", "{props.user.email}" }
                        }
                        button { class: "linkish", onclick: move |_| on_signout.call(()), "Sign out" }
                    }
                }
                section { class: "app-content", {props.children} }
            }
        }
    }
```

- [ ] **Step 2: Replace `Dashboard` RSX body**

In `crates/features-courses/src/dashboard.rs`, replace the current `rsx! { ... }` block inside `Dashboard` with:

```rust
    rsx! {
        div { class: "dashboard page-stack",
            header { class: "page-header",
                p { class: "page-kicker", "Dashboard" }
                h1 { "Welcome back, {props.display_name}" }
                p { class: "page-subtitle", "Courses, sessions, and classroom activity in one view." }
            }
            div { class: "dashboard-stats",
                div { class: "dashboard-stat",
                    span { class: "dashboard-stat-label", "Courses" }
                    strong { "{props.courses.len()}" }
                }
                div { class: "dashboard-stat",
                    span { class: "dashboard-stat-label", "Upcoming" }
                    strong { "{props.upcoming_count}" }
                }
            }
            div { class: "dashboard-grid",
                Card {
                    div { class: "card-header",
                        h2 { "Your Courses" }
                    }
                    if props.courses.is_empty() {
                        EmptyState {
                            title: "No courses yet".to_string(),
                            description: "Create one or redeem an enrollment code to get started.".to_string(),
                            cta: None,
                        }
                    } else {
                        ul { class: "course-list-mini",
                            for course in &props.courses {
                                {
                                    let slug = course.slug.clone();
                                    let title = course.title.clone();
                                    let role = course.role.clone();
                                    let next = course.next_session_at.clone();
                                    rsx! {
                                        li { class: "course-index-row",
                                            a { href: "/courses/{slug}", "{title}" }
                                            span { class: "role-badge", "{role}" }
                                            if let Some(n) = next {
                                                span { class: "next-session", "next: {n}" }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                Card {
                    div { class: "card-header",
                        h2 { "Upcoming" }
                    }
                    p { class: "dashboard-copy", "{props.upcoming_count} session(s) in the next 30 days." }
                    a { class: "primary-link", href: "/me/schedule", "View full schedule" }
                }
            }
        }
    }
```

- [ ] **Step 3: Rename the empty course cover class**

In `crates/features-courses/src/course_list.rs`, find the fallback cover `div`
inside the `else` branch after `if let Some(asset_id) = &course.cover_asset_id`
and change it to:

```rust
                                div { class: "course-card-cover-empty" }
```

- [ ] **Step 4: Run shell and course tests**

Run:

```powershell
cargo test -p shell-web --test dashboard_smoke
cargo test -p shell-web --test shell_routes_smoke
cargo test -p features-courses course_list
```

Expected: all selected tests PASS. Existing warnings in unrelated crates may still print.

- [ ] **Step 5: Commit shell and dashboard structure**

```powershell
git add -- crates/features-courses/src/app_shell.rs crates/features-courses/src/dashboard.rs crates/features-courses/src/course_list.rs
git commit -m "style: polish shell dashboard and course structure"
```

---

### Task 6: Polish Live Room Copy And Verify Shared Styling Coverage

**Files:**
- Modify: `crates/features-courses/src/live_room_broadcast.rs`

- [ ] **Step 1: Replace the teacher live-state copy**

In `crates/features-courses/src/live_room_broadcast.rs`, find the paragraph in
the `PublishState::Live` branch that starts with `Camera preview` and replace
that paragraph with:

```rust
                        p { class: "broadcast-copy", "Streaming is active. Keep this room open until class ends." }
```

- [ ] **Step 2: Run live room smoke tests**

Run:

```powershell
cargo test -p features-courses --test live_room_smoke
```

Expected: PASS.

- [ ] **Step 3: Run full UI contract tests**

Run:

```powershell
cargo test -p shell-web --test editorial_assets
cargo test -p features-auth --test login
cargo test -p shell-web --test dashboard_smoke
cargo test -p shell-web --test shell_routes_smoke
```

Expected: PASS.

- [ ] **Step 4: Commit live room polish**

```powershell
git add -- crates/features-courses/src/live_room_broadcast.rs
git commit -m "style: polish live room broadcast copy"
```

---

### Task 7: Docker Browser Verification

**Files:**
- No source edits expected.

- [ ] **Step 1: Rebuild and recreate the frontend container**

Run:

```powershell
docker compose up -d --build frontend
```

Expected: command exits `0`. If the build takes longer than the shell timeout but Docker finishes the image, run:

```powershell
docker image ls aulalite-frontend --format "table {{.Repository}}\t{{.Tag}}\t{{.ID}}\t{{.CreatedSince}}"
docker compose up -d --no-build --force-recreate frontend
```

Expected: `aulalite-frontend-1` starts on port `3000`.

- [ ] **Step 2: Check generated frontend HTML**

Run:

```powershell
$html = (Invoke-WebRequest -UseBasicParsing http://localhost:3000/).Content
if ($html -match '\{app_title\}') { throw 'raw Dioxus template was served' }
if ($html -notmatch 'assets/shell-web.*\.js') { throw 'generated app bootstrap script missing' }
if ($html -notmatch '/assets/tokens.css') { throw 'tokens.css missing from generated HTML' }
if ($html -notmatch '/assets/components.css') { throw 'components.css missing from generated HTML' }
'frontend html check passed'
```

Expected: `frontend html check passed`.

- [ ] **Step 3: Run browser runtime check**

Run:

```powershell
$chrome = Join-Path $env:LOCALAPPDATA 'ms-playwright\chromium_headless_shell-1223\chrome-headless-shell-win64\chrome-headless-shell.exe'
$output = & $chrome --headless --disable-gpu --virtual-time-budget=7000 --dump-dom http://localhost:3000/ 2>&1
$text = $output -join "`n"
if ($text -match 'Uncaught|RuntimeError|TypeError|SyntaxError') {
  $text
  throw 'browser runtime error detected'
}
if ($text -notmatch 'Sign in to AulaLite') {
  $text
  throw 'login UI not rendered'
}
'browser runtime check passed'
```

Expected: `browser runtime check passed`.

- [ ] **Step 4: Verify backend proxy still works through frontend origin**

Run:

```powershell
$json = (Invoke-WebRequest -UseBasicParsing http://localhost:3000/v1/dev/login/config).Content
if ($json -match '<!DOCTYPE html>') { throw 'frontend nginx served SPA HTML for API route' }
$json | ConvertFrom-Json | Out-Null
'frontend api proxy check passed'
```

Expected: `frontend api proxy check passed`.

- [ ] **Step 5: Capture desktop screenshot**

Run:

```powershell
npx --yes playwright screenshot --wait-for-timeout=5000 --viewport-size="1280,800" http://localhost:3000 "C:\Users\Chiranjib Chaudhuri\Documents\Chiranjib\Elementors_Aula\target\editorial-ui-login.png"
```

Expected: screenshot file is written at `target\editorial-ui-login.png`.

- [ ] **Step 6: Run final smoke tests**

Run:

```powershell
cargo test -p shell-web --test editorial_assets
cargo test -p features-auth --test login
cargo test -p shell-web --test dashboard_smoke
cargo test -p shell-web --test shell_routes_smoke
cargo test -p features-courses --test live_room_smoke
```

Expected: all tests PASS.

- [ ] **Step 7: Commit any remaining UI refresh files**

Run:

```powershell
git status --short
```

If the only uncommitted files are from this UI refresh, commit them:

```powershell
git add -- crates/shell-web/public/assets/tokens.css crates/shell-web/public/assets/components.css crates/design-system/assets/tokens.css crates/design-system/assets/components.css crates/features-auth/src/login.rs crates/shell-web/src/routes/login.rs crates/features-courses/src/app_shell.rs crates/features-courses/src/dashboard.rs crates/features-courses/src/course_list.rs crates/features-courses/src/live_room_broadcast.rs crates/shell-web/tests/editorial_assets.rs crates/features-auth/tests/login.rs crates/shell-web/tests/dashboard_smoke.rs
git commit -m "style: refresh web editorial ui"
```

If previous task commits already captured all UI refresh files, no commit is needed.

---

## Appendix A: `tokens.css`

```css
:root {
  --color-paper: #f4efe6;
  --color-paper-warm: #fbf7ef;
  --color-surface: #fffdf8;
  --color-surface-subtle: #f8f2e8;
  --color-ink: #171a17;
  --color-text: #171a17;
  --color-text-muted: #6f716c;
  --color-rule: rgba(32, 42, 35, 0.14);
  --color-rule-strong: rgba(32, 42, 35, 0.24);
  --color-accent: #2f5d50;
  --color-accent-strong: #183f36;
  --color-accent-soft: #dce9e2;
  --color-navy: #1d3144;
  --color-primary: #2f5d50;
  --color-primary-hover: #21483e;
  --color-info: #2d5f88;
  --color-success: #2f6b45;
  --color-warning: #9b6a1e;
  --color-danger: #a33a36;
  --color-live: #b8324a;
  --color-focus: rgba(47, 93, 80, 0.24);

  --space-1: 4px;
  --space-2: 8px;
  --space-3: 12px;
  --space-4: 16px;
  --space-5: 24px;
  --space-6: 32px;
  --space-7: 40px;
  --space-8: 56px;

  --radius-sm: 4px;
  --radius-md: 8px;
  --radius-lg: 8px;

  --font-display: Georgia, "Times New Roman", serif;
  --font-body: "Segoe UI", Roboto, Arial, sans-serif;
  --font-mono: "Cascadia Code", "Consolas", monospace;

  --shadow-sm: 0 1px 2px rgba(23, 26, 23, 0.06);
  --shadow-md: 0 14px 30px rgba(23, 26, 23, 0.08);
  --shadow-lg: 0 28px 60px rgba(23, 26, 23, 0.12);
}

* {
  box-sizing: border-box;
}

html {
  min-height: 100%;
  background: var(--color-paper);
}

body {
  min-height: 100%;
  margin: 0;
  color: var(--color-text);
  background:
    linear-gradient(180deg, rgba(255, 253, 248, 0.88), rgba(244, 239, 230, 0.92)),
    var(--color-paper);
  font-family: var(--font-body);
  font-size: 16px;
  line-height: 1.5;
}

button,
input,
select,
textarea {
  font: inherit;
}

a {
  color: var(--color-accent-strong);
  text-decoration-thickness: 1px;
  text-underline-offset: 3px;
}

a:hover {
  color: var(--color-navy);
}
```

## Appendix B: `components.css`

```css
.ds-button,
.btn,
.btn-primary,
.linkish {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-height: 40px;
  padding: 0 var(--space-4);
  border: 1px solid transparent;
  border-radius: var(--radius-md);
  font-family: var(--font-body);
  font-weight: 700;
  letter-spacing: 0;
  cursor: pointer;
  transition: background-color 120ms ease, border-color 120ms ease, box-shadow 120ms ease, transform 120ms ease;
}

.ds-button:hover:not(:disabled),
.btn:hover:not(:disabled),
.btn-primary:hover:not(:disabled) {
  transform: translateY(-1px);
}

.ds-button:disabled,
.btn:disabled,
.btn-primary:disabled {
  cursor: not-allowed;
  opacity: 0.55;
}

.ds-button--primary,
.btn-primary {
  color: #fff;
  background: var(--color-primary);
  border-color: var(--color-primary);
  box-shadow: 0 10px 22px rgba(47, 93, 80, 0.22);
}

.ds-button--primary:hover:not(:disabled),
.btn-primary:hover:not(:disabled) {
  background: var(--color-primary-hover);
  border-color: var(--color-primary-hover);
}

.ds-button--secondary,
.btn {
  color: var(--color-ink);
  background: rgba(255, 253, 248, 0.82);
  border-color: var(--color-rule-strong);
}

.ds-button--danger {
  color: #fff;
  background: var(--color-danger);
  border-color: var(--color-danger);
}

.linkish {
  min-height: 32px;
  padding: 0;
  color: var(--color-accent-strong);
  background: transparent;
  border: 0;
  box-shadow: none;
}

.linkish:hover {
  color: var(--color-navy);
  text-decoration: underline;
  text-underline-offset: 3px;
}

.ds-input,
.ds-select,
.ds-datetime,
textarea,
select {
  display: block;
  width: 100%;
  min-height: 42px;
  padding: 0 var(--space-3);
  color: var(--color-text);
  background: var(--color-surface);
  border: 1px solid var(--color-rule-strong);
  border-radius: var(--radius-md);
  font-family: var(--font-body);
  font-size: 14px;
}

textarea {
  min-height: 132px;
  padding-top: var(--space-3);
  resize: vertical;
}

.ds-input:focus,
.ds-select:focus,
.ds-datetime:focus,
textarea:focus,
select:focus {
  border-color: var(--color-accent);
  outline: 3px solid var(--color-focus);
}

.field {
  display: grid;
  gap: var(--space-2);
  margin-bottom: var(--space-4);
}

.field-label,
.field > label {
  color: var(--color-ink);
  font-size: 13px;
  font-weight: 800;
}

.actions,
.modal-actions {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-3);
  margin-top: var(--space-4);
}

.ds-card,
.card {
  padding: var(--space-5);
  background: rgba(255, 253, 248, 0.94);
  border: 1px solid var(--color-rule);
  border-radius: var(--radius-md);
  box-shadow: var(--shadow-md);
}

.card-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-4);
  margin-bottom: var(--space-4);
}

.card-header h2 {
  margin: 0;
  font-family: var(--font-display);
  font-size: 24px;
  line-height: 1.1;
}

.ds-form-error,
.form-error,
.error {
  margin: var(--space-2) 0 0;
  color: var(--color-danger);
  font-size: 13px;
  font-weight: 800;
}

.muted,
.empty-state-desc,
.course-desc,
.dashboard-copy {
  color: var(--color-text-muted);
}

.badge,
.role-badge,
.type-pill,
.track-active,
.live-pill {
  display: inline-flex;
  align-items: center;
  min-height: 24px;
  padding: 2px 9px;
  border-radius: 999px;
  border: 1px solid var(--color-rule);
  font-size: 12px;
  font-weight: 800;
  letter-spacing: 0;
  text-transform: capitalize;
}

.badge-neutral,
.role-badge,
.type-pill {
  color: var(--color-text-muted);
  background: var(--color-surface-subtle);
}

.badge-info {
  color: var(--color-info);
  background: #e8f0f6;
  border-color: rgba(45, 95, 136, 0.22);
}

.badge-success {
  color: var(--color-success);
  background: #e5efe7;
  border-color: rgba(47, 107, 69, 0.22);
}

.badge-warning {
  color: var(--color-warning);
  background: #f4ead6;
  border-color: rgba(155, 106, 30, 0.22);
}

.badge-danger,
.live-pill {
  color: var(--color-live);
  background: #f7e3e8;
  border-color: rgba(184, 50, 74, 0.24);
}

.tabs {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-2);
  padding-bottom: var(--space-4);
  border-bottom: 1px solid var(--color-rule);
}

.tab,
.chip {
  min-height: 36px;
  padding: 0 var(--space-4);
  color: var(--color-text-muted);
  background: rgba(255, 253, 248, 0.7);
  border: 1px solid var(--color-rule);
  border-radius: 999px;
  cursor: pointer;
  font-weight: 800;
}

.tab-active,
.chip-active {
  color: #fff;
  background: var(--color-accent-strong);
  border-color: var(--color-accent-strong);
}

.empty-state {
  padding: var(--space-6);
  background: var(--color-surface-subtle);
  border: 1px dashed var(--color-rule-strong);
  border-radius: var(--radius-md);
}

.empty-state-title {
  margin: 0 0 var(--space-2);
  font-family: var(--font-display);
}

.ds-spinner {
  width: 24px;
  height: 24px;
  border: 3px solid rgba(23, 26, 23, 0.1);
  border-top-color: var(--color-accent);
  border-radius: 50%;
  animation: ds-spin 1s linear infinite;
}

@keyframes ds-spin {
  to {
    transform: rotate(360deg);
  }
}

.auth-composite {
  min-height: 100vh;
  display: grid;
  grid-template-columns: minmax(280px, 420px) minmax(320px, 520px);
  align-items: center;
  justify-content: center;
  gap: var(--space-6);
  padding: var(--space-7);
}

.auth-screen {
  width: 100%;
}

.auth-login-card .ds-card {
  padding: var(--space-6);
}

.auth-local-panel {
  align-self: stretch;
  display: flex;
  flex-direction: column;
  justify-content: center;
  gap: var(--space-4);
  background: var(--color-accent-strong);
  color: #fff;
}

.auth-local-panel .auth-eyebrow {
  color: rgba(255, 255, 255, 0.72);
}

.auth-local-panel h2 {
  margin: 0;
  font-family: var(--font-display);
  font-size: 36px;
  line-height: 1.05;
}

.auth-account-grid {
  display: grid;
  gap: var(--space-3);
}

.auth-account-button {
  justify-content: flex-start;
  min-height: 48px;
  color: var(--color-ink);
  background: var(--color-paper-warm);
}

.auth-heading-block {
  margin-bottom: var(--space-5);
}

.auth-eyebrow,
.page-kicker,
.topbar-kicker {
  margin: 0 0 var(--space-2);
  color: var(--color-accent);
  font-size: 12px;
  font-weight: 900;
  letter-spacing: 0.08em;
  text-transform: uppercase;
}

.auth-title,
.page-header h1 {
  margin: 0;
  font-family: var(--font-display);
  color: var(--color-ink);
  font-size: clamp(34px, 5vw, 56px);
  line-height: 0.98;
}

.auth-form {
  display: grid;
  gap: var(--space-2);
}

.app-shell-layout {
  min-height: 100vh;
  display: grid;
  grid-template-columns: 260px minmax(0, 1fr);
}

.app-side {
  position: sticky;
  top: 0;
  height: 100vh;
  padding: var(--space-5);
  background: #162822;
  color: #fff;
  border-right: 1px solid rgba(255, 255, 255, 0.08);
}

.app-brand {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  margin-bottom: var(--space-6);
}

.app-brand-mark {
  width: 40px;
  height: 40px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 1px solid rgba(255, 255, 255, 0.34);
  border-radius: var(--radius-md);
  font-family: var(--font-display);
  font-size: 24px;
}

.app-brand-name {
  font-family: var(--font-display);
  font-size: 24px;
  font-weight: 700;
}

.app-nav {
  display: grid;
  gap: var(--space-2);
}

.nav-link {
  display: block;
  padding: 10px 12px;
  color: rgba(255, 255, 255, 0.78);
  border-radius: var(--radius-md);
  text-decoration: none;
}

.nav-link:hover {
  color: #fff;
  background: rgba(255, 255, 255, 0.08);
}

.app-main {
  min-width: 0;
}

.app-topbar {
  height: 72px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-4);
  padding: 0 var(--space-6);
  background: rgba(251, 247, 239, 0.88);
  border-bottom: 1px solid var(--color-rule);
  backdrop-filter: blur(12px);
}

.user-menu,
.user-meta {
  display: flex;
  align-items: center;
  gap: var(--space-3);
}

.user-meta {
  align-items: flex-end;
  flex-direction: column;
  gap: 0;
}

.user-name {
  font-weight: 900;
}

.user-email {
  color: var(--color-text-muted);
  font-size: 12px;
}

.app-content {
  width: min(1180px, calc(100vw - 260px - 48px));
  margin: 0 auto;
  padding: var(--space-6) var(--space-5) var(--space-8);
}

.page-stack {
  display: grid;
  gap: var(--space-5);
}

.page-header {
  display: grid;
  gap: var(--space-2);
  margin-bottom: var(--space-2);
}

.page-subtitle {
  max-width: 720px;
  margin: 0;
  color: var(--color-text-muted);
}

.dashboard-stats {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: var(--space-4);
}

.dashboard-stat {
  padding: var(--space-4);
  background: var(--color-surface);
  border: 1px solid var(--color-rule);
  border-radius: var(--radius-md);
}

.dashboard-stat-label {
  display: block;
  color: var(--color-text-muted);
  font-size: 12px;
  font-weight: 900;
  text-transform: uppercase;
}

.dashboard-stat strong {
  display: block;
  margin-top: var(--space-1);
  font-family: var(--font-display);
  font-size: 36px;
  line-height: 1;
}

.dashboard-grid {
  display: grid;
  grid-template-columns: minmax(0, 1.4fr) minmax(280px, 0.8fr);
  gap: var(--space-5);
}

.course-list-mini,
.schedule-list,
.assignment-list__items,
.lesson-files-list,
.outline-modules,
.outline-lessons {
  list-style: none;
  margin: 0;
  padding: 0;
}

.course-index-row {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  gap: var(--space-2) var(--space-3);
  padding: var(--space-3) 0;
  border-top: 1px solid var(--color-rule);
}

.course-index-row:first-child {
  border-top: 0;
}

.next-session {
  grid-column: 1 / -1;
  color: var(--color-text-muted);
  font-size: 13px;
}

.course-list-page,
.course-detail,
.course-create-page,
.course-schedule-tab,
.accept-page,
.redeem-page {
  display: grid;
  gap: var(--space-5);
}

.page-header,
.course-detail-header {
  display: flex;
  align-items: flex-end;
  justify-content: space-between;
  gap: var(--space-4);
}

.course-detail-header h1,
.course-list-page h1,
.course-create-page h1 {
  margin: 0;
  font-family: var(--font-display);
  font-size: clamp(34px, 4vw, 52px);
  line-height: 1;
}

.filter-chips,
.weekday-chips {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-2);
}

.course-cards {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(260px, 1fr));
  gap: var(--space-5);
}

.course-card-cover,
.course-card-cover-empty {
  width: 100%;
  aspect-ratio: 16 / 9;
  object-fit: cover;
  display: block;
  margin: calc(var(--space-5) * -1) calc(var(--space-5) * -1) var(--space-4);
  width: calc(100% + var(--space-5) * 2);
  border-bottom: 1px solid var(--color-rule);
}

.course-card-cover-empty {
  background:
    linear-gradient(135deg, rgba(47, 93, 80, 0.16), rgba(29, 49, 68, 0.16)),
    var(--color-surface-subtle);
}

.course-cards h3 {
  margin: 0 0 var(--space-3);
  font-family: var(--font-display);
  font-size: 24px;
}

.card-row {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-2);
  margin-bottom: var(--space-3);
}

.course-detail-banner {
  overflow: hidden;
  border-radius: var(--radius-md);
  border: 1px solid var(--color-rule);
}

.course-banner-img {
  width: 100%;
  max-height: 280px;
  object-fit: cover;
  display: block;
}

.course-detail-body {
  display: grid;
  gap: var(--space-5);
}

.outline-module {
  padding: var(--space-4) 0;
  border-top: 1px solid var(--color-rule);
}

.outline-lesson,
.assignment-list__row,
.schedule-list li {
  display: grid;
  gap: var(--space-2);
  padding: var(--space-4);
  background: var(--color-surface);
  border: 1px solid var(--color-rule);
  border-radius: var(--radius-md);
}

.schedule-list {
  display: grid;
  gap: var(--space-3);
}

.row-1,
.row-2,
.row-3 {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-2) var(--space-4);
}

.row-1 .title,
.lesson-title {
  font-weight: 900;
}

.due {
  color: var(--color-text-muted);
  font-size: 13px;
}

.live-room-shell,
.live-room-view,
.live-room-broadcast,
.live-room-lobby,
.live-room-replay {
  display: grid;
  gap: var(--space-5);
}

.live-room-view,
.live-room-broadcast {
  grid-template-columns: minmax(0, 1.5fr) minmax(280px, 0.75fr);
  align-items: start;
}

.live-room-view > .live-pill,
.live-room-broadcast > h2 {
  grid-column: 1 / -1;
}

.live-room-video-area,
.live-room-broadcast-controls,
.live-room-lobby,
.replay-video-pane {
  padding: var(--space-5);
  background: #101815;
  color: #fff;
  border-radius: var(--radius-md);
  box-shadow: var(--shadow-lg);
}

.live-video-main,
.replay-video,
.lesson-video {
  width: 100%;
  min-height: 320px;
  background: #050706;
  border-radius: var(--radius-md);
}

.live-room-sidebars {
  display: grid;
  gap: var(--space-3);
}

.live-room-chat,
.live-room-presence,
.live-room-hand-raise,
.replay-chat-pane {
  padding: var(--space-4);
  background: var(--color-surface);
  border: 1px solid var(--color-rule);
  border-radius: var(--radius-md);
  box-shadow: var(--shadow-sm);
}

.chat-list,
.participant-list,
.hand-queue {
  list-style: none;
  margin: 0;
  padding: 0;
  display: grid;
  gap: var(--space-2);
}

.chat-input {
  display: flex;
  gap: var(--space-2);
  margin-top: var(--space-3);
}

.presence-count,
.broadcast-status {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-2);
}

.presence-pulse,
.lobby-pulse {
  color: var(--color-live);
}

.broadcast-copy,
.lobby-subtitle,
.lobby-meta {
  color: rgba(255, 255, 255, 0.78);
}

.live-room-banner {
  grid-column: 1 / -1;
  padding: var(--space-3) var(--space-4);
  border-radius: var(--radius-md);
  border: 1px solid rgba(155, 106, 30, 0.24);
  background: #f4ead6;
  color: var(--color-warning);
  font-weight: 800;
}

.modal-backdrop {
  position: fixed;
  inset: 0;
  display: grid;
  place-items: center;
  padding: var(--space-5);
  background: rgba(23, 26, 23, 0.48);
  z-index: 1000;
}

.modal {
  width: min(640px, 100%);
  max-height: calc(100vh - 48px);
  overflow: auto;
  padding: var(--space-5);
  background: var(--color-surface);
  border-radius: var(--radius-md);
  box-shadow: var(--shadow-lg);
}

.modal-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: var(--space-4);
  margin-bottom: var(--space-4);
}

.modal-title {
  margin: 0;
  font-family: var(--font-display);
}

.modal-close {
  border: 0;
  background: transparent;
  cursor: pointer;
  font-size: 24px;
}

.members-table,
.submissions-grading-table table {
  width: 100%;
  border-collapse: collapse;
  background: var(--color-surface);
  border: 1px solid var(--color-rule);
  border-radius: var(--radius-md);
  overflow: hidden;
}

.members-table th,
.members-table td,
.submissions-grading-table th,
.submissions-grading-table td {
  padding: var(--space-3);
  border-bottom: 1px solid var(--color-rule);
  text-align: left;
}

@media (max-width: 900px) {
  .auth-composite,
  .app-shell-layout,
  .dashboard-grid,
  .live-room-view,
  .live-room-broadcast {
    grid-template-columns: 1fr;
  }

  .app-side {
    position: static;
    height: auto;
  }

  .app-content {
    width: 100%;
    padding: var(--space-5) var(--space-4) var(--space-7);
  }

  .app-topbar,
  .page-header,
  .course-detail-header {
    align-items: flex-start;
    flex-direction: column;
    height: auto;
    padding: var(--space-4);
  }

  .dashboard-stats {
    grid-template-columns: 1fr;
  }
}
```
