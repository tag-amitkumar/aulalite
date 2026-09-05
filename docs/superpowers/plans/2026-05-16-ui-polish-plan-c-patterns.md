# UI Polish — Plan C: Patterns Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build composite patterns that surfaces (Plan D) will assemble — Form Field wrapper, List patterns (Table + CardList), refined Empty State, Skeleton primitives, Toast (context-Signal driven, replaces inline JS), Page Header pattern, Loading composite. Built on Plan A tokens and Plan B primitives. Backward-compatible — existing call sites keep working.

**Architecture:** New composite files in `crates/design-system/src/` next to the primitives. Pattern CSS layered on top of the primitive CSS in `components.css`. Toast uses a `Signal<Vec<ToastEntry>>` provided via `use_context` from the app root; replaces `window.showDXToast` calls and the inline JS toast in `crates/shell-web/index.html`.

**Tech Stack:** Rust + Dioxus 0.7, CSS custom properties, SSR tests, `use_context` for cross-cutting state.

**Vision spec:** `docs/superpowers/specs/2026-05-16-ui-polish-vision-design.md` (commit `5bf5c27`).
**Foundation plan:** `docs/superpowers/plans/2026-05-16-ui-polish-plan-a-foundation.md` (executed — provides token vocabulary).
**Primitives plan:** `docs/superpowers/plans/2026-05-16-ui-polish-plan-b-primitives.md` (executed — provides component vocabulary).

---

## File Structure

**Modify:**
- `crates/design-system/src/empty_state.rs` — refine existing component (add illustration slot + variants).
- `crates/design-system/src/lib.rs` — export new composites.
- `crates/design-system/assets/components.css` — pattern CSS appended; existing `.empty-state` polished.
- `crates/shell-web/public/assets/components.css` — mirror.
- `crates/shell-web/index.html` — remove inline JS toast block (kept as bridge during transition; full removal in Task 5).

**Create:**
- `crates/design-system/src/field.rs` — Form Field wrapper.
- `crates/design-system/src/table.rs` — Table pattern.
- `crates/design-system/src/card_list.rs` — Card grid pattern.
- `crates/design-system/src/skeleton.rs` — Skeleton primitives (Line/Circle/Card/TableRow).
- `crates/design-system/src/toast.rs` — Toast component + context Signal.
- `crates/design-system/src/page_header.rs` — Page Header pattern.
- `crates/design-system/src/loading.rs` — Loading composite (Spinner + message).

**Test conventions:** same as Plan B — SSR tests in each module's `mod tests` block.

---

## Conventions (referenced by every task)

### Token-only CSS

All values from Plan A tokens. Exceptions: `#fff`, `rgba(...)` for translucent overlays, intrinsic geometry.

### Mirror discipline

After every CSS edit:
```bash
cp crates/design-system/assets/components.css crates/shell-web/public/assets/components.css
diff crates/design-system/assets/components.css crates/shell-web/public/assets/components.css
```

### Backward compat

Existing call sites must continue to compile. New components are additive. Where an existing component (EmptyState) is refined, all current props remain working.

---

## Task 1: Form Field wrapper

**Files:**
- Create: `crates/design-system/src/field.rs`
- Modify: `crates/design-system/src/lib.rs`
- Modify: `crates/design-system/assets/components.css` (+ mirror)

### API

Slot-based design: `Field { label, helper, error, children }` wraps any input element.

