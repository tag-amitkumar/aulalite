use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct CardProps {
    pub children: Element,
    /// When set, the card becomes a clickable surface with hover affordance.
    #[props(default)]
    pub on_click: Option<EventHandler<MouseEvent>>,
    #[props(default)]
    pub variant: CardVariant,
    /// Opt-in warm-halo hover (also implied when `on_click` is set).
    #[props(default)]
    pub interactive: bool,
}

#[derive(Clone, PartialEq, Default)]
pub enum CardVariant {
    #[default]
    Default,
    Accent,
    Danger,
    /// Gold inner border (used sparingly for premium tiers).
    Premium,
}

#[component]
pub fn Card(props: CardProps) -> Element {
    let mut class = String::from("ds-card");
    match props.variant {
        CardVariant::Default => {}
        CardVariant::Accent => class.push_str(" ds-card--accent"),
        CardVariant::Danger => class.push_str(" ds-card--danger"),
        CardVariant::Premium => class.push_str(" ds-card--premium"),
    }
    let is_interactive = props.interactive || props.on_click.is_some();
    if is_interactive {
        class.push_str(" ds-card--interactive");
    }
    if props.on_click.is_some() {
        class.push_str(" ds-card--clickable");
    }

    if let Some(on_click) = props.on_click {
        rsx! {
            div {
                class: "{class}",
                role: "button",
                tabindex: "0",
                onclick: move |evt| on_click.call(evt),
                {props.children}
            }
        }
    } else {
        rsx! { div { class: "{class}", {props.children} } }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct CardSlotProps {
    pub children: Element,
}

#[component]
pub fn CardHeader(props: CardSlotProps) -> Element {
    rsx! { div { class: "ds-card-header", {props.children} } }
}

#[component]
pub fn CardTitle(props: CardSlotProps) -> Element {
    rsx! { h3 { class: "ds-card-title", {props.children} } }
}

#[component]
pub fn CardDescription(props: CardSlotProps) -> Element {
    rsx! { p { class: "ds-card-description", {props.children} } }
}

#[component]
pub fn CardContent(props: CardSlotProps) -> Element {
    rsx! { div { class: "ds-card-content", {props.children} } }
}

#[component]
pub fn CardFooter(props: CardSlotProps) -> Element {
    rsx! { div { class: "ds-card-footer", {props.children} } }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dioxus::prelude::VirtualDom;

    #[test]
    fn renders_children_inside_card_div() {
        fn app() -> Element {
            rsx! { Card { p { "hello" } } }
        }

        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);

        assert!(html.contains("ds-card"));
        assert!(html.contains("hello"));
    }

    #[test]
    fn card_renders_default_class() {
        fn app() -> Element {
            rsx! {
                Card { div { "body" } }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("ds-card"), "ds-card class missing: {html}");
    }

    #[test]
    fn card_accent_variant_renders_class() {
        fn app() -> Element {
            rsx! {
                Card {
                    variant: CardVariant::Accent,
                    div { "body" }
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("ds-card--accent"),
            "ds-card--accent class missing: {html}"
        );
    }

    #[test]
    fn card_clickable_renders_class_when_on_click_set() {
        fn app() -> Element {
            rsx! {
                Card {
                    on_click: |_| {},
                    div { "body" }
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("ds-card--clickable"),
            "ds-card--clickable class missing: {html}"
        );
    }

    #[test]
    fn card_premium_variant_renders_class() {
        fn app() -> Element {
            rsx! { Card { variant: CardVariant::Premium, "p" } }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("ds-card--premium"),
            "premium class missing: {html}"
        );
    }

    #[test]
    fn card_interactive_renders_class() {
        fn app() -> Element {
            rsx! { Card { interactive: true, "p" } }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("ds-card--interactive"),
            "interactive class missing: {html}"
        );
    }

    #[test]
    fn card_header_subcomponent_renders() {
        fn app() -> Element {
            rsx! { Card { CardHeader { CardTitle { "T" } CardDescription { "D" } } } }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("ds-card-header"));
        assert!(html.contains("ds-card-title"));
        assert!(html.contains("ds-card-description"));
    }
}
