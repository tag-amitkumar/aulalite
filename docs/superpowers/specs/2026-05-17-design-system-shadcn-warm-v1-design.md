# Design System v1 — "Shadcn-feel on warm cream"

**Date:** 2026-05-17
**Owner:** chiranjib.chaudhuri@geosapiens.com
**Status:** Approved (brainstorming) — pending implementation plan.

## 1. Purpose & framing

Lift the current Dioxus/Rust design system from "bare minimum" to a feature-dense, polished SaaS aesthetic that visibly matches the shadcn/Next.js polish bar while keeping the project's existing **warm cream + serif** brand identity. This is v1 of a sequenced effort (D in the brainstorming options):

- **v1 (this spec):** Token overhaul + 10 primitives + motion catalog + kitchen-sink route + smoke tests.
- v2 (future spec): Dashboard route redesign on top of v1.
- v3 (future spec): Flagship route redesign (course detail / live session).

Out of scope for v1: dashboard/course/live-session route redesigns, dark mode, components beyond the 10 listed, marketing pages.

## 2. Aesthetic direction

Locked decisions from brainstorming:

| Axis | Decision |
|------|---------|
| Brand identity | Warm cream paper + Source Serif 4 display + Inter body — preserved. |
| Density | shadcn-tight: 36px default control height, 14px base text, tight rows. |
| Accent strategy | Green is sole `--brand-primary`. Gold, navy, oxblood remain in the system as *meaningful* accents only: gold = premium / paywall / focus ring tint, navy = info, oxblood = live + destructive flair. No decorative accent leakage. |
| Motion | Restrained polish (hover/focus/dialog enter-exit) **+** page-in stagger choreography **+** warm-paper personality moments (paper-press on buttons, gold-tinted focus rings, warm-halo card hover, gold tab underline slide). |
| Radius | Unified `--radius` root (`8px`) drives `--radius-sm/md/lg/xl` like shadcn. Existing radius tokens kept for back-compat. |
| Shadows | Flatter shadcn-leaning defaults (`--shadow-flat`) + warm-tinted hover (`--shadow-warm-hover`) + paper-press inset. Existing layered shadow tokens preserved. |

The combination is intentional: the brand stays editorial (cream + serif + gold-tinted personality), the product feels modern SaaS (tight density, shadcn motion vocabulary, sleek primitives).

## 3. Architecture & file layout

No new crates. All work lives in `crates/design-system/`.

- `assets/tokens.css` — overhauled in place, additive. Existing tokens stay so all routes keep compiling. New tokens layer on top.
- `assets/components.css` — restructured into clearly labeled `/* === component === */` sections, one per primitive. The new motion catalog gets its own labeled block at the top.
- `src/` — the 10 component Rust modules updated. **Public Rust signatures stay backwards-compatible. New variants are additive (optional props with sensible defaults).** Two net-new files: `src/dropdown_menu.rs`, `src/sheet.rs`. `lib.rs` re-exports them.
- `crates/shell-web/src/routes/` — one debug-only route `dev_components.rs` (kitchen sink for visual QA). Gated behind `cfg(debug_assertions)`; not linked from the main app.
- `crates/shell-web/tests/e2e/design_system_smoke.spec.js` — new Playwright smoke spec.

**Migration discipline:** every commit must keep `cargo check -p shell-web` green. Existing class names (e.g., `.ds-btn`) stay valid; new classes layer alongside until routes opt in.

## 4. Token overhaul

Additions to `crates/design-system/assets/tokens.css` (existing tokens unchanged).

### 4.1 Control sizing (tight density)
```
--control-h-sm: 28px;     /* dense table actions, chips */
--control-h-md: 36px;     /* default — buttons, inputs, selects */
--control-h-lg: 44px;     /* hero CTAs only */
--control-px-sm: 8px;
--control-px-md: 12px;
--control-px-lg: 16px;
--control-gap: 6px;       /* icon ↔ label inside a control */
```

### 4.2 Radius (unified, shadcn-style)
```
--radius: 8px;
--radius-sm: calc(var(--radius) - 4px);   /* 4px */
--radius-md: calc(var(--radius) - 2px);   /* 6px */
--radius-lg: var(--radius);                /* 8px */
--radius-xl: calc(var(--radius) + 4px);    /* 12px */
```
Existing `--radius-xs / --radius-2xl / --radius-full` are kept for back-compat. New components use the unified names above.

