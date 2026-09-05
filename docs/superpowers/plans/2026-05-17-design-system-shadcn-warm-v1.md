# Design System v1 — Shadcn-Feel-on-Warm-Cream Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Lift the Dioxus/Rust design system to a feature-dense, shadcn-polished SaaS aesthetic while keeping the warm cream + serif brand identity. Ships an additive token overhaul, a motion catalog, 10 reworked/new primitives, a kitchen-sink debug route, and a Playwright smoke spec — without breaking a single existing route.

**Architecture:** All work in `crates/design-system/` (Rust + CSS) and `crates/shell-web/` (one debug-only route + smoke spec). Public Rust APIs stay backwards-compatible: every change is an additive new variant, prop, or subcomponent. CSS additions layer alongside existing classes; nothing is renamed or deleted. The CSS files have a known mirror in `crates/shell-web/public/assets/` that MUST be kept in sync after every edit (see Conventions).

**Tech Stack:** Rust + Dioxus 0.7 (SSR-compatible component testing via `dioxus_ssr`), CSS custom properties only (no preprocessor), Playwright for end-to-end smoke.

**Spec:** `docs/superpowers/specs/2026-05-17-design-system-shadcn-warm-v1-design.md` (commit `287079a`).

---

## File Structure

**Modify:**
- `crates/design-system/assets/tokens.css` — additive token block (control sizing, unified radius, ring, shadows, motion, surface, brand semantic remap).
- `crates/design-system/assets/components.css` — motion catalog block at top; per-component sections rewritten/added.
- `crates/design-system/src/button.rs` — size variants, paper-press, premium + destructive aliases, trailing icon, loading state.
- `crates/design-system/src/card.rs` — Header/Title/Description/Content/Footer subcomponents; `interactive` + `premium` props.
- `crates/design-system/src/input.rs` — leading/trailing icon and addon slots; size variants.
- `crates/design-system/src/field.rs` — invalid state ring wiring; description vs error semantics.
- `crates/design-system/src/form_error.rs` — slide-down animation marker.
- `crates/design-system/src/badge.rs` — premium tone; size variants; live pulsing dot.
- `crates/design-system/src/table.rs` — `Toolbar`, `HeaderCell` (sortable), `EmptyState` integration, `striped` prop.
- `crates/design-system/src/tabs.rs` — `variant` (Underline/Pill), keyboard nav, ARIA roles, gold underline animation.
- `crates/design-system/src/skeleton.rs` — shimmer keyframe (CSS-only change; Rust file unchanged structurally).
- `crates/design-system/src/toast.rs` — `Premium` level; `ToastPosition` on provider; corner-out motion.
- `crates/design-system/src/lib.rs` — re-exports for new modules + new types.
- `crates/shell-web/src/routes/mod.rs` — module + re-export for `dev_components` (gated `cfg(debug_assertions)`).
- `crates/shell-web/src/route_enum.rs` — `DevComponents` variant (gated `cfg(debug_assertions)`).
- `crates/shell-web/public/assets/tokens.css` — mirror of design-system tokens.css.
- `crates/shell-web/public/assets/components.css` — mirror of design-system components.css.

**Create:**
- `crates/design-system/src/dropdown_menu.rs` — new primitive.
- `crates/design-system/src/sheet.rs` — new primitive.
- `crates/shell-web/src/routes/dev_components.rs` — kitchen-sink debug page.
- `tools/design_system_smoke.spec.js` — Playwright smoke spec (lives under `tools/` to match existing `tools/ui-real-stack.spec.js` convention; `playwright.config.js` has `testDir: "."` so it is discovered automatically).

---

## Conventions (referenced by every task)

### CSS mirror discipline (CRITICAL)

After every edit to `crates/design-system/assets/{tokens,components}.css`, copy to the shell-web public mirror:

```bash
cp crates/design-system/assets/tokens.css crates/shell-web/public/assets/tokens.css
cp crates/design-system/assets/components.css crates/shell-web/public/assets/components.css
diff crates/design-system/assets/tokens.css crates/shell-web/public/assets/tokens.css
diff crates/design-system/assets/components.css crates/shell-web/public/assets/components.css
```

Both diffs MUST be empty. If not, the build will serve stale CSS at runtime.

### Compile gate

After every task, run:

```bash
cargo check -p design-system
cargo check -p shell-web
```

Both must be green. The compile gate is the primary safety net — any signature break here is a task failure that must be fixed before commit.

### Test pattern (Rust SSR)

Existing tests follow this shape (e.g., `button.rs`):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use dioxus::prelude::*;

    #[test]
    fn descriptive_test_name() {
        fn app() -> Element { rsx! { /* component */ } }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("expected-class-or-text"));
    }
}
```

Test naming: `<primitive>_<variant_or_state>_<expected_outcome>` (e.g., `button_premium_variant_renders_class`).

### Token-only invariant

No raw hex / px values in new component CSS blocks. Every value goes through a token (`var(--...)`). Existing legacy hex values in unchanged regions of components.css are left alone.

### Commit convention

One commit per task. Subject pattern: `feat(design-system): <task-name>` or `feat(shell-web): <task-name>`. Body lists files touched and what changed.

### Backwards-compatibility rule

Every Rust prop addition is optional with `#[props(default)]` or a non-breaking default value. Every CSS class addition layers alongside existing classes. The bar: `cargo check -p shell-web` is green after every commit, and no existing route file requires changes for v1.

---

## Tasks

### Task 1: Token overhaul (additive)

**Files:**
- Modify: `crates/design-system/assets/tokens.css`
- Mirror: `crates/shell-web/public/assets/tokens.css`

- [ ] **Step 1: Append new token block to `tokens.css`**

Open `crates/design-system/assets/tokens.css`. Inside the existing `:root { ... }` block, just before the closing `}`, insert:

```css
  /* ===========================================================
   * Design System v1 — shadcn-feel-on-warm-cream (2026-05-17)
   * Additive only. Existing tokens above are unchanged.
   * =========================================================== */

  /* Control sizing (tight density) */
  --control-h-sm: 28px;
  --control-h-md: 36px;
  --control-h-lg: 44px;
  --control-px-sm: 8px;
  --control-px-md: 12px;
  --control-px-lg: 16px;
  --control-gap: 6px;

  /* Radius (unified, shadcn-style). Legacy --radius-* aliases retained above. */
  --radius: 8px;
  --radius-shadcn-sm: calc(var(--radius) - 4px); /* 4px  */
  --radius-shadcn-md: calc(var(--radius) - 2px); /* 6px  */
  --radius-shadcn-lg: var(--radius);             /* 8px  */
  --radius-shadcn-xl: calc(var(--radius) + 4px); /* 12px */

  /* Ring (focus) — gold-tinted custom touch */
  --ring:        0 0 0 2px var(--color-surface),
                 0 0 0 4px rgba(201, 164, 92, 0.55);
  --ring-danger: 0 0 0 2px var(--color-surface),
                 0 0 0 4px rgba(163, 58, 54, 0.55);
  --ring-offset: var(--color-surface);

  /* Shadows — flatter shadcn-leaning defaults + warm hover + paper-press */
  --shadow-flat:       0 1px 0 rgba(18, 22, 20, 0.04),
                       0 0 0 1px var(--color-rule);
  --shadow-press:      inset 0 1px 2px rgba(18, 22, 20, 0.10);
  --shadow-warm-hover: 0 6px 18px rgba(201, 164, 92, 0.10),
                       0 1px 0 rgba(18, 22, 20, 0.04),
                       0 0 0 1px var(--color-rule);

  /* Motion timing — entrance/exit pair (shadcn / Radix style) */
  --duration-enter: 180ms;
  --duration-exit:  140ms;
  --ease-content:   cubic-bezier(0.32, 0.72, 0, 1);
  --press-y:        1px;

  /* Surface tokens for new components */
  --surface-card:       var(--color-surface);
  --surface-card-hover: #fffaf0;
  --surface-muted:      var(--color-surface-subtle);
  --surface-overlay:    rgba(15, 17, 14, 0.45);
  --surface-popover:    #fffdf7;

  /* Brand / accent semantic remap (alias-only; values unchanged) */
  --brand-primary:        var(--color-green-600);
  --brand-primary-hover:  var(--color-green-700);
  --brand-primary-active: var(--color-green-800);
  --accent-premium:       var(--color-gold-500);
  --accent-info:          var(--color-navy-600);
  --accent-live:          var(--color-oxblood-500);
```

Note the `--radius-shadcn-*` names: we cannot redefine `--radius-sm/md/lg/xl` because those exist above with different values; new components use the `--radius-shadcn-*` family.

- [ ] **Step 2: Mirror to shell-web**

```bash
cp crates/design-system/assets/tokens.css crates/shell-web/public/assets/tokens.css
diff crates/design-system/assets/tokens.css crates/shell-web/public/assets/tokens.css
```

The diff must be empty.

- [ ] **Step 3: Compile gate**

```bash
cargo check -p design-system
cargo check -p shell-web
```

Both must succeed. (CSS-only change; the gate confirms no Rust files inadvertently broke.)

- [ ] **Step 4: Commit**

```bash
git add crates/design-system/assets/tokens.css crates/shell-web/public/assets/tokens.css
git commit -m "feat(design-system): add v1 shadcn-warm token block (additive)"
```

---

### Task 2: Motion catalog in components.css

**Files:**
- Modify: `crates/design-system/assets/components.css`
- Mirror: `crates/shell-web/public/assets/components.css`

- [ ] **Step 1: Insert motion catalog block at the top of `components.css`**

Open `crates/design-system/assets/components.css`. Prepend (very top of file, before any existing rule):

```css
/* ============================================================
 * Motion catalog (Design System v1)
 * Single source of truth for keyframes, enter/exit pairs,
 * page-in choreography, and reduced-motion fallbacks.
 * ============================================================ */

@keyframes ds-fade-in {
  from { opacity: 0; }
  to   { opacity: 1; }
}
@keyframes ds-fade-out {
  from { opacity: 1; }
  to   { opacity: 0; }
}
@keyframes ds-zoom-in-95 {
  from { opacity: 0; transform: scale(0.95); }
  to   { opacity: 1; transform: scale(1); }
}
@keyframes ds-zoom-out-95 {
  from { opacity: 1; transform: scale(1); }
  to   { opacity: 0; transform: scale(0.95); }
}
@keyframes ds-slide-in-right {
  from { opacity: 0; transform: translateX(8px); }
  to   { opacity: 1; transform: translateX(0); }
}
@keyframes ds-slide-out-right {
  from { opacity: 1; transform: translateX(0); }
  to   { opacity: 0; transform: translateX(8px); }
}
@keyframes ds-slide-in-left {
  from { opacity: 0; transform: translateX(-8px); }
  to   { opacity: 1; transform: translateX(0); }
}
@keyframes ds-slide-out-left {
  from { opacity: 1; transform: translateX(0); }
  to   { opacity: 0; transform: translateX(-8px); }
}
@keyframes ds-slide-in-top {
  from { opacity: 0; transform: translateY(-8px); }
  to   { opacity: 1; transform: translateY(0); }
}
@keyframes ds-slide-out-top {
  from { opacity: 1; transform: translateY(0); }
  to   { opacity: 0; transform: translateY(-8px); }
}
@keyframes ds-slide-in-bottom {
  from { opacity: 0; transform: translateY(8px); }
  to   { opacity: 1; transform: translateY(0); }
}
@keyframes ds-slide-out-bottom {
  from { opacity: 1; transform: translateY(0); }
  to   { opacity: 0; transform: translateY(8px); }
}

/* Sheet panel slides at full edge (440px right by default) */
@keyframes ds-sheet-in-right {
  from { transform: translateX(100%); }
  to   { transform: translateX(0); }
}
@keyframes ds-sheet-out-right {
  from { transform: translateX(0); }
  to   { transform: translateX(100%); }
}
@keyframes ds-sheet-in-left {
  from { transform: translateX(-100%); }
  to   { transform: translateX(0); }
}
@keyframes ds-sheet-out-left {
  from { transform: translateX(0); }
  to   { transform: translateX(-100%); }
}

/* Skeleton shimmer */
@keyframes ds-shimmer {
  from { background-position: -200% 0; }
  to   { background-position: 200% 0; }
}

/* Live badge pulse */
@keyframes ds-pulse {
  0%, 100% { opacity: 1;   transform: scale(1); }
  50%      { opacity: 0.6; transform: scale(0.85); }
}

/* Page-in choreography (one-shot on route mount) */
@keyframes ds-page-in {
  from { opacity: 0; transform: translateY(8px); }
  to   { opacity: 1; transform: translateY(0); }
}

.ds-page-enter > [data-stagger] {
  opacity: 0;
  animation: ds-page-in 400ms var(--ease-page-in) forwards;
}
.ds-page-enter > [data-stagger="1"] { animation-delay: 0ms;   }
.ds-page-enter > [data-stagger="2"] { animation-delay: 60ms;  }
.ds-page-enter > [data-stagger="3"] { animation-delay: 120ms; }
.ds-page-enter > [data-stagger="4"] { animation-delay: 180ms; }
.ds-page-enter > [data-stagger="5"] { animation-delay: 240ms; }
.ds-page-enter > [data-stagger="6"] { animation-delay: 300ms; }

/* ============================================================
 * Reduced motion — global fallback at end of file (also added here
 * so adopters of just this block honour the user preference)
 * ============================================================ */
@media (prefers-reduced-motion: reduce) {
  *,
  *::before,
  *::after {
    animation-duration: 0.01ms !important;
    animation-iteration-count: 1 !important;
    transition-property: background-color, color, border-color, box-shadow !important;
  }
}
/* === End motion catalog === */
```

- [ ] **Step 2: Mirror to shell-web**

