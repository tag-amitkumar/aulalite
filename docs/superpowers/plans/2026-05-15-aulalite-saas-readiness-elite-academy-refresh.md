# AulaLite SaaS Readiness Elite Academy Refresh Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Upgrade the current AulaLite web app into a polished elite-academy SaaS experience while fixing concrete frontend/backend consistency defects and recording larger SaaS expansion items for the next phase.

**Architecture:** Keep the existing Rust workspace boundaries. Centralize styling, motion, brand, icons, and reusable states in `crates/design-system`, mirror static assets into `crates/shell-web/public/assets`, and make focused Dioxus markup changes in `features-auth`, `features-courses`, and `shell-web`. Backend work is defect-driven and stays in existing handlers/tests, with a small confirmed schedule-validation fix and an audit/backlog artifact for larger SaaS expansion.

**Tech Stack:** Rust 2021, Dioxus 0.7.4, Dioxus Router, Axum 0.7, SQLx/Postgres, CSS static assets, `dioxus-free-icons` with Lucide icons, built-in image generation for raster brand imagery.

---

## Scope Check

This current-phase plan is intentionally broad, but it is one cohesive slice: polish and harden the existing product surface. New SaaS modules such as billing, analytics, onboarding, tenant settings, support, and a full admin console are not implemented here. They are captured in a backlog document in Task 10.

## File Structure

Create:

- `crates/design-system/src/brand.rs` - Dioxus brand/logo primitive.
- `crates/design-system/src/icon.rs` - small Lucide icon wrapper used by shell and feature pages.
- `crates/design-system/assets/brand/aulalite-mark.svg` - vector mark.
- `crates/design-system/assets/brand/aulalite-wordmark.svg` - vector wordmark.
- `crates/design-system/assets/brand/academy-hero.png` - generated auth hero image.
- `crates/design-system/assets/brand/course-cover-academy.png` - generated fallback course cover.
- `crates/shell-web/public/assets/brand/aulalite-mark.svg` - mirrored vector mark.
- `crates/shell-web/public/assets/brand/aulalite-wordmark.svg` - mirrored vector wordmark.
- `crates/shell-web/public/assets/brand/academy-hero.png` - mirrored generated auth hero image.
- `crates/shell-web/public/assets/brand/course-cover-academy.png` - mirrored generated fallback course cover.
- `docs/superpowers/specs/2026-05-15-aulalite-saas-expansion-backlog.md` - next-phase SaaS backlog and backend audit summary.

Modify:

- `Cargo.toml` - workspace dependency for `dioxus-free-icons`.
- `crates/design-system/Cargo.toml` - design-system dependency on `dioxus-free-icons`.
- `crates/design-system/src/lib.rs` - export brand/icon primitives.
- `crates/design-system/src/button.rs` - preserve button API while allowing icon-adjacent content through CSS classes only; no API break required.
- `crates/design-system/src/empty_state.rs` - add icon/image slots if tests require them.
- `crates/design-system/assets/tokens.css` - elite academy palette, type scale, motion tokens.
- `crates/design-system/assets/components.css` - full route styling, motion, system states, brand classes.
- `crates/shell-web/public/assets/tokens.css` - mirror of design-system tokens.
- `crates/shell-web/public/assets/components.css` - mirror of design-system components.
- `crates/shell-web/index.html` - favicon/app icon links and richer metadata.
- `crates/shell-web/src/route_enum.rs` - keep schedule route canonical as `/schedule`.
- `crates/shell-web/src/routes/login.rs` - auth layout, generated hero, local account polish.
- `crates/shell-web/src/routes/dashboard.rs` - route-level loading and redirect state polish.
- `crates/shell-web/src/routes/course_list.rs` - route-level loading and error wrapper polish.
- `crates/shell-web/src/routes/course_detail.rs` - course tabs, people/edit/schedule state polish.
- `crates/shell-web/src/routes/my_schedule.rs` - route shell and state polish.
- `crates/shell-web/src/routes/redeem.rs` - redeem route state polish.
- `crates/shell-web/src/routes/accept_invite.rs` - invite state polish.
- `crates/shell-web/src/routes/assignments_*.rs` - assignment route state polish.
- `crates/shell-web/src/routes/live_session.rs` - live session state polish.
- `crates/features-auth/src/login.rs` - branded auth card copy and structure.
- `crates/features-auth/src/signup.rs` - branded signup structure.
- `crates/features-auth/src/forgot.rs` - branded forgot-password structure.
- `crates/features-courses/src/app_shell.rs` - logo, icon nav, `/schedule` link fix, account area.
- `crates/features-courses/src/dashboard.rs` - dashboard hero, stats, schedule link fix.
- `crates/features-courses/src/course_list.rs` - course card art and state polish.
- `crates/features-courses/src/course_detail.rs` - detail hero/tabs polish.
- `crates/features-courses/src/assignment_list.rs` - loading/empty/error state polish.
- `crates/features-courses/src/assignment_detail.rs` - loading/error/detail state polish.
- `crates/features-courses/src/assignment_editor.rs` - form polish classes.
- `crates/features-courses/src/submission_form.rs` - form polish classes.
- `crates/features-courses/src/submissions_grading_table.rs` - table and state polish.
- `crates/features-courses/src/schedule_view.rs` - agenda styling and state polish.
- `crates/features-courses/src/redeem_code.rs` - focused branded workflow.
- `crates/features-courses/src/accept_invite.rs` - focused branded workflow.
- `crates/features-courses/src/live_room_broadcast.rs` - broadcast control polish; preserve existing uncommitted edits.
- `crates/features-courses/src/live_room_view.rs` - watch layout polish; preserve existing uncommitted edits.
- `crates/features-courses/src/live_room_lobby.rs` - lobby polish.
- `crates/features-courses/src/live_room_replay.rs` - replay polish.
- `crates/features-courses/src/live_room_chat.rs` - side panel polish.
- `crates/features-courses/src/live_room_presence.rs` - side panel polish.
- `crates/features-courses/src/live_room_hand_raise.rs` - side panel polish.
- `crates/backend/src/handlers/me.rs` - validate `/v1/me/schedule?days=`.

Test:

- `crates/shell-web/tests/editorial_assets.rs`
- `crates/shell-web/tests/dashboard_smoke.rs`
- `crates/shell-web/tests/shell_routes_smoke.rs`
- `crates/features-auth/tests/login.rs`
- `crates/features-auth/tests/signup.rs`
- `crates/features-auth/tests/forgot.rs`
- `crates/features-courses/tests/assignments_ssr.rs`
- `crates/features-courses/tests/live_room_smoke.rs`
- Unit tests in `crates/backend/src/handlers/me.rs`
- Existing backend integration tests listed under `crates/backend/tests/`

## Task 1: Workspace Baseline And Dirty Worktree Guard

**Files:**
- Inspect only: repository root and changed files shown by git.

- [ ] **Step 1: Record the starting worktree**

Run:

```powershell
git status --short
git diff --stat
```

Expected: existing uncommitted changes are present in live-room/auth files and `.claude/` remains untracked. Do not revert them.

