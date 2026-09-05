// crates/design-system/src/course_cover_art.rs
//! Deterministic, on-brand generated cover art for courses that have no
//! uploaded cover image. Previously every coverless course shared one raster
//! PNG, so they all looked identical; this derives a distinct gradient + motif
//! + initials from a stable seed (the course slug) so each course reads as its own
//!   while staying within the brand palette.
//!
//! The SVG is built as a string and injected via `dangerous_inner_html`: it is
//! fully generated here (the only external text — the initials — is XML-escaped)
//! and this avoids per-renderer SVG-element quirks. `build_cover_svg` is pure
//! and unit-tested for determinism.

use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct CourseCoverArtProps {
    /// Stable seed — the same seed always yields the same art. Use the course
    /// slug (or id) so a course's generated cover never changes between renders.
    pub seed: String,
    /// Title used to derive the 1–2 initials shown on the cover.
    pub title: String,
    /// Wrapper class. Defaults to the generated-cover class; callers pass the
    /// box-sizing class alongside (e.g. `"course-card-cover-empty course-gen-cover"`).
    #[props(default = "course-gen-cover".to_string())]
    pub class: String,
}

/// Curated brand-adjacent gradient pairs (top-left → bottom-right). Selected by
/// the seed hash so covers vary but always read on-brand on cream or dark.
const COVER_GRADIENTS: [(&str, &str); 6] = [
    ("#2f6552", "#10251f"), // green (primary)
    ("#1d3144", "#0d1b27"), // navy
    ("#b08842", "#6d4f1f"), // gold
    ("#8a3b4d", "#46202b"), // oxblood
    ("#356b73", "#13343a"), // teal
    ("#4b4a6e", "#23233a"), // indigo
];

/// FNV-1a — small, stable, dependency-free hash so the cover is deterministic
/// across builds and platforms (unlike `DefaultHasher`, which is unspecified).
fn hash_seed(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// 1–2 uppercase initials from the first words of the title; falls back to a
/// dot when the title has no usable letters.
fn initials(title: &str) -> String {
    let mut out = String::new();
    for word in title.split_whitespace().take(2) {
        if let Some(c) = word.chars().find(|c| c.is_alphanumeric()) {
            out.extend(c.to_uppercase());
        }
    }
    if out.is_empty() {
        "•".to_string()
    } else {
        out
    }
}

fn xml_escape(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '&' => "&amp;".to_string(),
            '<' => "&lt;".to_string(),
            '>' => "&gt;".to_string(),
            '"' => "&quot;".to_string(),
            '\'' => "&apos;".to_string(),
            other => other.to_string(),
        })
        .collect()
}

/// Build the deterministic cover SVG markup for a seed + title.
pub fn build_cover_svg(seed: &str, title: &str) -> String {
    let h = hash_seed(seed);
    let (c0, c1) = COVER_GRADIENTS[(h % COVER_GRADIENTS.len() as u64) as usize];
    // Unique gradient id so multiple covers on one page don't collide on a
    // document-global SVG id.
    let gid = format!("ccg{h:016x}");
    let inits = xml_escape(&initials(title));
    // Vary the motif position a little per seed.
    let cx = 70 + (h >> 17) % 260; // 70..330
    format!(
        "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 400 225' \
preserveAspectRatio='xMidYMid slice' role='img' width='100%' height='100%'>\
<defs><linearGradient id='{gid}' x1='0' y1='0' x2='1' y2='1'>\
<stop offset='0' stop-color='{c0}'/><stop offset='1' stop-color='{c1}'/>\
</linearGradient></defs>\
<rect width='400' height='225' fill='url(#{gid})'/>\
<g fill='none' stroke='#ffffff' stroke-opacity='0.16' stroke-width='2'>\
<circle cx='{cx}' cy='52' r='120'/><circle cx='{cx}' cy='52' r='78'/></g>\
<path d='M64 190 L122 98 L180 190 Z' fill='#ffffff' fill-opacity='0.08'/>\
<text x='200' y='133' text-anchor='middle' \
font-family=\"Source Serif 4, Georgia, serif\" font-size='78' font-weight='600' \
fill='#ffffff' fill-opacity='0.92'>{inits}</text>\
</svg>"
    )
}

#[component]
pub fn CourseCoverArt(props: CourseCoverArtProps) -> Element {
    let svg = build_cover_svg(&props.seed, &props.title);
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
    fn cover_svg_is_deterministic_for_a_seed() {
        let a = build_cover_svg("intro-to-rust", "Intro to Rust");
        let b = build_cover_svg("intro-to-rust", "Intro to Rust");
        assert_eq!(a, b, "same seed must produce identical art");
        assert!(a.contains("<svg"));
        assert!(a.contains("linearGradient"));
    }

    #[test]
    fn different_seeds_pick_different_gradient_ids() {
        let a = build_cover_svg("course-a", "Alpha");
        let b = build_cover_svg("course-b", "Beta");
        assert_ne!(a, b, "distinct seeds should differ");
    }

    #[test]
    fn initials_are_uppercased_first_two_words() {
        assert_eq!(initials("intro to rust"), "IT");
        assert_eq!(initials("Biology"), "B");
        assert_eq!(initials("   "), "•");
        assert_eq!(initials("3d modelling"), "3M");
    }

    #[test]
    fn titles_cannot_inject_markup_into_the_svg() {
        // initials() takes only alphanumeric chars, so a markup-y title yields
        // safe letters (here "S") and can't break the SVG; xml_escape stays as
        // defence in depth for the (currently unreachable) symbol case.
        let svg = build_cover_svg("x", "<script>alert(1)</script>");
        assert!(!svg.contains("<script>"), "raw markup leaked: {svg}");
        assert!(svg.contains(">S<"), "expected safe initial in text: {svg}");
    }

    #[test]
    fn cover_renders_via_component() {
        fn app() -> Element {
            rsx! { CourseCoverArt { seed: "s".to_string(), title: "Demo Course".to_string() } }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("course-gen-cover"),
            "wrapper class missing: {html}"
        );
        assert!(html.contains("<svg"), "svg missing: {html}");
    }
}