```bash
cp crates/design-system/assets/components.css crates/shell-web/public/assets/components.css
diff crates/design-system/assets/components.css crates/shell-web/public/assets/components.css
```

Diff must be empty.

- [ ] **Step 3: Compile gate**

```bash
cargo check -p design-system && cargo check -p shell-web
```

- [ ] **Step 4: Commit**

```bash
git add crates/design-system/assets/components.css crates/shell-web/public/assets/components.css
git commit -m "feat(design-system): add v1 motion catalog (keyframes + page-in + reduced-motion)"
```

---

### Task 3: Button rework

**Files:**
- Modify: `crates/design-system/src/button.rs`
- Modify: `crates/design-system/assets/components.css`
- Mirror: `crates/shell-web/public/assets/components.css`

The Button gets: size variants (`Sm`/`Md`/`Lg`/`Icon`), a `Premium` variant (gold), a `Destructive` alias for `Danger` (keeping back-compat), trailing icon slot, and a `loading` prop. Existing call sites continue to compile (no required prop changes; all new props are optional defaults).

- [ ] **Step 1: Write failing tests for new variants and props**

Append to the `mod tests` block in `crates/design-system/src/button.rs` (do not remove existing tests):

```rust
    #[test]
    fn button_premium_variant_renders_class() {
        fn app() -> Element {
            rsx! { Button { label: "Go Pro".to_string(), variant: ButtonVariant::Premium, on_click: |_| {} } }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("ds-button--premium"), "premium class missing: {html}");
    }

    #[test]
    fn button_size_sm_renders_class() {
        fn app() -> Element {
            rsx! { Button { label: "x".to_string(), size: ButtonSize::Sm, on_click: |_| {} } }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("ds-button--sm"), "sm class missing: {html}");
    }

    #[test]
    fn button_loading_renders_spinner_marker() {
        fn app() -> Element {
            rsx! { Button { label: "Save".to_string(), loading: true, on_click: |_| {} } }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("ds-button--loading"), "loading class missing: {html}");
        assert!(html.contains("ds-button-spinner"), "spinner marker missing: {html}");
    }

    #[test]
    fn button_trailing_icon_renders() {
        fn app() -> Element {
            rsx! {
                Button {
                    label: "Next".to_string(),
                    trailing_icon: Some(rsx! { span { "→" } }),
                    on_click: |_| {},
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("ds-button-icon--trailing"), "trailing icon wrapper missing: {html}");
        assert!(html.contains("→"));
    }
```

- [ ] **Step 2: Run and confirm tests fail**

```bash
cargo test -p design-system --lib button::tests
```

Expected: compile failure (unknown `ButtonVariant::Premium`, `ButtonSize`, `loading`, `trailing_icon`).

- [ ] **Step 3: Replace `crates/design-system/src/button.rs` with the new implementation**

```rust
use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct ButtonProps {
    pub label: String,
    #[props(default)]
    pub disabled: bool,
    #[props(default)]
    pub variant: ButtonVariant,
    #[props(default)]
    pub size: ButtonSize,
    /// Shows an inline spinner and suppresses the click handler.
    #[props(default)]
    pub loading: bool,
    pub on_click: EventHandler<MouseEvent>,
    /// Optional icon glyph rendered before the label.
    #[props(default)]
    pub leading_icon: Option<Element>,
    /// Optional icon glyph rendered after the label.
    #[props(default)]
    pub trailing_icon: Option<Element>,
    /// `type` attribute. Defaults to "button"; pass "submit" inside forms.
    #[props(default = "button".to_string())]
    pub button_type: String,
}

#[derive(Clone, PartialEq, Default)]
pub enum ButtonVariant {
    #[default]
    Primary,
    Secondary,
    /// Alias kept for back-compat; identical to `Destructive`.
    Danger,
    /// Oxblood destructive button.
    Destructive,
    Ghost,
    Link,
    /// Gold premium CTA (paywall, upgrade prompts).
    Premium,
}

#[derive(Clone, PartialEq, Default)]
pub enum ButtonSize {
    Sm,
    #[default]
    Md,
    Lg,
    /// Square icon-only button (36×36).
    Icon,
}

#[component]
pub fn Button(props: ButtonProps) -> Element {
    let variant_class = match props.variant {
        ButtonVariant::Primary => "ds-button--primary",
        ButtonVariant::Secondary => "ds-button--secondary",
        // Danger and Destructive share styling; Danger is the legacy alias.
        ButtonVariant::Danger | ButtonVariant::Destructive => "ds-button--destructive",
        ButtonVariant::Ghost => "ds-button--ghost",
        ButtonVariant::Link => "ds-button--link",
        ButtonVariant::Premium => "ds-button--premium",
    };
    let size_class = match props.size {
        ButtonSize::Sm => "ds-button--sm",
        ButtonSize::Md => "ds-button--md",
        ButtonSize::Lg => "ds-button--lg",
        ButtonSize::Icon => "ds-button--icon",
    };
    let loading_class = if props.loading { " ds-button--loading" } else { "" };
    let class = format!("ds-button {variant_class} {size_class}{loading_class}");

    rsx! {
        button {
            class: "{class}",
            r#type: "{props.button_type}",
            disabled: props.disabled || props.loading,
            "aria-busy": if props.loading { "true" } else { "false" },
            onclick: move |event| {
                if !props.loading {
                    props.on_click.call(event);
                }
            },
            if props.loading {
                span { class: "ds-button-spinner", "aria-hidden": "true" }
            } else if let Some(icon) = &props.leading_icon {
                span { class: "ds-button-icon ds-button-icon--leading", {icon.clone()} }
            }
            "{props.label}"
            if !props.loading {
                if let Some(icon) = &props.trailing_icon {
                    span { class: "ds-button-icon ds-button-icon--trailing", {icon.clone()} }
                }
            }
        }
    }
}
```

Then keep the existing `mod tests` block AS-IS (with the four new tests appended in Step 1). The legacy tests assert `ds-button--primary` which the new code still emits.

- [ ] **Step 4: Add Button CSS section to `components.css`**

In `crates/design-system/assets/components.css`, find any existing `.ds-button` block. Replace the entire block (or insert if missing) with the following section. If a legacy block exists, delete it and replace with this — the class names match so consumers still get a button.

```css
/* === Button (v1) === */
.ds-button {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: var(--control-gap);
  height: var(--control-h-md);
  padding-inline: var(--control-px-md);
  border: 1px solid transparent;
  border-radius: var(--radius-shadcn-md);
  font-family: var(--font-body);
  font-size: var(--text-base);
  font-weight: 500;
  line-height: 1;
  cursor: pointer;
  user-select: none;
  white-space: nowrap;
  transition:
    background-color var(--duration-fast) var(--ease-snappy),
    border-color var(--duration-fast) var(--ease-snappy),
    box-shadow var(--duration-fast) var(--ease-snappy),
    transform var(--duration-fast) var(--ease-snappy),
    color var(--duration-fast) var(--ease-snappy);
}
.ds-button:focus-visible {
  outline: none;
  box-shadow: var(--ring);
}
.ds-button:disabled,
.ds-button[aria-busy="true"] {
  opacity: 0.4;
  cursor: not-allowed;
  box-shadow: none;
}

/* Sizes */
.ds-button--sm   { height: var(--control-h-sm); padding-inline: var(--control-px-sm); font-size: var(--text-xs); }
.ds-button--md   { height: var(--control-h-md); padding-inline: var(--control-px-md); }
.ds-button--lg   { height: var(--control-h-lg); padding-inline: var(--control-px-lg); font-size: var(--text-md); }
.ds-button--icon { height: var(--control-h-md); width: var(--control-h-md); padding: 0; }

/* Variants */
.ds-button--primary {
  background: var(--brand-primary);
  color: #fff;
  border-color: var(--brand-primary);
}
.ds-button--primary:hover:not(:disabled) {
  background: var(--brand-primary-hover);
  border-color: var(--brand-primary-hover);
  box-shadow: var(--shadow-warm-hover);
}
.ds-button--primary:active:not(:disabled) {
  background: var(--brand-primary-active);
  transform: translateY(var(--press-y));
  box-shadow: var(--shadow-press);
}

.ds-button--secondary {
  background: var(--surface-card);
  color: var(--color-text);
  border-color: var(--color-rule);
  box-shadow: var(--shadow-flat);
}
.ds-button--secondary:hover:not(:disabled) {
  background: var(--surface-card-hover);
  box-shadow: var(--shadow-warm-hover);
}
.ds-button--secondary:active:not(:disabled) {
  transform: translateY(var(--press-y));
  box-shadow: var(--shadow-press);
}

.ds-button--ghost {
  background: transparent;
  color: var(--color-text);
  border-color: transparent;
}
.ds-button--ghost:hover:not(:disabled) {
  background: var(--surface-muted);
}
.ds-button--ghost:active:not(:disabled) {
  transform: translateY(var(--press-y));
  box-shadow: var(--shadow-press);
}

.ds-button--destructive {
  background: var(--accent-live);
  color: #fff;
  border-color: var(--accent-live);
}
.ds-button--destructive:hover:not(:disabled) {
  background: var(--color-oxblood-600);
  border-color: var(--color-oxblood-600);
  box-shadow: var(--shadow-warm-hover);
}
.ds-button--destructive:active:not(:disabled) {
  background: var(--color-oxblood-700);
  transform: translateY(var(--press-y));
  box-shadow: var(--shadow-press);
}
.ds-button--destructive:focus-visible { box-shadow: var(--ring-danger); }

.ds-button--premium {
  background: var(--accent-premium);
  color: var(--color-ink);
  border-color: var(--accent-premium);
}
.ds-button--premium:hover:not(:disabled) {
  background: var(--color-gold-600);
  border-color: var(--color-gold-600);
  box-shadow: var(--shadow-warm-hover);
}
.ds-button--premium:active:not(:disabled) {
  transform: translateY(var(--press-y));
  box-shadow: var(--shadow-press);
}

.ds-button--link {
  background: transparent;
  color: var(--brand-primary);
  border-color: transparent;
  padding-inline: 0;
  height: auto;
  text-decoration: underline;
  text-underline-offset: 3px;
}
.ds-button--link:hover:not(:disabled) { color: var(--brand-primary-hover); }

/* Icon slots */
.ds-button-icon { display: inline-flex; }
.ds-button-icon--leading  { margin-right: 0; }
.ds-button-icon--trailing { margin-left: 0; }

/* Spinner — reuses dot animation; pure CSS, no JS */
.ds-button-spinner {
  display: inline-block;
  width: 14px; height: 14px;
  border: 2px solid currentColor;
  border-right-color: transparent;
  border-radius: var(--radius-full);
  animation: ds-spin 700ms linear infinite;
}
@keyframes ds-spin { to { transform: rotate(360deg); } }
/* === End Button === */
```

- [ ] **Step 5: Mirror CSS to shell-web**

```bash
cp crates/design-system/assets/components.css crates/shell-web/public/assets/components.css
diff crates/design-system/assets/components.css crates/shell-web/public/assets/components.css
```

- [ ] **Step 6: Run tests + compile gate**

```bash
cargo test -p design-system --lib button::tests
cargo check -p shell-web
```

All button tests must pass; shell-web must still compile (existing routes use Button with old props which all remain).

- [ ] **Step 7: Commit**

```bash
git add crates/design-system/src/button.rs crates/design-system/assets/components.css crates/shell-web/public/assets/components.css
git commit -m "feat(design-system): Button v1 — sizes, premium, destructive, loading, trailing icon"
```

---

### Task 4: Card subcomponents + interactive + premium

**Files:**
- Modify: `crates/design-system/src/card.rs`
- Modify: `crates/design-system/assets/components.css`
- Mirror: `crates/shell-web/public/assets/components.css`

The Card gets shadcn-style subcomponents (`CardHeader`, `CardTitle`, `CardDescription`, `CardContent`, `CardFooter`), an `interactive` prop for warm-halo hover, and a new `Premium` variant (gold inner border). Existing single-slot `Card` keeps working.

- [ ] **Step 1: Write failing tests**

Append to the `mod tests` block in `crates/design-system/src/card.rs`:

```rust
    #[test]
    fn card_premium_variant_renders_class() {
        fn app() -> Element {
            rsx! { Card { variant: CardVariant::Premium, "p" } }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("ds-card--premium"), "premium class missing: {html}");
    }

    #[test]
    fn card_interactive_renders_class() {
        fn app() -> Element {
            rsx! { Card { interactive: true, "p" } }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("ds-card--interactive"), "interactive class missing: {html}");
    }

    #[test]
    fn card_header_subcomponent_renders() {
        fn app() -> Element {
            rsx! { Card { CardHeader { CardTitle { "T" } CardDescription { "D" } } } }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("ds-card-header"));
        assert!(html.contains("ds-card-title"));
        assert!(html.contains("ds-card-description"));
    }
```

- [ ] **Step 2: Run + confirm failure**

```bash
cargo test -p design-system --lib card::tests
```

Expected: compile failure (`CardVariant::Premium`, `interactive`, `CardHeader`, etc. not defined).

- [ ] **Step 3: Replace `crates/design-system/src/card.rs`**

