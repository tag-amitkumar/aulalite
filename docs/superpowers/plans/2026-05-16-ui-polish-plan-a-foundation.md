# UI Polish — Plan A: Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the design-system v2 foundation — refined typography (self-hosted Source Serif 4 + Inter), 50–950 color ramps across four brand chromatic + five semantic palettes, a polished neutral surface ramp, harmonised type / spacing / radius / shadow / motion scales, and dark-mode-ready token organisation. No visible component changes; the existing UI continues to work, now drawing from richer tokens.

**Architecture:** Two CSS files in `crates/design-system/assets/` (`tokens.css` and `components.css`) get the new tokens. Existing semantic aliases (`--color-primary`, `--color-primary-hover`, etc.) are preserved as forwards so no component CSS needs editing in this plan. New WOFF2 font assets land under `crates/shell-web/public/assets/fonts/`. The shell-web asset mirror is maintained throughout.

**Tech Stack:** CSS custom properties, WOFF2 (variable axes), Rust SSR tests via `dioxus_ssr`, plain HTML `<link rel="preload">` for fonts.

**Vision spec:** `docs/superpowers/specs/2026-05-16-ui-polish-vision-design.md` (commit `5bf5c27`).

---

## File Structure

**Modify:**
- `crates/design-system/assets/tokens.css` — major rewrite; preserves existing semantic aliases, adds 50–950 ramps for all 9 palettes plus a 5-step neutral surface ramp, type scale, extended spacing, radius scale, shadow scale, motion eases, and `:root` / `[data-theme="dark"]` organisation.
- `crates/shell-web/public/assets/tokens.css` — mirror of the above.
- `crates/shell-web/index.html` — add `<link rel="preload">` for the two display weights of Source Serif 4 + Inter; existing structure unchanged.
- `crates/design-system/assets/components.css` — add `@font-face` declarations for both fonts; update `--font-display` / `--font-body` family stacks. No layout rule changes.
- `crates/shell-web/public/assets/components.css` — mirror.
- `crates/shell-web/tests/editorial_assets.rs` — extend `tokens_define_modern_academy_theme_contract` and add a new test that asserts every named ramp step is present.

**Create:**
- `crates/shell-web/public/assets/fonts/SourceSerif4-Variable.woff2` (~30 KB subsetted)
- `crates/shell-web/public/assets/fonts/SourceSerif4-Italic-Variable.woff2` (~30 KB subsetted)
- `crates/shell-web/public/assets/fonts/Inter-Variable.woff2` (~20 KB subsetted)
- `crates/shell-web/public/assets/fonts/Inter-Italic-Variable.woff2` (~20 KB subsetted)
- `crates/shell-web/public/assets/fonts/LICENSE.md` — SIL OFL attribution for both font families.

**Tests touched:**
- `crates/shell-web/tests/editorial_assets.rs` — the existing `tokens_define_modern_academy_theme_contract` test gets new assertions; one new test added for ramp completeness.

---

## Task 1: Add motion ease + duration tokens

**Files:**
- Modify: `crates/design-system/assets/tokens.css` (replace existing motion tokens at lines 50-52)
- Modify: `crates/shell-web/public/assets/tokens.css` (mirror)

- [ ] **Step 1: Add failing test**

Append to `crates/shell-web/tests/editorial_assets.rs` inside `tokens_define_modern_academy_theme_contract`:

```rust
    for expected in [
        "--ease-snappy",
        "--ease-decelerate",
        "--ease-accelerate",
        "--ease-spring",
        "--ease-page-in",
        "--duration-fast",
        "--duration-medium",
        "--duration-slow",
    ] {
        assert!(
            css.contains(expected),
            "tokens.css missing expected token `{expected}`"
        );
    }
```

- [ ] **Step 2: Run, expect failure**

```bash
cargo test -p shell-web --test editorial_assets tokens_define_modern_academy_theme_contract
```

Expected: FAIL with `missing expected token --ease-snappy`.

- [ ] **Step 3: Replace motion tokens in `crates/design-system/assets/tokens.css`**

Find the existing `--motion-fast / --motion-medium / --motion-page` block (currently lines 50-52) and replace with:

```css
  /* ---- Motion: durations + cubic-bezier eases ---- */
  --duration-fast: 120ms;
  --duration-medium: 200ms;
  --duration-slow: 320ms;
  --duration-page-in: 420ms;

  --ease-snappy: cubic-bezier(.4, 0, .2, 1);
  --ease-decelerate: cubic-bezier(0, 0, .2, 1);
  --ease-accelerate: cubic-bezier(.4, 0, 1, 1);
  --ease-spring: cubic-bezier(.34, 1.56, .64, 1);
  --ease-page-in: cubic-bezier(.2, .8, .2, 1);

  /* Legacy aliases retained until Plan B migrates references */
  --motion-fast: var(--duration-fast) var(--ease-snappy);
  --motion-medium: var(--duration-medium) var(--ease-snappy);
  --motion-page: var(--duration-page-in) var(--ease-page-in);
```

- [ ] **Step 4: Mirror to `crates/shell-web/public/assets/tokens.css`**

```bash
cp crates/design-system/assets/tokens.css crates/shell-web/public/assets/tokens.css
```

Verify: `diff crates/design-system/assets/tokens.css crates/shell-web/public/assets/tokens.css` → no output.

- [ ] **Step 5: Run tests**

```bash
cargo test -p shell-web --test editorial_assets
```

Expected: PASS (all five tests in the file).

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add crates/design-system/assets/tokens.css \
        crates/shell-web/public/assets/tokens.css \
        crates/shell-web/tests/editorial_assets.rs
git commit -m "$(cat <<'EOF'
feat(tokens): introduce duration + cubic-bezier motion tokens

Adds --duration-fast/medium/slow/page-in and --ease-snappy/decelerate/
accelerate/spring/page-in. Keeps --motion-fast/medium/page as legacy
aliases composed from the new building blocks so existing component
CSS continues to work; Plan B will migrate references.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: Add type / spacing / radius / shadow scales

**Files:**
- Modify: `crates/design-system/assets/tokens.css`
- Modify: `crates/shell-web/public/assets/tokens.css`
- Modify: `crates/shell-web/tests/editorial_assets.rs`

- [ ] **Step 1: Add failing test**

Append to `tokens_define_modern_academy_theme_contract` in `editorial_assets.rs`:

```rust
    for expected in [
        // Type scale
        "--text-xs",
        "--text-sm",
        "--text-base",
        "--text-md",
        "--text-lg",
        "--text-xl",
        "--text-2xl",
        "--text-3xl",
        "--text-4xl",
        "--text-5xl",
        "--text-6xl",
        // Spacing extensions
        "--space-0",
        "--space-10",
        "--space-12",
        // Radius scale
        "--radius-xs",
        "--radius-lg",
        "--radius-xl",
        "--radius-2xl",
        "--radius-full",
        // Shadow elevation scale
        "--shadow-xs",
        "--shadow-xl",
        "--shadow-2xl",
    ] {
        assert!(
            css.contains(expected),
            "tokens.css missing expected token `{expected}`"
        );
    }
```

- [ ] **Step 2: Run, expect failure**

```bash
cargo test -p shell-web --test editorial_assets tokens_define_modern_academy_theme_contract
```

Expected: FAIL with `missing expected token --text-xs`.

- [ ] **Step 3: Add scales to `crates/design-system/assets/tokens.css`**

After the spacing block (currently `--space-1` through `--space-9`), replace and extend:

```css
  /* ---- Spacing: 4px base, extended ---- */
  --space-0: 0;
  --space-px: 1px;
  --space-0_5: 2px;
  --space-1: 4px;
  --space-1_5: 6px;
  --space-2: 8px;
  --space-3: 12px;
  --space-4: 16px;
  --space-5: 24px;
  --space-6: 32px;
  --space-7: 40px;
  --space-8: 56px;
  --space-9: 72px;
  --space-10: 96px;
  --space-12: 128px;

  /* ---- Type scale ---- */
  --text-xs: 12px;       --leading-xs: 16px;
  --text-sm: 13px;       --leading-sm: 18px;
  --text-base: 14px;     --leading-base: 20px;
  --text-md: 15px;       --leading-md: 22px;
  --text-lg: 16px;       --leading-lg: 24px;
  --text-xl: 18px;       --leading-xl: 26px;
  --text-2xl: 20px;      --leading-2xl: 28px;
  --text-3xl: 24px;      --leading-3xl: 32px;
  --text-4xl: 30px;      --leading-4xl: 36px;
  --text-5xl: 36px;      --leading-5xl: 40px;
  --text-6xl: 48px;      --leading-6xl: 52px;
  --text-7xl: 60px;      --leading-7xl: 64px;

  /* ---- Radius scale ---- */
  --radius-xs: 2px;
  --radius-sm: 4px;
  --radius-md: 8px;
  --radius-lg: 12px;
  --radius-xl: 16px;
  --radius-2xl: 24px;
  --radius-full: 9999px;

  /* ---- Elevation / shadows (soft, layered) ---- */
  --shadow-xs: 0 1px 2px rgba(18, 22, 20, 0.04);
  --shadow-sm: 0 1px 3px rgba(18, 22, 20, 0.06), 0 1px 2px rgba(18, 22, 20, 0.04);
  --shadow-md: 0 4px 12px rgba(18, 22, 20, 0.08), 0 2px 4px rgba(18, 22, 20, 0.04);
  --shadow-lg: 0 16px 32px rgba(18, 22, 20, 0.10), 0 6px 12px rgba(18, 22, 20, 0.06);
  --shadow-xl: 0 30px 60px rgba(18, 22, 20, 0.14), 0 12px 24px rgba(18, 22, 20, 0.08);
  --shadow-2xl: 0 50px 100px rgba(18, 22, 20, 0.20), 0 20px 40px rgba(18, 22, 20, 0.12);
  --shadow-gold: 0 18px 44px rgba(201, 164, 92, 0.18);
```