```rust
use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct FieldProps {
    pub children: Element,
    pub label: String,
    #[props(default)]
    pub helper: Option<String>,
    #[props(default)]
    pub error: Option<String>,
    /// HTML id of the wrapped input — used by the <label> for accessibility.
    #[props(default)]
    pub for_id: Option<String>,
    /// When true, the helper/error region renders even if empty (preserves vertical rhythm).
    #[props(default)]
    pub reserve_helper_space: bool,
}

#[component]
pub fn Field(props: FieldProps) -> Element {
    let helper_or_error = props.error.clone().or_else(|| props.helper.clone());
    let show_error = props.error.is_some();
    let helper_class = if show_error { "ds-field-helper ds-field-helper--error" } else { "ds-field-helper" };

    rsx! {
        div { class: "ds-field",
            label {
                class: "ds-field-label",
                r#for: props.for_id.clone().unwrap_or_default(),
                "{props.label}"
            }
            {props.children}
            if let Some(text) = &helper_or_error {
                p { class: "{helper_class}", "{text}" }
            } else if props.reserve_helper_space {
                p { class: "ds-field-helper", "" }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_renders_label_and_helper() {
        fn app() -> Element {
            rsx! {
                Field {
                    label: "Email".to_string(),
                    helper: "We'll never share your email".to_string(),
                    input { r#type: "email", value: "" }
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("Email"));
        assert!(html.contains("We'll never share your email"));
        assert!(html.contains("ds-field-label"));
        assert!(html.contains("ds-field-helper"));
    }

    #[test]
    fn field_renders_error_overrides_helper() {
        fn app() -> Element {
            rsx! {
                Field {
                    label: "Email".to_string(),
                    helper: "We'll never share your email".to_string(),
                    error: "Email is required".to_string(),
                    input { r#type: "email", value: "" }
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("Email is required"));
        assert!(html.contains("ds-field-helper--error"));
        // Helper text is suppressed when an error is present.
        assert!(!html.contains("We'll never share your email"));
    }
}
```

### CSS (append to components.css)

```css
.ds-field {
  display: grid;
  gap: var(--space-2);
  margin-bottom: var(--space-4);
}

.ds-field-label {
  color: var(--color-ink);
  font-size: var(--text-sm);
  line-height: var(--leading-sm);
  font-weight: 800;
}

.ds-field-helper {
  margin: 0;
  color: var(--color-text-muted);
  font-size: var(--text-xs);
  line-height: var(--leading-xs);
}

.ds-field-helper--error {
  color: var(--color-danger-700);
}
```

### `lib.rs`

```rust
pub mod field;
pub use field::{Field, FieldProps};
```

### Tests

The two SSR tests above. Add a third for `for_id` if straightforward.

### Commit

```
feat(field): Form Field wrapper with label + helper + error slots
```

---

## Task 2: List patterns (Table + CardList)

**Files:**
- Create: `crates/design-system/src/table.rs`
- Create: `crates/design-system/src/card_list.rs`
- Modify: `crates/design-system/src/lib.rs`
- Modify: `crates/design-system/assets/components.css` (+ mirror)

### Table API

```rust
#[derive(Props, Clone, PartialEq)]
pub struct TableProps {
    /// The <thead> row. Caller provides <th> children.
    pub head: Element,
    /// The <tbody> rows. Caller provides <tr><td>... children.
    pub body: Element,
    /// Density: applies the `ds-table--compact` modifier when true.
    #[props(default)]
    pub compact: bool,
    /// Stick the header to the top of the scroll container.
    #[props(default)]
    pub sticky_header: bool,
}

#[component]
pub fn Table(props: TableProps) -> Element {
    let mut class = String::from("ds-table");
    if props.compact { class.push_str(" ds-table--compact"); }
    if props.sticky_header { class.push_str(" ds-table--sticky-head"); }
    rsx! {
        table { class: "{class}",
            thead { {props.head} }
            tbody { {props.body} }
        }
    }
}
```

### CardList API

```rust
#[derive(Props, Clone, PartialEq)]
pub struct CardListProps {
    pub children: Element,
    /// Number of columns at default viewport. Defaults to auto-fit / 260px min.
    #[props(default)]
    pub columns: Option<u8>,
}

#[component]
pub fn CardList(props: CardListProps) -> Element {
    let style = if let Some(cols) = props.columns {
        format!("--card-list-columns: {};", cols)
    } else {
        String::new()
    };
    rsx! {
        ul { class: "ds-card-list",
            style: "{style}",
            {props.children}
        }
    }
}
```

### CSS

