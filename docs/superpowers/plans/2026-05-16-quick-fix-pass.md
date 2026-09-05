# Quick-Fix Pass Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship three small fixes batched together — replace broken `/dashboard` links, make module/lesson names renamable inline, and add a manual test runbook for live video sessions.

**Architecture:** Pure frontend work (Dioxus 0.7) plus one docs file. Backend already exposes the required `PATCH /v1/courses/:cid/modules/:mid` and `PATCH /v1/courses/:cid/modules/:mid/lessons/:lid` endpoints — we only add API-client wrappers and wire UI to call them. UI polish (shadcn/Next-quality look) is deferred to a separate brainstorm.

**Tech Stack:** Rust + Dioxus 0.7.4 (workspace under `crates/`), dioxus-router, Playwright (Node) for e2e, dioxus_ssr for unit tests.

**Spec:** `docs/superpowers/specs/2026-05-16-quick-fix-pass-design.md` (commit `4ee5f09`).

---

## File Structure

**Modify:**
- `crates/features-courses/src/app_shell.rs` — five literal `"/dashboard"` strings → `"/"`.
- `crates/features-courses/src/accept_invite.rs` — one literal `"/dashboard"` → `"/"`.
- `crates/features-courses/src/api.rs` — append `PatchModuleBody`, `patch_module`, `PatchLessonBody`, `patch_lesson` (alongside the existing `patch_course` block, ~line 545–616).
- `crates/features-courses/src/course_builder.rs` — replace read-only `h3 { "{m.title}" }` and lesson title `span` with a small in-file `RenamableTitle` component; extend `CourseBuilderProps` with `on_module_renamed` and `on_lesson_renamed` event handlers.
- `crates/features-courses/src/course_detail.rs` (lines 56–67) — render a small "Edit course settings" button next to the `h1` for admins; clicking it calls `on_tab_change("edit")`.
- `crates/shell-web/src/routes/course_detail.rs` — wire two new closures into `OutlineTab`'s `CourseBuilder` invocation that call `api::patch_module` / `api::patch_lesson` and restart the `outline` resource.
- `crates/design-system/assets/components.css` — append a few CSS rules so the renamable button visually matches the surrounding heading (no new design tokens).

**Create:**
- `docs/testing/live-session-manual-test.md` — the manual-test runbook.

**Tests touched:**
- `crates/shell-web/tests/shell_routes_smoke.rs` — add assertion that no rendered `href="/dashboard"` appears.
- `crates/features-courses/src/course_builder.rs` — extend the existing `#[cfg(test)] mod tests` block with SSR tests for the renamable display and the new props.
- `crates/features-courses/src/api.rs` — extend `mod tests` with body-serialisation tests for the new patch helpers.
- `crates/shell-web/src/routes/course_detail.rs` — extend the existing test mod with a render test for the Edit affordance on the course header.

---

## Task 1: Fix `/dashboard` → `/` routing

**Files:**
- Modify: `crates/features-courses/src/app_shell.rs:27,32,36,40,46`
- Modify: `crates/features-courses/src/accept_invite.rs:40`
- Test: `crates/shell-web/tests/shell_routes_smoke.rs` (extend)

- [ ] **Step 1: Add a failing regression test**

Append this to `crates/shell-web/tests/shell_routes_smoke.rs` (under the existing `#[test] fn dashboard_route_renders_for_signed_in_user` block):

```rust
#[test]
fn nav_links_do_not_point_at_unknown_dashboard_route() {
    // The router defines Dashboard at "/", not "/dashboard". If any nav
    // link ships pointing at "/dashboard", clicking it surfaces
    // "Failed to parse route".
    let mut dom = dom_for_path("/", true);
    let _ = dom.rebuild_in_place();
    let html = render(&dom);
    assert!(
        !html.contains("href=\"/dashboard\""),
        "found nav link to /dashboard, which is not a real route. html: {html}"
    );
}
```

- [ ] **Step 2: Run the test and confirm it fails**

```bash
cargo test -p shell-web --test shell_routes_smoke nav_links_do_not_point_at_unknown_dashboard_route
```

Expected: FAIL with `found nav link to /dashboard, which is not a real route` (because `app_shell.rs` still emits those links for signed-in users).

- [ ] **Step 3: Edit `crates/features-courses/src/app_shell.rs`**

Change the five `"/dashboard"` literals to `"/"` on lines 27, 32, 36, 40, 46. After the edit, lines 25–48 read:

```rust
    let menu = match role {
        Some(core_types::TenantRole::OrgAdmin) => vec![
            ("Dashboard", "/", UiIcon::Dashboard),
            ("My Courses", "/courses", UiIcon::Courses),
            ("All Tenant Courses", "/courses?scope=all", UiIcon::Settings),
        ],
        Some(core_types::TenantRole::Teacher) => vec![
            ("Dashboard", "/", UiIcon::Dashboard),
            ("My Courses", "/courses", UiIcon::Courses),
        ],
        Some(core_types::TenantRole::Ta) => vec![
            ("Dashboard", "/", UiIcon::Dashboard),
            ("My Courses", "/courses", UiIcon::Courses),
        ],
        Some(core_types::TenantRole::Student) | None => vec![
            ("Dashboard", "/", UiIcon::Dashboard),
            ("My Courses", "/courses", UiIcon::Courses),
            ("My Schedule", "/schedule", UiIcon::Schedule),
            ("Redeem Code", "/redeem", UiIcon::Redeem),
        ],
        Some(core_types::TenantRole::Parent) => {
            vec![("Dashboard", "/", UiIcon::Dashboard)]
        }
    };
```

- [ ] **Step 4: Edit `crates/features-courses/src/accept_invite.rs:40`**

Change:

```rust
a { href: "/dashboard", "Back to dashboard" }
```

to:

```rust
a { href: "/", "Back to dashboard" }
```

- [ ] **Step 5: Re-run the test and confirm it passes**

```bash
cargo test -p shell-web --test shell_routes_smoke nav_links_do_not_point_at_unknown_dashboard_route
```

Expected: PASS.

- [ ] **Step 6: Run the broader workspace tests to confirm no regression**

```bash
cargo test --workspace --no-fail-fast
```

Expected: all green. If a snapshot/SSR test in `features_courses` or `shell-web` was checking for the literal `/dashboard`, update it to `/` and re-run.

- [ ] **Step 7: Format and commit**

```bash
cargo fmt --all
git add crates/features-courses/src/app_shell.rs \
        crates/features-courses/src/accept_invite.rs \
        crates/shell-web/tests/shell_routes_smoke.rs
git commit -m "$(cat <<'EOF'
fix(routing): replace /dashboard nav links with / to match Route::Dashboard

The Dashboard variant is mounted at "/", so every nav link pointing at
"/dashboard" produced a route-mismatch crash. Updates the five sidebar
links per tenant role plus the accept-invite back link, and adds an SSR
smoke test that asserts no href="/dashboard" survives in the rendered
shell.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: Add `patch_module` and `patch_lesson` API helpers

**Files:**
- Modify: `crates/features-courses/src/api.rs` (append to the patch-bodies block ~line 540 and to the `#[cfg(test)] mod tests` block at the bottom)

- [ ] **Step 1: Add failing serialisation tests**

In `crates/features-courses/src/api.rs`, inside `mod tests` (currently at the end of the file, around line 855–883), append:

```rust
    #[test]
    fn patch_module_body_omits_unset_fields() {
        let omitted = PatchModuleBody { title: None };
        assert_eq!(
            serde_json::to_value(&omitted).unwrap(),
            serde_json::json!({})
        );

        let with_title = PatchModuleBody {
            title: Some("Renamed module"),
        };
        assert_eq!(
            serde_json::to_value(&with_title).unwrap(),
            serde_json::json!({ "title": "Renamed module" })
        );
    }

    #[test]
    fn patch_lesson_body_omits_unset_fields() {
        let omitted = PatchLessonBody { title: None };
        assert_eq!(
            serde_json::to_value(&omitted).unwrap(),
            serde_json::json!({})
        );

        let with_title = PatchLessonBody {
            title: Some("Renamed lesson"),
        };
        assert_eq!(
            serde_json::to_value(&with_title).unwrap(),
            serde_json::json!({ "title": "Renamed lesson" })
        );
    }
```

- [ ] **Step 2: Run the new tests and confirm they fail to compile**

```bash
cargo test -p features-courses --lib api::tests
```

Expected: FAIL with `cannot find type PatchModuleBody`/`PatchLessonBody`.

- [ ] **Step 3: Add the two body structs**

In `crates/features-courses/src/api.rs`, find the existing `PatchCourseBody` definition (around line 545) and append immediately after the existing `PatchAssignmentBody` (around line 581) the following two body structs:

```rust
#[derive(serde::Serialize)]
pub struct PatchModuleBody<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<&'a str>,
}

#[derive(serde::Serialize)]
pub struct PatchLessonBody<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<&'a str>,
}
```

- [ ] **Step 4: Add the two helper functions**

In the same file, after `pub async fn reorder_lessons(...)` (around line 700–713), append:

```rust
pub async fn patch_module(
    ctx: &ApiContext,
    course_id: &str,
    module_id: &str,
    body: &PatchModuleBody<'_>,
) -> Result<ModuleDto, ApiError> {
    fetch_json(
        ctx,
        "PATCH",
        &format!("/v1/courses/{course_id}/modules/{module_id}"),
        Some(body),
    )
    .await
}

pub async fn patch_lesson(
    ctx: &ApiContext,
    course_id: &str,
    module_id: &str,
    lesson_id: &str,
    body: &PatchLessonBody<'_>,
) -> Result<LessonSummaryDto, ApiError> {
    fetch_json(
        ctx,
        "PATCH",
        &format!("/v1/courses/{course_id}/modules/{module_id}/lessons/{lesson_id}"),
        Some(body),
    )
    .await
}
```

- [ ] **Step 5: Run the tests and confirm they pass**

```bash
cargo test -p features-courses --lib api::tests
```

Expected: PASS for both new tests plus the existing `patch_course_body_preserves_cover_asset_double_option_semantics`.

- [ ] **Step 6: Format and commit**

```bash
cargo fmt --all
git add crates/features-courses/src/api.rs
git commit -m "$(cat <<'EOF'
feat(api): add patch_module and patch_lesson client helpers

Backend already serves PATCH /v1/courses/:cid/modules/:mid and
PATCH /v1/courses/:cid/modules/:mid/lessons/:lid. These helpers wrap
them in the same style as patch_course so the course builder can wire
inline rename without bespoke fetch code.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: Add `RenamableTitle` helper to `course_builder.rs`

**Files:**
- Modify: `crates/features-courses/src/course_builder.rs` (add component, extend tests)

- [ ] **Step 1: Add the failing SSR test for the renamable display element**

Append to the existing `#[cfg(test)] mod tests` block in `crates/features-courses/src/course_builder.rs`:

```rust
    use super::RenamableTitle;
    use dioxus::prelude::*;

    #[test]
    fn renamable_title_renders_value_inside_clickable_button() {
        fn app() -> Element {
            rsx! {
                RenamableTitle {
                    value: "Week 1".to_string(),
                    on_commit: move |_: String| {},
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("Week 1"),
            "expected title text in render, got: {html}"
        );
        assert!(
            html.contains("renamable-title-display"),
            "expected display class in render, got: {html}"
        );
        assert!(
            !html.contains("renamable-title-input"),
            "input class should not render in the non-editing state, got: {html}"
        );
    }
```

- [ ] **Step 2: Run the test and confirm it fails to compile**

```bash
cargo test -p features-courses --lib course_builder::tests::renamable_title_renders_value_inside_clickable_button
```

Expected: FAIL with `cannot find RenamableTitle`.

- [ ] **Step 3: Implement `RenamableTitle` at the top of `course_builder.rs`**

In `crates/features-courses/src/course_builder.rs`, immediately after the existing `use design_system::{...}` / `use dioxus::prelude::*;` imports (around line 2–3), insert:

```rust
#[derive(Props, Clone, PartialEq)]
pub struct RenamableTitleProps {
    pub value: String,
    pub on_commit: EventHandler<String>,
}

/// Click-to-edit title. Renders a button by default; on click swaps to
/// a text input that commits on Enter / blur and cancels on Escape.
/// Empty / whitespace-only / unchanged values are treated as cancels.
#[component]
pub fn RenamableTitle(props: RenamableTitleProps) -> Element {
    let mut editing = use_signal(|| false);
    let mut draft = use_signal(|| props.value.clone());
    let original = props.value.clone();
    let on_commit = props.on_commit;

    if *editing.read() {
        let original_for_keys = original.clone();
        let original_for_blur = original.clone();
        rsx! {
            input {
                class: "ds-input renamable-title-input",
                r#type: "text",
                value: "{draft}",
                autofocus: true,
                oninput: move |e| draft.set(e.value()),
                onkeydown: move |e| {
                    let key = e.key().to_string();
                    if key == "Enter" {
                        e.prevent_default();
                        let new_title = draft.read().trim().to_string();
                        if !new_title.is_empty() && new_title != original_for_keys {
                            on_commit.call(new_title);
                        }
                        editing.set(false);
                    } else if key == "Escape" {
                        editing.set(false);
                    }
                },
                onblur: move |_| {
                    if !*editing.read() {
                        return;
                    }
                    let new_title = draft.read().trim().to_string();
                    if !new_title.is_empty() && new_title != original_for_blur {
                        on_commit.call(new_title);
                    }
                    editing.set(false);
                },
            }
        }
    } else {
        rsx! {
            button {
                class: "renamable-title-display",
                r#type: "button",
                onclick: move |_| {
                    draft.set(original.clone());
                    editing.set(true);
                },
                "{original}"
            }
        }
    }
}
```

(The commit logic is inlined in both `onkeydown` Enter and `onblur` so each closure owns its own captures. `EventHandler` is `Copy` in Dioxus 0.7, so `on_commit` can be captured directly without cloning. The `if !*editing.read() { return; }` guard in `onblur` prevents a double-commit when Enter has already flipped `editing` to false.)

