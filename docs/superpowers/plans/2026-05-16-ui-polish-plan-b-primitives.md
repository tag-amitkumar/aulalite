# UI Polish — Plan B: Primitives Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Polish all 13 design-system primitives to the locked state-coverage standard (default / hover / active / focus / focus-visible / disabled / error), using the Plan A token vocabulary. Backward-compatible — existing call sites continue to compile and render. New variants and primitives (Tooltip, Avatar, Switch rename, Ghost/Link button variants) added additively.

**Architecture:** In-place enhancement of `crates/design-system/src/*.rs` Rust components and `crates/design-system/assets/components.css` CSS. Two new primitive files (Tooltip, Avatar). All values reference Plan A tokens (color ramps, type scale, spacing, radius, shadow, motion eases). Asset-sync mirror maintained throughout.

**Tech Stack:** Rust + Dioxus 0.7, CSS custom properties, SSR tests via `dioxus_ssr`.

**Vision spec:** `docs/superpowers/specs/2026-05-16-ui-polish-vision-design.md` (commit `5bf5c27`).
**Foundation plan:** `docs/superpowers/plans/2026-05-16-ui-polish-plan-a-foundation.md` (executed — provides token vocabulary).

---

## File Structure

**Modify (per-primitive):**
- `crates/design-system/src/button.rs`, `input.rs`, `select.rs`, `checkbox.rs`, `toggle.rs`, `card.rs`, `modal.rs`, `tabs.rs`, `badge.rs`, `spinner.rs`, `progress_bar.rs` — Rust API additions (new variants, optional props for error/size).
- `crates/design-system/src/lib.rs` — export new primitives (Tooltip, Avatar, Switch).
- `crates/design-system/assets/components.css` — full CSS state coverage for each primitive, all values from Plan A tokens.
- `crates/shell-web/public/assets/components.css` — mirror.

**Create:**
- `crates/design-system/src/tooltip.rs` — new primitive.
- `crates/design-system/src/avatar.rs` — new primitive.
- `crates/design-system/src/switch.rs` — new file (renaming `toggle.rs` is risky — too many call sites; instead add `switch.rs` as the polished v2 and keep `toggle.rs` as a deprecated alias re-export).

**Test conventions:**
- SSR tests live inside each primitive's `mod tests` block (existing pattern).
- One test per new state / variant.
- Pure-Rust unit tests for any non-trivial helper logic.

---

## Conventions (referenced by every task)

### State coverage CSS pattern

For every primitive that has interactive states, add CSS rules using the new tokens:

```css
.ds-X {                              /* default */
  transition:
    background-color var(--duration-fast) var(--ease-snappy),
    border-color var(--duration-fast) var(--ease-snappy),
    box-shadow var(--duration-fast) var(--ease-snappy),
    transform var(--duration-fast) var(--ease-snappy);
}
.ds-X:hover:not(:disabled) { /* hover */ }
.ds-X:active:not(:disabled) { /* active — transform: translateY(1px) typical */ }
.ds-X:focus-visible {
  outline: 2px solid var(--color-gold-400);
  outline-offset: 2px;
  border-color: var(--color-gold-400);
}
.ds-X:disabled { cursor: not-allowed; opacity: 0.55; }
.ds-X--error { border-color: var(--color-danger-500); }   /* inputs only */
```

