# UI Polish Vision — AulaLite Design System v2

Date: 2026-05-16
Status: Vision spec for a four-plan polish initiative

This document captures the target aesthetic, quality bar, and decomposition for a coordinated polish pass across the AulaLite UI. It is the **shared contract** that four phased plans will execute against. It deliberately does not specify task-level work — that belongs in the per-plan brainstorms.

---

## Vision Statement

Raise the AulaLite UI from "basic warm-academy theme" to **Apple/iCloud Education-grade polish** while preserving the existing warm cream/green/gold identity. The goal is **craftsmanship, not redesign**: same soul, dramatically better execution.

A first-time visitor should feel that the product was made with care — refined typography, deliberate spacing, considered states, restrained motion, and quiet color. Students and teachers should feel they are using a premium tool, not a generic LMS.

---

## Reference and Influences

**Primary reference:** Apple / iCloud Education — pristine paper feel, soft shadows, museum-like restraint, big type, sparse interactions.

**Polish discipline reference:** shadcn/ui — full primitive state coverage, token-first architecture, accessible by default, motion-aware.

**Not references:** Linear/Notion (too dense and cool), Stripe Dashboard (too neutral), Cal.com/Vercel (good but different visual register). We borrow shadcn's *engineering discipline*, not its visual neutrality.

---

## Locked Decisions

These nine decisions are the contract; every downstream plan must respect them.

### 1. Direction: craft, not redesign

The existing warm-academy palette and identity stay. No major aesthetic shift. Improvements are about *execution quality*: refined states, harmonious spacing, considered typography, polished motion.

### 2. Density: two-tier

| Tier | Surfaces | Spacing baseline |
|--|--|--|
| Roomy (breathing) | Dashboard, course detail header/hero, auth screens, course list cards, post-session screens, marketing-ish moments | Default spacing scale, used generously |
| Compact (workhorse) | Course builder, schedule list, submissions table, live-room sidebars (chat/presence/hand-raise), assignment editor body | Same scale but at the next-tighter step (e.g. `var(--space-3)` where roomy uses `var(--space-4)`) |

Density is a **surface-level property**, not a primitive-level property. Primitives have one canonical size; the surrounding container picks the spacing rhythm.

### 3. Typography: Source Serif 4 + Inter, self-hosted

- **Display:** Source Serif 4 (variable weight 200-900) — for `h1`, page hero titles, course titles, marketing moments.
- **Body:** Inter (variable, 100-900) — for everything else (`h2`-`h6`, body, labels, captions, code-adjacent).
- **Mono:** Cascadia Code / Consolas / monospace fallback chain (unchanged).
- **Hosting:** self-host both fonts under `crates/shell-web/public/assets/fonts/`. Use `font-display: swap` and `preload` the most-used weights. Subset to Latin where viable to minimize bytes.
- **Total budget:** ~80-120 KB across both families.

### 4. Palette: 4 brand chromatic + 5 semantic, ramped 50-950