### 4.3 Ring (focus) — gold-tinted (custom touch)
```
--ring:        0 0 0 2px var(--color-surface),
               0 0 0 4px rgba(201, 164, 92, 0.55);   /* gold halo */
--ring-danger: 0 0 0 2px var(--color-surface),
               0 0 0 4px rgba(163, 58, 54, 0.55);
--ring-offset: var(--color-surface);
```

### 4.4 Shadows — flat default + warm hover + paper-press
Existing `--shadow-xs/sm/md/lg/xl/2xl/gold` are kept. Add:
```
--shadow-flat:        0 1px 0 rgba(18,22,20,0.04),
                      0 0 0 1px var(--color-rule);
--shadow-press:       inset 0 1px 2px rgba(18,22,20,0.10);
--shadow-warm-hover:  0 6px 18px rgba(201, 164, 92, 0.10),
                      0 1px 0 rgba(18,22,20,0.04),
                      0 0 0 1px var(--color-rule);
```

### 4.5 Motion timing
Existing `--duration-fast/medium/slow/page-in` and ease tokens kept. Add:
```
--duration-enter: 180ms;
--duration-exit:  140ms;
--ease-content:   cubic-bezier(0.32, 0.72, 0, 1);   /* Radix-style */
--press-y:        1px;                              /* paper-press translate */
```

### 4.6 Surface tokens (for new components)
```
--surface-card:        var(--color-surface);
--surface-card-hover:  #fffaf0;             /* faintly warmer */
--surface-muted:       var(--color-surface-subtle);
--surface-overlay:     rgba(15, 17, 14, 0.45);   /* dialog/sheet backdrop */
--surface-popover:     #fffdf7;             /* dropdown, tooltip, popover */
```

### 4.7 Brand / accent semantic remap (alias-only; no value changes)
```
--brand-primary:        var(--color-green-600);
--brand-primary-hover:  var(--color-green-700);
--brand-primary-active: var(--color-green-800);
--accent-premium:       var(--color-gold-500);     /* gold — paywall, CTA glow */
--accent-info:          var(--color-navy-600);     /* navy — info badges */
--accent-live:          var(--color-oxblood-500);  /* oxblood — Live, destructive flair */
```
Existing `--color-primary`, `--color-accent`, `--color-live`, etc. kept; new aliases clarify semantic role for new components.

## 5. Motion catalog

A single block at the top of `components.css`. All CSS, no JS animation library.

### 5.1 Interactive feedback (every interactive surface)
- Hover: `transition: background-color, border-color, box-shadow var(--duration-fast) var(--ease-snappy)`.
- Active (paper-press, **buttons only**): `translateY(var(--press-y))` + `--shadow-press`, 120ms.
- Focus-visible: `--ring` fades in over 140ms (suppressed on mouse-only focus per `:focus-visible`).
- Disabled: no transitions; static 40% opacity.

### 5.2 Component entrance/exit (Dialog, Sheet, DropdownMenu, Tooltip, Toast, Popover)
Pattern: `data-state="open"` triggers enter animation; `data-state="closed"` triggers exit. CSS keyframes named:
- `ds-fade-in` / `ds-fade-out`
- `ds-slide-in-{right,left,top,bottom}` / `ds-slide-out-{right,left,top,bottom}`
- `ds-zoom-in-95` / `ds-zoom-out-95`

Standard pairings:

| Component | Enter | Exit |
|-----------|-------|------|
| Tooltip | zoom-in-95 + fade, 120ms | reverse, 80ms |
| Dropdown / Popover | slide-in 4px from trigger side + fade, 180ms | reverse, 140ms |
| Dialog | zoom-in-95 + fade, 220ms; backdrop fade 160ms | reverse, 160ms |
| Sheet | slide-in from edge + backdrop fade, 220ms | reverse, 160ms |
| Toast | slide-in-right 12px + fade, 200ms | fade + slide-right 8px, 140ms |

### 5.3 Page-in choreography (route mount)
- Top-level layout adds class `ds-page-enter` on route change.
- Within `.ds-page-enter`, direct children declaring `data-stagger="N"` (N=1..6) animate `opacity 0 → 1` + `translateY(8px → 0)` over 400ms, staggered 60ms each (`animation-delay: calc(var(--stagger) * 60ms)`).
- **One-shot** — uses `animation` (not `transition`) on a mount-keyed class so it never replays on re-render of the same route.