```css
/* Table */
.ds-table {
  width: 100%;
  border-collapse: collapse;
  background: var(--color-surface);
  border: 1px solid var(--color-rule);
  border-radius: var(--radius-md);
  overflow: hidden;
  font-size: var(--text-sm);
}

.ds-table thead th {
  text-align: left;
  padding: var(--space-3);
  background: var(--color-neutral-100);
  color: var(--color-text-muted);
  font-size: var(--text-xs);
  text-transform: uppercase;
  letter-spacing: 0.04em;
  font-weight: 800;
  border-bottom: 1px solid var(--color-rule);
}

.ds-table tbody td {
  padding: var(--space-3);
  border-bottom: 1px solid var(--color-rule);
  color: var(--color-text);
}

.ds-table tbody tr:hover {
  background: var(--color-neutral-50);
}

.ds-table--compact thead th,
.ds-table--compact tbody td { padding: var(--space-2); }

.ds-table--sticky-head thead th {
  position: sticky;
  top: 0;
  z-index: 1;
}

/* Card list */
.ds-card-list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: grid;
  gap: var(--space-5);
  grid-template-columns: repeat(
    var(--card-list-columns, auto-fit),
    minmax(260px, 1fr)
  );
}
```

### `lib.rs`

```rust
pub mod table;
pub mod card_list;
pub use table::{Table, TableProps};
pub use card_list::{CardList, CardListProps};
```

### Tests

For Table:
- Default render produces `<table class="ds-table">` with provided head/body slots.
- `compact: true` adds `--compact` class.
- `sticky_header: true` adds `--sticky-head` class.

For CardList:
- Default render produces `<ul class="ds-card-list">`.
- `columns: 3` sets the CSS custom property.

### Commit

```
feat(list): Table and CardList composite patterns
```

---

## Task 3: Empty State refinement

**Files:**
- Modify: `crates/design-system/src/empty_state.rs`
- Modify: `crates/design-system/assets/components.css` (+ mirror)

### API additions

Existing `EmptyState` props (likely `title`, `description`, `cta`). Add:
- `illustration: Option<Element>` — optional decorative element above the title.
- `variant: EmptyStateVariant` — Default | Subtle | Accent (gold-tinted).

```rust
#[derive(Clone, PartialEq, Default)]
pub enum EmptyStateVariant {
    #[default]
    Default,
    Subtle,
    Accent,
}
```

### CSS refinement

Replace the existing `.empty-state` block with:

```css
.empty-state {
  display: grid;
  gap: var(--space-3);
  padding: var(--space-7);
  background: var(--color-surface);
  border: 1px dashed var(--color-rule-strong);
  border-radius: var(--radius-lg);
  text-align: center;
  align-items: center;
  justify-items: center;
}

.empty-state--subtle {
  background: var(--color-neutral-50);
  border-style: solid;
  border-color: var(--color-rule);
}

.empty-state--accent {
  background: linear-gradient(135deg, var(--color-gold-50), var(--color-surface));
  border-color: var(--color-gold-200);
}

.empty-state-illustration {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  margin-bottom: var(--space-3);
  color: var(--color-text-muted);
}

.empty-state-title {
  margin: 0;
  font-family: var(--font-display);
  font-size: var(--text-2xl);
  line-height: var(--leading-2xl);
  color: var(--color-ink);
}

.empty-state-desc {
  margin: 0;
  max-width: 420px;
  color: var(--color-text-muted);
  font-size: var(--text-base);
  line-height: var(--leading-base);
}
```

### Tests

- `empty_state_renders_title_and_description`.
- `empty_state_accent_variant_renders_class`.
- `empty_state_renders_illustration_when_provided`.

### Commit

```
feat(empty-state): illustration slot + variant tints, refined typography
```

---

## Task 4: Skeleton primitives

**Files:**
- Create: `crates/design-system/src/skeleton.rs`
- Modify: `crates/design-system/src/lib.rs`
- Modify: `crates/design-system/assets/components.css` (+ mirror)

### API