`focus-visible` (keyboard focus only) replaces the older `focus` outline so mouse-clicks don't show a ring. Fall back to `:focus` for browsers that don't support `:focus-visible` is automatic (the CSS rule simply doesn't apply).

### Token-only invariant

No raw hex / px / rem inside the primitive CSS blocks. Every value comes from a Plan A token (`var(--color-...)`, `var(--space-...)`, `var(--text-...)`, `var(--radius-...)`, `var(--shadow-...)`, `var(--duration-...)`, `var(--ease-...)`). The audit step in Task 14 greps for violations.

### Mirror discipline

After every CSS edit:
```bash
cp crates/design-system/assets/components.css crates/shell-web/public/assets/components.css
diff crates/design-system/assets/components.css crates/shell-web/public/assets/components.css
```

If the diff isn't empty, fix immediately.

### Test naming

`<primitive>_<state_or_variant>_<expected_outcome>` — e.g. `button_ghost_variant_renders_class`, `input_error_state_adds_error_class`.

---

## Task 1: Button — variants + state coverage

**Files:**
- Modify: `crates/design-system/src/button.rs`
- Modify: `crates/design-system/assets/components.css` (+ mirror)

### API additions
Extend `ButtonVariant`:
```rust
#[derive(Clone, PartialEq, Default)]
pub enum ButtonVariant {
    #[default]
    Primary,
    Secondary,
    Danger,
    Ghost,
    Link,
}
```

Extend `ButtonProps` (backward-compat — all new props optional with defaults):
```rust
#[derive(Props, Clone, PartialEq)]
pub struct ButtonProps {
    pub label: String,
    #[props(default)]
    pub disabled: bool,
    #[props(default)]
    pub variant: ButtonVariant,
    pub on_click: EventHandler<MouseEvent>,
    /// Optional icon glyph rendered before the label.
    #[props(default)]
    pub leading_icon: Option<Element>,
    /// `type` attribute. Defaults to "button"; pass "submit" inside forms.
    #[props(default = "button".to_string())]
    pub button_type: String,
}
```

Component body adds the leading-icon rendering and uses `button_type` for the `r#type` attribute.

### Tasks

- [ ] **Step 1: Add failing tests**

Append to `button.rs::mod tests`:
```rust
    #[test]
    fn ghost_variant_renders_class() {
        fn app() -> Element {
            rsx! {
                Button { label: "G".to_string(), variant: ButtonVariant::Ghost, on_click: |_| {} }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("ds-button--ghost"), "ghost class missing: {html}");
    }

    #[test]
    fn link_variant_renders_class() {
        fn app() -> Element {
            rsx! {
                Button { label: "L".to_string(), variant: ButtonVariant::Link, on_click: |_| {} }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("ds-button--link"), "link class missing: {html}");
    }
```

- [ ] **Step 2: Run, expect failure**

```bash
cargo test -p design-system --lib button::tests
```

Expected: FAIL — `ghost class missing` / `link class missing`.

- [ ] **Step 3: Implement variant enum + class map**

In `button.rs`, update the variant enum and class match arms. New class strings:
- `Ghost` → `"ds-button ds-button--ghost"`
- `Link` → `"ds-button ds-button--link"`

- [ ] **Step 4: Implement Rust API additions for leading_icon + button_type**

Update `ButtonProps` per the snippet above. Update the component body:
```rust
rsx! {
    button {
        class,
        r#type: "{props.button_type}",
        disabled: props.disabled,
        onclick: move |event| props.on_click.call(event),
        if let Some(icon) = &props.leading_icon {
            span { class: "ds-button-icon", {icon.clone()} }
        }
        "{props.label}"
    }
}
```

- [ ] **Step 5: Implement CSS state coverage**

In `crates/design-system/assets/components.css`, find the existing `.ds-button`-family block (currently styled around line 33-... with Primary / Secondary / Danger variants) and replace with the polished version. The new block uses Plan A tokens for every value. Key rules:

```css
.ds-button,
.btn,
.btn-primary,
.linkish {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: var(--space-2);
  min-height: 40px;
  padding: 0 var(--space-4);
  border: 1px solid transparent;
  border-radius: var(--radius-md);
  font-family: var(--font-body);
  font-weight: 700;
  font-size: var(--text-base);
  line-height: var(--leading-base);
  cursor: pointer;
  transition:
    background-color var(--duration-fast) var(--ease-snappy),
    border-color var(--duration-fast) var(--ease-snappy),
    box-shadow var(--duration-fast) var(--ease-snappy),
    transform var(--duration-fast) var(--ease-snappy);
}

.ds-button:hover:not(:disabled) { transform: translateY(-1px); }
.ds-button:active:not(:disabled) { transform: translateY(0); }
.ds-button:focus-visible {
  outline: 2px solid var(--color-gold-400);
  outline-offset: 2px;
}
.ds-button:disabled { cursor: not-allowed; opacity: 0.55; }

.ds-button--primary {
  color: #fff;
  background: var(--color-green-600);
  border-color: var(--color-green-600);
  box-shadow: var(--shadow-sm);
}
.ds-button--primary:hover:not(:disabled) {
  background: var(--color-green-700);
  border-color: var(--color-green-700);
  box-shadow: var(--shadow-md);
}

.ds-button--secondary,
.btn {
  color: var(--color-ink);
  background: var(--color-surface);
  border-color: var(--color-rule-strong);
}
.ds-button--secondary:hover:not(:disabled) {
  background: var(--color-neutral-100);
  border-color: var(--color-neutral-300);
}

.ds-button--danger {
  color: #fff;
  background: var(--color-danger-500);
  border-color: var(--color-danger-500);
}
.ds-button--danger:hover:not(:disabled) {
  background: var(--color-danger-600);
  border-color: var(--color-danger-600);
}

.ds-button--ghost {
  color: var(--color-ink);
  background: transparent;
  border-color: transparent;
}
.ds-button--ghost:hover:not(:disabled) {
  background: var(--color-neutral-100);
}

.ds-button--link,
.linkish {
  min-height: 32px;
  padding: 0;
  color: var(--color-accent-strong);
  background: transparent;
  border: 0;
  box-shadow: none;
}
.ds-button--link:hover:not(:disabled),
.linkish:hover {
  color: var(--color-navy-600);
  text-decoration: underline;
  text-underline-offset: 3px;
}

.ds-button-icon {
  display: inline-flex;
  align-items: center;
}
```

Critically: existing `.btn-primary` legacy class should remain styled, since some surfaces use it. Keep the shared base block.

- [ ] **Step 6: Mirror CSS and run tests**
```bash
cp crates/design-system/assets/components.css crates/shell-web/public/assets/components.css
diff crates/design-system/assets/components.css crates/shell-web/public/assets/components.css
cargo test -p design-system --lib button::tests
cargo test --workspace --no-fail-fast
```

Expected: all tests pass. Compile errors in call sites would indicate accidental breaking change; STOP and report.

- [ ] **Step 7: Commit**
```bash
cargo fmt --all
git add crates/design-system/src/button.rs \
        crates/design-system/assets/components.css \
        crates/shell-web/public/assets/components.css
git commit -m "$(cat <<'EOF'
feat(button): add ghost + link variants, leading icon, full state coverage

Extends ButtonVariant with Ghost and Link, adds optional leading_icon
slot and button_type (defaults to "button"), and rewires the entire
.ds-button block to draw from Plan A tokens — color ramps for hover
states, motion eases for transitions, focus-visible outline on
gold-400.

Backward compat: existing call sites continue to compile (all new
props are optional).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: Input + Textarea — error state + token-driven polish

**Files:**
- Modify: `crates/design-system/src/input.rs`
- Modify: `crates/design-system/assets/components.css` (+ mirror)

### API additions
Add optional error state and accessibility props:
```rust
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
    pub on_input: EventHandler<String>,
}
```

Component class becomes:
```rust
let class = if props.error { "ds-input ds-input--error" } else { "ds-input" };
```

### CSS state coverage
Replace the existing `.ds-input` (and `.ds-select`, `.ds-datetime`, `textarea`, `select`) block with a Plan A-token version:

```css
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
  font-size: var(--text-base);
  line-height: var(--leading-base);
  transition:
    border-color var(--duration-fast) var(--ease-snappy),
    box-shadow var(--duration-fast) var(--ease-snappy),
    background-color var(--duration-fast) var(--ease-snappy);
}