### 5.4 Warm-paper personality moments
- **Button press:** paper-press is the *only* control that gets `--shadow-press` inset.
- **Card hover** (when `interactive=true`): swaps `--shadow-flat` → `--shadow-warm-hover` over 200ms.
- **Focus rings:** gold-tinted by default (`--ring`); oxblood when invalid (`--ring-danger`).
- **Tabs underline slide:** 200ms `--ease-content`, gold 1px (matches focus-ring intent).
- **Gold is reserved.** Beyond the focus ring and tab underline, gold appears only via explicit `premium` variants on Button, Card, Badge, and Toast — never as decoration. The whole-system rule is: if it's not communicating "premium," it isn't gold.

### 5.5 Reduced motion (global)
A `@media (prefers-reduced-motion: reduce)` block at the bottom of `components.css`:
- All `animation-duration` → `0.01ms`.
- Transitions on `transform` removed.
- Hover/focus *color* transitions retained (color change is information, not motion).

## 6. Component specs (10)

API rule across all: **public Rust signatures stay backwards-compatible**. New variants are additive optional props with sensible defaults.

### 6.1 Button — `dsButton`
- **Variants:** `primary` (green), `secondary` (cream/outline), `ghost` (transparent + hover surface), `destructive` (oxblood), `premium` (gold, used sparingly), `link` (text-only).
- **Sizes:** `sm` 28px, `md` 36px (default), `lg` 44px, `icon` 36×36 square.
- **States:** hover (color shift + `--shadow-warm-hover`); active (paper-press: `translateY(--press-y)` + `--shadow-press`); focus-visible (`--ring`); disabled (40% opacity, no shadows); `loading` prop swaps label for inline spinner.
- **Slots:** `leading` and `trailing` icon props.

### 6.2 Card — `dsCard`
- Default: flat (`--shadow-flat`), `--radius-lg`; optional `--shadow-warm-hover` on hover when `interactive=true`.
- **New subcomponents** (shadcn-pattern composition): `Card::Header`, `Card::Title`, `Card::Description`, `Card::Content`, `Card::Footer`. Existing single-slot `Card` keeps working.
- Variant `tone="premium"` adds a 1px gold inner border.

### 6.3 Input + Field + FormError — `dsInput`, `dsField`, `dsFormError`
- **Input:** 36px height, 12px x-padding, `--radius-md`, 1px rule border. Focus → `--ring` + green border. Slots: `leading_icon`, `trailing_icon`, `addon_left`, `addon_right` (e.g. "$" prefix / "kg" suffix).
- **Field** wraps Input with `Label` (above), `Description` (below, muted), `FormError` (oxblood, slide-down on error).
- **Invalid state:** oxblood border + `--ring-danger`.

### 6.4 Badge — `dsBadge`
- **Variants:** `default` (neutral), `primary` (green), `success`, `warning`, `danger`, `info` (navy), `premium` (gold), `live` (oxblood + 6px pulsing dot on the left).
- **Sizes:** `sm` 18px, `md` 22px. `--radius-sm`. 12px text, weight 500.

### 6.5 Table — `dsTable`
- Tight rows (40px); sticky `<thead>` with subtle bottom rule.
- Zebra OFF by default; available via `striped` prop. Row hover = `--surface-card-hover`.
- **Subcomponents:** `Table::Row`, `Table::Cell`, `Table::HeaderCell` (sortable variant with chevron icon + 180ms rotate on toggle), `Table::EmptyState` (reuses `dsEmptyState`).
- **New:** `Table::Toolbar` slot above the head — for search/filter chips, renders inline with the table card.

### 6.6 Tabs — `dsTabs`
- **Underline style (default):** 1px gold underline slides between active tabs (200ms `--ease-content`).
- **Pill style** (`variant="pill"`): rounded `--radius-md` background slides under active tab.
- **Keyboard:** arrow keys, Home/End. ARIA roles: tablist/tab/tabpanel.

### 6.7 DropdownMenu — `dsDropdownMenu` (NEW)
- Composition: Trigger + Content + Item + Separator + Label + SubMenu.
- Content: `--surface-popover`, `--shadow-lg`, `--radius-md`. Enter: fade + 4px slide-down (`--duration-enter`, `--ease-content`); Exit: reverse, `--duration-exit`.
- Item hover: `--surface-muted`. Item `tone="danger"` for destructive entries.
- **Positioning (v1):** simple absolute, anchored via `align="start|end"` prop. No floating-ui port.