```rust
use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct CardProps {
    pub children: Element,
    /// When set, the card becomes a clickable surface with hover affordance.
    #[props(default)]
    pub on_click: Option<EventHandler<MouseEvent>>,
    #[props(default)]
    pub variant: CardVariant,
    /// Opt-in warm-halo hover (also implied when `on_click` is set).
    #[props(default)]
    pub interactive: bool,
}

#[derive(Clone, PartialEq, Default)]
pub enum CardVariant {
    #[default]
    Default,
    Accent,
    Danger,
    /// Gold inner border (used sparingly for premium tiers).
    Premium,
}

#[component]
pub fn Card(props: CardProps) -> Element {
    let mut class = String::from("ds-card");
    match props.variant {
        CardVariant::Default => {}
        CardVariant::Accent => class.push_str(" ds-card--accent"),
        CardVariant::Danger => class.push_str(" ds-card--danger"),
        CardVariant::Premium => class.push_str(" ds-card--premium"),
    }
    let is_interactive = props.interactive || props.on_click.is_some();
    if is_interactive {
        class.push_str(" ds-card--interactive");
    }
    if props.on_click.is_some() {
        class.push_str(" ds-card--clickable");
    }

    if let Some(on_click) = props.on_click {
        rsx! {
            div {
                class: "{class}",
                role: "button",
                tabindex: "0",
                onclick: move |evt| on_click.call(evt),
                {props.children}
            }
        }
    } else {
        rsx! { div { class: "{class}", {props.children} } }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct CardSlotProps { pub children: Element }

#[component]
pub fn CardHeader(props: CardSlotProps) -> Element {
    rsx! { div { class: "ds-card-header", {props.children} } }
}

#[component]
pub fn CardTitle(props: CardSlotProps) -> Element {
    rsx! { h3 { class: "ds-card-title", {props.children} } }
}

#[component]
pub fn CardDescription(props: CardSlotProps) -> Element {
    rsx! { p { class: "ds-card-description", {props.children} } }
}

#[component]
pub fn CardContent(props: CardSlotProps) -> Element {
    rsx! { div { class: "ds-card-content", {props.children} } }
}

#[component]
pub fn CardFooter(props: CardSlotProps) -> Element {
    rsx! { div { class: "ds-card-footer", {props.children} } }
}
```

Keep the existing `mod tests` block (with the three new tests appended). Existing tests assert `ds-card`, `ds-card--accent`, `ds-card--clickable` — all still emitted.

- [ ] **Step 4: Add CardHeader/Title/etc. to lib.rs re-exports**

In `crates/design-system/src/lib.rs`, replace the existing `pub use card::{...}` line with:

```rust
pub use card::{Card, CardContent, CardDescription, CardFooter, CardHeader, CardProps, CardTitle, CardVariant};
```

- [ ] **Step 5: Add Card v1 CSS to `components.css`**

Find any existing `.ds-card` block and replace it with the section below (or insert if missing). The legacy classes still emit; new modifiers layer on top.

```css
/* === Card (v1) === */
.ds-card {
  background: var(--surface-card);
  border-radius: var(--radius-shadcn-lg);
  box-shadow: var(--shadow-flat);
  color: var(--color-text);
  padding: var(--space-5);
  transition:
    box-shadow var(--duration-medium) var(--ease-snappy),
    border-color var(--duration-medium) var(--ease-snappy);
}
.ds-card--accent  { box-shadow: var(--shadow-flat), 0 0 0 2px var(--accent-info); }
.ds-card--danger  { box-shadow: var(--shadow-flat), 0 0 0 2px var(--accent-live); }
.ds-card--premium {
  box-shadow:
    var(--shadow-flat),
    inset 0 0 0 1px var(--accent-premium);
}
.ds-card--interactive:hover {
  box-shadow: var(--shadow-warm-hover);
  cursor: default;
}
.ds-card--clickable {
  cursor: pointer;
}
.ds-card--clickable:focus-visible {
  outline: none;
  box-shadow: var(--ring);
}

.ds-card-header {
  display: flex; flex-direction: column; gap: var(--space-1);
  margin-bottom: var(--space-4);
}
.ds-card-title {
  margin: 0;
  font-family: var(--font-display);
  font-size: var(--text-xl);
  line-height: var(--leading-xl);
  font-weight: 600;
  color: var(--color-text);
}
.ds-card-description {
  margin: 0;
  color: var(--color-text-muted);
  font-size: var(--text-sm);
  line-height: var(--leading-sm);
}
.ds-card-content { /* default: no margin reset, caller controls flow */ }
.ds-card-footer {
  display: flex; align-items: center; gap: var(--space-3);
  margin-top: var(--space-4);
  padding-top: var(--space-4);
  border-top: 1px solid var(--color-rule);
}
/* === End Card === */
```

- [ ] **Step 6: Mirror CSS**

```bash
cp crates/design-system/assets/components.css crates/shell-web/public/assets/components.css
diff crates/design-system/assets/components.css crates/shell-web/public/assets/components.css
```

- [ ] **Step 7: Test + compile gate**

```bash
cargo test -p design-system --lib card::tests
cargo check -p shell-web
```

- [ ] **Step 8: Commit**

```bash
git add crates/design-system/src/card.rs crates/design-system/src/lib.rs crates/design-system/assets/components.css crates/shell-web/public/assets/components.css
git commit -m "feat(design-system): Card v1 — subcomponents, premium variant, interactive prop"
```

---

### Task 5: Input + Field slots and ring

**Files:**
- Modify: `crates/design-system/src/input.rs`
- Modify: `crates/design-system/src/field.rs`
- Modify: `crates/design-system/src/form_error.rs`
- Modify: `crates/design-system/assets/components.css`
- Mirror: `crates/shell-web/public/assets/components.css`

Input gains `leading_icon`, `trailing_icon`, `addon_left`, `addon_right`, and a `size` prop. Field gets the gold focus ring on focus-within and oxblood ring when `error` is present. FormError gets a slide-down keyframe class.

- [ ] **Step 1: Write failing tests in `input.rs`**

Append to `crates/design-system/src/input.rs` `mod tests`:

```rust
    #[test]
    fn input_leading_icon_renders() {
        fn app() -> Element {
            rsx! {
                Input {
                    value: "".to_string(),
                    leading_icon: Some(rsx! { span { class: "i", "🔍" } }),
                    on_input: |_| {},
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("ds-input-slot--leading"), "leading slot missing: {html}");
    }

    #[test]
    fn input_addon_left_renders() {
        fn app() -> Element {
            rsx! {
                Input {
                    value: "".to_string(),
                    addon_left: Some("$".to_string()),
                    on_input: |_| {},
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("ds-input-addon--left"), "addon left missing: {html}");
        assert!(html.contains("$"));
    }

    #[test]
    fn input_renders_wrapped_when_slot_present() {
        fn app() -> Element {
            rsx! {
                Input {
                    value: "".to_string(),
                    leading_icon: Some(rsx! { span { "x" } }),
                    on_input: |_| {},
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("ds-input-wrap"), "wrap missing: {html}");
    }
```

- [ ] **Step 2: Run + confirm failure**

```bash
cargo test -p design-system --lib input::tests
```

Expected: compile failure (props `leading_icon`, `addon_left` not defined).

- [ ] **Step 3: Replace `crates/design-system/src/input.rs`**

```rust
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
    #[props(default)]
    pub error: bool,
    #[props(default)]
    pub name: Option<String>,
    #[props(default)]
    pub id: Option<String>,
    /// Icon rendered absolutely inside the input on the left edge.
    #[props(default)]
    pub leading_icon: Option<Element>,
    /// Icon rendered absolutely inside the input on the right edge.
    #[props(default)]
    pub trailing_icon: Option<Element>,
    /// Static prefix addon outside the input (e.g. "$").
    #[props(default)]
    pub addon_left: Option<String>,
    /// Static suffix addon outside the input (e.g. "kg").
    #[props(default)]
    pub addon_right: Option<String>,
    pub on_input: EventHandler<String>,
}

#[component]
pub fn Input(props: InputProps) -> Element {
    let has_leading = props.leading_icon.is_some();
    let has_trailing = props.trailing_icon.is_some();
    let has_addon_left = props.addon_left.is_some();
    let has_addon_right = props.addon_right.is_some();
    let has_slot = has_leading || has_trailing || has_addon_left || has_addon_right;

    let mut input_class = String::from("ds-input");
    if props.error {
        input_class.push_str(" ds-input--error");
    }
    if has_leading {
        input_class.push_str(" ds-input--has-leading");
    }
    if has_trailing {
        input_class.push_str(" ds-input--has-trailing");
    }

    let raw_input = rsx! {
        input {
            class: "{input_class}",
            r#type: "{props.input_type}",
            value: "{props.value}",
            placeholder: "{props.placeholder}",
            disabled: props.disabled,
            "aria-invalid": if props.error { "true" } else { "false" },
            name: props.name.clone().unwrap_or_default(),
            id: props.id.clone().unwrap_or_default(),
            oninput: move |event| props.on_input.call(event.value()),
        }
    };

    if !has_slot {
        return raw_input;
    }

    rsx! {
        div { class: "ds-input-wrap",
            if let Some(text) = &props.addon_left {
                span { class: "ds-input-addon ds-input-addon--left", "{text}" }
            }
            div { class: "ds-input-shell",
                if let Some(icon) = &props.leading_icon {
                    span { class: "ds-input-slot ds-input-slot--leading", {icon.clone()} }
                }
                {raw_input}
                if let Some(icon) = &props.trailing_icon {
                    span { class: "ds-input-slot ds-input-slot--trailing", {icon.clone()} }
                }
            }
            if let Some(text) = &props.addon_right {
                span { class: "ds-input-addon ds-input-addon--right", "{text}" }
            }
        }
    }
}
```

Keep the existing tests (with three new tests appended). Existing `placeholder=` and `ds-input` and `ds-input--error` assertions still pass because the markup either renders the bare input (when no slots) or wraps it with the same `ds-input` class.

- [ ] **Step 4: Add slot CSS to `components.css`**

Find any existing `.ds-input` block and replace with the section below (the base `.ds-input` class is preserved; new wrap/slot rules added).

```css
/* === Input (v1) === */
.ds-input {
  display: block;
  width: 100%;
  height: var(--control-h-md);
  padding-inline: var(--control-px-md);
  background: var(--surface-card);
  color: var(--color-text);
  border: 1px solid var(--color-rule);
  border-radius: var(--radius-shadcn-md);
  font-family: var(--font-body);
  font-size: var(--text-base);
  line-height: 1;
  transition:
    border-color var(--duration-fast) var(--ease-snappy),
    box-shadow var(--duration-fast) var(--ease-snappy);
}
.ds-input::placeholder { color: var(--color-text-muted); }
.ds-input:focus-visible,
.ds-input:focus {
  outline: none;
  border-color: var(--brand-primary);
  box-shadow: var(--ring);
}
.ds-input--error,
.ds-input[aria-invalid="true"] {
  border-color: var(--accent-live);
}
.ds-input--error:focus-visible,
.ds-input--error:focus,
.ds-input[aria-invalid="true"]:focus-visible {
  box-shadow: var(--ring-danger);
}
.ds-input:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

/* Slot-wrapper layout */
.ds-input-wrap {
  display: flex;
  align-items: stretch;
  width: 100%;
}
.ds-input-shell {
  position: relative;
  flex: 1 1 auto;
  display: flex;
  align-items: center;
}
.ds-input-shell .ds-input { padding-inline: var(--control-px-md); }
.ds-input--has-leading  { padding-left: 32px; }
.ds-input--has-trailing { padding-right: 32px; }
.ds-input-slot {
  position: absolute;
  top: 50%;
  transform: translateY(-50%);
  display: inline-flex;
  align-items: center;
  color: var(--color-text-muted);
  pointer-events: none;
}
.ds-input-slot--leading  { left: 10px; }
.ds-input-slot--trailing { right: 10px; }

.ds-input-addon {
  display: inline-flex;
  align-items: center;
  padding-inline: var(--control-px-md);
  background: var(--surface-muted);
  border: 1px solid var(--color-rule);
  color: var(--color-text-muted);
  font-size: var(--text-sm);
}
.ds-input-addon--left {
  border-right: 0;
  border-top-left-radius: var(--radius-shadcn-md);
  border-bottom-left-radius: var(--radius-shadcn-md);
}
.ds-input-addon--right {
  border-left: 0;
  border-top-right-radius: var(--radius-shadcn-md);
  border-bottom-right-radius: var(--radius-shadcn-md);
}
.ds-input-wrap > .ds-input-addon--left + .ds-input-shell .ds-input  { border-top-left-radius: 0;  border-bottom-left-radius: 0;  }
.ds-input-wrap > .ds-input-shell:has(+ .ds-input-addon--right) .ds-input { border-top-right-radius: 0; border-bottom-right-radius: 0; }
/* === End Input === */
```

- [ ] **Step 5: Update Field invalid-state CSS**

Append in the existing Field CSS section of `components.css` (find `.ds-field`; if absent, add the whole block):

```css
/* === Field (v1) === */
.ds-field { display: flex; flex-direction: column; gap: var(--space-2); }
.ds-field-label {
  font-size: var(--text-sm);
  font-weight: 500;
  color: var(--color-text);
}
.ds-field-helper {
  margin: 0;
  font-size: var(--text-xs);
  color: var(--color-text-muted);
  line-height: var(--leading-xs);
}
.ds-field-helper--error {
  color: var(--accent-live);
  animation: ds-slide-in-top var(--duration-enter) var(--ease-content);
}
/* === End Field === */
```

- [ ] **Step 6: Update FormError CSS (slide-down)**