textarea {
  min-height: 132px;
  padding-top: var(--space-3);
  resize: vertical;
}

.ds-input:hover:not(:disabled):not(:focus),
.ds-select:hover:not(:disabled):not(:focus),
textarea:hover:not(:disabled):not(:focus) {
  border-color: var(--color-neutral-400);
}

.ds-input:focus-visible,
.ds-select:focus-visible,
.ds-datetime:focus-visible,
textarea:focus-visible,
select:focus-visible {
  border-color: var(--color-accent);
  outline: 3px solid var(--color-focus);
}

.ds-input:disabled,
.ds-select:disabled,
textarea:disabled,
select:disabled {
  background: var(--color-neutral-100);
  color: var(--color-text-muted);
  cursor: not-allowed;
}

.ds-input--error {
  border-color: var(--color-danger-500);
}
.ds-input--error:focus-visible {
  outline-color: rgba(163, 58, 54, 0.32);
}
```

### Tests

Tests added to `input.rs::mod tests`:
1. `input_error_state_adds_error_class` — `error: true` props produces `ds-input--error` in the rendered HTML.
2. `input_disabled_renders_disabled_attribute` (likely already exists in some form; ensure coverage).

### Commit message
```
feat(input): add error state, accessibility props, token-driven polish
```

---

## Task 3: Select — error state + polish

**Files:**
- Modify: `crates/design-system/src/select.rs`
- Modify: `crates/design-system/assets/components.css` (Select block; the bulk is shared with `.ds-input` block from Task 2)

### API additions
Add `error: bool` and `name: Option<String>` / `id: Option<String>` props similar to Input.

### CSS
Select inherits most styling from the shared `.ds-input` block in Task 2. Add Select-specific:

```css
.ds-select {
  /* Native dropdown caret styling */
  appearance: none;
  background-image: linear-gradient(45deg, transparent 50%, currentColor 50%),
                    linear-gradient(135deg, currentColor 50%, transparent 50%);
  background-position: calc(100% - 18px) 50%, calc(100% - 12px) 50%;
  background-size: 6px 6px;
  background-repeat: no-repeat;
  padding-right: var(--space-7);
}
```

### Tests
- `select_error_state_adds_error_class`.

### Commit message
```
feat(select): add error state, native caret styling, token alignment
```

---

## Task 4: Checkbox + Radio — state coverage, indeterminate prop

**Files:**
- Modify: `crates/design-system/src/checkbox.rs`
- Create: A `Radio` component in the same file (export from lib.rs).
- Modify: `crates/design-system/assets/components.css` (+ mirror).

### API additions
Existing `Checkbox` props get:
- `indeterminate: bool` (optional).
- `error: bool` (optional).
- `disabled: bool` (optional).

New `Radio` component:
```rust
#[derive(Props, Clone, PartialEq)]
pub struct RadioProps {
    pub checked: bool,
    pub name: String,
    pub value: String,
    #[props(default)]
    pub disabled: bool,
    pub on_change: EventHandler<String>,
}

#[component]
pub fn Radio(props: RadioProps) -> Element { ... }
```

### CSS state coverage

```css
.ds-checkbox,
.ds-radio {
  appearance: none;
  width: 18px;
  height: 18px;
  border: 1px solid var(--color-rule-strong);
  background: var(--color-surface);
  cursor: pointer;
  transition:
    background-color var(--duration-fast) var(--ease-snappy),
    border-color var(--duration-fast) var(--ease-snappy);
}
.ds-checkbox { border-radius: var(--radius-xs); }
.ds-radio { border-radius: var(--radius-full); }