- [ ] **Step 2: Inspect current uncommitted files before editing**

Run:

```powershell
git diff -- crates/backend/src/auth/middleware.rs crates/backend/src/handlers/live_sessions.rs crates/features-courses/src/live_room_broadcast.rs crates/features-courses/src/live_room_view.rs crates/shell-web/src/lib.rs crates/shell-web/src/routes/live_session.rs
```

Expected: output shows current local edits. Read the diff and preserve it when later tasks touch the same files.

- [ ] **Step 3: Run a fast frontend test baseline**

Run:

```powershell
cargo test -p shell-web --test editorial_assets
cargo test -p features-auth --test login
cargo test -p features-courses --test assignments_ssr
```

Expected: tests pass or fail with current known local-worktree failures. Record failing commands in the final implementation notes before changing code.

- [ ] **Step 4: Commit only after each task**

Use this commit rule throughout execution:

```powershell
git status --short
git add -- <files changed by the current task>
git commit -m "<type>: <short task summary>"
```

Expected: each task commit excludes unrelated pre-existing changes unless the current task intentionally builds on them.

## Task 2: Route And Backend Consistency Bug Fixes

**Files:**
- Modify: `crates/features-courses/src/app_shell.rs`
- Modify: `crates/features-courses/src/dashboard.rs`
- Modify: `crates/backend/src/handlers/me.rs`
- Test: `crates/shell-web/tests/dashboard_smoke.rs`
- Test: unit tests in `crates/backend/src/handlers/me.rs`

- [ ] **Step 1: Write the failing shell route test**

Add this assertion to `renders_role_aware_dashboard_for_teacher` in `crates/shell-web/tests/dashboard_smoke.rs`:

```rust
assert!(
    !html.contains("/me/schedule"),
    "shell must not link to the backend API path as a frontend route: {html}"
);
assert!(
    html.contains("/schedule") || !html.contains("My Schedule"),
    "frontend schedule links should use /schedule when present: {html}"
);
```

Add this student-specific test in the same file:

```rust
#[test]
fn renders_student_schedule_nav_to_frontend_route() {
    fn app() -> Element {
        rsx! {
            AppShell {
                user: ShellUser {
                    display_name: "Student".to_string(),
                    email: "s@x".to_string(),
                    tenant_role: Some(core_types::TenantRole::Student),
                    is_platform_admin: false,
                },
                on_signout: |_| {},
                Dashboard {
                    display_name: "Student".to_string(),
                    courses: vec![],
                    upcoming_count: 0,
                }
            }
        }
    }
    let mut vdom = VirtualDom::new(app);
    vdom.rebuild_in_place();
    let html = dioxus_ssr::render(&vdom);
    assert!(html.contains("href=\"/schedule\""), "student schedule link should use /schedule: {html}");
    assert!(!html.contains("href=\"/me/schedule\""), "student nav leaked API path: {html}");
}
```

- [ ] **Step 2: Run the frontend test to verify it fails**

Run:

```powershell
cargo test -p shell-web --test dashboard_smoke renders_student_schedule_nav_to_frontend_route -- --nocapture
```

Expected: FAIL because `AppShell` currently links students to `/me/schedule`.

- [ ] **Step 3: Fix schedule links**

In `crates/features-courses/src/app_shell.rs`, change the student menu entry:

```rust
("My Schedule", "/schedule"),
```

In `crates/features-courses/src/dashboard.rs`, change the dashboard schedule link:

```rust
a { class: "primary-link", href: "/schedule", "View full schedule" }
```

- [ ] **Step 4: Verify the route fix passes**

Run:

```powershell
cargo test -p shell-web --test dashboard_smoke
```

Expected: PASS.

- [ ] **Step 5: Write backend days-validation unit tests**

Add this module to the bottom of `crates/backend/src/handlers/me.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::normalize_schedule_days;

    #[test]
    fn schedule_days_defaults_to_thirty() {
        assert_eq!(normalize_schedule_days(None).unwrap(), 30);
    }

    #[test]
    fn schedule_days_accepts_bounds() {
        assert_eq!(normalize_schedule_days(Some(1)).unwrap(), 1);
        assert_eq!(normalize_schedule_days(Some(180)).unwrap(), 180);
    }

    #[test]
    fn schedule_days_rejects_zero_negative_and_large_values() {
        assert!(normalize_schedule_days(Some(0)).is_err());
        assert!(normalize_schedule_days(Some(-1)).is_err());
        assert!(normalize_schedule_days(Some(181)).is_err());
    }
}
```

- [ ] **Step 6: Run the backend unit test to verify it fails**

Run:

```powershell
cargo test -p backend handlers::me::tests::schedule_days_defaults_to_thirty
```

Expected: FAIL because `normalize_schedule_days` does not exist.

- [ ] **Step 7: Implement backend days validation**

Add this helper above `my_schedule` in `crates/backend/src/handlers/me.rs`:

```rust
fn normalize_schedule_days(days: Option<i64>) -> Result<i32, ApiError> {
    let days = days.unwrap_or(30);
    if !(1..=180).contains(&days) {
        return Err(ApiError::BadRequest(
            "days must be between 1 and 180".into(),
        ));
    }
    Ok(days as i32)
}
```

Replace the first line inside `my_schedule`:

```rust
let days = normalize_schedule_days(q.days)?;
```

Replace the SQL bind:

```rust
.bind(days)
```

- [ ] **Step 8: Verify backend days validation passes**

Run:

```powershell
cargo test -p backend handlers::me::tests::schedule_days
```

Expected: all three tests in `handlers::me::tests` pass.

- [ ] **Step 9: Commit route and backend consistency fixes**

Run:

```powershell
git add -- crates/features-courses/src/app_shell.rs crates/features-courses/src/dashboard.rs crates/backend/src/handlers/me.rs crates/shell-web/tests/dashboard_smoke.rs
git commit -m "fix: align schedule route and days validation"
```

Expected: commit succeeds without staging unrelated files.

## Task 3: Brand Assets And Metadata

**Files:**
- Create: `crates/design-system/assets/brand/aulalite-mark.svg`
- Create: `crates/design-system/assets/brand/aulalite-wordmark.svg`
- Create: `crates/design-system/assets/brand/academy-hero.png`
- Create: `crates/design-system/assets/brand/course-cover-academy.png`
- Create mirrored files under `crates/shell-web/public/assets/brand/`
- Modify: `crates/shell-web/index.html`
- Test: `crates/shell-web/tests/editorial_assets.rs`

- [ ] **Step 1: Write failing asset presence tests**

Add this helper and test to `crates/shell-web/tests/editorial_assets.rs`:

```rust
fn shell_asset_bytes(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("public")
        .join("assets")
        .join(name);
    fs::read(&path).unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()))
}

fn design_asset_bytes(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("design-system")
        .join("assets")
        .join(name);
    fs::read(&path).unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()))
}

#[test]
fn brand_assets_exist_and_stay_in_sync() {
    for asset in [
        "brand/aulalite-mark.svg",
        "brand/aulalite-wordmark.svg",
        "brand/academy-hero.png",
        "brand/course-cover-academy.png",
    ] {
        let shell = shell_asset_bytes(asset);
        let design = design_asset_bytes(asset);
        assert!(shell.len() > 100, "asset too small: {asset}");
        assert_eq!(shell, design, "asset not mirrored: {asset}");
    }
}
```