```rust
#[derive(Props, Clone, PartialEq)]
pub struct SkeletonLineProps {
    /// Width as a CSS length (e.g. "100%", "12em"). Defaults to "100%".
    #[props(default = "100%".to_string())]
    pub width: String,
    /// Height as a CSS length. Defaults to "14px".
    #[props(default = "14px".to_string())]
    pub height: String,
}

#[component]
pub fn SkeletonLine(props: SkeletonLineProps) -> Element {
    rsx! {
        span {
            class: "ds-skeleton ds-skeleton-line",
            style: "width: {props.width}; height: {props.height};",
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct SkeletonCircleProps {
    #[props(default = "32px".to_string())]
    pub size: String,
}

#[component]
pub fn SkeletonCircle(props: SkeletonCircleProps) -> Element {
    rsx! {
        span {
            class: "ds-skeleton ds-skeleton-circle",
            style: "width: {props.size}; height: {props.size};",
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct SkeletonCardProps {
    /// Height as a CSS length. Defaults to "180px".
    #[props(default = "180px".to_string())]
    pub height: String,
}

#[component]
pub fn SkeletonCard(props: SkeletonCardProps) -> Element {
    rsx! {
        div {
            class: "ds-skeleton ds-skeleton-card",
            style: "height: {props.height};",
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct SkeletonTableRowProps {
    /// Number of cells to render. Defaults to 4.
    #[props(default = 4)]
    pub cells: u8,
}

#[component]
pub fn SkeletonTableRow(props: SkeletonTableRowProps) -> Element {
    let cells: Vec<u8> = (0..props.cells).collect();
    rsx! {
        tr { class: "ds-skeleton-table-row",
            for _ in cells {
                td { SkeletonLine { width: "80%".to_string() } }
            }
        }
    }
}
```

### CSS

```css
.ds-skeleton {
  display: inline-block;
  position: relative;
  overflow: hidden;
  background: var(--color-neutral-200);
  border-radius: var(--radius-sm);
}

.ds-skeleton::after {
  content: "";
  position: absolute;
  inset: 0;
  background: linear-gradient(
    90deg,
    transparent,
    rgba(255, 255, 255, 0.5),
    transparent
  );
  animation: ds-skeleton-shimmer 1.4s var(--ease-page-in) infinite;
}

.ds-skeleton-line { display: block; }
.ds-skeleton-circle { border-radius: var(--radius-full); }
.ds-skeleton-card { display: block; width: 100%; border-radius: var(--radius-lg); }

.ds-skeleton-table-row td {
  padding: var(--space-3);
}

@keyframes ds-skeleton-shimmer {
  0% { transform: translateX(-100%); }
  100% { transform: translateX(100%); }
}
```

### `lib.rs`

```rust
pub mod skeleton;
pub use skeleton::{SkeletonLine, SkeletonCircle, SkeletonCard, SkeletonTableRow};
```

### Tests

- `skeleton_line_renders_with_class`.
- `skeleton_circle_renders_with_class`.
- `skeleton_table_row_renders_n_cells`.

### Commit

```
feat(skeleton): introduce Line/Circle/Card/TableRow skeleton primitives
```

---

## Task 5: Toast component + context Signal

**Files:**
- Create: `crates/design-system/src/toast.rs`
- Modify: `crates/design-system/src/lib.rs`
- Modify: `crates/design-system/assets/components.css` (+ mirror)
- Modify: `crates/shell-web/index.html` — remove the inline `<script>` toast block + its `<style>` + `<div>` template at the END of this task (Step 8).

### Architecture

A `Signal<Vec<ToastEntry>>` lives in a context provider near the app root. Callers acquire the signal via `use_context::<Signal<ToastQueue>>()` and push entries. A `ToastViewport` component subscribes to the signal and renders the toast stack.

### API