The current named colors stay:
- Brand chromatic: green (primary), gold, navy, oxblood (live).
- Semantic: info, success, warning, danger, live (overlaps oxblood; that's OK).

Each gets a proper **50–950 ramp** (11 steps) so component states (resting / hover / active / focus / disabled / pressed / bg-tinted) draw from the same well. Today's tokens like `--color-primary` and `--color-primary-hover` become `--color-primary-600` and `--color-primary-700`; semantic aliases (`--color-primary`, `--color-primary-hover`, etc.) remain for ergonomic component CSS.

Backgrounds (paper, paper-warm, surface, surface-subtle, ink) become a 5-step neutral ramp anchored on the existing warm-cream identity, not pure white/gray.

### 5. Dark mode: structurally prepared, not shipped

- Tokens reorganize under `:root` (light) and `[data-theme="dark"]` (placeholder, empty for now).
- Every primitive and surface references only token names — no hardcoded hex.
- The dark theme is a future plan; this initiative ships light only but leaves zero refactoring debt for when dark lands.

### 6. Motion: layered cubic-bezier (CSS only)

Replace today's three eases (`fast`, `medium`, `page`) with a richer set:

| Token | Duration | Curve | Use |
|--|--|--|--|
| `--ease-snappy` | 120ms | `cubic-bezier(.4, 0, .2, 1)` | hover, focus, small state changes |
| `--ease-decelerate` | 240ms | `cubic-bezier(0, 0, .2, 1)` | element entries, dropdowns opening |
| `--ease-accelerate` | 180ms | `cubic-bezier(.4, 0, 1, 1)` | element exits, dropdowns closing |
| `--ease-spring` | 320ms | `cubic-bezier(.34, 1.56, .64, 1)` | modal pop, popovers, "alive" moments |
| `--ease-page-in` | 420ms | `cubic-bezier(.2, .8, .2, 1)` | full-page transitions (existing) |

No JS/wasm springs. The Rust/wasm ecosystem has no battle-tested spring library and the maintenance cost isn't worth the marginal feel gain over careful cubic-bezier.

Honor `prefers-reduced-motion` everywhere; the existing media query in `components.css` stays and gets extended to anything new.

### 7. Iconography: stay with Lucide

Keep `dioxus-free-icons` + Lucide. Line-art weights well against the Source Serif 4 display headlines and the Inter body. Migration cost would be high; visual return is low. Plans may introduce a small set of *custom* glyphs (e.g. a refined live indicator) where Lucide is genuinely missing one.

### 8. State coverage standard: Practical

Each primitive ships with these states (where applicable to its semantics):

- `default` — resting
- `hover` — pointer over
- `active` — pressed / mouse-down
- `focus` — has keyboard focus (always-visible ring)
- `focus-visible` — keyboard-focus-only ring (no ring on mouse focus)
- `disabled` — non-interactive
- `error` — invalid (inputs, selects, textareas only)

Loading is a **composite pattern** (Plan C) not a primitive state. Density is a **surface property** (per decision 2) not a primitive prop. Selected / indeterminate are component-specific (Tabs, Checkbox) and called out per primitive.

### 9. Architecture: token-first

No primitive should reference a hex or rem value directly outside its own variant lookup table. Every spacing, color, font-size, line-height, radius, shadow, and ease must come from a token. This enables dark mode and future density adjustments without component-code changes.

---

## Decomposition

Four phased plans, executed in order. Each plan is shippable on its own and visibly raises quality.

### Plan A — Foundation (tokens v2 + fonts)

Lays the new design vocabulary the rest of the initiative depends on. No visible component change; the existing UI continues to work, now drawing from the new tokens. Concrete scope:

- Self-host Source Serif 4 + Inter under `crates/shell-web/public/assets/fonts/`. `@font-face` declarations + `preload` links in `index.html`.
- Build the 50–950 ramps for each chromatic + semantic color in `tokens.css`. Preserve existing `--color-primary` etc. as semantic aliases.
- Build the neutral surface ramp (paper / surface / ink) as a 5-step scale.
- Type scale tokens (`--text-xs` through `--text-5xl`, plus matching line-height pairs).
- Spacing scale (preserve 4px base, extend if missing tokens — e.g. `--space-0`, `--space-10`).
- Radius scale (`--radius-xs/sm/md/lg/xl`, replacing the current ad-hoc trio).
- Elevation/shadow scale (`--shadow-xs/sm/md/lg/xl`, layered for soft-Apple feel).
- Motion ease tokens (decision 6).
- Dark-mode-ready `:root` / `[data-theme="dark"]` structure (dark variant blank).
- Mirror to `crates/shell-web/public/assets/tokens.css` (asset-sync test).

**Out of scope for Plan A:** any primitive or surface change beyond what's needed to compile against the new tokens.

### Plan B — Primitives (full state coverage)

Polish every primitive in `crates/design-system/src/` to the locked state-coverage standard. Concrete scope (one task per primitive family):

- Button (variants: primary / secondary / danger / ghost / link)
- Input + Textarea
- Select
- Checkbox + Radio
- Switch (new — replace current ad-hoc toggle)
- Card
- Modal
- Tabs
- Badge
- Tooltip (new)
- Avatar (new — for member rows)
- Spinner (cleaner SVG, sized variants)

Each primitive: state coverage per decision 8, motion using Plan A eases, all hex/rem values referencing tokens, accessibility props (`aria-*`, keyboard handling) where the primitive needs them.

**Out of scope for Plan B:** composite patterns, surface application.

### Plan C — Patterns (composites)

Build the composite patterns that surfaces will use. Concrete scope:

- Form pattern: Field wrapper (label / control / helper / error), inline validation rhythm, field groups (sectioned form areas).
- Table / List pattern: row hover, zebra (subtle), density-aware, sticky header, empty/loading variants.
- Empty state pattern: typography + illustration slot + CTA slot. Replace today's bare `EmptyState`.
- Skeleton pattern: per-content shapes (line / circle / card / table-row).
- Toast: replace the inline `dx-toast` in `index.html` with a token-driven, accessibility-aware toast pattern that lives in `design-system`.
- Page header pattern: kicker + title + subtitle + actions row, used by every page.
- Card-list pattern: course cards, schedule entries, assignment cards.
- Loading composite: spinner + skeleton orchestration for async data.

**Out of scope for Plan C:** applying these to specific pages.

### Plan D — Surfaces (apply A-C)

Apply the new foundation, primitives, and patterns to specific surfaces. One task per surface (or surface family). Concrete scope:

- Auth screens (login, signup, forgot, accept-invite, redeem)
- Dashboard
- Course list
- Course detail (outline / people / schedule / edit tabs)
- Course builder (modules + lessons; respects the Task 4/5 rename UI from the quick-fix pass)
- Schedule view (`/schedule`)
- Assignment editor + detail + grading
- Live room (lobby → broadcast → in-session → replay → ended)
- Page transitions and motion polish at the route level

For each surface: switch to the two-tier density (decision 2), apply patterns from Plan C, refine empty / loading / error states, polish hero moments.

**Out of scope for Plan D:** new features. This is polish, not new functionality.

---

## Cross-Cutting Concerns

### Asset sync hazard

CSS and font files in `crates/design-system/assets/` must mirror to `crates/shell-web/public/assets/` (enforced by `shell_and_design_system_assets_stay_in_sync`). Every plan that touches design-system assets MUST include the mirror step. The quick-fix pass hit this trap twice; this initiative pre-empts it.

### Token-first audit

Every plan in this initiative ends with a small audit: grep for hardcoded hex / rem / px values in the surfaces touched and replace with tokens. This is the only way to keep the dark-mode-ready promise honest.

### Accessibility floor

Every primitive in Plan B and every pattern in Plan C must hit at minimum:
- Visible focus indicator for keyboard users (focus-visible).
- Color contrast 4.5:1 for body text, 3:1 for large text (WCAG AA).
- Semantic markup (`button`, `nav`, `main`, `aside`, `dialog`, `label`, etc.).
- `aria-*` where the role isn't carried by the tag.
- Respect `prefers-reduced-motion`.

Beyond AA is welcome but not required.

### Performance budget

- Self-hosted fonts: ≤120 KB total before gzip across the two families.
- Critical CSS (tokens.css + components.css served on first paint): ≤80 KB before gzip.
- No new JS dependencies (Rust/Dioxus only).
- Lighthouse / DevTools sanity check at each plan's wrap-up.

### Per-plan brainstorm

Each phased plan A-D gets its own brainstorm before its writing-plans pass. This vision spec sets the constraints; the per-plan brainstorm picks tasks and concrete approaches within them.

---

## Out of Scope (for the whole initiative)

- Dark theme values (decision 5 — structure only).
- New features. This is polish, not product.
- Mobile-app shells (`crates/shell-mobile`, `crates/shell-desktop`) — the focus is `shell-web`.
- I18n / RTL — these may benefit from token-first CSS but are not scoped here.
- Marketing site / external pages.
- Icon migration (decision 7).
- Backend changes (none required).

---

## Risks & Notes

- **Scope creep into product changes.** Plan D in particular will surface UX problems (e.g. "the schedule view should also do X"). Discipline: flag, defer, ship polish only. Each surface task ends with a "follow-ups discovered" section that becomes future feature work.
- **Compounding visual changes.** Plans A-C are infrastructure with little visible payoff until Plan D applies them. Communicate this to stakeholders so progress isn't measured by screenshots alone.
- **Font flash (FOUT/FOIT).** Self-hosting + `font-display: swap` + `preload` minimizes the perceived flash. Plan A includes a brief test plan for FOUT severity across slow connections.
- **CSS bloat.** A 50–950 ramp × 4 chromatic × 5 semantic = ~99 color tokens. Add ramps for neutrals and that's 110+. We trade CSS size for design-system flexibility. Critical-CSS budget keeps us honest.
- **Test brittleness.** Existing SSR tests assert on specific class names or hex colors. Each plan's wrap-up should grep for and update such tests, not work around them.

---

## Success Criteria

The initiative is done when:

1. A first-time visitor describes AulaLite as "polished" or "premium" unprompted.
2. Every primitive in `design-system/src/` has full state coverage per decision 8.
3. Every surface in `crates/shell-web/src/routes/` renders without hardcoded hex / rem / px outside the variant lookup tables.
4. `cargo test --workspace` is green (modulo the two pre-existing failures noted in the quick-fix pass retrospective).
5. Lighthouse "best practices" / "accessibility" scores on dashboard + course-detail + live-room ≥ 90.
6. Filling in the empty `[data-theme="dark"]` block ships dark mode with **zero** Rust changes.