Add this metadata test:

```rust
#[test]
fn index_references_brand_metadata() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("index.html");
    let html = fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
    assert!(html.contains("AulaLite"));
    assert!(html.contains("/assets/brand/aulalite-mark.svg"));
    assert!(html.contains("theme-color"));
}
```

- [ ] **Step 2: Run asset test to verify it fails**

Run:

```powershell
cargo test -p shell-web --test editorial_assets brand_assets_exist_and_stay_in_sync -- --nocapture
```

Expected: FAIL because brand files do not exist.

- [ ] **Step 3: Create vector mark and wordmark**

Create both `brand` directories:

```powershell
New-Item -ItemType Directory -Force crates\design-system\assets\brand
New-Item -ItemType Directory -Force crates\shell-web\public\assets\brand
```

Create `crates/design-system/assets/brand/aulalite-mark.svg` with this content:

```xml
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 96 96" role="img" aria-labelledby="title desc">
  <title id="title">AulaLite mark</title>
  <desc id="desc">A monogram A shaped like an academy arch and open book.</desc>
  <rect width="96" height="96" rx="20" fill="#10251f"/>
  <path d="M24 74 45 22h6l21 52h-9l-5-13H38l-5 13h-9Z" fill="#f7efe0"/>
  <path d="M41 53h14l-7-18-7 18Z" fill="#c9a45c"/>
  <path d="M25 76c13-7 28-7 46 0" fill="none" stroke="#c9a45c" stroke-width="5" stroke-linecap="round"/>
  <path d="M29 23c10-9 28-9 38 0" fill="none" stroke="#f7efe0" stroke-width="4" stroke-linecap="round" opacity=".82"/>
</svg>
```

Create `crates/design-system/assets/brand/aulalite-wordmark.svg` with this content:

```xml
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 360 96" role="img" aria-labelledby="title desc">
  <title id="title">AulaLite wordmark</title>
  <desc id="desc">AulaLite wordmark with academy mark.</desc>
  <rect width="96" height="96" rx="20" fill="#10251f"/>
  <path d="M24 74 45 22h6l21 52h-9l-5-13H38l-5 13h-9Z" fill="#f7efe0"/>
  <path d="M41 53h14l-7-18-7 18Z" fill="#c9a45c"/>
  <path d="M25 76c13-7 28-7 46 0" fill="none" stroke="#c9a45c" stroke-width="5" stroke-linecap="round"/>
  <text x="120" y="58" fill="#10251f" font-family="Georgia, 'Times New Roman', serif" font-size="43" font-weight="700">AulaLite</text>
  <text x="123" y="76" fill="#6d6558" font-family="Segoe UI, Arial, sans-serif" font-size="12" font-weight="700" letter-spacing="3">LIVE LEARNING ACADEMY</text>
</svg>
```

- [ ] **Step 4: Generate raster academy imagery**

Use the built-in image generation tool twice with these prompts.

Prompt for `academy-hero.png`:

```text
Use case: ads-marketing
Asset type: web app authentication hero image
Primary request: an elite modern academy learning environment for a premium SaaS LMS
Scene/backdrop: sunlit contemporary library studio with oak desks, arched windows, tasteful brass details, digital class screens, and notebooks
Subject: no identifiable people; focus on place, tools, light, and premium academic atmosphere
Style/medium: polished editorial photography, realistic, refined, not stock-like
Composition/framing: landscape 16:10 composition with clear central depth and no text
Lighting/mood: warm morning light, confident, scholarly, calm
Color palette: ivory, ink, deep academy green, restrained gold, oxblood accents
Constraints: no logos, no readable text, no watermark, no distorted screens, no faces
```

Prompt for `course-cover-academy.png`:

```text
Use case: ads-marketing
Asset type: default course cover image
Primary request: premium abstract academy course cover for a SaaS learning platform
Scene/backdrop: layered paper, subtle book pages, thin architectural grid lines, and a small live-session light motif
Subject: no people; no text; no logos
Style/medium: editorial mixed-media still life, clean and modern
Composition/framing: landscape 16:9 with safe cropping at the edges
Lighting/mood: refined, focused, high-end academic
Color palette: ivory, ink, deep green, restrained gold, cool blue accents
Constraints: no readable text, no watermark, no faces, no brand names
```

Copy the selected outputs to:

```powershell
Copy-Item <generated-hero-path> crates\design-system\assets\brand\academy-hero.png
Copy-Item <generated-cover-path> crates\design-system\assets\brand\course-cover-academy.png
```

- [ ] **Step 5: Mirror brand assets into shell-web**

Run:

```powershell
Copy-Item crates\design-system\assets\brand\aulalite-mark.svg crates\shell-web\public\assets\brand\aulalite-mark.svg
Copy-Item crates\design-system\assets\brand\aulalite-wordmark.svg crates\shell-web\public\assets\brand\aulalite-wordmark.svg
Copy-Item crates\design-system\assets\brand\academy-hero.png crates\shell-web\public\assets\brand\academy-hero.png
Copy-Item crates\design-system\assets\brand\course-cover-academy.png crates\shell-web\public\assets\brand\course-cover-academy.png
```

- [ ] **Step 6: Wire browser metadata**

Add these lines inside `<head>` in `crates/shell-web/index.html`:

```html
<meta name="theme-color" content="#10251f">
<meta name="description" content="AulaLite is a premium live learning workspace for modern academies.">
<link rel="icon" type="image/svg+xml" href="/assets/brand/aulalite-mark.svg">
<link rel="apple-touch-icon" href="/assets/brand/aulalite-mark.svg">
```

- [ ] **Step 7: Verify brand assets and metadata**

Run:

```powershell
cargo test -p shell-web --test editorial_assets
```

Expected: PASS.

- [ ] **Step 8: Commit brand assets**

Run:

```powershell
git add -- crates/design-system/assets/brand crates/shell-web/public/assets/brand crates/shell-web/index.html crates/shell-web/tests/editorial_assets.rs
git commit -m "feat: add aulalite brand assets"
```

Expected: commit succeeds.

## Task 4: Design-System Brand And Icon Primitives

**Files:**
- Modify: `Cargo.toml`
- Modify: `crates/design-system/Cargo.toml`
- Modify: `crates/design-system/src/lib.rs`
- Create: `crates/design-system/src/brand.rs`
- Create: `crates/design-system/src/icon.rs`

- [ ] **Step 1: Add icon dependency**

In root `Cargo.toml`, add this workspace dependency:

```toml
dioxus-free-icons = { version = "0.10", default-features = false, features = ["lucide"] }
```

In `crates/design-system/Cargo.toml`, add:

```toml
dioxus-free-icons = { workspace = true }
```

- [ ] **Step 2: Create brand primitive**

Create `crates/design-system/src/brand.rs`:

```rust
use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct AulaLogoProps {
    #[props(default = "aula-logo".to_string())]
    pub class: String,
    #[props(default = false)]
    pub compact: bool,
}

#[component]
pub fn AulaLogo(props: AulaLogoProps) -> Element {
    let src = if props.compact {
        "/assets/brand/aulalite-mark.svg"
    } else {
        "/assets/brand/aulalite-wordmark.svg"
    };
    let alt = if props.compact {
        "AulaLite"
    } else {
        "AulaLite Live Learning Academy"
    };

    rsx! {
        span { class: "{props.class}",
            img { class: "aula-logo__image", src, alt }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logo_renders_wordmark_by_default() {
        fn app() -> Element {
            rsx! { AulaLogo {} }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("aulalite-wordmark.svg"), "got: {html}");
        assert!(html.contains("AulaLite Live Learning Academy"), "got: {html}");
    }

    #[test]
    fn logo_renders_compact_mark() {
        fn app() -> Element {
            rsx! { AulaLogo { compact: true } }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("aulalite-mark.svg"), "got: {html}");
    }
}
```

- [ ] **Step 3: Create Lucide icon wrapper**

Create `crates/design-system/src/icon.rs`:

```rust
use dioxus::prelude::*;
use dioxus_free_icons::icons::ld_icons::{
    LdBookOpen, LdCalendar, LdClipboardList, LdFileText, LdGraduationCap, LdHome,
    LdLogOut, LdMessageSquareText, LdPlus, LdRadio, LdSettings, LdUser, LdUsers,
    LdVideo,
};
use dioxus_free_icons::Icon;

#[derive(Clone, PartialEq)]
pub enum UiIcon {
    Dashboard,
    Courses,
    Schedule,
    Redeem,
    Assignment,
    File,
    Live,
    Video,
    Chat,
    People,
    User,
    SignOut,
    Add,
    Settings,
    Academy,
}

#[derive(Props, Clone, PartialEq)]
pub struct UiIconProps {
    pub icon: UiIcon,
    #[props(default = 18)]
    pub size: u32,
    #[props(default = "ui-icon".to_string())]
    pub class: String,
    #[props(default)]
    pub title: Option<String>,
}

#[component]
pub fn UiIconView(props: UiIconProps) -> Element {
    let size = props.size;
    let class = props.class.clone();
    let title = props.title.clone();
    match props.icon {
        UiIcon::Dashboard => rsx! { Icon { icon: LdHome, width: size, height: size, class, title } },
        UiIcon::Courses => rsx! { Icon { icon: LdBookOpen, width: size, height: size, class, title } },
        UiIcon::Schedule => rsx! { Icon { icon: LdCalendar, width: size, height: size, class, title } },
        UiIcon::Redeem => rsx! { Icon { icon: LdGraduationCap, width: size, height: size, class, title } },
        UiIcon::Assignment => rsx! { Icon { icon: LdClipboardList, width: size, height: size, class, title } },
        UiIcon::File => rsx! { Icon { icon: LdFileText, width: size, height: size, class, title } },
        UiIcon::Live => rsx! { Icon { icon: LdRadio, width: size, height: size, class, title } },
        UiIcon::Video => rsx! { Icon { icon: LdVideo, width: size, height: size, class, title } },
        UiIcon::Chat => rsx! { Icon { icon: LdMessageSquareText, width: size, height: size, class, title } },
        UiIcon::People => rsx! { Icon { icon: LdUsers, width: size, height: size, class, title } },
        UiIcon::User => rsx! { Icon { icon: LdUser, width: size, height: size, class, title } },
        UiIcon::SignOut => rsx! { Icon { icon: LdLogOut, width: size, height: size, class, title } },
        UiIcon::Add => rsx! { Icon { icon: LdPlus, width: size, height: size, class, title } },
        UiIcon::Settings => rsx! { Icon { icon: LdSettings, width: size, height: size, class, title } },
        UiIcon::Academy => rsx! { Icon { icon: LdGraduationCap, width: size, height: size, class, title } },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_renders_svg() {
        fn app() -> Element {
            rsx! { UiIconView { icon: UiIcon::Dashboard, title: Some("Dashboard".to_string()) } }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("<svg"), "got: {html}");
        assert!(html.contains("Dashboard"), "got: {html}");
    }
}
```

- [ ] **Step 4: Export primitives**

Modify `crates/design-system/src/lib.rs`:

```rust
pub mod brand;
pub mod icon;

pub use brand::{AulaLogo, AulaLogoProps};
pub use icon::{UiIcon, UiIconProps, UiIconView};
```

- [ ] **Step 5: Verify design-system compiles**

Run:

```powershell
cargo test -p design-system
```

Expected: PASS.

- [ ] **Step 6: Commit primitives**

Run:

```powershell
git add -- Cargo.toml Cargo.lock crates/design-system/Cargo.toml crates/design-system/src/lib.rs crates/design-system/src/brand.rs crates/design-system/src/icon.rs
git commit -m "feat: add brand and lucide icon primitives"
```

Expected: commit succeeds.

## Task 5: Elite Academy Tokens, Motion, And Shared CSS

**Files:**
- Modify: `crates/design-system/assets/tokens.css`
- Modify: `crates/design-system/assets/components.css`
- Modify mirrored files under `crates/shell-web/public/assets/`
- Test: `crates/shell-web/tests/editorial_assets.rs`

- [ ] **Step 1: Extend CSS contract tests**

In `tokens_define_modern_academy_theme_contract`, add expected tokens:

```rust
for expected in [
    "--color-paper",
    "--color-ink",
    "--color-accent",
    "--color-gold",
    "--color-oxblood",
    "--motion-fast",
    "--motion-page",
    "--font-display",
    "--font-body",
    "--radius-md: 8px",
] {
    assert!(
        css.contains(expected),
        "tokens.css missing expected token `{expected}`"
    );
}
```

In `components_cover_editorial_web_surface`, add selectors:

```rust
for selector in [
    ".aula-logo",
    ".ui-icon",
    ".auth-hero-visual",
    ".system-state",
    ".skeleton-line",
    ".motion-page",
    ".dashboard-hero",
    ".course-card-art",
    ".assignment-shell",
    ".live-room-stage",
] {
    assert!(
        css.contains(selector),
        "components.css missing expected selector `{selector}`"
    );
}
```

- [ ] **Step 2: Run CSS contract test to verify it fails**

Run:

```powershell
cargo test -p shell-web --test editorial_assets components_cover_editorial_web_surface -- --nocapture
```

Expected: FAIL because the new selectors are absent.

- [ ] **Step 3: Replace token palette and motion tokens**

In `crates/design-system/assets/tokens.css`, keep existing token names and add these tokens:

```css
:root {
  --color-paper: #f6f0e6;
  --color-paper-warm: #fbf6eb;
  --color-surface: #fffdf7;
  --color-surface-subtle: #f3eadb;
  --color-ink: #121614;
  --color-text: #171a17;
  --color-text-muted: #6d6558;
  --color-rule: rgba(18, 22, 20, 0.13);
  --color-rule-strong: rgba(18, 22, 20, 0.25);
  --color-accent: #244f43;
  --color-accent-strong: #10251f;
  --color-accent-soft: #dbe7df;
  --color-navy: #1d3144;
  --color-gold: #c9a45c;
  --color-gold-soft: #efe1bf;
  --color-oxblood: #8f243d;
  --color-primary: #244f43;
  --color-primary-hover: #18372f;
  --color-info: #2d5f88;
  --color-success: #2f6b45;
  --color-warning: #9b6a1e;
  --color-danger: #a33a36;
  --color-live: #b8324a;
  --color-focus: rgba(201, 164, 92, 0.32);

  --space-1: 4px;
  --space-2: 8px;
  --space-3: 12px;
  --space-4: 16px;
  --space-5: 24px;
  --space-6: 32px;
  --space-7: 40px;
  --space-8: 56px;
  --space-9: 72px;

  --radius-sm: 4px;
  --radius-md: 8px;
  --radius-lg: 8px;

  --font-display: Georgia, "Times New Roman", serif;
  --font-body: "Segoe UI", Roboto, Arial, sans-serif;
  --font-mono: "Cascadia Code", "Consolas", monospace;

  --shadow-sm: 0 1px 2px rgba(18, 22, 20, 0.06);
  --shadow-md: 0 16px 34px rgba(18, 22, 20, 0.09);
  --shadow-lg: 0 30px 72px rgba(18, 22, 20, 0.16);
  --shadow-gold: 0 18px 44px rgba(201, 164, 92, 0.18);

  --motion-fast: 120ms ease;
  --motion-medium: 180ms ease;
  --motion-page: 420ms cubic-bezier(.2, .8, .2, 1);
}
```

Keep existing base element rules and update `body` background to:

```css
body {
  min-height: 100%;
  margin: 0;
  color: var(--color-text);
  background:
    linear-gradient(180deg, rgba(255, 253, 247, 0.92), rgba(246, 240, 230, 0.96)),
    radial-gradient(circle at 10% 0%, rgba(201, 164, 92, 0.18), transparent 34%),
    var(--color-paper);
  font-family: var(--font-body);
  font-size: 16px;
  line-height: 1.5;
}
```

- [ ] **Step 4: Add shared brand, motion, system-state CSS**

Append to `crates/design-system/assets/components.css`:

```css
.aula-logo {
  display: inline-flex;
  align-items: center;
  min-width: 0;
}

.aula-logo__image {
  display: block;
  width: auto;
  height: 44px;
  max-width: 220px;
}

.app-brand .aula-logo__image {
  height: 52px;
  filter: drop-shadow(0 10px 22px rgba(0, 0, 0, .16));
}

.ui-icon {
  flex: 0 0 auto;
  display: inline-block;
  vertical-align: -0.18em;
}

.motion-page {
  animation: aula-page-in var(--motion-page) both;
}

@keyframes aula-page-in {
  from {
    opacity: 0;
    transform: translateY(12px);
  }
  to {
    opacity: 1;
    transform: translateY(0);
  }
}

@media (prefers-reduced-motion: reduce) {
  *,
  *::before,
  *::after {
    animation-duration: 1ms !important;
    animation-iteration-count: 1 !important;
    scroll-behavior: auto !important;
    transition-duration: 1ms !important;
  }
}

.system-state {
  display: grid;
  gap: var(--space-3);
  padding: var(--space-5);
  color: var(--color-text);
  background: var(--color-surface);
  border: 1px solid var(--color-rule);
  border-radius: var(--radius-md);
  box-shadow: var(--shadow-sm);
}

.system-state--error {
  border-color: rgba(163, 58, 54, .28);
  background: #fbebe8;
}

.system-state--loading {
  color: var(--color-text-muted);
}

.skeleton-line {
  width: 100%;
  height: 14px;
  overflow: hidden;
  border-radius: 999px;
  background: linear-gradient(90deg, rgba(18, 22, 20, .08), rgba(201, 164, 92, .18), rgba(18, 22, 20, .08));
  background-size: 220% 100%;
  animation: aula-skeleton 1.4s linear infinite;
}

@keyframes aula-skeleton {
  to {
    background-position: -220% 0;
  }
}

.auth-hero-visual {
  position: relative;
  min-height: 620px;
  overflow: hidden;
  background:
    linear-gradient(180deg, rgba(16, 37, 31, .08), rgba(16, 37, 31, .82)),
    url("/assets/brand/academy-hero.png") center / cover no-repeat;
  border-radius: var(--radius-md);
  box-shadow: var(--shadow-lg);
}

.auth-hero-visual::after {
  content: "";
  position: absolute;
  inset: auto var(--space-5) var(--space-5);
  height: 1px;
  background: linear-gradient(90deg, transparent, rgba(247, 239, 224, .86), transparent);
}

.dashboard-hero,
.course-detail-hero,
.assignment-shell {
  position: relative;
  overflow: hidden;
}

.dashboard-hero::before,
.course-detail-hero::before {
  content: "";
  position: absolute;
  inset: 0;
  pointer-events: none;
  background: linear-gradient(135deg, rgba(201, 164, 92, .16), transparent 42%);
}

.course-card-art {
  min-height: 164px;
  background:
    linear-gradient(135deg, rgba(16, 37, 31, .14), rgba(201, 164, 92, .18)),
    url("/assets/brand/course-cover-academy.png") center / cover no-repeat;
}

.live-room-stage {
  position: relative;
}
```

- [ ] **Step 5: Mirror CSS assets**

Run:

```powershell
Copy-Item crates\design-system\assets\tokens.css crates\shell-web\public\assets\tokens.css
Copy-Item crates\design-system\assets\components.css crates\shell-web\public\assets\components.css
```

- [ ] **Step 6: Verify CSS contracts**

Run:

```powershell
cargo test -p shell-web --test editorial_assets
```

Expected: PASS.

- [ ] **Step 7: Commit CSS foundation**

Run:

```powershell
git add -- crates/design-system/assets/tokens.css crates/design-system/assets/components.css crates/shell-web/public/assets/tokens.css crates/shell-web/public/assets/components.css crates/shell-web/tests/editorial_assets.rs
git commit -m "style: add elite academy visual system"
```

Expected: commit succeeds.

## Task 6: Auth And Shell Experience

**Files:**
- Modify: `crates/features-auth/src/login.rs`
- Modify: `crates/features-auth/src/signup.rs`
- Modify: `crates/features-auth/src/forgot.rs`
- Modify: `crates/shell-web/src/routes/login.rs`
- Modify: `crates/features-courses/src/app_shell.rs`
- Test: `crates/features-auth/tests/login.rs`
- Test: `crates/features-auth/tests/signup.rs`
- Test: `crates/features-auth/tests/forgot.rs`
- Test: `crates/shell-web/tests/dashboard_smoke.rs`

- [ ] **Step 1: Extend auth tests**

In `crates/features-auth/tests/login.rs`, add:

```rust
assert!(html.contains("AulaLite"));
assert!(html.contains("auth-heading-block"));
assert!(html.contains("auth-kicker"));
```