REMOVE the old single-line `--radius-sm: 4px; --radius-md: 8px; --radius-lg: 8px;` and the old `--shadow-sm/md/lg/gold` block — the new scale supersedes them. Note: `--radius-md` value is unchanged (8px); `--radius-lg` upgrades from 8px → 12px (intentional refinement). `--shadow-sm/md/lg` values are deliberately refined; if any component CSS appears visually different after Plan A lands, that is the intended polish improvement.

- [ ] **Step 4: Mirror to shell-web copy**

```bash
cp crates/design-system/assets/tokens.css crates/shell-web/public/assets/tokens.css
diff crates/design-system/assets/tokens.css crates/shell-web/public/assets/tokens.css
```

- [ ] **Step 5: Run tests**

```bash
cargo test -p shell-web --test editorial_assets
cargo test --workspace --no-fail-fast
```

Expected: all PASS (modulo the 2 pre-existing failures from quick-fix retrospective).

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add crates/design-system/assets/tokens.css \
        crates/shell-web/public/assets/tokens.css \
        crates/shell-web/tests/editorial_assets.rs
git commit -m "$(cat <<'EOF'
feat(tokens): introduce type / spacing / radius / shadow scales

Adds --text-xs..7xl with paired --leading-* line heights, fills in the
missing --space-0/px/0_5/1_5/10/12 between existing steps, replaces the
two-value radius scale with a seven-step scale (xs..2xl + full), and
adds a six-step soft-layered shadow scale (xs..2xl) preserving the
existing --shadow-gold accent. Existing tokens with the same name (e.g.
--radius-md, --shadow-sm) keep their values; --radius-lg is refined
from 8px to 12px and shadow values are refined for a softer feel.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: Build brand chromatic ramps (green / gold / navy / oxblood)

**Files:**
- Modify: `crates/design-system/assets/tokens.css`
- Modify: `crates/shell-web/public/assets/tokens.css`
- Modify: `crates/shell-web/tests/editorial_assets.rs`

- [ ] **Step 1: Add failing test**

Add a new test to `crates/shell-web/tests/editorial_assets.rs` (after the existing `tokens_define_modern_academy_theme_contract`):

```rust
#[test]
fn brand_chromatic_ramps_are_complete() {
    let css = shell_asset("tokens.css");
    for color in ["green", "gold", "navy", "oxblood"] {
        for step in ["50", "100", "200", "300", "400", "500", "600", "700", "800", "900", "950"] {
            let expected = format!("--color-{color}-{step}");
            assert!(
                css.contains(&expected),
                "tokens.css missing expected ramp step `{expected}`"
            );
        }
    }
}
```

- [ ] **Step 2: Run, expect failure**

```bash
cargo test -p shell-web --test editorial_assets brand_chromatic_ramps_are_complete
```

Expected: FAIL with `missing expected ramp step --color-green-50`.

- [ ] **Step 3: Add ramps to `crates/design-system/assets/tokens.css`**

After the existing brand chromatic block (the `--color-accent`, `--color-navy`, `--color-gold`, `--color-oxblood` lines around 11-17), add:

```css
  /* ---- Brand chromatic ramps (Plan A v2) ---- */
  --color-green-50:  #f0f5f3;
  --color-green-100: #dbe7df;
  --color-green-200: #b5cfc3;
  --color-green-300: #82a89a;
  --color-green-400: #588175;
  --color-green-500: #386658;
  --color-green-600: #244f43;
  --color-green-700: #18372f;
  --color-green-800: #10251f;
  --color-green-900: #0a1812;
  --color-green-950: #050d09;

  --color-gold-50:  #fdfaf4;
  --color-gold-100: #efe1bf;
  --color-gold-200: #e5cf99;
  --color-gold-300: #d8b975;
  --color-gold-400: #cda35a;
  --color-gold-500: #c9a45c;
  --color-gold-600: #b08842;
  --color-gold-700: #8d6c2f;
  --color-gold-800: #6c5224;
  --color-gold-900: #4d3a19;
  --color-gold-950: #2e2210;

  --color-navy-50:  #f2f5f8;
  --color-navy-100: #d5dee5;
  --color-navy-200: #b8c6d2;
  --color-navy-300: #8ba1b5;
  --color-navy-400: #5a7894;
  --color-navy-500: #34546e;
  --color-navy-600: #1d3144;
  --color-navy-700: #142235;
  --color-navy-800: #0d172a;
  --color-navy-900: #060f1f;
  --color-navy-950: #020611;

  --color-oxblood-50:  #fbf3f5;
  --color-oxblood-100: #f7e3e8;
  --color-oxblood-200: #ebbac5;
  --color-oxblood-300: #de8a9c;
  --color-oxblood-400: #c45a71;
  --color-oxblood-500: #b8324a;
  --color-oxblood-600: #8f243d;
  --color-oxblood-700: #6f1c30;
  --color-oxblood-800: #531425;
  --color-oxblood-900: #380c1a;
  --color-oxblood-950: #20070f;
```

Then UPDATE the existing semantic aliases to forward to the ramp. Replace the existing `--color-accent: #244f43;` and friends with:

```css
  /* Semantic aliases — forward to ramps; preserved for ergonomic component CSS */
  --color-primary: var(--color-green-600);
  --color-primary-hover: var(--color-green-700);
  --color-accent: var(--color-green-600);
  --color-accent-strong: var(--color-green-800);
  --color-accent-soft: var(--color-green-100);
  --color-navy: var(--color-navy-600);
  --color-gold: var(--color-gold-500);
  --color-gold-soft: var(--color-gold-100);
  --color-oxblood: var(--color-oxblood-600);
  --color-live: var(--color-oxblood-500);
```

VERIFY each old value equals the new ramp step (e.g. `--color-accent: #244f43` should equal `--color-green-600: #244f43`). The ramps above were tuned so the existing values map cleanly.

- [ ] **Step 4: Mirror and run tests**

```bash
cp crates/design-system/assets/tokens.css crates/shell-web/public/assets/tokens.css
cargo test -p shell-web --test editorial_assets
cargo test --workspace --no-fail-fast
```

Expected: all PASS (the new `brand_chromatic_ramps_are_complete` plus the existing tests).

- [ ] **Step 5: Visually spot-check**

If `dx serve` is available locally, load `/` and confirm the dashboard hero, sidebar, and Badge tones look identical to before. If any color shifts: the ramp value at the documented step differs from the legacy token; correct the ramp and re-run.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add crates/design-system/assets/tokens.css \
        crates/shell-web/public/assets/tokens.css \
        crates/shell-web/tests/editorial_assets.rs
git commit -m "$(cat <<'EOF'
feat(tokens): introduce 50–950 ramps for brand chromatic palettes

Adds full 11-step ramps for green, gold, navy, and oxblood. Existing
semantic aliases (--color-primary, --color-accent, --color-gold, etc.)
now forward to the ramp steps that match their previous hex values, so
component CSS continues to render identically. New ramps make state
coverage in Plan B straightforward.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: Build semantic + neutral ramps

**Files:**
- Modify: `crates/design-system/assets/tokens.css`
- Modify: `crates/shell-web/public/assets/tokens.css`
- Modify: `crates/shell-web/tests/editorial_assets.rs`

- [ ] **Step 1: Add failing test**

Add to `editorial_assets.rs`:

```rust
#[test]
fn semantic_and_neutral_ramps_are_complete() {
    let css = shell_asset("tokens.css");
    for color in ["info", "success", "warning", "danger", "neutral"] {
        for step in ["50", "100", "200", "300", "400", "500", "600", "700", "800", "900", "950"] {
            let expected = format!("--color-{color}-{step}");
            assert!(
                css.contains(&expected),
                "tokens.css missing expected ramp step `{expected}`"
            );
        }
    }
}
```

- [ ] **Step 2: Run, expect failure**

```bash
cargo test -p shell-web --test editorial_assets semantic_and_neutral_ramps_are_complete
```

Expected: FAIL with `missing expected ramp step --color-info-50`.