- [ ] **Step 4: Run the test and confirm it passes**

```bash
cargo test -p features-courses --lib course_builder::tests::renamable_title_renders_value_inside_clickable_button
```

Expected: PASS.

- [ ] **Step 5: Add CSS for the renamable display**

Append to `crates/design-system/assets/components.css`:

```css
.renamable-title-display {
  display: inline-flex;
  align-items: center;
  padding: 2px 6px;
  margin: 0;
  background: transparent;
  border: 1px dashed transparent;
  border-radius: var(--radius-sm);
  color: inherit;
  font: inherit;
  cursor: text;
  text-align: left;
}

.renamable-title-display:hover {
  border-color: var(--color-rule-strong);
}

.renamable-title-input {
  max-width: 420px;
}
```

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add crates/features-courses/src/course_builder.rs \
        crates/design-system/assets/components.css
git commit -m "$(cat <<'EOF'
feat(builder): add RenamableTitle helper for inline rename

Small click-to-edit component used next by the course builder for
module and lesson titles. Commits on Enter or blur, cancels on Escape,
treats empty/unchanged values as cancels.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: Wire inline rename for modules in `CourseBuilder`

**Files:**
- Modify: `crates/features-courses/src/course_builder.rs` (extend props, replace `h3`)
- Modify: `crates/shell-web/src/routes/course_detail.rs:OutlineTab` (wire handler)

- [ ] **Step 1: Add a failing SSR test verifying the module title is rendered inside `RenamableTitle`**

Append to `crates/features-courses/src/course_builder.rs` test module:

```rust
    #[test]
    fn course_builder_renders_module_title_in_renamable_display() {
        fn app() -> Element {
            rsx! {
                CourseBuilder {
                    modules: vec![ModuleNode {
                        id: "m1".to_string(),
                        title: "Week 1".to_string(),
                        lessons: vec![],
                    }],
                    on_add_module: move |_| {},
                    on_add_lesson: move |_: String| {},
                    on_lesson_clicked: move |_: String| {},
                    on_modules_reordered: move |_: Vec<String>| {},
                    on_lessons_reordered: move |_: (String, Vec<String>)| {},
                    on_module_renamed: move |_: (String, String)| {},
                    on_lesson_renamed: move |_: (String, String, String)| {},
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("Week 1"), "missing module title: {html}");
        assert!(
            html.contains("renamable-title-display"),
            "module title should render through RenamableTitle: {html}"
        );
        // The old read-only h3 should be gone.
        assert!(
            !html.contains("<h3>Week 1</h3>"),
            "module title should not be a plain h3: {html}"
        );
    }
```

- [ ] **Step 2: Run, expect compile failure**

```bash
cargo test -p features-courses --lib course_builder::tests::course_builder_renders_module_title_in_renamable_display
```

Expected: FAIL with `missing field on_module_renamed` (or compile error on `CourseBuilderProps`).

- [ ] **Step 3: Extend `CourseBuilderProps`**

In `crates/features-courses/src/course_builder.rs`, replace the existing `CourseBuilderProps` definition (lines 19–27) with:

```rust
#[derive(Props, Clone, PartialEq)]
pub struct CourseBuilderProps {
    pub modules: Vec<ModuleNode>,
    pub on_add_module: EventHandler<()>,
    pub on_add_lesson: EventHandler<String>,
    pub on_lesson_clicked: EventHandler<String>,
    pub on_modules_reordered: EventHandler<Vec<String>>,
    pub on_lessons_reordered: EventHandler<(String, Vec<String>)>,
    /// (module_id, new_title)
    pub on_module_renamed: EventHandler<(String, String)>,
    /// (module_id, lesson_id, new_title)
    pub on_lesson_renamed: EventHandler<(String, String, String)>,
}
```

- [ ] **Step 4: Replace the module `h3` with `RenamableTitle`**

In `crates/features-courses/src/course_builder.rs`, the current module render (around line 97–98) is:

```rust
Card {
    h3 { "{m.title}" }
    ol { class: "builder-lessons",
```

Replace with:

```rust
Card {
    {
        let module_id_for_rename = m.id.clone();
        let on_module_renamed = props.on_module_renamed.clone();
        rsx! {
            h3 {
                class: "builder-module-title",
                RenamableTitle {
                    value: m.title.clone(),
                    on_commit: move |new_title: String| {
                        on_module_renamed.call((module_id_for_rename.clone(), new_title));
                    },
                }
            }
        }
    }
    ol { class: "builder-lessons",
```

(The wrapping `h3` is retained for visual hierarchy; the renamable button lives inside it.)

- [ ] **Step 5: Run the test and confirm it passes**