```rust
use dioxus::prelude::*;

#[derive(Clone, PartialEq)]
pub enum ToastLevel {
    Info,
    Success,
    Warning,
    Danger,
}

#[derive(Clone, PartialEq)]
pub struct ToastEntry {
    pub id: u64,
    pub level: ToastLevel,
    pub title: String,
    pub message: String,
    /// Auto-dismiss timeout in ms. None = sticky (must be dismissed explicitly).
    pub duration_ms: Option<u32>,
}

pub type ToastQueue = Vec<ToastEntry>;

/// Convenience: returns a `ToastSender` that callers can use to push entries.
pub fn use_toast_sender() -> ToastSender {
    use_context::<Signal<ToastQueue>>().into()
}

#[derive(Clone)]
pub struct ToastSender(pub Signal<ToastQueue>);

impl From<Signal<ToastQueue>> for ToastSender {
    fn from(s: Signal<ToastQueue>) -> Self { Self(s) }
}

impl ToastSender {
    pub fn push(&mut self, level: ToastLevel, title: impl Into<String>, message: impl Into<String>) {
        let id = next_toast_id();
        self.0.write().push(ToastEntry {
            id,
            level,
            title: title.into(),
            message: message.into(),
            duration_ms: Some(4000),
        });
    }
    pub fn dismiss(&mut self, id: u64) {
        self.0.write().retain(|e| e.id != id);
    }
}

fn next_toast_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    N.fetch_add(1, Ordering::Relaxed)
}

#[component]
pub fn ToastViewport() -> Element {
    let queue = use_context::<Signal<ToastQueue>>();
    rsx! {
        div { class: "ds-toast-viewport",
            for entry in queue.read().iter() {
                {
                    let id = entry.id;
                    let level_class = match entry.level {
                        ToastLevel::Info => "ds-toast--info",
                        ToastLevel::Success => "ds-toast--success",
                        ToastLevel::Warning => "ds-toast--warning",
                        ToastLevel::Danger => "ds-toast--danger",
                    };
                    rsx! {
                        div {
                            key: "{id}",
                            class: "ds-toast {level_class}",
                            role: "status",
                            div { class: "ds-toast-title", "{entry.title}" }
                            div { class: "ds-toast-message", "{entry.message}" }
                        }
                    }
                }
            }
        }
    }
}

/// App-root provider. Call once near the top of the component tree.
#[component]
pub fn ToastProvider(children: Element) -> Element {
    use_context_provider(|| Signal::new(ToastQueue::new()));
    rsx! {
        {children}
        ToastViewport {}
    }
}
```

### CSS

```css
.ds-toast-viewport {
  position: fixed;
  top: var(--space-4);
  right: var(--space-4);
  display: grid;
  gap: var(--space-3);
  z-index: 2147483647;
  pointer-events: none;
}

.ds-toast {
  pointer-events: auto;
  min-width: 280px;
  max-width: 420px;
  padding: var(--space-3) var(--space-4);
  background: var(--color-surface);
  border: 1px solid var(--color-rule);
  border-left-width: 4px;
  border-radius: var(--radius-md);
  box-shadow: var(--shadow-lg);
  animation: ds-toast-in var(--duration-medium) var(--ease-decelerate) both;
}

@keyframes ds-toast-in {
  from { opacity: 0; transform: translateY(-8px); }
  to   { opacity: 1; transform: translateY(0); }
}

.ds-toast--info    { border-left-color: var(--color-info-500); }
.ds-toast--success { border-left-color: var(--color-success-500); }
.ds-toast--warning { border-left-color: var(--color-warning-500); }
.ds-toast--danger  { border-left-color: var(--color-danger-500); }

.ds-toast-title {
  font-weight: 800;
  font-size: var(--text-sm);
  margin-bottom: var(--space-1);
}
.ds-toast-message {
  font-size: var(--text-sm);
  color: var(--color-text-muted);
}
```

### `lib.rs`

```rust
pub mod toast;
pub use toast::{ToastEntry, ToastLevel, ToastQueue, ToastProvider, ToastSender, ToastViewport, use_toast_sender};
```

### Auto-dismiss (deferred to a follow-up)

The `duration_ms` field is wired into the data, but the auto-dismiss timer requires `wasm-bindgen` `setTimeout` plumbing. For this task, the toast stays visible until manually dismissed via `ToastSender::dismiss`. Implementing the timer is a follow-up for Plan D's polish pass.

### index.html cleanup

After the Rust ToastProvider lands and is wired into the app shell, remove:
- The inline `<script>` block defining `showDXToast`, `scheduleDXToast`, `closeDXToast` (~80 lines).
- The `<style>` block defining `.dx-toast` and its sub-rules (~80 lines).
- The `<div id="__dx-toast">` template at the start of `<body>` (~15 lines).

Keep the structure intact otherwise (the SPA-router script, the font preloads, etc.).

### Tests

- `toast_viewport_renders_empty_when_no_entries`.
- `toast_viewport_renders_entry_with_correct_level_class`.

The provider/sender behavior is tested via integration: a small harness component that uses `use_context_provider` to inject a Signal, calls `push`, and checks the rendered HTML.

### Commit

```
feat(toast): Rust-native Toast composite with context Signal queue
```

---

## Task 6: Page Header pattern

**Files:**
- Create: `crates/design-system/src/page_header.rs`
- Modify: `crates/design-system/src/lib.rs`
- Modify: `crates/design-system/assets/components.css` (+ mirror)

