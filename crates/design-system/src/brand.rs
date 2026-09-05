use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct AulaLogoProps {
    #[props(default = "aula-logo".to_string())]
    pub class: String,
    #[props(default = false)]
    pub compact: bool,
}

#[component]
pub fn AulaLogo(props: AulaLogoProps) -> Element {
    // Compact: keep the standalone emblem mark as-is — it's a self-contained
    // dark badge that reads on either theme.
    if props.compact {
        return rsx! {
            span { class: "{props.class}",
                img { class: "aula-logo__image", src: "/assets/brand/aulalite-mark.svg", alt: "AulaLite" }
            }
        };
    }

    // Non-compact: the old <img> wordmark hard-coded light fills and went
    // low-contrast in dark mode. Pair the emblem mark with a CSS-text wordmark
    // ("Aula" + gold "Lite") so the type recolors via currentColor + the gold
    // token across themes (mirrors the login hero's CSS wordmark).
    rsx! {
        span { class: "{props.class} aula-logo--lockup",
            img {
                class: "aula-logo__mark",
                src: "/assets/brand/aulalite-mark.svg",
                alt: "",
                "aria-hidden": "true",
            }
            span {
                class: "aula-logo__wordmark",
                "aria-label": "AulaLite Live Learning Academy",
                "Aula"
                span { class: "aula-logo__lite", "Lite" }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logo_renders_wordmark_by_default() {
        fn app() -> Element {
            rsx! { AulaLogo {} }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        // The non-compact lockup pairs the emblem mark with a CSS-text wordmark
        // (so the type recolors via currentColor + the gold token in dark mode)
        // rather than the old hard-light wordmark <img>.
        assert!(
            html.contains("aula-logo--lockup"),
            "lockup class missing: {html}"
        );
        assert!(
            html.contains("aulalite-mark.svg"),
            "emblem mark missing: {html}"
        );
        assert!(
            html.contains("aula-logo__wordmark"),
            "css wordmark missing: {html}"
        );
        assert!(
            html.contains("aula-logo__lite"),
            "gold lite span missing: {html}"
        );
        // Accessible name is preserved on the wordmark.
        assert!(
            html.contains("AulaLite Live Learning Academy"),
            "accessible name missing: {html}"
        );
        // The old <img> wordmark is gone (it was the dark-mode contrast bug).
        assert!(
            !html.contains("aulalite-wordmark.svg"),
            "old hard-light wordmark img leaked: {html}"
        );
    }

    #[test]
    fn logo_renders_compact_mark() {
        fn app() -> Element {
            rsx! { AulaLogo { compact: true } }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("aulalite-mark.svg"), "got: {html}");
    }
}