Add comparable assertions to signup and forgot tests:

```rust
assert!(html.contains("auth-screen"));
assert!(html.contains("auth-heading-block"));
assert!(html.contains("AulaLite"));
```

- [ ] **Step 2: Run auth tests to verify current gaps**

Run:

```powershell
cargo test -p features-auth
```

Expected: at least signup or forgot tests fail until those screens adopt the shared heading block.

- [ ] **Step 3: Update login heading structure**

In `crates/features-auth/src/login.rs`, replace the heading block with:

```rust
div { class: "auth-heading-block",
    p { class: "auth-kicker", "AulaLite Academy" }
    h1 { class: "auth-title", "Sign in to the academy" }
    p { class: "auth-subtitle", "Live classes, assignments, schedules, and course operations in one polished workspace." }
}
```

Keep the existing form behavior unchanged.

- [ ] **Step 4: Update signup and forgot heading structure**

In `crates/features-auth/src/signup.rs`, place this inside `Card` before the form:

```rust
div { class: "auth-heading-block",
    p { class: "auth-kicker", "AulaLite Academy" }
    h1 { class: "auth-title", "Create your AulaLite account" }
    p { class: "auth-subtitle", "Join your courses and live sessions with a secure academy profile." }
}
```

In `crates/features-auth/src/forgot.rs`, place this inside `Card` before the conditional content:

```rust
div { class: "auth-heading-block",
    p { class: "auth-kicker", "AulaLite Academy" }
    h1 { class: "auth-title", "Reset your password" }
    p { class: "auth-subtitle", "We will send a reset link when your account can receive one." }
}
```

- [ ] **Step 5: Add auth hero and local-profile polish**

In `crates/shell-web/src/routes/login.rs`, change the root to:

```rust
div { class: "auth-composite motion-page",
    section { class: "auth-hero-visual",
        div { class: "auth-hero-copy",
            design_system::AulaLogo { class: "auth-hero-logo".to_string(), compact: false }
            p { class: "auth-hero-kicker", "Elite live learning" }
            h2 { "A workspace built for modern academies." }
            p { "Run courses, live rooms, assignments, and schedules with the polish students expect." }
        }
    }
    div { class: "auth-panel-stack",
        if let Some(Ok(config)) = dev_login_config.read().as_ref() {
            if config.enabled && !config.profiles.is_empty() {
                section { class: "auth-local-panel ds-card",
                    p { class: "auth-eyebrow", "Local testing" }
                    h2 { "Choose a test profile" }
                    div { class: "auth-account-grid",
                        /* keep the existing profile loop and onclick body unchanged */
                    }
                }
            }
        }
        features_auth::Login { on_success: on_success }
    }
}
```

Preserve the existing `for profile in &config.profiles` loop and `dev_login` call body exactly when moving it into `auth-panel-stack`.

- [ ] **Step 6: Add shell logo and nav icons**

In `crates/features-courses/src/app_shell.rs`, import:

```rust
use design_system::{AulaLogo, UiIcon, UiIconView};
```

Change menu entries to include icons:

```rust
let menu = match role {
    Some(core_types::TenantRole::OrgAdmin) => vec![
        ("Dashboard", "/dashboard", UiIcon::Dashboard),
        ("My Courses", "/courses", UiIcon::Courses),
        ("All Tenant Courses", "/courses?scope=all", UiIcon::Settings),
    ],
    Some(core_types::TenantRole::Teacher) => vec![
        ("Dashboard", "/dashboard", UiIcon::Dashboard),
        ("My Courses", "/courses", UiIcon::Courses),
    ],
    Some(core_types::TenantRole::Ta) => vec![
        ("Dashboard", "/dashboard", UiIcon::Dashboard),
        ("My Courses", "/courses", UiIcon::Courses),
    ],
    Some(core_types::TenantRole::Student) | None => vec![
        ("Dashboard", "/dashboard", UiIcon::Dashboard),
        ("My Courses", "/courses", UiIcon::Courses),
        ("My Schedule", "/schedule", UiIcon::Schedule),
        ("Redeem Code", "/redeem", UiIcon::Redeem),
    ],
    Some(core_types::TenantRole::Parent) => vec![
        ("Dashboard", "/dashboard", UiIcon::Dashboard),
    ],
};
```

Replace the brand markup:

```rust
div { class: "app-brand",
    AulaLogo { class: "app-brand-logo".to_string(), compact: false }
}
```

Replace each nav link body:

```rust
for (label, href, icon) in &menu {
    a { class: "nav-link", href: "{href}",
        UiIconView { icon: icon.clone(), title: Some(label.to_string()) }
        span { "{label}" }
    }
}
```

- [ ] **Step 7: Verify auth and shell tests**

Run:

```powershell
cargo test -p features-auth
cargo test -p shell-web --test dashboard_smoke
```

Expected: PASS.

- [ ] **Step 8: Commit auth and shell polish**

Run:

```powershell
git add -- crates/features-auth/src/login.rs crates/features-auth/src/signup.rs crates/features-auth/src/forgot.rs crates/shell-web/src/routes/login.rs crates/features-courses/src/app_shell.rs crates/features-auth/tests crates/shell-web/tests/dashboard_smoke.rs
git commit -m "style: polish auth and shell experience"
```

Expected: commit succeeds.

## Task 7: Dashboard, Courses, And Schedule Polish

**Files:**
- Modify: `crates/features-courses/src/dashboard.rs`
- Modify: `crates/features-courses/src/course_list.rs`
- Modify: `crates/features-courses/src/course_detail.rs`
- Modify: `crates/features-courses/src/schedule_view.rs`
- Test: `crates/shell-web/tests/dashboard_smoke.rs`
- Test: `crates/shell-web/tests/shell_routes_smoke.rs`

- [ ] **Step 1: Extend dashboard smoke test**

In `renders_role_aware_dashboard_for_teacher`, add:

```rust
assert!(html.contains("dashboard-hero"), "expected dashboard hero: {html}");
assert!(html.contains("motion-page"), "expected page animation class: {html}");
```

- [ ] **Step 2: Run dashboard test to verify it fails**

Run:

```powershell
cargo test -p shell-web --test dashboard_smoke renders_role_aware_dashboard_for_teacher -- --nocapture
```

Expected: FAIL until dashboard receives `dashboard-hero` and `motion-page`.

- [ ] **Step 3: Update dashboard structure**

In `crates/features-courses/src/dashboard.rs`, change the root and header:

```rust
div { class: "dashboard page-stack motion-page",
    header { class: "page-header dashboard-hero",
        div {
            p { class: "page-kicker", "Command center" }
            h1 { "Welcome back, {props.display_name}" }
            p { class: "page-subtitle", "Courses, live sessions, assignments, and academy activity in one view." }
        }
    }
```

Keep the existing stats and course list behavior.

- [ ] **Step 4: Add course card art and refined empty state**

In `crates/features-courses/src/course_list.rs`, change the page root:

```rust
div { class: "course-list-page motion-page",
```

For courses without a cover asset, replace:

```rust
div { class: "course-card-cover-empty" }
```