### API

Slot-based with optional kicker/subtitle/actions.

```rust
#[derive(Props, Clone, PartialEq)]
pub struct PageHeaderProps {
    pub title: String,
    #[props(default)]
    pub kicker: Option<String>,
    #[props(default)]
    pub subtitle: Option<String>,
    /// Right-aligned actions slot (buttons, links, etc.).
    #[props(default)]
    pub actions: Option<Element>,
    /// Variant: default | hero (uses display font for title)
    #[props(default)]
    pub variant: PageHeaderVariant,
}

#[derive(Clone, PartialEq, Default)]
pub enum PageHeaderVariant {
    #[default]
    Default,
    Hero,
}

#[component]
pub fn PageHeader(props: PageHeaderProps) -> Element {
    let class = match props.variant {
        PageHeaderVariant::Default => "ds-page-header",
        PageHeaderVariant::Hero => "ds-page-header ds-page-header--hero",
    };
    rsx! {
        header { class: "{class}",
            div { class: "ds-page-header-text",
                if let Some(k) = &props.kicker {
                    p { class: "ds-page-header-kicker", "{k}" }
                }
                h1 { class: "ds-page-header-title", "{props.title}" }
                if let Some(s) = &props.subtitle {
                    p { class: "ds-page-header-subtitle", "{s}" }
                }
            }
            if let Some(actions) = &props.actions {
                div { class: "ds-page-header-actions", {actions.clone()} }
            }
        }
    }
}
```

### CSS

```css
.ds-page-header {
  display: flex;
  justify-content: space-between;
  align-items: flex-end;
  gap: var(--space-4);
  margin-bottom: var(--space-4);
}

.ds-page-header-text {
  display: grid;
  gap: var(--space-2);
}

.ds-page-header-kicker {
  margin: 0;
  color: var(--color-accent);
  font-size: var(--text-xs);
  line-height: var(--leading-xs);
  font-weight: 900;
  letter-spacing: 0.08em;
  text-transform: uppercase;
}

.ds-page-header-title {
  margin: 0;
  color: var(--color-ink);
  font-family: var(--font-body);
  font-size: var(--text-3xl);
  line-height: var(--leading-3xl);
  font-weight: 800;
}

.ds-page-header--hero .ds-page-header-title {
  font-family: var(--font-display);
  font-size: clamp(34px, 5vw, 56px);
  line-height: 1;
}

.ds-page-header-subtitle {
  margin: 0;
  max-width: 720px;
  color: var(--color-text-muted);
  font-size: var(--text-base);
  line-height: var(--leading-base);
}

.ds-page-header-actions {
  display: flex;
  align-items: center;
  gap: var(--space-3);
}

@media (max-width: 900px) {
  .ds-page-header {
    flex-direction: column;
    align-items: flex-start;
  }
}
```

### Tests

- `page_header_renders_title_only`.
- `page_header_renders_kicker_subtitle_actions`.
- `page_header_hero_variant_renders_class`.

### Commit

```
feat(page-header): kicker + title + subtitle + actions composite pattern
```

---

## Task 7: Loading composite

**Files:**
- Create: `crates/design-system/src/loading.rs`
- Modify: `crates/design-system/src/lib.rs`
- Modify: `crates/design-system/assets/components.css` (+ mirror)

### API

A composite that wraps Spinner with an optional message and supports the common "loading…" UX pattern in a single component.

```rust
#[derive(Props, Clone, PartialEq)]
pub struct LoadingProps {
    /// Message text. Defaults to "Loading…".
    #[props(default = "Loading…".to_string())]
    pub message: String,
    /// Spinner size — passes through.
    #[props(default)]
    pub size: crate::spinner::SpinnerSize,
    /// Layout: inline (spinner left of text) or block (spinner above text).
    #[props(default)]
    pub layout: LoadingLayout,
}

#[derive(Clone, PartialEq, Default)]
pub enum LoadingLayout {
    #[default]
    Inline,
    Block,
}

#[component]
pub fn Loading(props: LoadingProps) -> Element {
    let layout_class = match props.layout {
        LoadingLayout::Inline => "ds-loading ds-loading--inline",
        LoadingLayout::Block => "ds-loading ds-loading--block",
    };
    rsx! {
        div { class: layout_class, role: "status",
            crate::spinner::Spinner { size: props.size, aria_label: Some(props.message.clone()) }
            span { class: "ds-loading-text", "{props.message}" }
        }
    }
}
```

