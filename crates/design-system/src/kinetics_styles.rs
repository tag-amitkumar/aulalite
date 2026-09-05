//! dioxus-kinetics token bridge + component CSS injection.
//!
//! `dioxus-kinetics` components are pure `.ui-*` class + `data-*` emitters that
//! read their styling exclusively from `--ui-*` CSS custom properties. The
//! library ships `library_css()` (base tokens + component rules), but its
//! `base_css()` also emits global `body {}` / `* {}` / `:root` overrides that
//! would fight AulaLite's existing `tokens.css` + `components.css`.
//!
//! Instead we inject ONLY:
//!   1. a `:root` **bridge** that defines every `--ui-*` variable the component
//!      CSS references, mapping the color/brand/radius/space/motion/elevation
//!      subset onto AulaLite's own `--color-*` / `--radius-*` / `--space-*` /
//!      `--shadow-*` ramps, and copying the (isolated) type scale verbatim; and
//!   2. `kinetics::prelude::COMPONENT_CSS` — the `.ui-*` component rules.
//!
//! `--ui-*` variables are inert until consumed by a `.ui-*` class, so defining
//! them globally on `:root` has zero side effects on existing AulaLite screens.
//! This gives kinetics components AulaLite's warm-cream + green-600 brand for
//! free, with no global regressions.

use dioxus::prelude::*;
use std::sync::OnceLock;

/// The `:root` bridge mapping kinetics `--ui-*` tokens onto AulaLite tokens.
const KINETICS_TOKEN_BRIDGE: &str = r#"
:root {
  /* dioxus-kinetics → AulaLite token bridge. Inert until a .ui-* class reads it. */
  --ui-font-sans: var(--font-body);

  --ui-bg: var(--color-paper);
  --ui-surface: var(--color-surface);
  --ui-surface-muted: var(--color-surface-subtle);
  --ui-surface-strong: var(--color-neutral-200);
  --ui-fg: var(--color-text);
  --ui-muted-fg: var(--color-text-muted);
  --ui-border: var(--color-rule);
  --ui-focus: var(--color-gold-600);
  --ui-focus-text: var(--color-gold-700);

  --ui-primary: var(--color-primary);
  --ui-success: var(--color-success);
  --ui-warning: var(--color-warning);
  --ui-danger: var(--color-danger);
  --ui-info: var(--color-info);
  --ui-accent: var(--color-primary);
  --ui-on-accent: #ffffff;

  /* Glass material — warm-cream translucency on AulaLite paper. */
  --ui-glass: rgba(255, 253, 247, 0.72);
  --ui-glass-solid: var(--color-surface);
  --ui-glass-blur: 18px;
  --ui-glass-saturate: 160%;
  --ui-glass-highlight: rgba(255, 255, 255, 0.55);
  --ui-glass-highlight-bottom: rgba(255, 255, 255, 0.10);

  --ui-shadow-soft: var(--shadow-lg);
  --ui-shadow-lifted: var(--shadow-2xl);
  --ui-elevation-0: var(--shadow-flat);
  --ui-elevation-1: var(--shadow-sm);
  --ui-elevation-2: var(--shadow-md);
  --ui-elevation-3: var(--shadow-lg);

  --ui-radius-sm: var(--radius-sm);
  --ui-radius-md: var(--radius);
  --ui-radius-lg: var(--radius-lg);
  --ui-radius-floating: var(--radius-xl);
  --ui-radius-full: var(--radius-full);

  --ui-space-0: 2px;
  --ui-space-1: var(--space-1);
  --ui-space-2: var(--space-2);
  --ui-space-3: var(--space-3);
  --ui-space-4: var(--space-4);
  --ui-space-5: var(--space-5);
  --ui-space-6: var(--space-6);
  --ui-space-7: 48px;
  --ui-space-8: 64px;

  --ui-control-height: var(--control-h-md);

  --ui-motion-fast: var(--duration-fast);
  --ui-motion-normal: var(--duration-enter);
  --ui-motion-press: 90ms;
  --ui-press-scale: 0.97;
  --ui-ease-standard: var(--ease-snappy);
  --ui-ease-emphasized: var(--ease-content);
  --ui-ease-decelerate: var(--ease-decelerate);
  --ui-ease-accelerate: var(--ease-accelerate);
  --ui-ease-spring: var(--ease-spring);

  /* Type scale (kinetics ramp; isolated to .ui-* component surfaces). */
  --ui-text-caption2: 11px; --ui-leading-caption2: 1.45; --ui-tracking-caption2: 0.005em;
  --ui-text-caption: 12px; --ui-leading-caption: 1.40; --ui-tracking-caption: 0.004em;
  --ui-text-footnote: 13px; --ui-leading-footnote: 1.45; --ui-tracking-footnote: 0em;
  --ui-text-subhead: 15px; --ui-leading-subhead: 1.40; --ui-tracking-subhead: -0.002em;
  --ui-text-callout: 16px; --ui-leading-callout: 1.45; --ui-tracking-callout: -0.004em;
  --ui-text-body: 16px; --ui-leading-body: 1.5; --ui-tracking-body: -0.004em;
  --ui-text-headline: 16px; --ui-leading-headline: 1.40; --ui-tracking-headline: -0.006em;
  --ui-text-title3: 20px; --ui-leading-title3: 1.25; --ui-tracking-title3: -0.010em;
  --ui-text-title2: 22px; --ui-leading-title2: 1.20; --ui-tracking-title2: -0.012em;
  --ui-text-title1: 28px; --ui-leading-title1: 1.15; --ui-tracking-title1: -0.016em;
  --ui-text-largetitle: 34px; --ui-leading-largetitle: 1.10; --ui-tracking-largetitle: -0.020em;
  --ui-text-display: clamp(40px, 5vw, 64px); --ui-leading-display: 1.04; --ui-tracking-display: -0.022em;

  --ui-weight-regular: 400;
  --ui-weight-medium: 500;
  --ui-weight-semibold: 600;
  --ui-weight-bold: 700;
}