```bash
cargo test -p features-courses --lib course_builder::tests::course_builder_renders_module_title_in_renamable_display
```

Expected: PASS.

- [ ] **Step 6: Wire `on_module_renamed` in the OutlineTab route**

In `crates/shell-web/src/routes/course_detail.rs`, locate the `CourseBuilder { ... }` invocation inside `OutlineTab` (currently the `if can_edit { ... }` branch around line 313–382). Add the two new handlers; the snippet to insert before the closing `}` of the `CourseBuilder { ... }` block:

```rust
on_module_renamed: {
    let api = api.clone();
    let course_id = course_id.clone();
    move |(module_id, new_title): (String, String)| {
        let api = api.clone();
        let course_id = course_id.clone();
        let mut outline = outline;
        spawn(async move {
            let body = api::PatchModuleBody {
                title: Some(new_title.as_str()),
            };
            if api::patch_module(&api, &course_id, &module_id, &body)
                .await
                .is_ok()
            {
                outline.restart();
            }
        });
    }
},
on_lesson_renamed: {
    let api = api.clone();
    let course_id = course_id.clone();
    move |(module_id, lesson_id, new_title): (String, String, String)| {
        let api = api.clone();
        let course_id = course_id.clone();
        let mut outline = outline;
        spawn(async move {
            let body = api::PatchLessonBody {
                title: Some(new_title.as_str()),
            };
            if api::patch_lesson(&api, &course_id, &module_id, &lesson_id, &body)
                .await
                .is_ok()
            {
                outline.restart();
            }
        });
    }
},
```

- [ ] **Step 7: Run the workspace tests**

```bash
cargo test --workspace --no-fail-fast
```

Expected: all green. If any existing test was relying on `<h3>Week 1</h3>` literal HTML output (the read-only-outline test in `shell-web/src/routes/course_detail.rs` only checks for `"Week 1"` substring, so it should still pass), update its assertion.

- [ ] **Step 8: Commit**

