// crates/design-system/src/empty_state.rs
use dioxus::prelude::*;

#[derive(Clone, PartialEq, Default)]
pub enum EmptyStateVariant {
    #[default]
    Default,
    Subtle,
    Accent,
}

#[derive(Props, Clone, PartialEq)]
pub struct EmptyStateProps {
    pub title: String,
    pub description: String,
    #[props(default)]
    pub illustration: Option<Element>,
    #[props(default)]
    pub cta: Option<Element>,
    #[props(default)]
    pub variant: EmptyStateVariant,
}

#[component]
pub fn EmptyState(props: EmptyStateProps) -> Element {
    let mut class = String::from("empty-state");
    match props.variant {
        EmptyStateVariant::Default => {}
        EmptyStateVariant::Subtle => class.push_str(" empty-state--subtle"),
        EmptyStateVariant::Accent => class.push_str(" empty-state--accent"),
    }

    rsx! {
        div { class: "{class}",
            if let Some(illustration) = &props.illustration {
                div { class: "empty-state-illustration", {illustration.clone()} }
            }
            h3 { class: "empty-state-title", "{props.title}" }
            p { class: "empty-state-desc", "{props.description}" }
            if let Some(cta) = &props.cta {
                div { class: "empty-state-cta", {cta.clone()} }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dioxus::prelude::VirtualDom;

    #[test]
    fn empty_state_renders_title_and_description() {
        fn app() -> Element {
            rsx! {
                EmptyState {
                    title: "Nothing here".to_string(),
                    description: "Try again".to_string(),
                    cta: None,
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("empty-state-title"),
            "title class missing: {html}"
        );
        assert!(html.contains("Nothing here"), "title text missing: {html}");
        assert!(
            html.contains("empty-state-desc"),
            "desc class missing: {html}"
        );
        assert!(html.contains("Try again"), "desc text missing: {html}");
    }

    #[test]
    fn empty_state_accent_variant_renders_class() {
        fn app() -> Element {
            rsx! {
                EmptyState {
                    title: "Nothing here".to_string(),
                    description: "Try again".to_string(),
                    variant: EmptyStateVariant::Accent,
                    cta: None,
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("empty-state--accent"),
            "accent class missing: {html}"
        );
    }

    #[test]
    fn empty_state_subtle_variant_renders_class() {
        fn app() -> Element {
            rsx! {
                EmptyState {
                    title: "Empty".to_string(),
                    description: "Nothing yet".to_string(),
                    variant: EmptyStateVariant::Subtle,
                    cta: None,
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("empty-state--subtle"),
            "subtle class missing: {html}"
        );
    }

    #[test]
    fn empty_state_renders_illustration_when_provided() {
        fn app() -> Element {
            rsx! {
                EmptyState {
                    title: "Empty".to_string(),
                    description: "Add some".to_string(),
                    illustration: rsx! { svg { width: "32", height: "32" } },
                    cta: None,
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("empty-state-illustration"),
            "illustration container missing: {html}"
        );
        assert!(html.contains("<svg"), "svg child missing: {html}");
    }
}