/* Dark theme — most --ui-* tokens flip for free because they alias AulaLite
 * --color-*/--shadow-* tokens, which are overridden in tokens.css under
 * [data-ui-theme="dark"]. Only the raw literal values need counterparts. */
[data-ui-theme="dark"] {
  --ui-glass: rgba(18, 28, 23, 0.72);
  --ui-glass-highlight: rgba(255, 255, 255, 0.10);
  --ui-glass-highlight-bottom: rgba(255, 255, 255, 0.04);
}

/* Density variants (kinetics data-ui-density contract). The default
 * (comfortable) values live on :root above. */
[data-ui-density="compact"] {
  --ui-control-height: var(--control-h-sm);
  --ui-space-3: 10px;
  --ui-space-4: 12px;
}
[data-ui-density="spacious"] {
  --ui-control-height: var(--control-h-lg);
  --ui-space-3: 14px;
  --ui-space-4: 20px;
}
"#;

/// Chart-series brand ramp. The kinetics `charts.css` defines `--ui-chart-1..6`
/// directly on `.ui-chart` / `.ui-sparkline` / `.ui-donut-gauge` (element
/// scope, so it beats anything the `:root` bridge says), which leaves series
/// 2+ on the semantic info/success accents instead of the brand. This block is
/// appended AFTER the component CSS so the same-specificity selectors win the
/// cascade and re-map the first three series onto the AulaLite palette:
/// deep green, gold, muted sage.
const KINETICS_CHART_BRAND: &str = r#"
.ui-chart,
.ui-sparkline,
.ui-donut-gauge {
  --ui-chart-1: var(--color-primary);   /* deep green (flips via tokens.css dark block) */
  --ui-chart-2: var(--color-gold-600);  /* gold */
  --ui-chart-3: var(--color-green-400); /* muted sage */
}
[data-ui-theme="dark"] .ui-chart,
[data-ui-theme="dark"] .ui-sparkline,
[data-ui-theme="dark"] .ui-donut-gauge {
  /* Raw gold/green ramp steps are theme-invariant, so lift them a step by
   * hand to hold contrast on the dark green-charcoal paper. */
  --ui-chart-2: var(--color-gold-500);
  --ui-chart-3: var(--color-green-300);
}
"#;