```bash
cargo fmt --all
git add crates/features-courses/src/course_builder.rs \
        crates/shell-web/src/routes/course_detail.rs
git commit -m "$(cat <<'EOF'
feat(builder): inline rename for module titles

Module titles in the course builder are now click-to-edit through the
new RenamableTitle helper. OutlineTab wires the commit handler to
api::patch_module and restarts the outline resource on success so the
new title appears immediately.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: Wire inline rename for lesson titles

**Files:**
- Modify: `crates/features-courses/src/course_builder.rs` (replace lesson title span)

- [ ] **Step 1: Add a failing SSR test for lesson-title rename**

Append to the `course_builder.rs` test module:

```rust
    #[test]
    fn course_builder_renders_lesson_title_in_renamable_display() {
        fn app() -> Element {
            rsx! {
                CourseBuilder {
                    modules: vec![ModuleNode {
                        id: "m1".to_string(),
                        title: "Week 1".to_string(),
                        lessons: vec![LessonNode {
                            id: "l1".to_string(),
                            title: "Limits".to_string(),
                            r#type: "rich_text".to_string(),
                        }],
                    }],
                    on_add_module: move |_| {},
                    on_add_lesson: move |_: String| {},
                    on_lesson_clicked: move |_: String| {},
                    on_modules_reordered: move |_: Vec<String>| {},
                    on_lessons_reordered: move |_: (String, Vec<String>)| {},
                    on_module_renamed: move |_: (String, String)| {},
                    on_lesson_renamed: move |_: (String, String, String)| {},
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("Limits"), "missing lesson title: {html}");
        // Lesson title is now inside a RenamableTitle, which renders its
        // display element with the renamable-title-display class.
        let renamable_count = html.matches("renamable-title-display").count();
        assert!(
            renamable_count >= 2,
            "expected both module and lesson renamable displays, got {renamable_count} in: {html}"
        );
    }
```

- [ ] **Step 2: Run, expect failure (lesson title still uses plain span)**

```bash
cargo test -p features-courses --lib course_builder::tests::course_builder_renders_lesson_title_in_renamable_display
```

Expected: FAIL with `expected both module and lesson renamable displays, got 1`.

- [ ] **Step 3: Replace the lesson title `span` with `RenamableTitle`**

In `crates/features-courses/src/course_builder.rs`, the current lesson render (inside the `for l in &m.lessons` loop, around line 100–113) is:

```rust
li {
    key: "{l.id}",
    class: "builder-lesson",
    onclick: move |_| on_click.call(lesson_id_click.clone()),
    span { class: "type-pill", "{l.r#type}" }
    span { class: "lesson-title", "{l.title}" }
}
```

Replace with:

```rust
{
    let lesson_id_click = l.id.clone();
    let lesson_id_for_rename = l.id.clone();
    let module_id_for_lesson_rename = m.id.clone();
    let on_click = props.on_lesson_clicked.clone();
    let on_lesson_renamed = props.on_lesson_renamed.clone();
    let lesson_title = l.title.clone();
    let lesson_type = l.r#type.clone();
    rsx! {
        li {
            key: "{l.id}",
            class: "builder-lesson",
            onclick: move |_| on_click.call(lesson_id_click.clone()),
            span { class: "type-pill", "{lesson_type}" }
            span {
                class: "lesson-title",
                onclick: move |evt: dioxus::events::MouseEvent| evt.stop_propagation(),
                RenamableTitle {
                    value: lesson_title.clone(),
                    on_commit: move |new_title: String| {
                        on_lesson_renamed.call((
                            module_id_for_lesson_rename.clone(),
                            lesson_id_for_rename.clone(),
                            new_title,
                        ));
                    },
                }
            }
        }
    }
}
```

(The outer `li` keeps its existing `onclick → on_lesson_clicked` so clicking blank space in the row still navigates as before. The inner `span.lesson-title` calls `stop_propagation` so clicking the renamable title triggers edit only — no navigation. This is the minimal change that preserves the old nav UX while adding rename.)

- [ ] **Step 4: (no CSS needed — the existing `.builder-lesson` styling continues to apply)**

Skip this step. The lesson row keeps its original layout; the only behavioural change is that clicking the title now stops propagation.

- [ ] **Step 5: Run the test and confirm it passes**

```bash
cargo test -p features-courses --lib course_builder::tests::course_builder_renders_lesson_title_in_renamable_display
```

Expected: PASS.

- [ ] **Step 6: Run the workspace tests**

```bash
cargo test --workspace --no-fail-fast
```

Expected: all green.

- [ ] **Step 7: Commit**

```bash
cargo fmt --all
git add crates/features-courses/src/course_builder.rs
git commit -m "$(cat <<'EOF'
feat(builder): inline rename for lesson titles

Lesson titles in the builder render through RenamableTitle. The
title's click handler stops propagation so renaming does not navigate
away; clicking elsewhere in the row still triggers the existing
navigate-to-lesson behaviour.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: Add "Edit course settings" affordance next to the course title

**Files:**
- Modify: `crates/features-courses/src/course_detail.rs` (lines 56–67)

- [ ] **Step 1: Add a failing SSR test**

Append to the existing tests in `crates/shell-web/src/routes/course_detail.rs` (the `#[cfg(test)] mod tests` block at the bottom):

```rust
    #[test]
    fn course_detail_header_shows_edit_affordance_for_admin() {
        fn app() -> Element {
            rsx! {
                features_courses::course_detail::CourseDetail {
                    course_title: "Math".to_string(),
                    course_status: "draft".to_string(),
                    course_cover_asset_id: None::<String>,
                    can_admin: true,
                    active_tab: "outline".to_string(),
                    on_tab_change: move |_: String| {},
                    div { "body" }
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        let _ = vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("course-title-edit"),
            "expected edit affordance class for admin: {html}"
        );
    }

    #[test]
    fn course_detail_header_hides_edit_affordance_for_non_admin() {
        fn app() -> Element {
            rsx! {
                features_courses::course_detail::CourseDetail {
                    course_title: "Math".to_string(),
                    course_status: "draft".to_string(),
                    course_cover_asset_id: None::<String>,
                    can_admin: false,
                    active_tab: "outline".to_string(),
                    on_tab_change: move |_: String| {},
                    div { "body" }
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        let _ = vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            !html.contains("course-title-edit"),
            "non-admin should not see edit affordance: {html}"
        );
    }
```

- [ ] **Step 2: Run the tests and confirm they fail**

```bash
cargo test -p shell-web --lib course_detail::tests::course_detail_header_shows_edit_affordance_for_admin \
    course_detail::tests::course_detail_header_hides_edit_affordance_for_non_admin
```

Expected: FAIL (`expected edit affordance class for admin`).

- [ ] **Step 3: Edit `course_detail.rs` to render the affordance**

In `crates/features-courses/src/course_detail.rs`, replace the existing header block (lines 56–67):

```rust
header { class: "course-detail-header course-detail-hero",
    h1 { "{props.course_title}" }
    Badge {
        label: props.course_status.clone(),
        tone: match props.course_status.as_str() {
            "draft" => BadgeTone::Neutral,
            "published" => BadgeTone::Success,
            "archived" => BadgeTone::Warning,
            _ => BadgeTone::Neutral,
        },
    }
}
```

with:

```rust
header { class: "course-detail-header course-detail-hero",
    div { class: "course-title-row",
        h1 { "{props.course_title}" }
        if props.can_admin {
            {
                let on_change = props.on_tab_change.clone();
                rsx! {
                    button {
                        class: "linkish course-title-edit",
                        r#type: "button",
                        title: "Edit course settings",
                        onclick: move |_| on_change.call("edit".to_string()),
                        "Edit"
                    }
                }
            }
        }
    }
    Badge {
        label: props.course_status.clone(),
        tone: match props.course_status.as_str() {
            "draft" => BadgeTone::Neutral,
            "published" => BadgeTone::Success,
            "archived" => BadgeTone::Warning,
            _ => BadgeTone::Neutral,
        },
    }
}
```

- [ ] **Step 4: Add minimal CSS**

Append to `crates/design-system/assets/components.css`:

```css
.course-title-row {
  display: flex;
  align-items: baseline;
  gap: var(--space-3);
}

.course-title-edit {
  font-size: 14px;
}
```

- [ ] **Step 5: Run the tests and confirm they pass**

```bash
cargo test -p shell-web --lib course_detail::tests
```

Expected: PASS (both new tests plus the existing ones).

- [ ] **Step 6: Run the workspace tests**

```bash
cargo test --workspace --no-fail-fast
```

Expected: all green.

- [ ] **Step 7: Commit**

```bash
cargo fmt --all
git add crates/features-courses/src/course_detail.rs \
        crates/design-system/assets/components.css \
        crates/shell-web/src/routes/course_detail.rs
git commit -m "$(cat <<'EOF'
feat(course-detail): surface 'Edit' affordance next to the course title

Course admins now see a small Edit button next to the course title in
the detail header that jumps to the existing Settings tab. Improves
discoverability of the existing rename UI without adding a new edit
surface.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 7: Write the live-session manual test runbook

**Files:**
- Create: `docs/testing/live-session-manual-test.md`

- [ ] **Step 1: Create the runbook**

Write the following content to `docs/testing/live-session-manual-test.md`:

```markdown
# Live Session — Manual Test Runbook

This runbook walks through verifying the live-session (video-call) feature end to end. It is intended for someone who has never run the feature before. Expect 15–20 minutes.

## Prerequisites

- The backend, frontend, and MediaMTX are running. Default local URLs:
  - Frontend: <http://127.0.0.1:3000>
  - Backend:  <http://127.0.0.1:8080>
- You can either bring up the stack with `docker compose up -d` (the recipe in `docker-compose.yml`) or run each crate by hand (`cargo run -p backend`, `dx serve crates/shell-web`).
- Two browser windows: a normal window for the **Teacher** and an incognito/private window for the **Student** (so the two sessions hold different ID tokens).
- Both browsers must have **microphone and camera permissions** granted for the frontend origin.
- Two test accounts:
  - Teacher: `local.teacher@example.test` / `local-teacher-pass`
  - Student: `local.student@example.test` / `local-student-pass`
  - These come from `tools/aulalite-admin` seeding (or `/v1/dev/audit-seed` if you use the Playwright seed helper).

## Step 1 — Sign in

1. In the Teacher window, go to `/login` and sign in with the teacher account. You should land on `/` (Dashboard) with `Welcome back, …` and a `dashboard-hero` block.
2. In the Student window, do the same with the student account.

**Expected:** Both windows show the dashboard. No console errors about missing routes or failed token refresh.

## Step 2 — Schedule a session (Teacher)

1. Teacher: go to `/courses` and click into a course you own.
2. Click the **Schedule** tab. Use the series scheduler to create a **one-off** session starting in ~2 minutes. Pick a duration of 15 minutes and leave recording enabled.
3. After the create succeeds, the new session should appear in the Schedule tab list and in `/schedule` on the same window.

**Expected:** Session row appears in both views with title, start time, and duration. No 401s or 500s in the network panel.

## Step 3 — Student lands in the lobby

1. Student: go to `/schedule`.
2. Click the upcoming session row for the course they're enrolled in.

**Expected:** Lobby view renders. You see the course title (`live_room_lobby.rs:18`), the scheduled time, and a **Join** button that becomes enabled within the join window (~5 min before start).

## Step 4 — Teacher goes live

1. Teacher: navigate to the session URL (`/courses/:slug/sessions/:session_id`).
2. Click **Go Live** in the broadcast surface. Approve microphone + camera prompts.

**Expected:**
- Teacher sees their own camera preview, a `● LIVE` pill, and broadcast controls.
- Student lobby auto-promotes to the live room and the student starts receiving media.

## Step 5 — In-session interactions

Run each of these and confirm the result:

1. **Chat round trip** — Teacher types in chat; Student sees it. Student replies; Teacher sees it.
2. **Presence** — both names appear in the Participants panel (`live_room_presence.rs:30`).
3. **Hand-raise** — Student clicks Raise Hand. Teacher's `Hand-raise queue` (`live_room_hand_raise.rs:28`) shows the request.
4. **Grant speaking** — Teacher grants the request. Student receives WHIP credentials and can start their mic; their audio reaches the Teacher.
5. **Screenshare** — Teacher shares a screen. Student sees the screen share. Teacher revokes.
6. **Revoke hand-raise** — Teacher revokes student's speaking permission. Student returns to listen-only.

## Step 6 — End the session, verify replay

1. Teacher clicks **End session**. Both windows transition to the post-session view.
2. After ~30–60 seconds (recording transcode), the session in the Schedule list should show a Replay link.
3. Click Replay (Teacher or Student). The replay player should load and play the recorded stream.

## Common failures and how to read them

| Symptom | Likely cause |
|--|--|
| Camera/mic prompt never appears | Browser denied permissions for the origin. Reset site permissions and retry. |
| Lobby never promotes to live | Clock skew between server and client > 30s, or the broadcaster never published. Check the backend `/v1/sessions/:id` status — it should flip to `live` once WHIP is up. |
| `[ws] auth …` lines flood the console during sign-in | Expected pre-auth noise; the Playwright console gate filters it (see commit `58255ef`). Not a failure. |
| Replay never appears | The MediaMTX recording → S3 → transcode pipeline did not complete. Check backend logs for `recording_ingest` warnings and S3 credentials. |
| `Failed to parse route` on any link | A stale `/dashboard` literal slipped past Task 1's regression. Re-run `cargo test -p shell-web nav_links_do_not_point_at_unknown_dashboard_route`. |

## What "passes" looks like

A successful run produces:
- One scheduled session created by the Teacher
- One student joined and promoted to live
- At least one chat message in each direction
- One granted-then-revoked hand-raise
- One screenshare
- One replay viewable after the session ends
```

- [ ] **Step 2: Commit**

```bash
git add docs/testing/live-session-manual-test.md
git commit -m "$(cat <<'EOF'
docs(testing): manual test runbook for live sessions

Step-by-step playbook for verifying scheduling, lobby, going live,
in-session interactions (chat, presence, hand-raise, screenshare), and
replay. Written so a first-time tester can follow it without prior
context.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 8: Final verification

- [ ] **Step 1: Run the full workspace test suite**

```bash
cargo test --workspace --no-fail-fast
```

Expected: all green.

- [ ] **Step 2: Format check**

```bash
cargo fmt --all -- --check
```

Expected: no diff.

- [ ] **Step 3: Build the web shell to catch wasm-only compile errors**

```bash
cargo build -p shell-web --target wasm32-unknown-unknown
```

Expected: builds cleanly. (If the wasm target isn't installed: `rustup target add wasm32-unknown-unknown`.)

- [ ] **Step 4: Smoke-test by hand**

1. Run `dx serve crates/shell-web` (or the equivalent for the local setup) and sign in.
2. Click "Dashboard" in the sidebar — no route-mismatch toast.
3. Open a course outline as a course admin. Click a module title — it becomes editable. Type a new name and press Enter — the title updates after the outline refreshes. Repeat for a lesson title.
4. From the course detail header, click the new "Edit" button — you land on the Settings tab.
5. Skim `docs/testing/live-session-manual-test.md` and confirm it reads cleanly.

- [ ] **Step 5: Tag the branch state (optional)**

If working in a worktree, no tag is needed — the seven commits speak for themselves. If working on `main`, leave the commits standalone for the user to push.

---

## Spec coverage check

| Spec requirement | Task |
|--|--|
| `/dashboard` → `/` in app_shell.rs (5 sites) | Task 1 |
| `/dashboard` → `/` in accept_invite.rs (1 site) | Task 1 |
| Playwright/regression test for routing | Task 1 step 1 (SSR-level smoke; covers the same surface without needing a running stack) |
| `api::patch_module` helper | Task 2 |
| `api::patch_lesson` helper | Task 2 |
| Module title click-to-edit | Tasks 3 + 4 |
| Lesson title click-to-edit | Tasks 3 + 5 |
| Wire handlers in OutlineTab | Task 4 step 6 |
| Course-title "Edit" affordance | Task 6 |
| Manual test runbook | Task 7 |
| Audit-and-list sweep for other rename gaps | Performed during this plan-writing; no in-scope gaps found beyond modules/lessons. Likely follow-ups (session-row rename in `schedule_view.rs`, assignment rename in `assignment_editor.rs`) are noted here, **not** addressed in this pass. |

## Out of scope (do not implement here)

- Visual polish — buttons, spacing, hover states beyond minimal CSS hooks.
- Optimistic UI for rename (relies on `outline.restart()` instead).
- Renaming sessions from `/schedule`.
- Renaming assignments outside the assignment editor.
- Backend rate-limiting on rename endpoints.