.ds-checkbox:checked,
.ds-radio:checked {
  background: var(--color-primary);
  border-color: var(--color-primary);
}

.ds-checkbox:indeterminate {
  background: var(--color-primary);
  border-color: var(--color-primary);
}

.ds-checkbox:focus-visible,
.ds-radio:focus-visible {
  outline: 2px solid var(--color-gold-400);
  outline-offset: 2px;
}

.ds-checkbox:disabled,
.ds-radio:disabled {
  cursor: not-allowed;
  opacity: 0.55;
}
```

### Tests
- `checkbox_indeterminate_renders_attribute`
- `radio_renders_with_name_and_value`

### Commit message
```
feat(checkbox-radio): add indeterminate + error states, introduce Radio primitive
```

---

## Task 5: Switch — new primitive (Toggle deprecated alias)

**Files:**
- Create: `crates/design-system/src/switch.rs`.
- Modify: `crates/design-system/src/lib.rs` (export Switch; keep Toggle as deprecated re-export).
- Modify: `crates/design-system/assets/components.css` (+ mirror).

### Rationale
The existing `Toggle` primitive's API is sparse. Rather than re-shape it (risking call-site breakage), introduce `Switch` as the polished v2 and deprecate `Toggle` (kept as `pub use Switch as Toggle;` so all current callers still work).

### API
```rust
#[derive(Props, Clone, PartialEq)]
pub struct SwitchProps {
    pub checked: bool,
    #[props(default)]
    pub disabled: bool,
    #[props(default)]
    pub label: Option<String>,
    pub on_change: EventHandler<bool>,
}

#[component]
pub fn Switch(props: SwitchProps) -> Element {
    rsx! {
        label { class: "ds-switch",
            input {
                r#type: "checkbox",
                class: "ds-switch-input",
                checked: props.checked,
                disabled: props.disabled,
                onchange: move |e| props.on_change.call(e.value().parse().unwrap_or(false)),
            }
            span { class: "ds-switch-track",
                span { class: "ds-switch-thumb" }
            }
            if let Some(text) = &props.label {
                span { class: "ds-switch-label", "{text}" }
            }
        }
    }
}
```

### CSS

```css
.ds-switch {
  display: inline-flex;
  align-items: center;
  gap: var(--space-2);
  cursor: pointer;
  user-select: none;
}
.ds-switch-input { position: absolute; opacity: 0; pointer-events: none; }
.ds-switch-track {
  position: relative;
  width: 36px;
  height: 20px;
  border-radius: var(--radius-full);
  background: var(--color-neutral-300);
  transition: background-color var(--duration-fast) var(--ease-snappy);
}
.ds-switch-thumb {
  position: absolute;
  top: 2px;
  left: 2px;
  width: 16px;
  height: 16px;
  border-radius: var(--radius-full);
  background: var(--color-surface);
  box-shadow: var(--shadow-sm);
  transition: transform var(--duration-medium) var(--ease-spring);
}
.ds-switch-input:checked + .ds-switch-track {
  background: var(--color-primary);
}
.ds-switch-input:checked + .ds-switch-track .ds-switch-thumb {
  transform: translateX(16px);
}
.ds-switch-input:focus-visible + .ds-switch-track {
  outline: 2px solid var(--color-gold-400);
  outline-offset: 2px;
}
.ds-switch-input:disabled + .ds-switch-track {
  cursor: not-allowed;
  opacity: 0.55;
}
```

### Tests
- `switch_renders_checked_state`
- `switch_renders_with_label`

### `lib.rs` changes
```rust
pub mod switch;
pub use switch::Switch;

// Backward compat: existing `Toggle` callers continue to work.
pub use switch::Switch as Toggle;
```

Delete `toggle` module re-export (the file `toggle.rs` can stay or be deleted; if deleted, lib.rs needs `pub mod` removed too).

### Commit message
```
feat(switch): introduce Switch primitive, keep Toggle as deprecated alias
```

---

## Task 6: Card — interactive variant + state coverage

**Files:**
- Modify: `crates/design-system/src/card.rs`
- Modify: `crates/design-system/assets/components.css` (+ mirror)

### API additions
```rust
#[derive(Props, Clone, PartialEq)]
pub struct CardProps {
    pub children: Element,
    /// When set, the card becomes a clickable surface with hover affordance.
    #[props(default)]
    pub on_click: Option<EventHandler<MouseEvent>>,
    /// Variant: default | accent (gold-tinted) | danger (oxblood-tinted)
    #[props(default)]
    pub variant: CardVariant,
}

#[derive(Clone, PartialEq, Default)]
pub enum CardVariant {
    #[default]
    Default,
    Accent,
    Danger,
}
```

### CSS

```css
.ds-card,
.card {
  padding: var(--space-5);
  background: var(--color-surface);
  border: 1px solid var(--color-rule);
  border-radius: var(--radius-lg);          /* upgraded 8 → 12 in Plan A */
  box-shadow: var(--shadow-sm);
  transition:
    box-shadow var(--duration-medium) var(--ease-snappy),
    border-color var(--duration-fast) var(--ease-snappy);
}