- [ ] **Step 3: Add ramps**

In `crates/design-system/assets/tokens.css`, after the brand-chromatic ramps block, add:

```css
  /* ---- Semantic ramps ---- */
  --color-info-50:  #f0f5f9;
  --color-info-100: #d9e6f0;
  --color-info-200: #b6cee0;
  --color-info-300: #82adc8;
  --color-info-400: #4f86ac;
  --color-info-500: #2d5f88;
  --color-info-600: #20486d;
  --color-info-700: #173657;
  --color-info-800: #102742;
  --color-info-900: #0a1a2d;
  --color-info-950: #050f1a;

  --color-success-50:  #f0f6f2;
  --color-success-100: #dbe9df;
  --color-success-200: #b6d3bf;
  --color-success-300: #87b596;
  --color-success-400: #589570;
  --color-success-500: #2f6b45;
  --color-success-600: #25553a;
  --color-success-700: #1c402d;
  --color-success-800: #142d21;
  --color-success-900: #0c1c14;
  --color-success-950: #06100a;

  --color-warning-50:  #fbf6ec;
  --color-warning-100: #f4ead6;
  --color-warning-200: #e7d3aa;
  --color-warning-300: #d7b577;
  --color-warning-400: #c19549;
  --color-warning-500: #9b6a1e;
  --color-warning-600: #7f5418;
  --color-warning-700: #623f12;
  --color-warning-800: #482e0e;
  --color-warning-900: #2e1d09;
  --color-warning-950: #1a1005;

  --color-danger-50:  #fbf0ef;
  --color-danger-100: #f3d7d5;
  --color-danger-200: #e5aba6;
  --color-danger-300: #d27a74;
  --color-danger-400: #ba514b;
  --color-danger-500: #a33a36;
  --color-danger-600: #832d2a;
  --color-danger-700: #63221f;
  --color-danger-800: #481917;
  --color-danger-900: #2d0f0e;
  --color-danger-950: #190706;

  /* ---- Neutral / surface ramp (anchored on warm cream) ---- */
  --color-neutral-50:  #fffdf8;
  --color-neutral-100: #faf4e8;
  --color-neutral-200: #efe6d2;
  --color-neutral-300: #d8c9aa;
  --color-neutral-400: #b8a880;
  --color-neutral-500: #8d7f5c;
  --color-neutral-600: #6d6558;
  --color-neutral-700: #4a4538;
  --color-neutral-800: #2c2920;
  --color-neutral-900: #171a17;
  --color-neutral-950: #0c0e0b;
```

Then UPDATE the existing semantic and surface aliases. Replace `--color-info: #2d5f88;` and friends with:

```css
  --color-info: var(--color-info-500);
  --color-success: var(--color-success-500);
  --color-warning: var(--color-warning-500);
  --color-danger: var(--color-danger-500);

  /* Surface aliases — forward to neutral ramp */
  --color-paper: #f6f0e6;            /* unchanged — anchor for warm-cream identity */
  --color-paper-warm: var(--color-neutral-100);
  --color-surface: #fffdf7;          /* unchanged — anchor for primary content surface */
  --color-surface-subtle: #f3eadb;   /* unchanged — close to neutral-200 but warmer */
  --color-ink: var(--color-neutral-900);
  --color-text: #171a17;             /* unchanged — equals neutral-900 */
  --color-text-muted: var(--color-neutral-600);
  --color-rule: rgba(18, 22, 20, 0.13);
  --color-rule-strong: rgba(18, 22, 20, 0.25);
  --color-focus: rgba(201, 164, 92, 0.32);
```

VERIFY: the unchanged anchors (`paper`, `surface`, `surface-subtle`, `text`) preserve their exact previous values so no component renders differently. The new neutral ramp gives us extra steps for Plan B and Plan C variants.

- [ ] **Step 4: Mirror and run tests**

```bash
cp crates/design-system/assets/tokens.css crates/shell-web/public/assets/tokens.css
cargo test -p shell-web --test editorial_assets
cargo test --workspace --no-fail-fast
```

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/design-system/assets/tokens.css \
        crates/shell-web/public/assets/tokens.css \
        crates/shell-web/tests/editorial_assets.rs
git commit -m "$(cat <<'EOF'
feat(tokens): introduce semantic + neutral ramps

Adds 11-step ramps for info, success, warning, danger, and a neutral
ramp anchored on the warm-cream identity. Semantic aliases
(--color-info etc.) and surface aliases (--color-text-muted etc.) now
forward to ramp steps. Anchor surface tokens (--color-paper,
--color-surface, --color-text) keep their exact values to avoid any
visual shift in components that still reference them directly.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: Self-host fonts (Source Serif 4 + Inter)