### 6.8 Sheet — `dsSheet` (NEW)
- Side drawer; replaces ad-hoc "edit panel" uses of Modal.
- **Sides:** `right` (default, 440px), `left`, `top`, `bottom`. Backdrop = `--surface-overlay`.
- **Motion:** backdrop fade 140ms; panel slides in from edge 220ms `--ease-content`. Exit reverses.
- **Subcomponents:** `Sheet::Header`, `Sheet::Title`, `Sheet::Description`, `Sheet::Body`, `Sheet::Footer`.
- Closes on Escape, backdrop click, and explicit close button.

### 6.9 Skeleton — `dsSkeleton` (rework)
- Replaces flat block with 1.4s shimmer:
  `linear-gradient(90deg, --color-neutral-200 0%, --color-neutral-100 50%, --color-neutral-200 100%)` translating left→right.
- **Helpers:** `Skeleton::Line(w)`, `Skeleton::Circle(size)`, `Skeleton::Card`.

### 6.10 Toast — `dsToast` (rework)
- Stacked bottom-right by default. Position is set on the Dioxus context provider (`ToastProvider { position: ToastPosition::BottomRight, ... }`); other positions (`TopRight`, `TopCenter`, `BottomCenter`) are available but the default ships unchanged for back-compat. Enter: slide-in-right 12px + fade, 200ms; Exit: fade + slide-right 8px, 140ms.
- **Tones** map to accents: `success`, `info` (navy), `warning`, `danger` (oxblood), `premium` (gold).
- One action slot + close button.

## 7. Testing & validation

- **Compile gate (primary safety net):** every existing route in `shell-web` must build with the new `design-system` at every commit. No public Rust signature changes; new variants additive.
- **Kitchen-sink route** `/dev/components`: renders every variant of every primitive in every state (default/hover/focus/active/disabled, all sizes, all tones). Used for manual visual QA.
- **Playwright smoke** — new `crates/shell-web/tests/e2e/design_system_smoke.spec.js`:
  - Navigates to `/dev/components`; full-page screenshot snapshot per primitive.
  - Asserts focus ring visible on Tab navigation through Button → Input → Tabs.
  - Asserts Dialog / Sheet / DropdownMenu open + close (state attr + focus return).
  - Asserts `prefers-reduced-motion` is honored (emulated).
- **No CSS unit tests.** Visual snapshots cover regressions cheaper than testing class strings.

## 8. Deliverable sequence (single PR for v1)

1. `tokens.css` overhaul (additive).
2. Motion catalog block in `components.css` (keyframes + reduced-motion media query).
3. Six existing primitives reworked: Button → Card → Input/Field/FormError → Badge → Table → Tabs.
4. Two net-new primitives: DropdownMenu → Sheet.
5. Two polish reworks: Skeleton (shimmer) → Toast (stacked corner motion).
6. `/dev/components` kitchen-sink route + Playwright smoke spec.
7. Final visual QA pass against kitchen sink; fix anything off-spec.

## 9. Out of scope (explicit)

- Dashboard / course detail / live session route redesigns (v2, v3).
- Dark mode (`[data-theme="dark"]` token stub stays empty).
- Components beyond the 10 listed (floating-ui port, Combobox, Calendar, etc.).
- Marketing pages / landing redesign.

## 10. Risks & mitigations

| Risk | Mitigation |
|------|-----------|
| Token overhaul accidentally breaks existing routes via aliased value shifts. | All changes additive; existing token names and values untouched. New tokens use new names. |
| Existing `Card` usages break when subcomponents added. | New subcomponents are additive; single-slot `Card` API retained. |
| Motion choreography feels heavy on slower devices. | Page-in is one-shot 400ms; component enter-exits are <220ms; `prefers-reduced-motion` falls back to color-only. |
| "Warm-paper" personality dilutes shadcn polish if overused. | Gold is reserved for the focus ring, tab underline, and explicit `premium` variants on Button/Card/Badge/Toast (rule documented in §5.4). Card warm-halo is opt-in via `interactive=true`. |
| Sheet + Modal coexistence confuses callers. | Sheet for side-anchored editing/detail panels; Modal for centered destructive confirms / focused tasks. Document the rule in the new kitchen sink. |

## 11. Open questions

None at brainstorming sign-off. Implementation plan will surface task-level questions.
