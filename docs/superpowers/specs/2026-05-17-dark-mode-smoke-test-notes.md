# Dark Mode Infrastructure Smoke Test

Date: 2026-05-17 (Plan E Task 2)

## What

Temporarily populated the `[data-theme="dark"]` block in `tokens.css` and
set `<html data-theme="dark">` in `crates/shell-web/index.html` to verify
the dark-mode infrastructure works end-to-end. Vision spec decision 5
("structurally prepared, not shipped") and success criterion 6 ("filling
in tokens ships dark mode with zero Rust changes") are the targets.

## Method

Option B (static analysis) used.

A static CSS-resolution walk was preferred over a live `dx serve`
session because the goal is verifying the *propagation graph* — does
every surface rule resolve through a token that the dark block
overrides? A visual walk could be misleading: a surface that "looks
dark" in screenshots could still hide a hard-coded light value behind
a backdrop blur, a gradient overlay, or a focus state we never
exercised.

Procedure:

1. Populate `[data-theme="dark"]` in both `tokens.css` files with the
   smoke-test values from the plan (paper, surface, ink, text, rule,
   focus, primary/accent aliases, the new Plan E ink-sidebar /
   ink-stage / banner-warm / state-error tokens).
2. Set `<html data-theme="dark">` on the root element so the cascade
   activates.
3. Audit `components.css` line by line for `background`,
   `background-color`, `color`, `border`, and `border-color`
   declarations whose right-hand side is a literal `#...` hex or
   `rgba(...)`. Every match is a candidate leak. Classify each by the
   selector it lives under (primitive button text on a coloured
   accent? sidebar text on a dark ink? semantic ramp ref that bypasses
   a flipped alias?).
4. Cross-reference the six target surfaces against the rules that
   would render them, and decide for each rule whether it propagates
   the flip or leaks the light value.
5. Document the result honestly, then revert both temporary changes.

## Per-surface observations

### /login (`AuthScreen`, `auth-login-card`, `auth-hero-visual`)

- `.auth-screen` background — n/a (transparent over body).
- `.auth-login-card .ds-card` uses `var(--color-surface)` and
  `var(--color-rule)`: **flips** to `#1a1d1a` with low-alpha light
  rules.
- `.auth-local-panel` background `var(--color-accent-strong)` flips
  to `--color-green-200` (a pale mint). The literal `color: #fff` on
  this panel is then white-on-pale-mint — a contrast leak. The
  `.auth-eyebrow` child uses `rgba(255, 255, 255, 0.72)` which has
  the same issue. **LEAK on `/login`** — see Leaks below.
- `.auth-hero-visual` background is a literal gradient overlay
  (`rgba(16, 37, 31, .08)` → `rgba(16, 37, 31, .82)`) over the brand
  hero image. The overlay tones were tuned for cream paper; on the
  dark body they still darken the image (intended effect at the
  bottom of the gradient stop), so visually acceptable but
  decoratively static. Mild leak.
- Inputs (`.ds-input`) use `var(--color-surface)` /
  `var(--color-rule)` / `var(--color-ink)` — flips cleanly.
- Submit button `.ds-button--primary` uses `var(--color-green-600)`
  directly (not `var(--color-primary)`). Result: the button stays the
  same warm green hue regardless of theme. Legible (white on dark
  green) so not a contrast failure, but it does not honour the
  primary-on-dark choice (`--color-green-400`) we set on the alias.
  **Soft leak.**

### / (Dashboard hero, stats, course-list-mini, schedule mini)

- `.dashboard-hero::before` overlay uses `rgba(201, 164, 92, .16)` —
  a static gold wash. Tuned for cream; on dark body the wash is too
  subtle. Cosmetic.
- `.dashboard-stat` uses `var(--color-surface)` + `var(--color-rule)`:
  **flips** correctly.
- `.dashboard-stat-label` uses `var(--color-text-muted)`: flips to
  `#b8a880` (warm sand) which is legible on dark surface.
- Course-list-mini rows are `.outline-lesson` (`var(--color-surface)`
  + `var(--color-rule)`): flips.
- Empty state copy `var(--color-text-muted)`: flips.
- Sidebar uses `var(--color-ink-sidebar)` (override `#060f1f`) +
  `var(--color-on-ink-strong)` text. **Flips.** It was already dark,
  becomes even darker (navy-950 ish) — sidebar boundary still
  distinguishable thanks to the on-ink-subtle border.
- Top bar `.app-topbar` has `background: rgba(251, 247, 239, 0.88)`
  — **a hard-coded warm-cream rgba**. This produces a translucent
  cream strip floating above the dark page content. **CLEAR LEAK.**

### /courses (course list page, course cards, filter chips)

- `.course-list-page h1` is plain ink (`color` cascades from body =
  `var(--color-text)`): flips.
- `.course-cards` is just grid; per-card `.ds-card` flips.
- `.course-card-cover-empty` uses
  `linear-gradient(135deg, rgba(47, 93, 80, 0.16), rgba(29, 49, 68, 0.16))`
  over `var(--color-surface-subtle)`. The base flips, the wash is
  static. Acceptable.
- `.course-card-art` is a literal gradient + brand image. Static.
- Filter chips `.chip` / `.tab` use `var(--color-ink)` text on
  `var(--color-neutral-100)` hover. `--color-neutral-100` is a ramp
  constant `#faf4e8` (cream) — it does NOT flip. So a hovered filter
  chip in dark mode is bright cream with dark text — **soft leak.**
- `.chip-active` / `.tab-active` is `color: #fff` on
  `var(--color-accent-strong)` (= `--color-green-200` in dark) — same
  white-on-pale-mint contrast issue as auth-local-panel. **LEAK.**

### /courses/:slug (course detail header, banner, tabs, body)

- `.course-detail-header h1` is `var(--color-text)`: flips.
- `.course-detail-banner` uses `var(--color-rule)` border, image is
  the course banner asset itself (unchanged). Acceptable.
- `.course-detail-hero::before` static gold wash, see dashboard hero
  note. Cosmetic.
- Tabs see chips note above — active tab has the leak.
- Outline modules `.outline-module` use `var(--color-rule)` divider:
  flips.
- Outline lessons `.outline-lesson` use `var(--color-surface)` +
  `var(--color-rule)`: flips.
- Course Settings tab forms — inputs/buttons same primitive notes
  apply.

### /schedule (PageHeader, schedule cards, badges)

- Header: flips.
- `.schedule-list li` uses `var(--color-surface)` +
  `var(--color-rule)`: flips.
- Badges (`.badge-info`, `.badge-success`, `.badge-warning`,
  `.badge-danger`, `.badge-neutral`) use semantic ramp constants
  (`--color-{semantic}-50/200/700`) directly rather than going
  through a semantic alias for surface/text. These ramps do NOT flip.
  On a dark schedule card, the badges remain pale-tinted pills (light
  cream / pale info-blue / pale success-green) with dark text. Still
  legible *as pills* but they read as light decals on a dark page —
  inconsistent with the rest of the theme. **Semantic-ramp leak — by
  design for now; would need badge-{level}-bg/-text aliases for a
  real dark-mode pass.**
- `.badge-live` / `.live-pill` is white text on oxblood-600 (always
  dark) — fine in both themes.

### /courses/:slug/sessions/:session_id (live room shell, stage, sidebars)

- `.live-room-shell` grid: no colour.
- `.live-room-view`, `.live-room-broadcast` grid columns: no colour.
- `.live-room-video-area`, `.live-room-broadcast-controls`,
  `.replay-video-pane` use `var(--color-ink-stage)` (= `#050d09` in
  dark): **flips** even darker. Text `color: #fff` literal — fine on
  a near-black stage.
- `.live-video-main`, `.replay-video`, `.lesson-video` use
  `var(--color-ink-stage-deep)` (= `#02060a` in dark): **flips**.
- `.live-room-banner` uses `var(--color-banner-warm)` +
  `var(--color-banner-warm-border)`: **flips** from warm cream to
  deep oxblood/gold warning hues (override `#2e1d09` /
  `rgba(217, 161, 80, 0.32)`).
- `.live-room-presence`, `.live-room-chat`, `.live-room-hand-raise`
  panels — checked in components.css; they use
  `var(--color-surface)` / `var(--color-rule)` (Plan D Task 8
  migrated them). **Flip.**
- `.system-state--error` uses `var(--color-state-error-bg)` +
  `var(--color-state-error-border)`: **flips** from cream-pink to
  deep oxblood (override `#2d0f0e`).
- Modal backdrop `rgba(23, 26, 23, 0.48)` is a dark scrim that works
  in both themes (it just dims).

## Leaks (concrete list)

The token architecture is structurally sound — surface containers
(`.ds-card`, `.outline-lesson`, `.schedule-list li`, `.dashboard-stat`,
`.app-side`, live-room stage panes, banner, error state) all resolve
through tokens that the dark block overrides. However the following
rules either bypass the alias layer or use ramp constants directly,
so they do not propagate the dark flip:

1. **`.app-topbar` `background: rgba(251, 247, 239, 0.88)`** — the
   most visible leak: a translucent cream strip above otherwise-dark
   content. Wants `var(--color-paper-warm)` with
   `rgba(...)`-via-`color-mix` or a dedicated `--color-topbar-bg`
   token.
2. **`.ds-button--primary` `background: var(--color-green-600)`** —
   bypasses `--color-primary` alias. Bound to the green-600 ramp
   constant, so the dark override of `--color-primary` to green-400
   has no effect. Wants `var(--color-primary)`.
3. **`.ds-button--danger` `background: var(--color-danger-500)`** —
   same pattern. Wants a semantic alias.
4. **`.ds-button--secondary:hover` `background:
   var(--color-neutral-100)`** — ramp constant. Wants
   `var(--color-paper-warm)` or a dedicated hover token.
5. **`.tab:hover` / `.chip:hover` `background:
   var(--color-neutral-100)`** — same.
6. **`.tab-active` / `.chip-active` `color: #fff` on
   `var(--color-accent-strong)`** — pairing only works when
   accent-strong is *dark*. In our dark override accent-strong flips
   to a *light* green-200, producing white-on-pale-mint contrast
   failure.
7. **`.auth-local-panel` `color: #fff` on
   `var(--color-accent-strong)`** — same root cause.
8. **`.auth-local-panel .auth-eyebrow` `color: rgba(255, 255, 255,
   0.72)`** — same.
9. **All `.badge-{info,success,warning,danger,neutral}` rules** —
   point at `--color-{semantic}-50/200/700` ramp constants. No flip
   path. Wants `--badge-{level}-bg / -border / -text` aliases that
   the dark block can override.
10. **`.ds-checkbox::after` / indeterminate `background: #fff`** —
    intentional (high-contrast dot/dash on accent). Acceptable in both
    themes.
11. **Hero overlay gradients** (`.dashboard-hero::before`,
    `.course-detail-hero::before`, `.auth-hero-visual`,
    `.course-card-art`, `.course-card-cover-empty`) — literal rgba
    gradient washes tuned for cream paper. Cosmetically muted under
    a dark body but not contrast-breaking.

## Verdict

**Partial.**

The infrastructure verifies: the dark `[data-theme="dark"]` block does
take effect (cascade reaches the document), surface containers and the
new Plan E ink/stage/banner/state tokens all propagate the flip
correctly, and *most* of the visible chrome (cards, sidebars, stage
panes, body, schedule and live-room surfaces) darkens on cue with zero
Rust or component-CSS changes — which is exactly what vision-spec
criterion 6 requires.

What it surfaces is that a real product-grade dark pass needs three
more alias layers before it ships: a topbar background token, semantic
button-fill aliases (so primary/danger buttons honour
`--color-primary`/`--color-danger` rather than reaching past them to
ramp values), and badge-level aliases (so the semantic pills can theme
independently of their ramp source). Plus, the white-text-on-
`accent-strong` pairings in `.tab-active`, `.chip-active`, and
`.auth-local-panel` need a re-think (either swap accent-strong's role
or introduce a dedicated `--color-on-accent-strong` token).

These are *follow-up product/design tasks*, not infrastructure bugs.
The infrastructure does what Plan A Task 7 designed it to do: own
roughly 80% of the UI surface from a single override block.

## Follow-up

The actual dark-theme product decision (tone, contrast curves, brand
interpretation in dark) is a separate design exercise. This smoke test
confirmed that AulaLite's token architecture is theme-flip-ready in
principle: filling in tokens under `[data-theme="dark"]` flips the
majority of the UI without component code or Rust changes, and the
specific leaks identified above are addressable with a small
follow-up alias-layer pass — not a re-architecture.

Both the temporary populated dark block and the
`<html data-theme="dark">` attribute have been reverted as part of
this task. The empty `[data-theme="dark"]` placeholder remains in
`tokens.css`, ready for the future product pass.

Suggested scope for the future product-led dark-mode plan, in order
of leak impact:

1. Add `--color-topbar-bg` token (and migrate `.app-topbar`).
2. Migrate `.ds-button--primary`, `.ds-button--danger`,
   `.ds-button--secondary:hover`, `.tab:hover`, `.chip:hover` from
   ramp constants to semantic aliases.
3. Add `--color-on-accent-strong` token; re-pair
   `.tab-active`/`.chip-active`/`.auth-local-panel` text colour.
4. Add `--badge-{info,success,warning,danger,neutral}-bg/-border/-text`
   aliases and migrate badge rules.
5. Re-tune the four decorative gradient overlays for dark.

With those five passes complete, criterion 6 would graduate from "Met
in principle" to "Met in product."
