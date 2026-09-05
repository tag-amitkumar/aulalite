// crates/design-system/src/illustration.rs
//! Small decorative line-art illustrations for empty states. Theme-aware: the
//! strokes use `currentColor` (so they inherit the surrounding text color in
//! light and dark) with a gold brand accent. Rendered via `dangerous_inner_html`
//! from fully-internal markup, so there is no injection surface.
//!
//! These fill the `EmptyState { illustration }` slot, which existed but was
//! never populated — every empty state in the app was text-only.

use dioxus::prelude::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum IllustrationKind {
    /// Stack of cards / courses.
    Courses,
    /// Generic empty list / inbox.
    List,
    /// Search with no results.
    NoResults,
    /// Calendar / sessions.
    Calendar,
}

const GOLD: &str = "#b08842";

fn svg_for(kind: IllustrationKind) -> &'static str {
    match kind {
        IllustrationKind::Courses => {
            "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 120 120' fill='none' \
stroke='currentColor' stroke-width='2.5' stroke-linecap='round' stroke-linejoin='round' \
width='120' height='120' aria-hidden='true'>\
<rect x='26' y='30' width='68' height='52' rx='6' opacity='0.35'/>\
<rect x='18' y='40' width='84' height='56' rx='7'/>\
<line x1='30' y1='58' x2='66' y2='58'/><line x1='30' y1='70' x2='80' y2='70'/>\
<line x1='30' y1='82' x2='58' y2='82'/>\
<path d='M78 24 l8 5 -8 5 -8 -5 z' fill='#b08842' stroke='#b08842'/></svg>"
        }
        IllustrationKind::List => {
            "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 120 120' fill='none' \
stroke='currentColor' stroke-width='2.5' stroke-linecap='round' stroke-linejoin='round' \
width='120' height='120' aria-hidden='true'>\
<rect x='24' y='28' width='72' height='72' rx='8'/>\
<circle cx='38' cy='46' r='3.2' fill='#b08842' stroke='#b08842'/>\
<circle cx='38' cy='64' r='3.2' fill='#b08842' stroke='#b08842'/>\
<circle cx='38' cy='82' r='3.2' opacity='0.4'/>\
<line x1='50' y1='46' x2='84' y2='46'/><line x1='50' y1='64' x2='84' y2='64'/>\
<line x1='50' y1='82' x2='72' y2='82' opacity='0.4'/></svg>"
        }
        IllustrationKind::NoResults => {
            "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 120 120' fill='none' \
stroke='currentColor' stroke-width='2.5' stroke-linecap='round' stroke-linejoin='round' \
width='120' height='120' aria-hidden='true'>\
<circle cx='54' cy='52' r='26'/><line x1='73' y1='71' x2='94' y2='92'/>\
<line x1='45' y1='52' x2='63' y2='52' stroke='#b08842'/></svg>"
        }
        IllustrationKind::Calendar => {
            "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 120 120' fill='none' \
stroke='currentColor' stroke-width='2.5' stroke-linecap='round' stroke-linejoin='round' \
width='120' height='120' aria-hidden='true'>\
<rect x='24' y='30' width='72' height='66' rx='8'/>\
<line x1='24' y1='46' x2='96' y2='46'/>\
<line x1='40' y1='24' x2='40' y2='36'/><line x1='80' y1='24' x2='80' y2='36'/>\
<circle cx='60' cy='70' r='9' fill='#b08842' stroke='#b08842'/></svg>"
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct IllustrationProps {
    pub kind: IllustrationKind,
    #[props(default = "ds-illustration".to_string())]
    pub class: String,
}

#[component]
pub fn Illustration(props: IllustrationProps) -> Element {
    // Touch GOLD so the accent constant is part of the public surface even if a
    // future kind stops using it inline.
    let _ = GOLD;
    let svg = svg_for(props.kind);
    rsx! {
        span {
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
    fn each_kind_renders_an_svg() {
        for kind in [
            IllustrationKind::Courses,
            IllustrationKind::List,
            IllustrationKind::NoResults,
            IllustrationKind::Calendar,
        ] {
            let s = svg_for(kind);
            assert!(s.starts_with("<svg"), "kind {kind:?} not an svg");
            assert!(s.contains("currentColor"), "kind {kind:?} not theme-aware");
        }
    }

    #[test]
    fn illustration_component_renders_wrapper_and_svg() {
        fn app() -> Element {
            rsx! { Illustration { kind: IllustrationKind::Courses } }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("ds-illustration"), "wrapper missing: {html}");
        assert!(html.contains("<svg"), "svg missing: {html}");
    }
}