.ds-card--clickable {
  cursor: pointer;
}
.ds-card--clickable:hover {
  box-shadow: var(--shadow-md);
  border-color: var(--color-neutral-300);
  transform: translateY(-1px);
}
.ds-card--clickable:active {
  transform: translateY(0);
  box-shadow: var(--shadow-sm);
}
.ds-card--clickable:focus-visible {
  outline: 2px solid var(--color-gold-400);
  outline-offset: 2px;
}

.ds-card--accent {
  background: linear-gradient(135deg, var(--color-gold-50), var(--color-surface));
  border-color: var(--color-gold-200);
}
.ds-card--danger {
  background: var(--color-danger-50);
  border-color: var(--color-danger-200);
}
```

### Tests
- `card_renders_clickable_class_when_on_click_set`
- `card_accent_variant_renders_class`

### Commit message
```
feat(card): add clickable + accent + danger variants, refined state coverage
```

---

## Task 7: Modal — motion polish + state coverage

**Files:**
- Modify: `crates/design-system/src/modal.rs`
- Modify: `crates/design-system/assets/components.css` (+ mirror)

### API additions
- `size: ModalSize` (Small / Medium / Large; default Medium).
- `dismissable: bool` (default true) — controls ESC + backdrop click handling.

### CSS

```css
.modal-backdrop {
  position: fixed;
  inset: 0;
  display: grid;
  place-items: center;
  padding: var(--space-5);
  background: rgba(23, 26, 23, 0.48);
  z-index: 1000;
  animation: modal-backdrop-in var(--duration-medium) var(--ease-decelerate) both;
}

@keyframes modal-backdrop-in {
  from { opacity: 0; }
  to { opacity: 1; }
}

.modal {
  width: min(640px, 100%);
  max-height: calc(100vh - 48px);
  overflow: auto;
  padding: var(--space-6);
  background: var(--color-surface);
  border-radius: var(--radius-xl);
  box-shadow: var(--shadow-xl);
  animation: modal-pop-in var(--duration-slow) var(--ease-spring) both;
}

.modal--small  { width: min(420px, 100%); }
.modal--medium { width: min(640px, 100%); }
.modal--large  { width: min(880px, 100%); }

@keyframes modal-pop-in {
  from { opacity: 0; transform: scale(.96) translateY(8px); }
  to   { opacity: 1; transform: scale(1) translateY(0); }
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
  font-size: var(--text-2xl);
}
.modal-close {
  border: 0;
  background: transparent;
  cursor: pointer;
  font-size: var(--text-3xl);
  color: var(--color-text-muted);
  transition: color var(--duration-fast) var(--ease-snappy);
}
.modal-close:hover { color: var(--color-ink); }
.modal-close:focus-visible {
  outline: 2px solid var(--color-gold-400);
  outline-offset: 2px;
}
```

### Tests
- `modal_size_small_renders_class`
- `modal_renders_close_button_by_default`

### Commit message
```
feat(modal): add size variants, motion polish (spring pop-in), close button state coverage
```

---

## Task 8: Tabs — selected / hover / disabled coverage

**Files:**
- Modify: `crates/design-system/src/tabs.rs`
- Modify: `crates/design-system/assets/components.css` (+ mirror)

### API additions
Add `disabled: bool` on each `Tab` struct so individual tabs can be greyed out.

### CSS

```css
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
  background: transparent;
  border: 1px solid transparent;
  border-radius: var(--radius-full);
  cursor: pointer;
  font-family: var(--font-body);
  font-size: var(--text-base);
  font-weight: 700;
  transition:
    color var(--duration-fast) var(--ease-snappy),
    background-color var(--duration-fast) var(--ease-snappy),
    border-color var(--duration-fast) var(--ease-snappy);
}

.tab:hover:not(:disabled),
.chip:hover:not(:disabled) {
  color: var(--color-ink);
  background: var(--color-neutral-100);
}

.tab-active,
.chip-active {
  color: #fff;
  background: var(--color-accent-strong);
  border-color: var(--color-accent-strong);
}

.tab:focus-visible,
.chip:focus-visible {
  outline: 2px solid var(--color-gold-400);
  outline-offset: 2px;
}