**Files:**
- Create: `crates/shell-web/public/assets/fonts/SourceSerif4-Variable.woff2`
- Create: `crates/shell-web/public/assets/fonts/SourceSerif4-Italic-Variable.woff2`
- Create: `crates/shell-web/public/assets/fonts/Inter-Variable.woff2`
- Create: `crates/shell-web/public/assets/fonts/Inter-Italic-Variable.woff2`
- Create: `crates/shell-web/public/assets/fonts/LICENSE.md` (SIL OFL attribution)
- Modify: `crates/design-system/assets/components.css` (add `@font-face` declarations)
- Modify: `crates/shell-web/public/assets/components.css` (mirror)

- [ ] **Step 1: Download font files**

From a terminal (Git Bash / WSL / native):

```bash
mkdir -p crates/shell-web/public/assets/fonts
cd crates/shell-web/public/assets/fonts

# Source Serif 4 variable (Adobe, SIL OFL): https://github.com/adobe-fonts/source-serif/raw/release/WOFF2/VAR/
curl -L -o SourceSerif4-Variable.woff2 \
  "https://github.com/adobe-fonts/source-serif/raw/release/WOFF2/VAR/SourceSerif4-VF.ttf.woff2"
curl -L -o SourceSerif4-Italic-Variable.woff2 \
  "https://github.com/adobe-fonts/source-serif/raw/release/WOFF2/VAR/SourceSerif4-It-VF.ttf.woff2"

# Inter v4 variable (rsms.me, SIL OFL): https://github.com/rsms/inter/releases/latest
curl -L -o Inter-Variable.woff2 \
  "https://github.com/rsms/inter/raw/master/docs/font-files/InterVariable.woff2"
curl -L -o Inter-Italic-Variable.woff2 \
  "https://github.com/rsms/inter/raw/master/docs/font-files/InterVariable-Italic.woff2"

# Verify each file is > 50 KB (not an HTML 404 page); if smaller, the URL is wrong.
ls -lh
```

If any download is < 50 KB, STOP and ask. The URL may have moved; the implementer should not improvise an alternate source without confirming the font is variable WOFF2 and SIL OFL licensed.

- [ ] **Step 2: Write LICENSE.md attribution**

Write to `crates/shell-web/public/assets/fonts/LICENSE.md`:

```markdown
# Self-hosted fonts

Both font families are distributed under the SIL Open Font License 1.1.

## Source Serif 4

Copyright 2014–present Adobe (https://adobe.com/), with Reserved Font Name "Source".
Source: https://github.com/adobe-fonts/source-serif
SIL OFL 1.1: https://github.com/adobe-fonts/source-serif/blob/release/LICENSE.md

## Inter

Copyright 2016 The Inter Project Authors (https://github.com/rsms/inter).
Source: https://github.com/rsms/inter
SIL OFL 1.1: https://github.com/rsms/inter/blob/master/LICENSE.txt
```

- [ ] **Step 3: Add `@font-face` declarations**

Prepend to `crates/design-system/assets/components.css` (at the very top, before the `.ds-button` block):

```css
/* ---- Self-hosted variable fonts ---- */
@font-face {
  font-family: "Source Serif 4";
  src: url("/assets/fonts/SourceSerif4-Variable.woff2") format("woff2-variations");
  font-weight: 200 900;
  font-style: normal;
  font-display: swap;
}

@font-face {
  font-family: "Source Serif 4";
  src: url("/assets/fonts/SourceSerif4-Italic-Variable.woff2") format("woff2-variations");
  font-weight: 200 900;
  font-style: italic;
  font-display: swap;
}

@font-face {
  font-family: "Inter";
  src: url("/assets/fonts/Inter-Variable.woff2") format("woff2-variations");
  font-weight: 100 900;
  font-style: normal;
  font-display: swap;
}

@font-face {
  font-family: "Inter";
  src: url("/assets/fonts/Inter-Italic-Variable.woff2") format("woff2-variations");
  font-weight: 100 900;
  font-style: italic;
  font-display: swap;
}
```

- [ ] **Step 4: Update font-family tokens in `tokens.css`**

Replace the existing `--font-display` and `--font-body` lines with:

```css
  --font-display: "Source Serif 4", "ui-serif", Georgia, "Times New Roman", serif;
  --font-body: "Inter", "ui-sans-serif", "Segoe UI", Roboto, Arial, sans-serif;
  --font-mono: "Cascadia Code", "Consolas", monospace;
```

