// crates/design-system/src/auth_hero.rs
//! Bespoke, theme-aware "live academy" hero illustration for the auth surfaces
//! (login / signup / forgot). It drops into the `.auth-hero-visual` slot of the
//! `.auth-composite` cover split alongside the editorial copy.
//!
//! The scene is a fully self-contained inline SVG injected via
//! `dangerous_inner_html` (no external text, so there is no injection surface —
//! same pattern as `course_cover_art` and `illustration`). Strokes use
//! `currentColor` so the line art inherits the hero text color and recolors for
//! light + dark automatically; the gold (`#b08842`) and oxblood "live"
//! (`#b8324a`) accents are painted inline so the brand reads on either canvas.
//!
//! The depicted scene: a stage/whiteboard framing a live lesson, an oxblood
//! "LIVE" pill, three seat/avatar nodes (the room), a play glyph, and a faint
//! orbital motif echoing the generated course covers — i.e. an "academy
//! broadcasting a live room", which is exactly what AulaLite is.

use dioxus::prelude::*;

/// Gold brand accent.
const GOLD: &str = "#b08842";
/// Oxblood "live" accent.
const LIVE: &str = "#b8324a";

/// Build the hero scene SVG. Pure + dependency-free so it is unit-testable and
/// deterministic across platforms.
pub fn build_auth_hero_svg() -> String {
    // viewBox 0 0 420 360. `currentColor` carries the line-art; gold + oxblood
    // are painted inline so they survive a theme flip.
    format!(
        "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 420 360' fill='none' \
role='img' aria-hidden='true' width='100%' height='100%' \
preserveAspectRatio='xMidYMid meet'>\
<defs>\
<linearGradient id='ah-stage' x1='0' y1='0' x2='1' y2='1'>\
<stop offset='0' stop-color='{GOLD}' stop-opacity='0.16'/>\
<stop offset='1' stop-color='{GOLD}' stop-opacity='0.04'/>\
</linearGradient>\
<linearGradient id='ah-screen' x1='0' y1='0' x2='0' y2='1'>\
<stop offset='0' stop-color='currentColor' stop-opacity='0.10'/>\
<stop offset='1' stop-color='currentColor' stop-opacity='0.03'/>\
</linearGradient>\
</defs>\
<!-- faint orbital motif (echoes the generated course covers) -->\
<g stroke='currentColor' stroke-opacity='0.12' stroke-width='1.5'>\
<circle cx='318' cy='78' r='118'/><circle cx='318' cy='78' r='80'/>\
</g>\
<!-- ground line -->\
<line x1='40' y1='300' x2='380' y2='300' stroke='currentColor' \
stroke-opacity='0.18' stroke-width='2' stroke-linecap='round'/>\
<!-- stage / whiteboard panel -->\
<rect x='58' y='62' width='304' height='190' rx='18' fill='url(#ah-stage)' \
stroke='currentColor' stroke-opacity='0.30' stroke-width='2.5'/>\
<rect x='58' y='62' width='304' height='34' rx='18' fill='currentColor' \
fill-opacity='0.06'/>\
<!-- title bar dots -->\
<circle cx='80' cy='79' r='4' fill='{LIVE}'/>\
<circle cx='96' cy='79' r='4' fill='{GOLD}'/>\
<circle cx='112' cy='79' r='4' fill='currentColor' fill-opacity='0.35'/>\
<!-- inner screen / shared content -->\
<rect x='86' y='116' width='168' height='108' rx='10' fill='url(#ah-screen)' \
stroke='currentColor' stroke-opacity='0.22' stroke-width='2'/>\
<!-- content lines on the screen -->\
<g stroke='{GOLD}' stroke-width='3.5' stroke-linecap='round'>\
<line x1='104' y1='140' x2='168' y2='140'/>\
</g>\
<g stroke='currentColor' stroke-opacity='0.30' stroke-width='3' stroke-linecap='round'>\
<line x1='104' y1='160' x2='236' y2='160'/>\
<line x1='104' y1='178' x2='214' y2='178'/>\
<line x1='104' y1='196' x2='192' y2='196'/>\
</g>\
<!-- play glyph (the live broadcast) -->\
<circle cx='300' cy='168' r='34' fill='{GOLD}' fill-opacity='0.12' \
stroke='{GOLD}' stroke-width='2.5'/>\
<path d='M292 152 L318 168 L292 184 Z' fill='{GOLD}'/>\
<!-- LIVE pill -->\
<rect x='270' y='104' width='66' height='26' rx='13' fill='{LIVE}'/>\
<circle cx='286' cy='117' r='5' fill='#ffffff'/>\
<text x='298' y='121' font-family='Inter, system-ui, sans-serif' font-size='12' \
font-weight='700' letter-spacing='1.5' fill='#ffffff'>LIVE</text>\
<!-- the room: three seat / avatar nodes -->\
<g>\
<circle cx='120' cy='282' r='20' fill='currentColor' fill-opacity='0.07' \
stroke='currentColor' stroke-opacity='0.30' stroke-width='2'/>\
<circle cx='120' cy='275' r='7' fill='currentColor' fill-opacity='0.35'/>\
<path d='M108 292 a12 10 0 0 1 24 0' fill='currentColor' fill-opacity='0.20'/>\
<circle cx='210' cy='288' r='24' fill='{GOLD}' fill-opacity='0.16' \
stroke='{GOLD}' stroke-width='2.5'/>\
<circle cx='210' cy='280' r='8' fill='{GOLD}'/>\
<path d='M196 299 a14 12 0 0 1 28 0' fill='{GOLD}' fill-opacity='0.55'/>\
<circle cx='300' cy='282' r='20' fill='currentColor' fill-opacity='0.07' \
stroke='currentColor' stroke-opacity='0.30' stroke-width='2'/>\
<circle cx='300' cy='275' r='7' fill='currentColor' fill-opacity='0.35'/>\
<path d='M288 292 a12 10 0 0 1 24 0' fill='currentColor' fill-opacity='0.20'/>\
</g>\
<!-- gold spark accents -->\
<path d='M48 120 l3 9 9 3 -9 3 -3 9 -3 -9 -9 -3 9 -3 z' fill='{GOLD}' \
fill-opacity='0.55'/>\
<circle cx='372' cy='250' r='3.5' fill='{LIVE}' fill-opacity='0.7'/>\
</svg>"
    )
}