.tab:disabled,
.chip:disabled {
  cursor: not-allowed;
  opacity: 0.55;
}
```

### Tests
- `tabs_render_active_class_for_selected`
- `tabs_disabled_tab_renders_disabled_attribute`

### Commit message
```
feat(tabs): per-tab disabled state, refined hover, focus-visible coverage
```

---

## Task 9: Badge — ramp-driven tones

**Files:**
- Modify: `crates/design-system/src/badge.rs`
- Modify: `crates/design-system/assets/components.css` (+ mirror)

### Background
`BadgeTone` already has Neutral/Info/Success/Warning/Danger variants. Plan B keeps the enum; refines the CSS to use Plan A ramps for richer color contrast and adds a `Live` tone for in-session indicators.

### API additions
```rust
#[derive(Clone, PartialEq, Default)]
pub enum BadgeTone {
    #[default]
    Neutral,
    Info,
    Success,
    Warning,
    Danger,
    Live,    // new — for ● LIVE in live-room hero
}
```

### CSS

Replace existing `.badge / .role-badge / .type-pill / .track-active / .live-pill` family:

```css
.badge,
.role-badge,
.type-pill,
.track-active,
.live-pill {
  display: inline-flex;
  align-items: center;
  gap: var(--space-1);
  min-height: 24px;
  padding: var(--space-0_5) var(--space-3);
  border-radius: var(--radius-full);
  border: 1px solid transparent;
  font-family: var(--font-body);
  font-size: var(--text-xs);
  font-weight: 800;
  text-transform: capitalize;
}

.badge-neutral, .role-badge, .type-pill {
  color: var(--color-neutral-700);
  background: var(--color-neutral-100);
  border-color: var(--color-neutral-200);
}
.badge-info {
  color: var(--color-info-700);
  background: var(--color-info-50);
  border-color: var(--color-info-200);
}
.badge-success {
  color: var(--color-success-700);
  background: var(--color-success-50);
  border-color: var(--color-success-200);
}
.badge-warning {
  color: var(--color-warning-700);
  background: var(--color-warning-50);
  border-color: var(--color-warning-200);
}
.badge-danger {
  color: var(--color-danger-700);
  background: var(--color-danger-50);
  border-color: var(--color-danger-200);
}
.badge-live,
.live-pill {
  color: #fff;
  background: var(--color-oxblood-600);
  border-color: var(--color-oxblood-700);
  animation: badge-pulse 2s var(--ease-snappy) infinite;
}

@keyframes badge-pulse {
  0%, 100% { box-shadow: 0 0 0 0 rgba(143, 36, 61, 0.45); }
  50% { box-shadow: 0 0 0 6px rgba(143, 36, 61, 0); }
}
```

### Tests
- `badge_live_tone_renders_class`
- `badge_warning_tone_uses_warning_class`

### Commit message
```
feat(badge): add Live tone, ramp-driven contrast across all tones
```

---

## Task 10: Spinner — sized variants

**Files:**
- Modify: `crates/design-system/src/spinner.rs`
- Modify: `crates/design-system/assets/components.css` (+ mirror)

### API additions
```rust
#[derive(Props, Clone, PartialEq)]
pub struct SpinnerProps {
    #[props(default)]
    pub size: SpinnerSize,
    #[props(default)]
    pub aria_label: Option<String>,
}

#[derive(Clone, PartialEq, Default)]
pub enum SpinnerSize {
    Xs,
    Sm,
    #[default]
    Md,
    Lg,
}
```

Class map: `"ds-spinner ds-spinner--{xs|sm|md|lg}"`.

### CSS

```css
.ds-spinner {
  display: inline-block;
  border: 3px solid var(--color-neutral-200);
  border-top-color: var(--color-accent);
  border-radius: var(--radius-full);
  animation: ds-spin 0.9s linear infinite;
}
.ds-spinner--xs { width: 12px; height: 12px; border-width: 2px; }
.ds-spinner--sm { width: 18px; height: 18px; border-width: 2px; }
.ds-spinner--md { width: 24px; height: 24px; border-width: 3px; }
.ds-spinner--lg { width: 36px; height: 36px; border-width: 4px; }

@keyframes ds-spin {
  to { transform: rotate(360deg); }
}
```

### Tests
- `spinner_sm_renders_class`
- `spinner_default_is_md`

### Commit message
```
feat(spinner): sized variants (xs/sm/md/lg), token-driven color and motion
```

---

## Task 11: Progress Bar — variants + accessibility

**Files:**
- Modify: `crates/design-system/src/progress_bar.rs`
- Modify: `crates/design-system/assets/components.css` (+ mirror)

### API additions
```rust
#[derive(Props, Clone, PartialEq)]
pub struct ProgressBarProps {
    pub value: f64,            // 0.0–1.0 (or 0–100 — pick existing convention)
    #[props(default)]
    pub variant: ProgressVariant,
    #[props(default)]
    pub indeterminate: bool,
    #[props(default)]
    pub aria_label: Option<String>,
}

#[derive(Clone, PartialEq, Default)]
pub enum ProgressVariant {
    #[default]
    Primary,
    Success,
    Warning,
    Danger,
}
```

### CSS

```css
.ds-progress-bar {
  width: 100%;
  height: 6px;
  background: var(--color-neutral-200);
  border-radius: var(--radius-full);
  overflow: hidden;
}
.ds-progress-bar-fill {
  height: 100%;
  background: var(--color-primary);
  border-radius: inherit;
  transition: width var(--duration-medium) var(--ease-decelerate);
}
.ds-progress-bar--success .ds-progress-bar-fill { background: var(--color-success-500); }
.ds-progress-bar--warning .ds-progress-bar-fill { background: var(--color-warning-500); }
.ds-progress-bar--danger  .ds-progress-bar-fill { background: var(--color-danger-500); }