### CSS

```css
.ds-loading {
  display: inline-flex;
  align-items: center;
  gap: var(--space-2);
  color: var(--color-text-muted);
  font-size: var(--text-sm);
  line-height: var(--leading-sm);
}

.ds-loading--block {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--space-3);
  padding: var(--space-6);
}

.ds-loading-text {
  display: inline-block;
}
```

### Tests

- `loading_default_renders_inline_with_message`.
- `loading_block_layout_renders_class`.

### Commit

```
feat(loading): composite wrapping Spinner + message with inline/block layouts
```

---

## Task 8: Final verification + cross-task review

- [ ] **Step 1: Workspace tests**
```bash
cargo test --workspace --no-fail-fast
```

- [ ] **Step 2: Format check**
```bash
cargo fmt --all -- --check
```

- [ ] **Step 3: Wasm build**
```bash
cargo build -p shell-web --target wasm32-unknown-unknown
```

- [ ] **Step 4: Asset-sync diff**
```bash
diff crates/design-system/assets/components.css crates/shell-web/public/assets/components.css
diff crates/design-system/assets/tokens.css crates/shell-web/public/assets/tokens.css
```

- [ ] **Step 5: Token-only audit on Plan C composite blocks**
```bash
grep -nE "^\s+(#[0-9a-fA-F]{3,8}|[0-9]+px|[0-9.]+rem)" crates/design-system/assets/components.css | \
  grep -vE "(@font-face|url\(|rgba\(|/\*|--|var\()" | \
  grep -E "(\.ds-field|\.ds-table|\.ds-card-list|\.empty-state|\.ds-skeleton|\.ds-toast|\.ds-page-header|\.ds-loading)" | head -20
```

Any match should be a deliberate exception (intrinsic geometry like `14px` skeleton-line height). If a non-exception slipped in, refactor.

- [ ] **Step 6: Dispatch cross-task code reviewer**

Single review pass over Tasks 1-7 with focus on:
- Pattern API consistency (slot-based vs prop-based).
- Composite reuse (do patterns use Plan B primitives properly?).
- Toast context wiring (provider/sender ergonomics).
- Backward-compat (existing call sites still work).

- [ ] **Step 7: Document follow-ups for Plan D**

Anything surfaced during implementation that belongs to surface-level work goes into a brief note for Plan D's brainstorm.

---

## Spec coverage check

| Vision-spec requirement | Task |
|--|--|
| Form Field wrapper | Task 1 |
| Table / List pattern | Task 2 (Table + CardList) |
| Card-list pattern | Task 2 (CardList) |
| Empty state pattern | Task 3 |
| Skeleton primitives | Task 4 |
| Toast | Task 5 |
| Page Header pattern | Task 6 |
| Loading composite | Task 7 |
| Token-only invariant | Audit at Task 8 |
| Asset-sync mirror | After every CSS edit |
| Backward compat | Discipline per task |

## Out of scope

- Auto-dismiss timer for Toast (deferred — needs `setTimeout` plumbing).
- Surface application (Plan D).
- Removing the legacy `.field-label` and inline `.dx-toast` CSS entirely (kept during transition; can be GC'd in Plan D once no surface uses them).
- Dark-theme values.

## Risks

- **Toast provider wiring touches the shell.** The Rust ToastProvider must be mounted near the app root in `crates/shell-web/src/lib.rs` (or equivalent) for `use_toast_sender()` to find the context. Plan D will be the natural place to wire all surface usages; for Plan C the provider exists in the design-system but isn't yet mounted in shell-web's root. Mount it in Task 5 Step 8 so it's live (even if no callers push to it yet).
- **CSS bloat.** `components.css` is ~1500 lines after Plan B + cleanup. Plan C adds ~400 lines for ~1900 total. At this point a future plan should consider splitting by component family.
- **Toast context propagation across route transitions.** Should be fine since the provider is above the router, but verify after wiring.
- **Empty state existing API.** Read `empty_state.rs` first; preserve all props. The `cta: Option<Element>` slot likely already exists.