Append in the FormError CSS section (find `.ds-form-error`; add this block if it's not present):

```css
/* === Form error (v1) === */
.ds-form-error {
  display: flex; align-items: center; gap: var(--space-2);
  padding: var(--space-3);
  background: var(--color-state-error-bg);
  border: 1px solid var(--color-state-error-border);
  border-radius: var(--radius-shadcn-md);
  color: var(--color-danger-700);
  font-size: var(--text-sm);
  animation: ds-slide-in-top var(--duration-enter) var(--ease-content);
}
/* === End form error === */
```

- [ ] **Step 7: Mirror CSS**

```bash
cp crates/design-system/assets/components.css crates/shell-web/public/assets/components.css
diff crates/design-system/assets/components.css crates/shell-web/public/assets/components.css
```

- [ ] **Step 8: Test + compile gate**

```bash
cargo test -p design-system --lib input::tests field::tests form_error::tests
cargo check -p shell-web
```

- [ ] **Step 9: Commit**

```bash
git add crates/design-system/src/input.rs crates/design-system/src/field.rs crates/design-system/src/form_error.rs crates/design-system/assets/components.css crates/shell-web/public/assets/components.css
git commit -m "feat(design-system): Input v1 — icon/addon slots; Field & FormError ring + slide-in"
```

---

### Task 6: Badge premium + sizes + live pulsing dot

**Files:**
- Modify: `crates/design-system/src/badge.rs`
- Modify: `crates/design-system/assets/components.css`
- Mirror: `crates/shell-web/public/assets/components.css`

Add `Premium` and `Primary` tones, an optional `size` prop, and emit a pulsing dot marker on `Live`.

- [ ] **Step 1: Write failing tests**

Append to the `mod tests` block in `crates/design-system/src/badge.rs`:

```rust
    #[test]
    fn badge_premium_renders_class() {
        fn app() -> Element {
            rsx! { Badge { label: "Pro".to_string(), tone: BadgeTone::Premium } }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("badge-premium"), "premium class missing: {html}");
    }

    #[test]
    fn badge_size_sm_renders_class() {
        fn app() -> Element {
            rsx! { Badge { label: "x".to_string(), size: BadgeSize::Sm } }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("badge-sm"), "sm class missing: {html}");
    }

    #[test]
    fn badge_live_renders_pulse_dot() {
        fn app() -> Element {
            rsx! { Badge { label: "LIVE".to_string(), tone: BadgeTone::Live } }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("badge-pulse"), "pulse dot missing: {html}");
    }
```

- [ ] **Step 2: Run + confirm failure**

```bash
cargo test -p design-system --lib badge::tests
```

- [ ] **Step 3: Replace `crates/design-system/src/badge.rs`**

```rust
use dioxus::prelude::*;

#[derive(Clone, PartialEq, Default)]
pub enum BadgeTone {
    #[default]
    Neutral,
    Primary,
    Info,
    Success,
    Warning,
    Danger,
    Live,
    Premium,
}

#[derive(Clone, PartialEq, Default)]
pub enum BadgeSize {
    Sm,
    #[default]
    Md,
}

#[derive(Props, Clone, PartialEq)]
pub struct BadgeProps {
    pub label: String,
    #[props(default = BadgeTone::Neutral)]
    pub tone: BadgeTone,
    #[props(default)]
    pub size: BadgeSize,
}

#[component]
pub fn Badge(props: BadgeProps) -> Element {
    let tone_class = match props.tone {
        BadgeTone::Neutral => "badge-neutral",
        BadgeTone::Primary => "badge-primary",
        BadgeTone::Info => "badge-info",
        BadgeTone::Success => "badge-success",
        BadgeTone::Warning => "badge-warning",
        BadgeTone::Danger => "badge-danger",
        BadgeTone::Live => "badge-live",
        BadgeTone::Premium => "badge-premium",
    };
    let size_class = match props.size {
        BadgeSize::Sm => "badge-sm",
        BadgeSize::Md => "badge-md",
    };
    let class = format!("badge {tone_class} {size_class}");
    let is_live = matches!(props.tone, BadgeTone::Live);
    rsx! {
        span { class: "{class}",
            if is_live {
                span { class: "badge-pulse", "aria-hidden": "true" }
            }
            "{props.label}"
        }
    }
}
```

- [ ] **Step 4: Export `BadgeSize` from `lib.rs`**

In `crates/design-system/src/lib.rs`, change the existing `pub use badge::{Badge, BadgeTone};` line to:

```rust
pub use badge::{Badge, BadgeSize, BadgeTone};
```

- [ ] **Step 5: Add Badge v1 CSS**

Find any existing `.badge` block; replace with this section.

```css
/* === Badge (v1) === */
.badge {
  display: inline-flex;
  align-items: center;
  gap: var(--space-1);
  border-radius: var(--radius-shadcn-sm);
  padding: 0 var(--space-2);
  font-family: var(--font-body);
  font-size: var(--text-xs);
  line-height: 1;
  font-weight: 500;
  border: 1px solid transparent;
  white-space: nowrap;
}
.badge-sm { height: 18px; font-size: 11px; }
.badge-md { height: 22px; }

.badge-neutral { background: var(--surface-muted);                color: var(--color-text);          border-color: var(--color-rule); }
.badge-primary { background: var(--color-green-100);              color: var(--color-green-800);     border-color: var(--color-green-200); }
.badge-info    { background: var(--color-navy-100);               color: var(--color-navy-800);      border-color: var(--color-navy-200); }
.badge-success { background: var(--color-success-100);            color: var(--color-success-800);   border-color: var(--color-success-200); }
.badge-warning { background: var(--color-warning-100);            color: var(--color-warning-800);   border-color: var(--color-warning-200); }
.badge-danger  { background: var(--color-danger-100);             color: var(--color-danger-800);    border-color: var(--color-danger-200); }
.badge-live    { background: var(--color-oxblood-100);            color: var(--color-oxblood-800);   border-color: var(--color-oxblood-200); }
.badge-premium { background: var(--color-gold-100);               color: var(--color-gold-900);      border-color: var(--color-gold-300); }

.badge-pulse {
  display: inline-block;
  width: 6px; height: 6px;
  border-radius: var(--radius-full);
  background: var(--accent-live);
  animation: ds-pulse 1.4s ease-in-out infinite;
}
/* === End Badge === */
```

- [ ] **Step 6: Mirror CSS**

```bash
cp crates/design-system/assets/components.css crates/shell-web/public/assets/components.css
```

- [ ] **Step 7: Test + compile gate**

```bash
cargo test -p design-system --lib badge::tests
cargo check -p shell-web
```

- [ ] **Step 8: Commit**

```bash
git add crates/design-system/src/badge.rs crates/design-system/src/lib.rs crates/design-system/assets/components.css crates/shell-web/public/assets/components.css
git commit -m "feat(design-system): Badge v1 — premium, primary, size, live pulse dot"
```

---

### Task 7: Table toolbar + sortable header cell + striped

**Files:**
- Modify: `crates/design-system/src/table.rs`
- Modify: `crates/design-system/assets/components.css`
- Mirror: `crates/shell-web/public/assets/components.css`

Add a `TableToolbar` subcomponent for above-header toolbar (search/filter chips), a `TableHeaderCell` with optional `sortable` + `sort_dir` props (chevron rotates 180° between asc/desc), and a `striped` prop on `Table`.

- [ ] **Step 1: Write failing tests**

Append to `crates/design-system/src/table.rs` `mod tests`:

```rust
    #[test]
    fn table_striped_renders_class() {
        fn app() -> Element {
            rsx! {
                Table {
                    striped: true,
                    head: rsx! { tr { th { "A" } } },
                    body: rsx! { tr { td { "1" } } },
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("ds-table--striped"), "striped class missing: {html}");
    }

    #[test]
    fn table_toolbar_renders_above_head() {
        fn app() -> Element {
            rsx! {
                Table {
                    toolbar: Some(rsx! { div { "filters here" } }),
                    head: rsx! { tr { th { "A" } } },
                    body: rsx! { tr { td { "1" } } },
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("ds-table-toolbar"), "toolbar wrap missing: {html}");
        assert!(html.contains("filters here"));
    }

    #[test]
    fn table_header_cell_sortable_emits_aria() {
        fn app() -> Element {
            rsx! {
                TableHeaderCell {
                    sortable: true,
                    sort_dir: Some(SortDir::Asc),
                    on_sort: |_| {},
                    "Name"
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("aria-sort=\"ascending\""), "aria-sort missing: {html}");
        assert!(html.contains("ds-table-sort-chevron"), "chevron marker missing: {html}");
    }
```

- [ ] **Step 2: Run + confirm failure**

```bash
cargo test -p design-system --lib table::tests
```

- [ ] **Step 3: Replace `crates/design-system/src/table.rs`**

```rust
use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct TableProps {
    pub head: Element,
    pub body: Element,
    #[props(default)]
    pub compact: bool,
    #[props(default)]
    pub sticky_header: bool,
    #[props(default)]
    pub striped: bool,
    /// Optional toolbar slot rendered above the <thead> inside the table card.
    #[props(default)]
    pub toolbar: Option<Element>,
}

#[component]
pub fn Table(props: TableProps) -> Element {
    let mut class = String::from("ds-table");
    if props.compact       { class.push_str(" ds-table--compact"); }
    if props.sticky_header { class.push_str(" ds-table--sticky-head"); }
    if props.striped       { class.push_str(" ds-table--striped"); }
    rsx! {
        div { class: "ds-table-shell",
            if let Some(toolbar) = props.toolbar {
                div { class: "ds-table-toolbar", {toolbar} }
            }
            table { class: "{class}",
                thead { {props.head} }
                tbody { {props.body} }
            }
        }
    }
}

#[derive(Clone, PartialEq)]
pub enum SortDir { Asc, Desc }

#[derive(Props, Clone, PartialEq)]
pub struct TableHeaderCellProps {
    pub children: Element,
    #[props(default)]
    pub sortable: bool,
    #[props(default)]
    pub sort_dir: Option<SortDir>,
    /// Fired when the header is clicked (sortable cells only).
    #[props(default)]
    pub on_sort: Option<EventHandler<MouseEvent>>,
}

#[component]
pub fn TableHeaderCell(props: TableHeaderCellProps) -> Element {
    let aria_sort = match props.sort_dir {
        Some(SortDir::Asc)  => "ascending",
        Some(SortDir::Desc) => "descending",
        None                => "none",
    };
    let chevron_class = match props.sort_dir {
        Some(SortDir::Desc) => "ds-table-sort-chevron ds-table-sort-chevron--desc",
        _                   => "ds-table-sort-chevron",
    };
    if props.sortable {
        rsx! {
            th { class: "ds-table-th ds-table-th--sortable", "aria-sort": "{aria_sort}",
                button {
                    r#type: "button",
                    class: "ds-table-sort-btn",
                    onclick: move |evt| {
                        if let Some(h) = &props.on_sort { h.call(evt); }
                    },
                    {props.children}
                    span { class: "{chevron_class}", "aria-hidden": "true", "▾" }
                }
            }
        }
    } else {
        rsx! { th { class: "ds-table-th", {props.children} } }
    }
}
```

(Existing tests must still pass; they assert `ds-table`, `ds-table--compact`, `ds-table--sticky-head`, plus the new ones.)

- [ ] **Step 4: Export new symbols from `lib.rs`**

In `crates/design-system/src/lib.rs`, replace `pub use table::{Table, TableProps};` with:

```rust
pub use table::{SortDir, Table, TableHeaderCell, TableHeaderCellProps, TableProps};
```

- [ ] **Step 5: Add Table v1 CSS**

Find any existing `.ds-table` block; replace with this section.

```css
/* === Table (v1) === */
.ds-table-shell {
  background: var(--surface-card);
  border-radius: var(--radius-shadcn-lg);
  box-shadow: var(--shadow-flat);
  overflow: hidden;
}
.ds-table-toolbar {
  display: flex; align-items: center; gap: var(--space-3);
  padding: var(--space-3) var(--space-4);
  border-bottom: 1px solid var(--color-rule);
  background: var(--surface-muted);
}
.ds-table {
  width: 100%;
  border-collapse: separate;
  border-spacing: 0;
  font-size: var(--text-sm);
}
.ds-table thead th {
  position: relative;
  text-align: left;
  font-weight: 500;
  color: var(--color-text-muted);
  height: 40px;
  padding: 0 var(--space-4);
  border-bottom: 1px solid var(--color-rule);
  background: var(--surface-card);
  white-space: nowrap;
}
.ds-table--sticky-head thead th { position: sticky; top: 0; z-index: 1; }
.ds-table tbody td {
  height: 40px;
  padding: 0 var(--space-4);
  border-bottom: 1px solid var(--color-rule);
  color: var(--color-text);
}
.ds-table tbody tr:last-child td { border-bottom: 0; }
.ds-table tbody tr:hover td { background: var(--surface-card-hover); }
.ds-table--striped tbody tr:nth-child(odd) td { background: var(--surface-muted); }
.ds-table--compact thead th,
.ds-table--compact tbody td { height: 32px; padding: 0 var(--space-3); }

.ds-table-th--sortable { padding: 0; }
.ds-table-sort-btn {
  display: inline-flex;
  align-items: center;
  gap: var(--space-1);
  width: 100%;
  height: 100%;
  padding: 0 var(--space-4);
  background: transparent;
  border: 0;
  color: inherit;
  font: inherit;
  cursor: pointer;
  text-align: left;
}
.ds-table-sort-btn:hover { background: var(--surface-card-hover); }
.ds-table-sort-chevron {
  display: inline-block;
  transition: transform var(--duration-medium) var(--ease-snappy);
  font-size: 10px;
}
.ds-table-sort-chevron--desc { transform: rotate(180deg); }
/* === End Table === */
```

- [ ] **Step 6: Mirror CSS**

```bash
cp crates/design-system/assets/components.css crates/shell-web/public/assets/components.css
```

- [ ] **Step 7: Test + compile gate**

```bash
cargo test -p design-system --lib table::tests
cargo check -p shell-web
```

- [ ] **Step 8: Commit**

```bash
git add crates/design-system/src/table.rs crates/design-system/src/lib.rs crates/design-system/assets/components.css crates/shell-web/public/assets/components.css
git commit -m "feat(design-system): Table v1 — toolbar, sortable header, striped, sticky polish"
```

---

### Task 8: Tabs underline + pill variant + keyboard

**Files:**
- Modify: `crates/design-system/src/tabs.rs`
- Modify: `crates/design-system/assets/components.css`
- Mirror: `crates/shell-web/public/assets/components.css`

Add a `variant` prop (`Underline`/`Pill`) with `Underline` as default, ARIA role attributes, and keyboard arrow navigation. The gold underline slide is done with a per-tab `aria-selected="true"` rule (a single rule paints the underline; no JS positioning needed for v1).

- [ ] **Step 1: Write failing tests**

Append to `crates/design-system/src/tabs.rs` `mod tests`:

```rust
    #[test]
    fn tabs_pill_variant_renders_class() {
        fn app() -> Element {
            rsx! {
                Tabs {
                    variant: TabsVariant::Pill,
                    tabs: vec![Tab { key: "a".to_string(), label: "A".to_string(), ..Default::default() }],
                    active: "a".to_string(),
                    on_change: |_: String| {},
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("ds-tabs--pill"), "pill class missing: {html}");
    }

    #[test]
    fn tabs_emit_tablist_role() {
        fn app() -> Element {
            rsx! {
                Tabs {
                    tabs: vec![Tab { key: "a".to_string(), label: "A".to_string(), ..Default::default() }],
                    active: "a".to_string(),
                    on_change: |_: String| {},
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("role=\"tablist\""), "tablist role missing: {html}");
        assert!(html.contains("role=\"tab\""),    "tab role missing: {html}");
        assert!(html.contains("aria-selected=\"true\""), "aria-selected missing: {html}");
    }
```

- [ ] **Step 2: Run + confirm failure**

```bash
cargo test -p design-system --lib tabs::tests
```

- [ ] **Step 3: Replace `crates/design-system/src/tabs.rs`**

```rust
use dioxus::prelude::*;

#[derive(Clone, PartialEq, Default)]
pub struct Tab {
    pub key: String,
    pub label: String,
    pub disabled: bool,
}

#[derive(Clone, PartialEq, Default)]
pub enum TabsVariant {
    #[default]
    Underline,
    Pill,
}

#[derive(Props, Clone, PartialEq)]
pub struct TabsProps {
    pub tabs: Vec<Tab>,
    pub active: String,
    pub on_change: EventHandler<String>,
    #[props(default)]
    pub variant: TabsVariant,
}

#[component]
pub fn Tabs(props: TabsProps) -> Element {
    let variant_class = match props.variant {
        TabsVariant::Underline => "ds-tabs ds-tabs--underline",
        TabsVariant::Pill => "ds-tabs ds-tabs--pill",
    };
    // Build an index map for arrow-key navigation: pressed key advances to next enabled tab.
    let enabled_keys: Vec<String> = props.tabs.iter().filter(|t| !t.disabled).map(|t| t.key.clone()).collect();
    rsx! {
        div { class: "{variant_class}", role: "tablist",
            for tab in &props.tabs {
                {
                    let key = tab.key.clone();
                    let key_for_class = tab.key.clone();
                    let label = tab.label.clone();
                    let disabled = tab.disabled;
                    let active = props.active.clone();
                    let is_active = active == key_for_class;
                    let handler = props.on_change.clone();
                    let handler_for_keys = props.on_change.clone();
                    let enabled = enabled_keys.clone();
                    let key_for_keys = tab.key.clone();
                    rsx! {
                        button {
                            class: if is_active { "ds-tab ds-tab--active" } else { "ds-tab" },
                            role: "tab",
                            r#type: "button",
                            "aria-selected": if is_active { "true" } else { "false" },
                            tabindex: if is_active { "0" } else { "-1" },
                            disabled,
                            onclick: move |_| handler.call(key.clone()),
                            onkeydown: move |evt| {
                                let data = evt.data();
                                let key_str = data.key().to_string();
                                let mut idx = enabled.iter().position(|k| k == &key_for_keys).unwrap_or(0);
                                if key_str == "ArrowRight" || key_str == "ArrowDown" {
                                    idx = (idx + 1) % enabled.len().max(1);
                                    if let Some(k) = enabled.get(idx) { handler_for_keys.call(k.clone()); }
                                } else if key_str == "ArrowLeft" || key_str == "ArrowUp" {
                                    idx = if idx == 0 { enabled.len().saturating_sub(1) } else { idx - 1 };
                                    if let Some(k) = enabled.get(idx) { handler_for_keys.call(k.clone()); }
                                } else if key_str == "Home" {
                                    if let Some(k) = enabled.first() { handler_for_keys.call(k.clone()); }
                                } else if key_str == "End" {
                                    if let Some(k) = enabled.last() { handler_for_keys.call(k.clone()); }
                                }
                            },
                            "{label}"
                        }
                    }
                }
            }
        }
    }
}
```

- [ ] **Step 4: Export `TabsVariant` from `lib.rs`**

Replace `pub use tabs::{Tab, Tabs};` with:

```rust
pub use tabs::{Tab, Tabs, TabsVariant};
```

- [ ] **Step 5: Add Tabs v1 CSS**

Find any existing `.tabs` or `.ds-tabs` block; replace with this section.

```css
/* === Tabs (v1) === */
.ds-tabs {
  display: inline-flex;
  align-items: stretch;
  gap: var(--space-1);
  position: relative;
}
.ds-tabs--underline { border-bottom: 1px solid var(--color-rule); }
.ds-tab {
  display: inline-flex; align-items: center;
  height: 36px;
  padding: 0 var(--space-3);
  background: transparent;
  border: 0;
  color: var(--color-text-muted);
  font: inherit;
  font-size: var(--text-sm);
  font-weight: 500;
  cursor: pointer;
  position: relative;
  transition: color var(--duration-fast) var(--ease-snappy), background-color var(--duration-fast) var(--ease-snappy);
}
.ds-tab:hover:not(:disabled) { color: var(--color-text); }
.ds-tab:disabled { color: var(--color-text-muted); opacity: 0.5; cursor: not-allowed; }
.ds-tab:focus-visible { outline: none; box-shadow: var(--ring); border-radius: var(--radius-shadcn-md); }

/* Underline variant — gold 1px slider drawn via ::after on active */
.ds-tabs--underline .ds-tab::after {
  content: "";
  position: absolute;
  left: var(--space-3); right: var(--space-3); bottom: -1px;
  height: 1px;
  background: var(--accent-premium);
  transform: scaleX(0);
  transform-origin: left center;
  transition: transform var(--duration-medium) var(--ease-content);
}
.ds-tabs--underline .ds-tab--active { color: var(--color-text); }
.ds-tabs--underline .ds-tab--active::after { transform: scaleX(1); }

/* Pill variant — rounded background under active */
.ds-tabs--pill { padding: var(--space-1); background: var(--surface-muted); border-radius: var(--radius-shadcn-md); }
.ds-tabs--pill .ds-tab { border-radius: var(--radius-shadcn-sm); }
.ds-tabs--pill .ds-tab--active {
  background: var(--surface-card);
  color: var(--color-text);
  box-shadow: var(--shadow-flat);
}
/* === End Tabs === */
```

- [ ] **Step 6: Mirror CSS**

```bash
cp crates/design-system/assets/components.css crates/shell-web/public/assets/components.css
```

- [ ] **Step 7: Test + compile gate**

```bash
cargo test -p design-system --lib tabs::tests
cargo check -p shell-web
```

- [ ] **Step 8: Commit**

```bash
git add crates/design-system/src/tabs.rs crates/design-system/src/lib.rs crates/design-system/assets/components.css crates/shell-web/public/assets/components.css
git commit -m "feat(design-system): Tabs v1 — pill variant, gold underline, keyboard nav, ARIA"
```

---

### Task 9: DropdownMenu (new primitive)

**Files:**
- Create: `crates/design-system/src/dropdown_menu.rs`
- Modify: `crates/design-system/src/lib.rs`
- Modify: `crates/design-system/assets/components.css`
- Mirror: `crates/shell-web/public/assets/components.css`

A composition-style menu: caller owns the open state (a `Signal<bool>` passed in); the component renders the trigger button, an absolutely-positioned content panel, and styled items. v1 uses simple `align="start" | "end"` (right- or left-aligned to trigger); no floating-ui calculations.

- [ ] **Step 1: Create `crates/design-system/src/dropdown_menu.rs` with tests + implementation in one shot**

The pattern in this codebase is "module-first"; SSR tests live inline. Since there is no existing dropdown to test against, write tests for the behaviour we want then implement to satisfy.

```rust
use dioxus::prelude::*;

/// Where the content aligns relative to the trigger.
#[derive(Clone, PartialEq, Default)]
pub enum DropdownAlign {
    #[default]
    Start, // left edge of content aligns with left edge of trigger
    End,   // right edge aligns
}

#[derive(Props, Clone, PartialEq)]
pub struct DropdownMenuProps {
    /// Open state. Caller owns it (e.g. `let open = use_signal(|| false);`).
    pub open: Signal<bool>,
    /// Trigger element (button, etc.). Click handler is attached by caller.
    pub trigger: Element,
    /// Menu content (typically <DropdownMenuItem ...>).
    pub children: Element,
    #[props(default)]
    pub align: DropdownAlign,
}

#[component]
pub fn DropdownMenu(props: DropdownMenuProps) -> Element {
    let is_open = *props.open.read();
    let align_class = match props.align {
        DropdownAlign::Start => "ds-dropdown-content--start",
        DropdownAlign::End => "ds-dropdown-content--end",
    };
    let state = if is_open { "open" } else { "closed" };
    rsx! {
        div { class: "ds-dropdown",
            {props.trigger}
            if is_open {
                div {
                    role: "menu",
                    class: "ds-dropdown-content {align_class}",
                    "data-state": "{state}",
                    {props.children}
                }
            }
        }
    }
}

#[derive(Clone, PartialEq, Default)]
pub enum DropdownItemTone {
    #[default]
    Default,
    Danger,
}

#[derive(Props, Clone, PartialEq)]
pub struct DropdownMenuItemProps {
    pub label: String,
    pub on_select: EventHandler<MouseEvent>,
    #[props(default)]
    pub tone: DropdownItemTone,
    #[props(default)]
    pub disabled: bool,
    /// Optional icon glyph (rendered before the label).
    #[props(default)]
    pub leading_icon: Option<Element>,
}

#[component]
pub fn DropdownMenuItem(props: DropdownMenuItemProps) -> Element {
    let tone_class = match props.tone {
        DropdownItemTone::Default => "",
        DropdownItemTone::Danger => " ds-dropdown-item--danger",
    };
    let class = format!("ds-dropdown-item{tone_class}");
    rsx! {
        button {
            role: "menuitem",
            r#type: "button",
            class: "{class}",
            disabled: props.disabled,
            onclick: move |evt| props.on_select.call(evt),
            if let Some(icon) = &props.leading_icon {
                span { class: "ds-dropdown-item-icon", {icon.clone()} }
            }
            "{props.label}"
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct DropdownMenuLabelProps { pub children: Element }
#[component]
pub fn DropdownMenuLabel(props: DropdownMenuLabelProps) -> Element {
    rsx! { div { class: "ds-dropdown-label", {props.children} } }
}

#[component]
pub fn DropdownMenuSeparator() -> Element {
    rsx! { hr { class: "ds-dropdown-separator" } }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropdown_hidden_when_closed() {
        fn app() -> Element {
            let open = use_signal(|| false);
            rsx! {
                DropdownMenu {
                    open,
                    trigger: rsx! { button { "x" } },
                    DropdownMenuItem { label: "a".to_string(), on_select: |_| {} }
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(!html.contains("ds-dropdown-content"), "content should not render when closed: {html}");
    }

    #[test]
    fn dropdown_renders_content_when_open() {
        fn app() -> Element {
            let open = use_signal(|| true);
            rsx! {
                DropdownMenu {
                    open,
                    trigger: rsx! { button { "x" } },
                    DropdownMenuItem { label: "Sign out".to_string(), on_select: |_| {} }
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("ds-dropdown-content"), "content missing: {html}");
        assert!(html.contains("Sign out"));
        assert!(html.contains("data-state=\"open\""));
    }

    #[test]
    fn dropdown_align_end_class() {
        fn app() -> Element {
            let open = use_signal(|| true);
            rsx! {
                DropdownMenu {
                    open,
                    align: DropdownAlign::End,
                    trigger: rsx! { button { "x" } },
                    DropdownMenuItem { label: "a".to_string(), on_select: |_| {} }
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("ds-dropdown-content--end"), "end alignment missing: {html}");
    }

    #[test]
    fn dropdown_item_danger_tone_class() {
        fn app() -> Element {
            let open = use_signal(|| true);
            rsx! {
                DropdownMenu {
                    open,
                    trigger: rsx! { button { "x" } },
                    DropdownMenuItem { label: "Delete".to_string(), tone: DropdownItemTone::Danger, on_select: |_| {} }
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("ds-dropdown-item--danger"), "danger class missing: {html}");
    }
}
```

- [ ] **Step 2: Register the module in `lib.rs`**

In `crates/design-system/src/lib.rs`, add the module declaration alphabetically near the existing modules:

```rust
pub mod dropdown_menu;
```

And add the re-export line:

```rust
pub use dropdown_menu::{
    DropdownAlign, DropdownItemTone, DropdownMenu, DropdownMenuItem,
    DropdownMenuLabel, DropdownMenuSeparator,
};
```

- [ ] **Step 3: Add DropdownMenu CSS**

Append to `crates/design-system/assets/components.css`:

```css
/* === Dropdown Menu (v1, new) === */
.ds-dropdown { position: relative; display: inline-block; }
.ds-dropdown-content {
  position: absolute;
  top: calc(100% + 4px);
  min-width: 180px;
  padding: var(--space-1);
  background: var(--surface-popover);
  border: 1px solid var(--color-rule);
  border-radius: var(--radius-shadcn-md);
  box-shadow: var(--shadow-lg);
  z-index: 50;
  animation: ds-fade-in var(--duration-enter) var(--ease-content);
}
.ds-dropdown-content[data-state="open"]   { animation: ds-slide-in-top  var(--duration-enter) var(--ease-content), ds-fade-in var(--duration-enter) var(--ease-content); }
.ds-dropdown-content[data-state="closed"] { animation: ds-slide-out-top var(--duration-exit)  var(--ease-content), ds-fade-out var(--duration-exit)  var(--ease-content); }
.ds-dropdown-content--start { left: 0; }
.ds-dropdown-content--end   { right: 0; }

.ds-dropdown-item {
  display: flex; align-items: center; gap: var(--space-2);
  width: 100%;
  padding: var(--space-2) var(--space-3);
  background: transparent;
  border: 0;
  text-align: left;
  font: inherit;
  font-size: var(--text-sm);
  color: var(--color-text);
  border-radius: var(--radius-shadcn-sm);
  cursor: pointer;
}
.ds-dropdown-item:hover:not(:disabled),
.ds-dropdown-item:focus-visible {
  background: var(--surface-muted);
  outline: none;
}
.ds-dropdown-item:disabled { opacity: 0.5; cursor: not-allowed; }
.ds-dropdown-item--danger { color: var(--color-danger-700); }
.ds-dropdown-item--danger:hover:not(:disabled) { background: var(--color-danger-50); }

.ds-dropdown-label {
  padding: var(--space-2) var(--space-3);
  font-size: var(--text-xs);
  text-transform: uppercase;
  letter-spacing: 0.04em;
  color: var(--color-text-muted);
}
.ds-dropdown-separator {
  border: 0;
  border-top: 1px solid var(--color-rule);
  margin: var(--space-1) 0;
}
.ds-dropdown-item-icon { display: inline-flex; }
/* === End Dropdown Menu === */
```

- [ ] **Step 4: Mirror CSS**

```bash
cp crates/design-system/assets/components.css crates/shell-web/public/assets/components.css
```

- [ ] **Step 5: Test + compile gate**

```bash
cargo test -p design-system --lib dropdown_menu::tests
cargo check -p shell-web
```

- [ ] **Step 6: Commit**

```bash
git add crates/design-system/src/dropdown_menu.rs crates/design-system/src/lib.rs crates/design-system/assets/components.css crates/shell-web/public/assets/components.css
git commit -m "feat(design-system): DropdownMenu v1 (new) — trigger/content/item/label/separator + ARIA"
```

---

### Task 10: Sheet (new primitive)

**Files:**
- Create: `crates/design-system/src/sheet.rs`
- Modify: `crates/design-system/src/lib.rs`
- Modify: `crates/design-system/assets/components.css`
- Mirror: `crates/shell-web/public/assets/components.css`

Side drawer with Escape + backdrop click + close-button to close. Caller owns the open state. Slot subcomponents for Header/Title/Description/Body/Footer.

- [ ] **Step 1: Create `crates/design-system/src/sheet.rs`**

```rust
use dioxus::prelude::*;

#[derive(Clone, PartialEq, Default)]
pub enum SheetSide {
    #[default]
    Right,
    Left,
    Top,
    Bottom,
}

#[derive(Props, Clone, PartialEq)]
pub struct SheetProps {
    pub open: Signal<bool>,
    pub children: Element,
    #[props(default)]
    pub side: SheetSide,
    /// Width in pixels for left/right sheets (default 440). Ignored for top/bottom.
    #[props(default = 440)]
    pub width: u32,
    /// Height in pixels for top/bottom sheets (default 320). Ignored for left/right.
    #[props(default = 320)]
    pub height: u32,
}

#[component]
pub fn Sheet(mut props: SheetProps) -> Element {
    let is_open = *props.open.read();
    if !is_open {
        return rsx! {};
    }
    let side_class = match props.side {
        SheetSide::Right => "ds-sheet--right",
        SheetSide::Left  => "ds-sheet--left",
        SheetSide::Top   => "ds-sheet--top",
        SheetSide::Bottom => "ds-sheet--bottom",
    };
    let inline_style = match props.side {
        SheetSide::Right | SheetSide::Left => format!("width: {}px;", props.width),
        SheetSide::Top   | SheetSide::Bottom => format!("height: {}px;", props.height),
    };
    let mut open_signal = props.open;
    rsx! {
        div { class: "ds-sheet-backdrop",
            "data-state": "open",
            onclick: move |_| { open_signal.set(false); },
            // Panel; clicks inside should NOT close.
            div {
                role: "dialog",
                "aria-modal": "true",
                class: "ds-sheet {side_class}",
                style: "{inline_style}",
                "data-state": "open",
                onclick: move |evt| { evt.stop_propagation(); },
                onkeydown: move |evt| {
                    if evt.data().key().to_string() == "Escape" {
                        open_signal.set(false);
                    }
                },
                tabindex: "-1",
                {props.children}
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct SheetSlotProps { pub children: Element }

#[component]
pub fn SheetHeader(props: SheetSlotProps) -> Element {
    rsx! { div { class: "ds-sheet-header", {props.children} } }
}
#[component]
pub fn SheetTitle(props: SheetSlotProps) -> Element {
    rsx! { h2 { class: "ds-sheet-title", {props.children} } }
}
#[component]
pub fn SheetDescription(props: SheetSlotProps) -> Element {
    rsx! { p { class: "ds-sheet-description", {props.children} } }
}
#[component]
pub fn SheetBody(props: SheetSlotProps) -> Element {
    rsx! { div { class: "ds-sheet-body", {props.children} } }
}
#[component]
pub fn SheetFooter(props: SheetSlotProps) -> Element {
    rsx! { div { class: "ds-sheet-footer", {props.children} } }
}

#[derive(Props, Clone, PartialEq)]
pub struct SheetCloseProps {
    pub open: Signal<bool>,
    #[props(default = "Close".to_string())]
    pub label: String,
}

#[component]
pub fn SheetClose(props: SheetCloseProps) -> Element {
    let mut open = props.open;
    rsx! {
        button {
            r#type: "button",
            class: "ds-sheet-close",
            "aria-label": "{props.label}",
            onclick: move |_| { open.set(false); },
            "×"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sheet_renders_nothing_when_closed() {
        fn app() -> Element {
            let open = use_signal(|| false);
            rsx! { Sheet { open, "body" } }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(!html.contains("ds-sheet-backdrop"), "should not render when closed: {html}");
    }

    #[test]
    fn sheet_renders_panel_when_open() {
        fn app() -> Element {
            let open = use_signal(|| true);
            rsx! { Sheet { open, SheetHeader { SheetTitle { "Edit" } } } }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("ds-sheet-backdrop"));
        assert!(html.contains("ds-sheet--right"), "default side missing: {html}");
        assert!(html.contains("ds-sheet-title"));
        assert!(html.contains("aria-modal=\"true\""));
    }

    #[test]
    fn sheet_left_side_class() {
        fn app() -> Element {
            let open = use_signal(|| true);
            rsx! { Sheet { open, side: SheetSide::Left, "body" } }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("ds-sheet--left"), "left class missing: {html}");
    }
}
```

- [ ] **Step 2: Register in `lib.rs`**

In `crates/design-system/src/lib.rs`, add:

```rust
pub mod sheet;
```

And the re-export line:

```rust
pub use sheet::{
    Sheet, SheetBody, SheetClose, SheetDescription, SheetFooter, SheetHeader, SheetSide, SheetTitle,
};
```

- [ ] **Step 3: Add Sheet CSS**

Append to `crates/design-system/assets/components.css`:

```css
/* === Sheet (v1, new) === */
.ds-sheet-backdrop {
  position: fixed; inset: 0;
  background: var(--surface-overlay);
  display: flex;
  z-index: 60;
  animation: ds-fade-in var(--duration-medium) var(--ease-content);
}
.ds-sheet {
  position: relative;
  background: var(--surface-card);
  box-shadow: var(--shadow-2xl);
  display: flex;
  flex-direction: column;
}
.ds-sheet--right  { margin-left: auto; height: 100%; animation: ds-sheet-in-right  var(--duration-slow) var(--ease-content); }
.ds-sheet--left   { margin-right: auto; height: 100%; animation: ds-sheet-in-left  var(--duration-slow) var(--ease-content); }
.ds-sheet--top    { margin-bottom: auto; width: 100%; animation: ds-slide-in-top   var(--duration-slow) var(--ease-content); }
.ds-sheet--bottom { margin-top: auto;    width: 100%; animation: ds-slide-in-bottom var(--duration-slow) var(--ease-content); }

.ds-sheet-header {
  padding: var(--space-5);
  border-bottom: 1px solid var(--color-rule);
  display: flex; flex-direction: column; gap: var(--space-1);
}
.ds-sheet-title {
  margin: 0;
  font-family: var(--font-display);
  font-size: var(--text-2xl);
  line-height: var(--leading-2xl);
  font-weight: 600;
  color: var(--color-text);
}
.ds-sheet-description {
  margin: 0;
  color: var(--color-text-muted);
  font-size: var(--text-sm);
}
.ds-sheet-body {
  flex: 1 1 auto;
  padding: var(--space-5);
  overflow-y: auto;
}
.ds-sheet-footer {
  padding: var(--space-4) var(--space-5);
  border-top: 1px solid var(--color-rule);
  display: flex; justify-content: flex-end; gap: var(--space-3);
}
.ds-sheet-close {
  position: absolute;
  top: var(--space-3);
  right: var(--space-3);
  width: 28px; height: 28px;
  border: 0;
  background: transparent;
  font-size: 20px;
  line-height: 1;
  color: var(--color-text-muted);
  cursor: pointer;
  border-radius: var(--radius-shadcn-sm);
}
.ds-sheet-close:hover { background: var(--surface-muted); color: var(--color-text); }
.ds-sheet-close:focus-visible { outline: none; box-shadow: var(--ring); }
/* === End Sheet === */
```

- [ ] **Step 4: Mirror CSS**

```bash
cp crates/design-system/assets/components.css crates/shell-web/public/assets/components.css
```

- [ ] **Step 5: Test + compile gate**

```bash
cargo test -p design-system --lib sheet::tests
cargo check -p shell-web
```

- [ ] **Step 6: Commit**

```bash
git add crates/design-system/src/sheet.rs crates/design-system/src/lib.rs crates/design-system/assets/components.css crates/shell-web/public/assets/components.css
git commit -m "feat(design-system): Sheet v1 (new) — sides, slots, escape/backdrop close"
```

---

### Task 11: Skeleton shimmer

**Files:**
- Modify: `crates/design-system/assets/components.css`
- Mirror: `crates/shell-web/public/assets/components.css`

The Rust `skeleton.rs` already emits `ds-skeleton` + role classes. Only the CSS needs to gain the shimmer.

- [ ] **Step 1: Replace any existing `.ds-skeleton` block with the v1 shimmer block**

In `crates/design-system/assets/components.css`, find any existing `.ds-skeleton` rule (likely a flat background-color). Replace with:

```css
/* === Skeleton (v1) === */
.ds-skeleton {
  display: inline-block;
  background:
    linear-gradient(
      90deg,
      var(--color-neutral-200) 0%,
      var(--color-neutral-100) 50%,
      var(--color-neutral-200) 100%
    );
  background-size: 200% 100%;
  border-radius: var(--radius-shadcn-sm);
  animation: ds-shimmer 1.4s linear infinite;
}
.ds-skeleton-line   { display: block; }
.ds-skeleton-circle { border-radius: var(--radius-full); }
.ds-skeleton-card   { width: 100%; border-radius: var(--radius-shadcn-lg); }
.ds-skeleton-table-row td > .ds-skeleton-line { height: 14px; }
/* === End Skeleton === */
```

- [ ] **Step 2: Mirror CSS**

```bash
cp crates/design-system/assets/components.css crates/shell-web/public/assets/components.css
```

- [ ] **Step 3: Test + compile gate**

```bash
cargo test -p design-system --lib skeleton::tests
cargo check -p shell-web
```

(Existing tests assert `ds-skeleton`, `ds-skeleton-line`, etc. classes are present in HTML — Rust file is unchanged so they still pass.)

- [ ] **Step 4: Commit**

```bash
git add crates/design-system/assets/components.css crates/shell-web/public/assets/components.css
git commit -m "feat(design-system): Skeleton v1 — shimmer animation"
```

---

### Task 12: Toast position + premium tone + motion

**Files:**
- Modify: `crates/design-system/src/toast.rs`
- Modify: `crates/design-system/src/lib.rs`
- Modify: `crates/design-system/assets/components.css`
- Mirror: `crates/shell-web/public/assets/components.css`

Adds `ToastLevel::Premium`, a `ToastPosition` enum, and a position prop on `ToastProvider` (defaulted to `BottomRight` to match current behaviour). Enter/exit animations driven by the motion catalog keyframes.

- [ ] **Step 1: Write failing tests**

Append to `crates/design-system/src/toast.rs` `mod tests`:

```rust
    #[test]
    fn viewport_renders_premium_level_class() {
        fn app() -> Element {
            let _ = use_context_provider(|| {
                Signal::new(vec![ToastEntry {
                    id: 1,
                    level: ToastLevel::Premium,
                    title: "Pro unlocked".to_string(),
                    message: "Enjoy 30 days free".to_string(),
                    duration_ms: None,
                }])
            });
            rsx! { ToastViewport {} }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("ds-toast--premium"), "premium class missing: {html}");
    }

    #[test]
    fn viewport_has_position_class_top_right_when_provider_set() {
        fn app() -> Element {
            rsx! {
                ToastProvider { position: ToastPosition::TopRight,
                    div { "child" }
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("ds-toast-viewport--top-right"), "top-right class missing: {html}");
    }
```

- [ ] **Step 2: Run + confirm failure**

```bash
cargo test -p design-system --lib toast::tests
```

- [ ] **Step 3: Update `crates/design-system/src/toast.rs`**

Replace the file with this version (existing public API preserved; `ToastLevel::Premium` and `ToastPosition` added; viewport reads position from a separate context).

```rust
use dioxus::prelude::*;

#[derive(Clone, PartialEq)]
pub enum ToastLevel {
    Info,
    Success,
    Warning,
    Danger,
    Premium,
}

#[derive(Clone, PartialEq, Default)]
pub enum ToastPosition {
    #[default]
    BottomRight,
    TopRight,
    BottomCenter,
    TopCenter,
}

#[derive(Clone, PartialEq)]
pub struct ToastEntry {
    pub id: u64,
    pub level: ToastLevel,
    pub title: String,
    pub message: String,
    pub duration_ms: Option<u32>,
}

pub type ToastQueue = Vec<ToastEntry>;

fn next_toast_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    N.fetch_add(1, Ordering::Relaxed)
}

#[derive(Clone, Copy)]
pub struct ToastSender(pub Signal<ToastQueue>);

impl ToastSender {
    pub fn push(
        &mut self,
        level: ToastLevel,
        title: impl Into<String>,
        message: impl Into<String>,
    ) {
        let id = next_toast_id();
        let duration_ms = 4000u32;
        self.0.write().push(ToastEntry {
            id,
            level,
            title: title.into(),
            message: message.into(),
            duration_ms: Some(duration_ms),
        });

        #[cfg(target_arch = "wasm32")]
        {
            use wasm_bindgen::closure::Closure;
            use wasm_bindgen::JsCast;
            let mut sender = *self;
            let closure = Closure::once_into_js(move || {
                sender.dismiss(id);
            });
            if let Some(window) = web_sys::window() {
                let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(
                    closure.as_ref().unchecked_ref(),
                    duration_ms as i32,
                );
            }
        }
    }
    pub fn dismiss(&mut self, id: u64) {
        self.0.write().retain(|e| e.id != id);
    }
}

pub fn use_toast_sender() -> ToastSender {
    let signal = use_context::<Signal<ToastQueue>>();
    ToastSender(signal)
}

#[component]
pub fn ToastViewport() -> Element {
    let queue = use_context::<Signal<ToastQueue>>();
    let entries = queue.read().clone();
    // Try to read a position from context; default to BottomRight if absent.
    let position = try_use_context::<ToastPosition>().unwrap_or_default();
    let position_class = match position {
        ToastPosition::BottomRight  => "ds-toast-viewport--bottom-right",
        ToastPosition::TopRight     => "ds-toast-viewport--top-right",
        ToastPosition::BottomCenter => "ds-toast-viewport--bottom-center",
        ToastPosition::TopCenter    => "ds-toast-viewport--top-center",
    };
    rsx! {
        div { class: "ds-toast-viewport {position_class}",
            for entry in entries.iter() {
                {
                    let id = entry.id;
                    let level_class = match entry.level {
                        ToastLevel::Info     => "ds-toast--info",
                        ToastLevel::Success  => "ds-toast--success",
                        ToastLevel::Warning  => "ds-toast--warning",
                        ToastLevel::Danger   => "ds-toast--danger",
                        ToastLevel::Premium  => "ds-toast--premium",
                    };
                    let title = entry.title.clone();
                    let message = entry.message.clone();
                    rsx! {
                        div {
                            key: "{id}",
                            class: "ds-toast {level_class}",
                            role: "status",
                            div { class: "ds-toast-title", "{title}" }
                            div { class: "ds-toast-message", "{message}" }
                        }
                    }
                }
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct ToastProviderProps {
    pub children: Element,
    #[props(default)]
    pub position: ToastPosition,
}

#[component]
pub fn ToastProvider(props: ToastProviderProps) -> Element {
    use_context_provider(|| Signal::new(ToastQueue::new()));
    use_context_provider(|| props.position.clone());
    rsx! {
        {props.children}
        ToastViewport {}
    }
}

/// Helper: read a context value if it exists, otherwise return None.
fn try_use_context<T: 'static + Clone>() -> Option<T> {
    use dioxus::prelude::has_context;
    if has_context::<T>().is_some() {
        Some(use_context::<T>())
    } else {
        None
    }
}
```

NOTE: `dioxus::prelude::has_context` returns `Option<&T>` in Dioxus 0.7. If the helper above fails to compile, fall back to:

```rust
fn try_use_context<T: 'static + Clone>() -> Option<T> {
    dioxus::prelude::try_consume_context::<T>()
}
```

Whichever variant compiles in this Dioxus version is fine. (If neither works, the engineer can simplify by always calling `use_context::<ToastPosition>()` and providing a default at `ToastProvider`, since `ToastProvider` now always inserts a position.)

- [ ] **Step 4: Update `lib.rs` re-exports**

Replace the existing `pub use toast::{ ... }` line with:

```rust
pub use toast::{
    use_toast_sender, ToastEntry, ToastLevel, ToastPosition, ToastProvider, ToastQueue,
    ToastSender, ToastViewport,
};
```

- [ ] **Step 5: Add Toast v1 CSS**

Find existing `.ds-toast-viewport` / `.ds-toast` blocks; replace with this section.

```css
/* === Toast (v1) === */
.ds-toast-viewport {
  position: fixed;
  z-index: 70;
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  pointer-events: none;
  max-width: 380px;
}
.ds-toast-viewport--bottom-right  { bottom: var(--space-4); right: var(--space-4); }
.ds-toast-viewport--top-right     { top:    var(--space-4); right: var(--space-4); }
.ds-toast-viewport--bottom-center { bottom: var(--space-4); left: 50%; transform: translateX(-50%); }
.ds-toast-viewport--top-center    { top:    var(--space-4); left: 50%; transform: translateX(-50%); }

.ds-toast {
  pointer-events: auto;
  background: var(--surface-card);
  border: 1px solid var(--color-rule);
  border-radius: var(--radius-shadcn-md);
  box-shadow: var(--shadow-lg);
  padding: var(--space-3) var(--space-4);
  display: flex; flex-direction: column; gap: var(--space-1);
  min-width: 260px;
  animation: ds-slide-in-right var(--duration-medium) var(--ease-content), ds-fade-in var(--duration-medium) var(--ease-content);
}
.ds-toast-title   { font-weight: 600; font-size: var(--text-sm); color: var(--color-text); }
.ds-toast-message { font-size: var(--text-sm); color: var(--color-text-muted); }

.ds-toast--info    { border-left: 3px solid var(--accent-info); }
.ds-toast--success { border-left: 3px solid var(--color-success-500); }
.ds-toast--warning { border-left: 3px solid var(--color-warning-500); }
.ds-toast--danger  { border-left: 3px solid var(--accent-live); }
.ds-toast--premium { border-left: 3px solid var(--accent-premium); }
/* === End Toast === */
```

- [ ] **Step 6: Mirror CSS**

```bash
cp crates/design-system/assets/components.css crates/shell-web/public/assets/components.css
```

- [ ] **Step 7: Test + compile gate**

```bash
cargo test -p design-system --lib toast::tests
cargo check -p shell-web
```

- [ ] **Step 8: Commit**

```bash
git add crates/design-system/src/toast.rs crates/design-system/src/lib.rs crates/design-system/assets/components.css crates/shell-web/public/assets/components.css
git commit -m "feat(design-system): Toast v1 — premium level, position prop, corner-out motion"
```

---

### Task 13: Kitchen-sink debug route `/dev/components`

**Files:**
- Create: `crates/shell-web/src/routes/dev_components.rs`
- Modify: `crates/shell-web/src/routes/mod.rs`
- Modify: `crates/shell-web/src/route_enum.rs`

The route renders every primitive in every variant/state. It is gated behind `cfg(debug_assertions)` so it never ships to release builds.

- [ ] **Step 1: Create `crates/shell-web/src/routes/dev_components.rs`**

```rust
//! Kitchen-sink page for visual QA of the design system. Debug builds only.
#![cfg(debug_assertions)]

use design_system::{
    Badge, BadgeSize, BadgeTone, Button, ButtonSize, ButtonVariant, Card, CardContent,
    CardDescription, CardFooter, CardHeader, CardTitle, CardVariant, DropdownAlign, DropdownMenu,
    DropdownMenuItem, DropdownMenuLabel, DropdownMenuSeparator, Field, Input, Sheet, SheetBody,
    SheetClose, SheetFooter, SheetHeader, SheetSide, SheetTitle, SkeletonCard, SkeletonCircle,
    SkeletonLine, SortDir, Tab, Table, TableHeaderCell, Tabs, TabsVariant,
};
use dioxus::prelude::*;

#[component]
pub fn DevComponents() -> Element {
    let mut active_tab = use_signal(|| "primary".to_string());
    let dropdown_open = use_signal(|| false);
    let sheet_open = use_signal(|| false);
    let mut input_val = use_signal(String::new);

    rsx! {
        div { class: "ds-page-enter", style: "max-width: 960px; margin: 32px auto; padding: 0 24px; display: grid; gap: 32px;",

            // --- Header section
            section { "data-stagger": "1",
                h1 { style: "font-family: var(--font-display); font-size: 36px; margin: 0 0 8px;", "Design System Kitchen Sink" }
                p { style: "color: var(--color-text-muted); margin: 0;", "All primitives, all variants, all states. Debug build only." }
            }

            // --- Buttons
            section { "data-stagger": "2",
                h2 { "Buttons" }
                div { style: "display: flex; gap: 8px; flex-wrap: wrap;",
                    Button { label: "Primary".to_string(),     on_click: |_| {} }
                    Button { label: "Secondary".to_string(),   variant: ButtonVariant::Secondary,   on_click: |_| {} }
                    Button { label: "Ghost".to_string(),       variant: ButtonVariant::Ghost,       on_click: |_| {} }
                    Button { label: "Destructive".to_string(), variant: ButtonVariant::Destructive, on_click: |_| {} }
                    Button { label: "Premium".to_string(),     variant: ButtonVariant::Premium,     on_click: |_| {} }
                    Button { label: "Link".to_string(),        variant: ButtonVariant::Link,        on_click: |_| {} }
                }
                div { style: "display: flex; gap: 8px; margin-top: 12px; align-items: center;",
                    Button { label: "Small".to_string(),  size: ButtonSize::Sm, on_click: |_| {} }
                    Button { label: "Medium".to_string(), size: ButtonSize::Md, on_click: |_| {} }
                    Button { label: "Large".to_string(),  size: ButtonSize::Lg, on_click: |_| {} }
                    Button { label: "Loading".to_string(), loading: true, on_click: |_| {} }
                    Button { label: "Disabled".to_string(), disabled: true, on_click: |_| {} }
                }
            }

            // --- Cards
            section { "data-stagger": "3",
                h2 { "Cards" }
                div { style: "display: grid; grid-template-columns: repeat(3, 1fr); gap: 16px;",
                    Card {
                        CardHeader { CardTitle { "Default card" } CardDescription { "Flat, on the warm-paper surface." } }
                        CardContent { p { "Body content goes here." } }
                        CardFooter { Button { label: "OK".to_string(), on_click: |_| {} } }
                    }
                    Card { interactive: true,
                        CardHeader { CardTitle { "Interactive card" } CardDescription { "Hover for the warm halo." } }
                        CardContent { p { "Used for clickable surfaces." } }
                    }
                    Card { variant: CardVariant::Premium,
                        CardHeader { CardTitle { "Premium card" } CardDescription { "1px gold inner border." } }
                        CardContent { p { "Reserved for paywalled surfaces." } }
                    }
                }
            }

            // --- Inputs / Fields
            section { "data-stagger": "4",
                h2 { "Inputs & fields" }
                div { style: "display: grid; grid-template-columns: 1fr 1fr; gap: 16px;",
                    Field { label: "Email".to_string(), helper: "We never share your email".to_string(),
                        Input { value: input_val.read().clone(), placeholder: "you@example.com".to_string(), on_input: move |v| input_val.set(v) }
                    }
                    Field { label: "Search".to_string(),
                        Input {
                            value: "".to_string(),
                            placeholder: "Find a course".to_string(),
                            leading_icon: Some(rsx! { span { "🔍" } }),
                            on_input: |_| {},
                        }
                    }
                    Field { label: "Price".to_string(),
                        Input {
                            value: "".to_string(),
                            addon_left: Some("$".to_string()),
                            addon_right: Some("USD".to_string()),
                            on_input: |_| {},
                        }
                    }
                    Field { label: "Invalid".to_string(), error: "Required".to_string(),
                        Input { value: "".to_string(), error: true, on_input: |_| {} }
                    }
                }
            }

            // --- Badges
            section { "data-stagger": "5",
                h2 { "Badges" }
                div { style: "display: flex; gap: 6px; flex-wrap: wrap; align-items: center;",
                    Badge { label: "Neutral".to_string() }
                    Badge { label: "Primary".to_string(), tone: BadgeTone::Primary }
                    Badge { label: "Info".to_string(),    tone: BadgeTone::Info }
                    Badge { label: "Success".to_string(), tone: BadgeTone::Success }
                    Badge { label: "Warning".to_string(), tone: BadgeTone::Warning }
                    Badge { label: "Danger".to_string(),  tone: BadgeTone::Danger }
                    Badge { label: "LIVE".to_string(),    tone: BadgeTone::Live }
                    Badge { label: "Pro".to_string(),     tone: BadgeTone::Premium }
                    Badge { label: "small".to_string(),   size: BadgeSize::Sm }
                }
            }

            // --- Table
            section { "data-stagger": "6",
                h2 { "Table" }
                Table {
                    striped: true,
                    sticky_header: true,
                    toolbar: Some(rsx! {
                        div { style: "display: flex; gap: 8px; flex: 1;",
                            Input { value: "".to_string(), placeholder: "Search".to_string(), on_input: |_| {} }
                            Button { label: "Filter".to_string(), variant: ButtonVariant::Secondary, on_click: |_| {} }
                        }
                    }),
                    head: rsx! {
                        tr {
                            TableHeaderCell { sortable: true, sort_dir: Some(SortDir::Asc), on_sort: |_| {}, "Name" }
                            TableHeaderCell { "Email" }
                            TableHeaderCell { "Role" }
                        }
                    },
                    body: rsx! {
                        tr { td { "Ada Lovelace" } td { "ada@example.com" } td { Badge { label: "Owner".to_string(), tone: BadgeTone::Primary } } }
                        tr { td { "Alan Turing" }  td { "alan@example.com" } td { Badge { label: "Editor".to_string() } } }
                        tr { td { "Grace Hopper" } td { "grace@example.com" } td { Badge { label: "Viewer".to_string() } } }
                    },
                }
            }

            // --- Tabs
            section { "data-stagger": "6",
                h2 { "Tabs" }
                Tabs {
                    tabs: vec![
                        Tab { key: "primary".to_string(),   label: "Primary".to_string(),   ..Default::default() },
                        Tab { key: "secondary".to_string(), label: "Secondary".to_string(), ..Default::default() },
                        Tab { key: "tertiary".to_string(),  label: "Tertiary".to_string(),  ..Default::default() },
                    ],
                    active: active_tab.read().clone(),
                    on_change: move |k| active_tab.set(k),
                }
                div { style: "margin-top: 16px;",
                    Tabs {
                        variant: TabsVariant::Pill,
                        tabs: vec![
                            Tab { key: "primary".to_string(),   label: "Primary".to_string(),   ..Default::default() },
                            Tab { key: "secondary".to_string(), label: "Secondary".to_string(), ..Default::default() },
                        ],
                        active: active_tab.read().clone(),
                        on_change: move |k| active_tab.set(k),
                    }
                }
            }

            // --- Dropdown menu
            section { "data-stagger": "6",
                h2 { "Dropdown menu" }
                {
                    let mut open = dropdown_open;
                    rsx! {
                        DropdownMenu {
                            open,
                            align: DropdownAlign::Start,
                            trigger: rsx! {
                                Button {
                                    label: "Open menu ▾".to_string(),
                                    variant: ButtonVariant::Secondary,
                                    on_click: move |_| { open.set(!*open.read()); },
                                }
                            },
                            DropdownMenuLabel { "Account" }
                            DropdownMenuItem { label: "Profile".to_string(),  on_select: |_| {} }
                            DropdownMenuItem { label: "Settings".to_string(), on_select: |_| {} }
                            DropdownMenuSeparator {}
                            DropdownMenuItem { label: "Sign out".to_string(), tone: design_system::DropdownItemTone::Danger, on_select: |_| {} }
                        }
                    }
                }
            }

            // --- Sheet
            section { "data-stagger": "6",
                h2 { "Sheet" }
                {
                    let mut open = sheet_open;
                    rsx! {
                        Button { label: "Open sheet".to_string(), on_click: move |_| { open.set(true); } }
                        Sheet {
                            open,
                            side: SheetSide::Right,
                            SheetClose { open }
                            SheetHeader { SheetTitle { "Edit profile" } }
                            SheetBody {
                                p { "Body content with form fields would live here." }
                            }
                            SheetFooter {
                                Button { label: "Cancel".to_string(), variant: ButtonVariant::Secondary, on_click: move |_| { open.set(false); } }
                                Button { label: "Save".to_string(),   on_click: move |_| { open.set(false); } }
                            }
                        }
                    }
                }
            }

            // --- Skeletons
            section { "data-stagger": "6",
                h2 { "Skeletons" }
                div { style: "display: flex; gap: 16px; align-items: center;",
                    SkeletonCircle { size: "40px".to_string() }
                    div { style: "flex: 1; display: grid; gap: 8px;",
                        SkeletonLine { width: "60%".to_string() }
                        SkeletonLine { width: "40%".to_string() }
                    }
                    SkeletonCard {}
                }
            }
        }
    }
}
```

- [ ] **Step 2: Wire the module in `crates/shell-web/src/routes/mod.rs`**

Append at the end of the file (after the existing `pub use assignments_new::AssignmentNew;` line):

```rust
#[cfg(debug_assertions)]
pub mod dev_components;
#[cfg(debug_assertions)]
pub use dev_components::DevComponents;
```

- [ ] **Step 3: Wire the route in `crates/shell-web/src/route_enum.rs`**

In the `use crate::routes::{...}` import list at the top, append `DevComponents` only in debug:

```rust
#[cfg(debug_assertions)]
use crate::routes::DevComponents;
```

Then inside the `#[derive(Routable)]` enum, add a variant at the bottom (gated):

```rust
    #[cfg(debug_assertions)]
    #[route("/dev/components")]
    DevComponents {},
```

Note: dioxus_router's `Routable` derive may not support `#[cfg]` on a variant. If `cargo check` fails with that error, the workaround is to NOT gate the variant at the enum level but only gate the import + component body — the variant always exists in the enum but the component renders nothing in release. Adjusted variant in that case:

```rust
    #[route("/dev/components")]
    DevComponents {},
```

And the component file's outer `#![cfg(debug_assertions)]` is replaced with a runtime check that returns an empty Element in release. Pick whichever path compiles cleanly.

- [ ] **Step 4: Compile + navigate manually**

```bash
cargo check -p shell-web
```

Then run the dev server (e.g. `cargo run -p shell-web` or whatever the existing dev-server command is in the repo) and visit `/dev/components`. Eyeball every section.

- [ ] **Step 5: Commit**

```bash
git add crates/shell-web/src/routes/dev_components.rs crates/shell-web/src/routes/mod.rs crates/shell-web/src/route_enum.rs
git commit -m "feat(shell-web): /dev/components kitchen-sink route (debug only)"
```

---

### Task 14: Playwright smoke spec

**Files:**
- Create: `tools/design_system_smoke.spec.js`

The spec lives at the project root under `tools/` (matches existing `tools/ui-real-stack.spec.js` convention; `playwright.config.js` has `testDir: "."` so it's discovered automatically). It boots the dev server, hits `/dev/components`, and screenshots each section.

- [ ] **Step 1: Create `tools/design_system_smoke.spec.js`**

```javascript
// tools/design_system_smoke.spec.js
// Smoke spec for the v1 design system kitchen sink. Requires the shell-web dev
// server to be running on the URL set in DS_KITCHEN_URL (default http://localhost:8080).
//
// Run: npx playwright test tools/design_system_smoke.spec.js

const { test, expect } = require("@playwright/test");

const BASE = process.env.DS_KITCHEN_URL || "http://localhost:8080";

test.describe("design system kitchen sink", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto(`${BASE}/dev/components`, { waitUntil: "networkidle" });
  });

  test("page renders all primitive sections", async ({ page }) => {
    await expect(page.locator("h1", { hasText: "Design System Kitchen Sink" })).toBeVisible();
    for (const heading of [
      "Buttons",
      "Cards",
      "Inputs & fields",
      "Badges",
      "Table",
      "Tabs",
      "Dropdown menu",
      "Sheet",
      "Skeletons",
    ]) {
      await expect(page.locator("h2", { hasText: heading })).toBeVisible();
    }
  });

  test("primary button has the v1 class", async ({ page }) => {
    await expect(page.locator(".ds-button--primary").first()).toBeVisible();
  });

  test("focus ring becomes visible on tab to first button", async ({ page }) => {
    await page.locator(".ds-button--primary").first().focus();
    const shadow = await page.locator(".ds-button--primary").first().evaluate(
      (el) => getComputedStyle(el).boxShadow,
    );
    expect(shadow).not.toBe("none");
  });

  test("dropdown opens and emits content with data-state", async ({ page }) => {
    const trigger = page.locator("button", { hasText: "Open menu" }).first();
    await trigger.click();
    await expect(page.locator(".ds-dropdown-content[data-state=\"open\"]")).toBeVisible();
    await expect(page.locator(".ds-dropdown-item", { hasText: "Sign out" })).toBeVisible();
  });

  test("sheet opens, escape closes", async ({ page }) => {
    const open = page.locator("button", { hasText: "Open sheet" }).first();
    await open.click();
    const panel = page.locator(".ds-sheet").first();
    await expect(panel).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(panel).toBeHidden();
  });

  test("reduced motion honoured", async ({ browser }) => {
    const context = await browser.newContext({ reducedMotion: "reduce" });
    const page = await context.newPage();
    await page.goto(`${BASE}/dev/components`, { waitUntil: "networkidle" });
    const dur = await page.locator(".ds-button--primary").first().evaluate(
      (el) => getComputedStyle(el).animationDuration,
    );
    // Reduced motion CSS forces animation-duration: 0.01ms !important.
    expect(dur).toBe("0.01ms");
    await context.close();
  });

  test("kitchen sink screenshot", async ({ page }) => {
    await page.locator("h1").first().waitFor();
    expect(await page.screenshot({ fullPage: true })).toMatchSnapshot("design-system-kitchen-sink.png", {
      maxDiffPixelRatio: 0.02,
    });
  });
});
```

- [ ] **Step 2: Smoke-run the spec locally**

```bash
# Start the shell-web dev server in another terminal first.
# Then in this terminal:
npx playwright test tools/design_system_smoke.spec.js
```

Expected: all assertion-style tests pass. The snapshot test will fail on first run because the baseline is missing — re-run with `--update-snapshots` once after eyeballing the screenshot:

```bash
npx playwright test tools/design_system_smoke.spec.js --update-snapshots
```

Commit the generated baseline (it lives next to the spec, typically in a `*-snapshots/` folder).

- [ ] **Step 3: Commit spec + baseline**

```bash
git add tools/design_system_smoke.spec.js tools/design_system_smoke.spec.js-snapshots
git commit -m "test(e2e): design-system v1 kitchen-sink smoke spec"
```

---

## Self-Review

(The plan author runs this checklist as a final pass; not an additional task.)

**1. Spec coverage:** Every spec section maps to a task.
- §4 Token overhaul → Task 1.
- §5 Motion catalog → Task 2.
- §6.1 Button → Task 3.
- §6.2 Card → Task 4.
- §6.3 Input / Field / FormError → Task 5.
- §6.4 Badge → Task 6.
- §6.5 Table → Task 7.
- §6.6 Tabs → Task 8.
- §6.7 DropdownMenu → Task 9.
- §6.8 Sheet → Task 10.
- §6.9 Skeleton → Task 11.
- §6.10 Toast → Task 12.
- §7 Kitchen-sink route + Playwright → Tasks 13 + 14.

**2. Placeholder scan:** No TBDs, no "implement later", every step has either commands or full code.

**3. Type consistency:**
- `ButtonVariant::Premium`, `ButtonSize::Sm/Md/Lg/Icon` — used consistently in Tasks 3 and 13.
- `CardVariant::Premium`, `Card::Header`/`Title`/`Description`/`Content`/`Footer` exported as `CardHeader` etc. — Tasks 4 and 13 match.
- `BadgeTone::Premium`, `BadgeTone::Primary`, `BadgeSize` — Tasks 6 and 13 match.
- `TableHeaderCell`, `SortDir::Asc/Desc`, `Table.toolbar` prop — Tasks 7 and 13 match.
- `Tabs.variant`, `TabsVariant::Underline/Pill` — Tasks 8 and 13 match.
- `DropdownMenu.open: Signal<bool>`, `DropdownAlign::Start/End`, `DropdownItemTone::Default/Danger`, `DropdownMenu`/`DropdownMenuItem`/`DropdownMenuLabel`/`DropdownMenuSeparator` — Tasks 9 and 13 match.
- `Sheet.open: Signal<bool>`, `SheetSide`, `SheetHeader`/`Title`/`Body`/`Footer`/`Close` — Tasks 10 and 13 match.
- `ToastLevel::Premium`, `ToastPosition`, `ToastProvider.position` — Tasks 12 and 13 match.

**4. CSS class consistency:**
- `ds-button--*`, `ds-card-*`, `ds-input-*`, `badge-*`, `ds-table-*`, `ds-tabs--*`, `ds-dropdown-*`, `ds-sheet-*`, `ds-skeleton-*`, `ds-toast-*` — class prefixes consistent across Rust emitters and CSS rules.
- The motion keyframe names referenced from component CSS (`ds-fade-in`, `ds-slide-in-*`, `ds-zoom-in-95`, `ds-shimmer`, `ds-pulse`, `ds-sheet-in-right`, `ds-page-in`) all exist in the motion catalog block from Task 2.

**5. Token naming:** New components use `--radius-shadcn-*` (not `--radius-*`) because the legacy `--radius-sm/md/lg/xl` tokens already exist with different values. This is intentional and consistent across every CSS section.

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-05-17-design-system-shadcn-warm-v1.md`. Two execution options:

**1. Subagent-Driven (recommended)** — I dispatch a fresh subagent per task (foundation → primitives → new components → kitchen sink → smoke), review between tasks, fast iteration.

**2. Inline Execution** — Execute tasks in this session using executing-plans, batch execution with checkpoints.

Which approach?