with:

```rust
div { class: "course-card-cover-empty course-card-art" }
```

Change the create button label:

```rust
label: "New Course".to_string(),
```

- [ ] **Step 5: Add course detail hero class**

In `crates/features-courses/src/course_detail.rs`, change the root and header classes:

```rust
div { class: "course-detail motion-page",
```

```rust
header { class: "course-detail-header course-detail-hero",
```

- [ ] **Step 6: Add schedule page motion and agenda classes**

In `crates/features-courses/src/schedule_view.rs`, wrap the list:

```rust
div { class: "schedule-agenda motion-page",
    ul { class: "schedule-list",
        /* existing loop */
    }
}
```

Keep the existing empty state return for no sessions.

- [ ] **Step 7: Verify route smoke tests**

Run:

```powershell
cargo test -p shell-web --test dashboard_smoke
cargo test -p shell-web --test shell_routes_smoke
```

Expected: PASS.

- [ ] **Step 8: Commit dashboard/course/schedule polish**

Run:

```powershell
git add -- crates/features-courses/src/dashboard.rs crates/features-courses/src/course_list.rs crates/features-courses/src/course_detail.rs crates/features-courses/src/schedule_view.rs crates/shell-web/tests/dashboard_smoke.rs
git commit -m "style: polish dashboard courses and schedule"
```

Expected: commit succeeds.

## Task 8: Assignments, Invites, Redeem, And System States

**Files:**
- Modify: `crates/features-courses/src/assignment_list.rs`
- Modify: `crates/features-courses/src/assignment_detail.rs`
- Modify: `crates/features-courses/src/assignment_editor.rs`
- Modify: `crates/features-courses/src/submission_form.rs`
- Modify: `crates/features-courses/src/submissions_grading_table.rs`
- Modify: `crates/features-courses/src/redeem_code.rs`
- Modify: `crates/features-courses/src/accept_invite.rs`
- Test: `crates/features-courses/tests/assignments_ssr.rs`

- [ ] **Step 1: Extend assignment SSR tests**

In `assignment_list_renders`, add:

```rust
assert!(html.contains("assignment-shell") || html.contains("assignment-list"), "got: {html}");
```

In `assignment_detail_renders_loading`, change the assertion:

```rust
assert!(html.contains("system-state--loading"), "got: {html}");
```

- [ ] **Step 2: Run assignment SSR tests to verify current loading state fails**

Run:

```powershell
cargo test -p features-courses --test assignments_ssr assignment_detail_renders_loading -- --nocapture
```

Expected: FAIL because assignment detail currently renders raw `Loading...`.

- [ ] **Step 3: Polish assignment list states**

In `crates/features-courses/src/assignment_list.rs`, import:

```rust
use design_system::EmptyState;
```

Change the root:

```rust
div { class: "assignment-list assignment-shell motion-page",
```

Replace the match state arms:

```rust
Some(Ok(items)) if items.is_empty() => rsx! {
    EmptyState {
        title: "No assignments yet".to_string(),
        description: "Published work, drafts, and grading activity will appear here.".to_string(),
        cta: None,
    }
},
Some(Err(e)) => rsx! {
    div { class: "system-state system-state--error", "{e}" }
},
None => rsx! {
    div { class: "system-state system-state--loading",
        div { class: "skeleton-line" }
        div { class: "skeleton-line" }
    }
},
```

- [ ] **Step 4: Polish assignment detail states**

In `crates/features-courses/src/assignment_detail.rs`, change the root:

```rust
div { class: "assignment-detail assignment-shell motion-page",
```

Replace error and loading arms:

```rust
Some(Err(e)) => rsx! { div { class: "system-state system-state--error", "{e}" } },
None => rsx! {
    div { class: "system-state system-state--loading",
        div { class: "skeleton-line" }
        div { class: "skeleton-line" }
    }
},
```

- [ ] **Step 5: Add branded workflow classes to redeem and invite**

In `crates/features-courses/src/redeem_code.rs`, change the card body start:

```rust
Card {
    div { class: "workflow-heading",
        p { class: "page-kicker", "Enrollment" }
        h1 { "Redeem an enrollment code" }
        p { "Paste the code your teacher sent you." }
    }
```

In `crates/features-courses/src/accept_invite.rs`, change the root:

```rust
div { class: "accept-page workflow-page motion-page",
```

- [ ] **Step 6: Verify assignments and workflows**

Run:

```powershell
cargo test -p features-courses --test assignments_ssr
cargo test -p shell-web --test shell_routes_smoke
```

Expected: PASS.

- [ ] **Step 7: Commit system state polish**

Run:

```powershell
git add -- crates/features-courses/src/assignment_list.rs crates/features-courses/src/assignment_detail.rs crates/features-courses/src/assignment_editor.rs crates/features-courses/src/submission_form.rs crates/features-courses/src/submissions_grading_table.rs crates/features-courses/src/redeem_code.rs crates/features-courses/src/accept_invite.rs crates/features-courses/tests/assignments_ssr.rs
git commit -m "style: polish assignments and workflow states"
```

Expected: commit succeeds.

## Task 9: Live Room And Replay Polish

**Files:**
- Modify: `crates/features-courses/src/live_room_broadcast.rs`
- Modify: `crates/features-courses/src/live_room_view.rs`
- Modify: `crates/features-courses/src/live_room_lobby.rs`
- Modify: `crates/features-courses/src/live_room_replay.rs`
- Modify: `crates/features-courses/src/live_room_chat.rs`
- Modify: `crates/features-courses/src/live_room_presence.rs`
- Modify: `crates/features-courses/src/live_room_hand_raise.rs`
- Test: `crates/features-courses/tests/live_room_smoke.rs`

- [ ] **Step 1: Inspect current local edits**

Run:

```powershell
git diff -- crates/features-courses/src/live_room_broadcast.rs crates/features-courses/src/live_room_view.rs crates/features-courses/src/live_room_socket.rs crates/features-courses/src/live_room_whep.rs crates/features-courses/src/live_room_whip.rs
```

Expected: local live-room work is visible. Preserve these changes when adding classes.

- [ ] **Step 2: Extend live room smoke tests**

In `live_renders_video_tag_for_webrtc`, add:

```rust
assert!(html.contains("live-room-stage"), "got: {html}");
assert!(html.contains("live-room-sidebars"), "got: {html}");
```

In `teacher_scheduled_renders_broadcast_with_go_live`, add:

```rust
assert!(html.contains("live-room-broadcast"), "got: {html}");
assert!(html.contains("Broadcast"), "got: {html}");
```

- [ ] **Step 3: Run live room tests to verify current gap**

Run:

```powershell
cargo test -p features-courses --test live_room_smoke live_renders_video_tag_for_webrtc -- --nocapture
```

Expected: FAIL until `live-room-stage` is added.

- [ ] **Step 4: Add stage class to student live view**

In `crates/features-courses/src/live_room_view.rs`, change the video wrapper:

```rust
div { class: "live-room-video-area live-room-stage",
    {video_body}
}
```

Keep socket, WHIP, WHEP, banner, and state logic unchanged.