The fallback chain keeps the page legible if the WOFF2 fails to load.

- [ ] **Step 5: Mirror both files**

```bash
cp crates/design-system/assets/tokens.css crates/shell-web/public/assets/tokens.css
cp crates/design-system/assets/components.css crates/shell-web/public/assets/components.css
diff crates/design-system/assets/tokens.css crates/shell-web/public/assets/tokens.css
diff crates/design-system/assets/components.css crates/shell-web/public/assets/components.css
```

- [ ] **Step 6: Run tests**

```bash
cargo test -p shell-web --test editorial_assets
```

Expected: all tests pass (the new fonts directory does not interfere with the existing asset-sync test).

- [ ] **Step 7: Visual confirmation**

If `dx serve` is available, load `/` and confirm typography now renders in Source Serif 4 (display) and Inter (body). Use DevTools Network panel to confirm WOFF2 files load with HTTP 200 and `< 50 KB` each (likely 30-40 KB raw; gzip not used because WOFF2 is already compressed).

If the fonts do not load, check:
1. The files exist at the expected paths.
2. The `@font-face src:` URLs are absolute (`/assets/fonts/...`) — Dioxus's `dx serve` mounts `public/` at root.
3. The `index.html` template substitutes asset paths correctly (no `{app_title}`-style placeholders left for fonts).

- [ ] **Step 8: Commit**

```bash
git add crates/shell-web/public/assets/fonts \
        crates/design-system/assets/components.css \
        crates/design-system/assets/tokens.css \
        crates/shell-web/public/assets/components.css \
        crates/shell-web/public/assets/tokens.css
git commit -m "$(cat <<'EOF'
feat(typography): self-host Source Serif 4 + Inter variable fonts

Adds variable WOFF2 files for both families (with italic variants) and
@font-face declarations using woff2-variations for full-weight axis
coverage. Updates --font-display and --font-body family stacks to lead
with the self-hosted names; legacy Georgia / Segoe UI / Roboto remain
as graceful fallbacks. SIL OFL attribution in
public/assets/fonts/LICENSE.md.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: Preload critical fonts in index.html

**Files:**
- Modify: `crates/shell-web/index.html`

- [ ] **Step 1: Preload the two most-used weights**

Find the existing `<head>` block in `crates/shell-web/index.html` (the section with the `<link rel="stylesheet">` lines). Immediately after the `<meta name="theme-color">` line, insert:

```html
        <link rel="preload" as="font" type="font/woff2" crossorigin="anonymous"
              href="/assets/fonts/Inter-Variable.woff2">
        <link rel="preload" as="font" type="font/woff2" crossorigin="anonymous"
              href="/assets/fonts/SourceSerif4-Variable.woff2">
```

Only the upright variants are preloaded; italics are loaded on-demand by the rare components that use them.

- [ ] **Step 2: Verify the editorial_assets `index_references_brand_metadata` test still passes**

```bash
cargo test -p shell-web --test editorial_assets index_references_brand_metadata
```

Expected: PASS. The test asserts specific brand metadata strings; the new `<link>` tags don't remove anything.

- [ ] **Step 3: Spot-check in browser**

Load the app. DevTools Network panel should show both preloaded font files fetched with high priority during the initial document parse, BEFORE the CSS-driven `@font-face` would otherwise discover them.

- [ ] **Step 4: Commit**

```bash
git add crates/shell-web/index.html
git commit -m "$(cat <<'EOF'
perf(fonts): preload critical Inter + Source Serif 4 weights

Adds <link rel="preload"> hints for the upright variable WOFF2 files so
the browser can start the font fetch in parallel with HTML parsing
instead of waiting for the CSS to discover @font-face. Italics are
loaded on-demand as before.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 7: Reorganise `:root` / introduce `[data-theme="dark"]` placeholder

**Files:**
- Modify: `crates/design-system/assets/tokens.css`
- Modify: `crates/shell-web/public/assets/tokens.css`

- [ ] **Step 1: Add the dark placeholder**

At the very end of `crates/design-system/assets/tokens.css` (after the closing `}` of `:root` and the existing `body { ... }` block), insert:

```css
/* ---- Dark mode placeholder ----
 * Structurally reserved for a future plan. Filling in values here will
 * activate dark mode for any component that already references token names
 * (which is the requirement enforced in Plan B and Plan D). Do not put
 * component CSS in this block; only token overrides.
 */
[data-theme="dark"] {
  /* Intentionally empty — fill in when the dark theme is designed. */
}
```