.ds-progress-bar--indeterminate .ds-progress-bar-fill {
  width: 40%;
  animation: ds-progress-indeterminate 1.4s var(--ease-page-in) infinite;
}
@keyframes ds-progress-indeterminate {
  from { transform: translateX(-100%); }
  to   { transform: translateX(250%); }
}
```

### Tests
- `progress_bar_renders_variant_class`
- `progress_bar_indeterminate_adds_class`

### Commit message
```
feat(progress-bar): variants, indeterminate mode, accessible aria-label
```

---

## Task 12: Tooltip — new primitive

**Files:**
- Create: `crates/design-system/src/tooltip.rs`
- Modify: `crates/design-system/src/lib.rs` (export Tooltip)
- Modify: `crates/design-system/assets/components.css` (+ mirror)

### Behaviour
Pure-CSS tooltip — show on hover/focus of the parent. No JS; CSS `:hover` / `:focus-within` on the wrapper drives visibility. Position is fixed to `top` by default.

### API
```rust
#[derive(Props, Clone, PartialEq)]
pub struct TooltipProps {
    pub label: String,
    pub children: Element,
    #[props(default)]
    pub side: TooltipSide,
}

#[derive(Clone, PartialEq, Default)]
pub enum TooltipSide {
    #[default]
    Top,
    Bottom,
    Left,
    Right,
}

#[component]
pub fn Tooltip(props: TooltipProps) -> Element {
    let side_class = match props.side {
        TooltipSide::Top => "ds-tooltip--top",
        TooltipSide::Bottom => "ds-tooltip--bottom",
        TooltipSide::Left => "ds-tooltip--left",
        TooltipSide::Right => "ds-tooltip--right",
    };
    rsx! {
        span { class: "ds-tooltip-wrap",
            {props.children}
            span {
                class: "ds-tooltip {side_class}",
                role: "tooltip",
                "{props.label}"
            }
        }
    }
}
```

### CSS

```css
.ds-tooltip-wrap {
  position: relative;
  display: inline-flex;
}
.ds-tooltip {
  position: absolute;
  z-index: 1100;
  padding: var(--space-1) var(--space-2);
  background: var(--color-neutral-900);
  color: var(--color-surface);
  border-radius: var(--radius-sm);
  font-size: var(--text-xs);
  line-height: var(--leading-xs);
  white-space: nowrap;
  opacity: 0;
  pointer-events: none;
  transform: translateY(-2px);
  transition:
    opacity var(--duration-fast) var(--ease-snappy),
    transform var(--duration-fast) var(--ease-snappy);
}
.ds-tooltip-wrap:hover .ds-tooltip,
.ds-tooltip-wrap:focus-within .ds-tooltip {
  opacity: 1;
  transform: translateY(0);
}

.ds-tooltip--top    { bottom: calc(100% + var(--space-1)); left: 50%; transform: translate(-50%, -2px); }
.ds-tooltip--bottom { top:    calc(100% + var(--space-1)); left: 50%; transform: translate(-50%,  2px); }
.ds-tooltip--left   { right: calc(100% + var(--space-1)); top: 50%;  transform: translate(-2px, -50%); }
.ds-tooltip--right  { left:  calc(100% + var(--space-1)); top: 50%;  transform: translate( 2px, -50%); }

.ds-tooltip-wrap:hover .ds-tooltip--top    { transform: translate(-50%, 0); }
.ds-tooltip-wrap:hover .ds-tooltip--bottom { transform: translate(-50%, 0); }
.ds-tooltip-wrap:hover .ds-tooltip--left   { transform: translate(0, -50%); }
.ds-tooltip-wrap:hover .ds-tooltip--right  { transform: translate(0, -50%); }
```

### Tests
- `tooltip_renders_label_and_role`
- `tooltip_side_class_applied`

### `lib.rs` additions
```rust
pub mod tooltip;
pub use tooltip::{Tooltip, TooltipSide};
```

### Commit message
```
feat(tooltip): introduce Tooltip primitive (pure-CSS hover/focus)
```

---

## Task 13: Avatar — new primitive

**Files:**
- Create: `crates/design-system/src/avatar.rs`
- Modify: `crates/design-system/src/lib.rs` (export Avatar)
- Modify: `crates/design-system/assets/components.css` (+ mirror)

### API
```rust
#[derive(Props, Clone, PartialEq)]
pub struct AvatarProps {
    /// Display name; used for initials fallback.
    pub name: String,
    #[props(default)]
    pub image_url: Option<String>,
    #[props(default)]
    pub size: AvatarSize,
}

#[derive(Clone, PartialEq, Default)]
pub enum AvatarSize {
    Xs,
    Sm,
    #[default]
    Md,
    Lg,
}