- [ ] **Step 5: Add broadcast and replay stage classes**

In `crates/features-courses/src/live_room_broadcast.rs`, make the main control/video region include:

```rust
class: "live-room-broadcast-controls live-room-stage"
```

In `crates/features-courses/src/live_room_replay.rs`, make the replay video pane include:

```rust
class: "replay-video-pane live-room-stage"
```

- [ ] **Step 6: Polish side-panel roots**

In chat, presence, and hand-raise components, keep existing behavior and ensure root classes remain:

```rust
class: "live-room-chat live-panel"
class: "live-room-presence live-panel"
class: "live-room-hand-raise live-panel"
```

If a file already has the first class, append `live-panel` rather than replacing the original class.

- [ ] **Step 7: Verify live room tests**

Run:

```powershell
cargo test -p features-courses --test live_room_smoke
```

Expected: PASS.

- [ ] **Step 8: Commit live room polish**

Run:

```powershell
git add -- crates/features-courses/src/live_room_broadcast.rs crates/features-courses/src/live_room_view.rs crates/features-courses/src/live_room_lobby.rs crates/features-courses/src/live_room_replay.rs crates/features-courses/src/live_room_chat.rs crates/features-courses/src/live_room_presence.rs crates/features-courses/src/live_room_hand_raise.rs crates/features-courses/tests/live_room_smoke.rs
git commit -m "style: polish live room experience"
```

Expected: commit succeeds and does not revert existing uncommitted live-room logic.

## Task 10: Backend Audit Artifact And Next SaaS Backlog

**Files:**
- Create: `docs/superpowers/specs/2026-05-15-aulalite-saas-expansion-backlog.md`
- Inspect: `crates/backend/src/handlers/*.rs`
- Inspect: `migrations/*.sql`
- Inspect: `crates/backend/tests/*.rs`

- [ ] **Step 1: Create backlog and audit document**

Create `docs/superpowers/specs/2026-05-15-aulalite-saas-expansion-backlog.md`:

```markdown
# AulaLite SaaS Expansion Backlog

**Date:** 2026-05-15
**Status:** Backlog created from SaaS readiness refresh

## Current Phase Fixes

- Schedule frontend links use `/schedule` instead of the backend API path `/v1/me/schedule`.
- `/v1/me/schedule?days=` accepts only values from 1 through 180.

## Backend Audit Checklist

| Area | Current-phase result | Next action |
| --- | --- | --- |
| Auth bootstrap and local login | Reviewed during implementation | Keep local/demo profile behavior documented in ops runbook |
| Tenant isolation | Existing tests cover core courses, RLS, assignments, submissions, and live room paths | Expand with admin console tests in SaaS expansion phase |
| Role permissions | Existing permission tests remain part of final verification | Add tenant admin UI permission matrix in SaaS expansion phase |
| API validation | Schedule days validation fixed in this phase | Normalize API error codes and user-facing messages in SaaS expansion phase |
| Live sessions and live room | Existing local edits preserved and smoke tests run | Add instructor operations dashboard in SaaS expansion phase |
| Migrations | Existing ordering preserved | Add migration linting/checksum workflow in SaaS expansion phase |

## Next SaaS Expansion Candidates

- Billing and subscription plan management.
- Tenant settings and tenant branding controls.
- Admin console for tenant, user, and role operations.
- Analytics and reporting for course activity, attendance, assignment progress, and live sessions.
- Onboarding and guided setup for new organizations.
- Notification settings and delivery.
- Support, help, and operational tooling.
- Deeper audit, compliance, export, and retention workflows.
```

- [ ] **Step 2: Run backend audit search commands**

Run:

```powershell
rg -n "unwrap\\(|expect\\(|todo!|unimplemented!|SELECT \\*|tenant_id|Forbidden|NotFound|BadRequest" crates\backend\src crates\backend\tests migrations
rg -n "caller_can_|tenant_role|is_platform_admin|course_id|user_id" crates\backend\src\handlers crates\backend\src\db crates\backend\tests
```

Expected: output is reviewed and concrete current-phase defects are either already covered by this plan or added as rows in the backlog document with a named next action.

- [ ] **Step 3: Run targeted backend tests**

Run:

```powershell
cargo test -p backend permissions_matrix
cargo test -p backend course_tenant_scope
cargo test -p backend rls_tenant_isolation
cargo test -p backend assignments_permissions
cargo test -p backend live_room
cargo test -p backend local_login_bypass
```

Expected: PASS, or failures are documented in the backlog with exact command and failure summary.

- [ ] **Step 4: Commit audit and backlog artifact**

Run:

```powershell
git add -- docs/superpowers/specs/2026-05-15-aulalite-saas-expansion-backlog.md
git commit -m "docs: record saas expansion backlog"
```

Expected: commit succeeds.

## Task 11: Final Verification And Browser Check

**Files:**
- Inspect: all files changed in this plan.
- Modify: no files unless verification exposes a concrete defect.

- [ ] **Step 1: Format and compile-check**

Run:

```powershell
cargo fmt --all --check
cargo check -p design-system
cargo check -p features-auth
cargo check -p features-courses
cargo check -p shell-web
```

Expected: PASS.

- [ ] **Step 2: Run frontend and SSR tests**

Run:

```powershell
cargo test -p design-system
cargo test -p features-auth
cargo test -p features-courses
cargo test -p shell-web
```

Expected: PASS.

- [ ] **Step 3: Run backend tests that do not require unavailable services**

Run:

```powershell
cargo test -p backend
```

Expected: PASS when Postgres and service dependencies from `docker compose up -d` are available. If the environment lacks a service, record the exact failing command and the missing service.

- [ ] **Step 4: Start local services for browser verification**

Run:

```powershell
docker compose up -d
curl.exe http://localhost:8080/healthz
```

Expected: backend health returns success.

- [ ] **Step 5: Start the web dev server**

Run from `crates/shell-web`:

```powershell
dx serve --platform web --port 3000
```

Expected: dev server starts at `http://localhost:3000`. Keep this process running until browser verification is complete.

- [ ] **Step 6: Verify browser routes manually**

Open `http://localhost:3000` and verify:

```text
/login
/
/courses
/schedule
/redeem
/courses/<existing-course-slug>
/courses/<existing-course-slug>/assignments
/courses/<existing-course-slug>/schedule
/courses/<existing-course-slug>/sessions/<existing-session-id>
```

Expected: each route renders styled content, the logo and generated imagery load, no text overlaps, and animations do not obscure controls. Use local demo profiles from the auth screen when enabled.

- [ ] **Step 7: Final status check**

Run:

```powershell
git status --short
git log --oneline -5
```

Expected: only intentional changes from this plan remain. Existing unrelated dirty files are not reverted.

- [ ] **Step 8: Final implementation summary**

Summarize:

```text
Changed files:
- ...

Generated assets:
- ...

Verification:
- command: result

Backend audit:
- fixed in this phase: ...
- next phase backlog: ...

Known environmental blockers:
- ...
```

Expected: final answer is concise and includes the local dev URL if the server is still running.