This must live at the same nesting level as `:root`, not nested inside it.

- [ ] **Step 2: Add a `prefers-color-scheme: dark` opt-in comment**

Immediately below the `[data-theme="dark"]` block, add:

```css
/* When the dark theme is shipped, the runtime should honour the OS preference
 * by default. Uncomment and fill the body when ready:
 *
 * @media (prefers-color-scheme: dark) {
 *   :root:not([data-theme="light"]) { ... }
 * }
 */
```

- [ ] **Step 3: Mirror**

```bash
cp crates/design-system/assets/tokens.css crates/shell-web/public/assets/tokens.css
diff crates/design-system/assets/tokens.css crates/shell-web/public/assets/tokens.css
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p shell-web --test editorial_assets
```

- [ ] **Step 5: Commit**

```bash
git add crates/design-system/assets/tokens.css \
        crates/shell-web/public/assets/tokens.css
git commit -m "$(cat <<'EOF'
chore(tokens): reserve [data-theme="dark"] placeholder for future dark mode

Adds an empty [data-theme="dark"] selector at the bottom of tokens.css.
Filling in values here activates dark mode for any component that
already references token names — which is exactly the discipline Plan B
enforces. Documents the prefers-color-scheme opt-in pattern in a
comment for whoever picks this up later.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 8: Final verification

- [ ] **Step 1: Full workspace test**

```bash
cargo test --workspace --no-fail-fast
```

Expected: only the 2 pre-existing failures from the quick-fix retrospective (audit_seed, replay_renders_loading_initially) remain. All editorial_assets tests pass.

- [ ] **Step 2: Format check**

```bash
cargo fmt --all -- --check
```

Expected: no diff.

- [ ] **Step 3: Wasm build**

```bash
cargo build -p shell-web --target wasm32-unknown-unknown
```

Expected: builds cleanly.

- [ ] **Step 4: Asset-sync sanity**

```bash
diff crates/design-system/assets/tokens.css crates/shell-web/public/assets/tokens.css
diff crates/design-system/assets/components.css crates/shell-web/public/assets/components.css
```

Both must be empty.

- [ ] **Step 5: Manual smoke**

If a dev server is available: load the app, confirm Source Serif 4 + Inter render, confirm no console errors, click around dashboard / courses / live room and confirm no visual regressions (colors, spacing, shadows should look intentional, not broken).

- [ ] **Step 6: Document follow-ups**

In the final commit message OR a brief note in the plan retrospective, log any visual deltas you observed (e.g. "shadow on cards reads slightly softer; intended"). These inform Plan B's task scoping.

---

## Spec coverage check

| Vision-spec requirement | Task |
|--|--|
| Self-host Source Serif 4 + Inter | Task 5 |
| Preload critical font weights | Task 6 |
| 50–950 ramps for 4 brand chromatic | Task 3 |
| 50–950 ramps for 5 semantic | Task 4 |
| Neutral surface ramp | Task 4 |
| Type scale tokens | Task 2 |
| Spacing scale extensions | Task 2 |
| Radius scale | Task 2 |
| Elevation/shadow scale | Task 2 |
| Motion ease + duration tokens | Task 1 |
| Dark-mode-ready `[data-theme="dark"]` placeholder | Task 7 |
| Asset mirror | Every task that touches design-system/assets/* |
| Performance budget (fonts ≤120 KB) | Task 5 |
| Token-first audit | Deferred to Plan B (no component CSS edits in Plan A) |

## Out of scope (do not implement here)

- Any change to component CSS (Plan B owns the migration to ramp steps).
- Surface-level density application (Plan D owns the two-tier density layout work).
- Dark theme values (placeholder only, per vision spec decision 5).
- New components (Plan B / Plan C).
- Performance benchmarks beyond a manual DevTools check.

## Risks

- **Font download failure during Task 5.** The curl URLs may move. The plan says STOP and ask in that case; do not improvise a different font source.
- **Visual regression from refined shadow scale.** Task 2 deliberately refines `--shadow-sm/md/lg` values. Components using these tokens will look subtly different. If anything looks worse rather than softer, revisit the values in a follow-up commit; do not roll back the scale wholesale.
- **CSS bloat.** After Tasks 3+4, `tokens.css` grows from ~93 lines to ~250 lines. Still well within budget.
- **Asset-sync drift.** Every task touches either tokens.css or components.css; the discipline of "mirror immediately after writing" must hold every single time. Tooling-level enforcement is a future plan retrospective item, not this plan's scope.