#[component]
pub fn Avatar(props: AvatarProps) -> Element {
    let size_class = match props.size {
        AvatarSize::Xs => "ds-avatar--xs",
        AvatarSize::Sm => "ds-avatar--sm",
        AvatarSize::Md => "ds-avatar--md",
        AvatarSize::Lg => "ds-avatar--lg",
    };
    let initials: String = props.name
        .split_whitespace()
        .filter_map(|w| w.chars().next())
        .take(2)
        .collect::<String>()
        .to_uppercase();

    rsx! {
        span {
            class: "ds-avatar {size_class}",
            "aria-label": "{props.name}",
            if let Some(url) = &props.image_url {
                img { class: "ds-avatar-img", src: "{url}", alt: "{props.name}" }
            } else {
                span { class: "ds-avatar-initials", "{initials}" }
            }
        }
    }
}
```

### CSS

```css
.ds-avatar {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  background: var(--color-gold-100);
  color: var(--color-gold-800);
  font-family: var(--font-body);
  font-weight: 800;
  border-radius: var(--radius-full);
  overflow: hidden;
  user-select: none;
}
.ds-avatar--xs { width: 20px; height: 20px; font-size: var(--text-xs); }
.ds-avatar--sm { width: 28px; height: 28px; font-size: var(--text-sm); }
.ds-avatar--md { width: 36px; height: 36px; font-size: var(--text-base); }
.ds-avatar--lg { width: 56px; height: 56px; font-size: var(--text-xl); }

.ds-avatar-img {
  width: 100%;
  height: 100%;
  object-fit: cover;
}
```

### Tests
- `avatar_initials_from_name`
- `avatar_with_image_renders_img_tag`

### `lib.rs` additions
```rust
pub mod avatar;
pub use avatar::{Avatar, AvatarSize};
```

### Commit message
```
feat(avatar): introduce Avatar primitive with image + initials fallback
```

---

## Task 14: Final verification + cross-task review

- [ ] **Step 1: Workspace tests**
```bash
cargo test --workspace --no-fail-fast
```
Expected: only the 2 known pre-existing failures.

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

Both must be empty.

- [ ] **Step 5: Token-only audit**

Grep for raw hex / px / rem in primitive CSS blocks (excluding the `@font-face` block and existing motion-page animation):

```bash
grep -nE "(#[0-9a-f]{3,8}|[0-9]+px|[0-9.]+rem)" crates/design-system/assets/components.css | \
  grep -vE "(@font-face|url\(|rgba\(|--font-|/\\*)" | head -30
```

Each match must be inside a comment OR an `rgba(...)` (used for transparent shadows where token doesn't fit). If a literal hex / px slipped into a primitive rule (not a comment, not rgba), STOP and refactor.

- [ ] **Step 6: Dispatch final cross-task code reviewer**

Run a single review pass over all 13 primitive commits, focused on: cross-component consistency (e.g. focus-visible uses the same gold-400 outline everywhere), state-coverage completeness, backward-compat at call sites.

- [ ] **Step 7: Manual smoke (if dev server available)**

Load `/` and click into a few surfaces. Buttons, badges, modals, tabs should all render with the new look; no console errors; no regressions.

- [ ] **Step 8: Log follow-ups for Plan C**

Anything surfaced during implementation that belongs to composite-pattern work (loading orchestration, form field wrapper, toast, table patterns) gets a one-line bullet in a follow-up note for Plan C's brainstorm.

---

## Spec coverage check

| Vision-spec requirement | Task |
|--|--|
| Button + variants | Task 1 |
| Input + Textarea + error state | Task 2 |
| Select | Task 3 |
| Checkbox + Radio + indeterminate | Task 4 |
| Switch | Task 5 |
| Card | Task 6 |
| Modal | Task 7 |
| Tabs | Task 8 |
| Badge | Task 9 |
| Spinner | Task 10 |
| Progress Bar | Task 11 |
| Tooltip (new) | Task 12 |
| Avatar (new) | Task 13 |
| Full state coverage standard | Convention applied per task |
| Token-only invariant | Audit step in Task 14 |
| Asset-sync mirror | After every CSS edit |
| Backward compat at call sites | Discipline per task; verified Task 14 |
| Density variants on primitives | Out of scope (surface property per vision spec) |
| Loading state on primitives | Out of scope (composite pattern per vision spec) |

## Out of scope

- Composite patterns (Form field wrapper, Empty state, Skeleton, Toast, Table — Plan C).
- Surface application (Plan D).
- Dark-theme values (placeholder block stays empty).
- `datetime_picker`, `markdown_editor`, `file_card`, `empty_state`, `form_error`, `brand` — these are composites or surfaces, deferred.

## Risks

- **Subtle visual delta from refined shadow scale on Card.** `--shadow-sm` value changed in Plan A; Card's default uses it. Manual smoke check at Task 14.
- **`--radius-lg` 8→12 jump on Card** (now uses lg instead of md). Intentional polish; flag visually if it feels off.
- **`Toggle` → `Switch` rename via re-export.** `pub use switch::Switch as Toggle;` keeps callers working but the Toggle component API is now Switch's API. If a caller was using a `Toggle` prop that doesn't exist on Switch, it'll break — verify with `cargo test --workspace` after Task 5.
- **CSS block size.** `components.css` is currently ~1100 lines after Plan A. Plan B will likely grow it to ~1600-1800 lines. Still manageable; future plan can split by component family.