#[derive(Props, Clone, PartialEq)]
pub struct AuthHeroProps {
    /// Wrapper class. Defaults to the hero-art class; the `.auth-hero-visual`
    /// slot already positions it.
    #[props(default = "auth-hero-art".to_string())]
    pub class: String,
}

#[component]
pub fn AuthHero(props: AuthHeroProps) -> Element {
    let svg = build_auth_hero_svg();
    rsx! {
        div {
            class: "{props.class}",
            "aria-hidden": "true",
            dangerous_inner_html: "{svg}",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hero_svg_is_theme_aware_and_branded() {
        let svg = build_auth_hero_svg();
        assert!(svg.starts_with("<svg"), "not an svg: {svg}");
        assert!(svg.contains("</svg>"), "unterminated svg");
        // Line art inherits the text color so it recolors for light + dark.
        assert!(svg.contains("currentColor"), "not theme-aware: {svg}");
        // Brand accents are painted inline so they survive a theme flip.
        assert!(svg.contains(GOLD), "gold accent missing: {svg}");
        assert!(svg.contains(LIVE), "oxblood live accent missing: {svg}");
        // The scene actually depicts a live room.
        assert!(svg.contains(">LIVE<"), "LIVE pill text missing: {svg}");
    }

    #[test]
    fn hero_svg_has_no_unescaped_external_text() {
        // The only <text> nodes are our own literals (LIVE) — no user data is
        // ever interpolated, so there is no injection surface.
        let svg = build_auth_hero_svg();
        assert_eq!(
            svg.matches("<text").count(),
            1,
            "unexpected text nodes: {svg}"
        );
    }

    #[test]
    fn auth_hero_component_renders_wrapper_and_svg() {
        fn app() -> Element {
            rsx! { AuthHero {} }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("auth-hero-art"),
            "wrapper class missing: {html}"
        );
        assert!(html.contains("<svg"), "svg missing: {html}");
    }

    #[test]
    fn auth_hero_component_accepts_custom_class() {
        fn app() -> Element {
            rsx! { AuthHero { class: "custom-hero".to_string() } }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("custom-hero"), "custom class missing: {html}");
    }
}