/// Returns the full kinetics stylesheet (token bridge + component CSS),
/// built once and cached for the process lifetime.
pub fn kinetics_css() -> &'static str {
    static CSS: OnceLock<String> = OnceLock::new();
    CSS.get_or_init(|| {
        // `library_css()` = base_css() + COMPONENT_CSS + the per-family sheets
        // (scene player, charts, sortable, tour, voice, learn). The facade only
        // exports the aggregate, so strip the `base_css()` prefix to keep the
        // no-global-overrides policy while picking up every family sheet.
        let library = kinetics::prelude::library_css();
        let base_len = kinetics::prelude::base_css().len();
        let component_css = library[base_len..].trim_start();
        let mut s = String::with_capacity(
            KINETICS_TOKEN_BRIDGE.len() + component_css.len() + KINETICS_CHART_BRAND.len() + 2,
        );
        s.push_str(KINETICS_TOKEN_BRIDGE);
        s.push('\n');
        s.push_str(component_css);
        s.push('\n');
        s.push_str(KINETICS_CHART_BRAND);
        s
    })
}

/// Injects the kinetics token bridge + component CSS as a `<style>` element.
/// Mount this once near the application root (it works on both the web and
/// desktop shells, unlike the `<link>`-based stylesheets in `index.html`).
#[component]
pub fn KineticsStyles() -> Element {
    rsx! {
        style { dangerous_inner_html: kinetics_css() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn css_contains_bridge_and_components() {
        let css = kinetics_css();
        // Token bridge maps onto AulaLite ramps.
        assert!(css.contains("--ui-primary: var(--color-primary)"));
        assert!(css.contains("--ui-surface: var(--color-surface)"));
        // Component rules came through from the kinetics facade.
        assert!(css.contains(".ui-metric-card"));
        assert!(css.contains(".ui-command-menu"));
        assert!(css.contains(".ui-assistant-panel"));
        // ui-learn surfaces shipped with the `learn` feature.
        assert!(css.contains(".ui-course-outline"));
        assert!(css.contains(".ui-certificate"));
    }

    #[test]
    fn dark_theme_and_density_bridge_blocks_present() {
        let css = kinetics_css();
        assert!(css.contains("[data-ui-theme=\"dark\"]"));
        assert!(css.contains("--ui-glass: rgba(18, 28, 23, 0.72)"));
        assert!(css.contains("[data-ui-density=\"compact\"]"));
        assert!(css.contains("[data-ui-density=\"spacious\"]"));
    }

    #[test]
    fn chart_series_brand_ramp_appended_after_component_css() {
        let css = kinetics_css();
        assert!(css.contains("--ui-chart-1: var(--color-primary)"));
        assert!(css.contains("--ui-chart-2: var(--color-gold-600)"));
        assert!(css.contains("--ui-chart-3: var(--color-green-400)"));
        // Dark counterparts for the theme-invariant raw ramp steps.
        assert!(css.contains("--ui-chart-2: var(--color-gold-500)"));
        assert!(css.contains("--ui-chart-3: var(--color-green-300)"));
        // Must sit after the kinetics default ramp (same specificity) so the
        // brand mapping wins the cascade.
        let default_pos = css
            .find("--ui-chart-1: var(--ui-primary)")
            .expect("kinetics default chart ramp missing");
        let brand_pos = css
            .find("--ui-chart-1: var(--color-primary)")
            .expect("brand chart ramp missing");
        assert!(
            brand_pos > default_pos,
            "brand chart ramp must come after the kinetics default"
        );
    }

    #[test]
    fn every_referenced_color_token_is_defined() {
        let css = kinetics_css();
        for tok in [
            "--ui-bg",
            "--ui-fg",
            "--ui-border",
            "--ui-accent",
            "--ui-on-accent",
            "--ui-glass",
            "--ui-elevation-2",
            "--ui-radius-md",
            "--ui-space-4",
            "--ui-control-height",
            "--ui-motion-fast",
            "--ui-text-body",
            "--ui-weight-bold",
        ] {
            // each must appear as a definition `<tok>:`
            assert!(
                css.contains(&format!("{tok}:")),
                "missing --ui token definition: {tok}"
            );
        }
    }
}
