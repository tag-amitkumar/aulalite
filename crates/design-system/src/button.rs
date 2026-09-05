use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct ButtonProps {
    pub label: String,
    #[props(default)]
    pub disabled: bool,
    #[props(default)]
    pub variant: ButtonVariant,
    #[props(default)]
    pub size: ButtonSize,
    /// Shows an inline spinner and suppresses the click handler.
    #[props(default)]
    pub loading: bool,
    pub on_click: EventHandler<MouseEvent>,
    /// Optional icon glyph rendered before the label.
    #[props(default)]
    pub leading_icon: Option<Element>,
    /// Optional icon glyph rendered after the label.
    #[props(default)]
    pub trailing_icon: Option<Element>,
    /// `type` attribute. Defaults to "button"; pass "submit" inside forms.
    #[props(default = "button".to_string())]
    pub button_type: String,
}

#[derive(Clone, PartialEq, Default)]
pub enum ButtonVariant {
    #[default]
    Primary,
    Secondary,
    /// Alias kept for back-compat; identical to `Destructive`.
    Danger,
    /// Oxblood destructive button.
    Destructive,
    Ghost,
    Link,
    /// Gold premium CTA (paywall, upgrade prompts).
    Premium,
}

#[derive(Clone, PartialEq, Default)]
pub enum ButtonSize {
    Sm,
    #[default]
    Md,
    Lg,
    /// Square icon-only button (36×36).
    Icon,
}

#[component]
pub fn Button(props: ButtonProps) -> Element {
    let variant_class = match props.variant {
        ButtonVariant::Primary => "ds-button--primary",
        ButtonVariant::Secondary => "ds-button--secondary",
        // Danger and Destructive share styling; Danger is the legacy alias.
        ButtonVariant::Danger | ButtonVariant::Destructive => "ds-button--destructive",
        ButtonVariant::Ghost => "ds-button--ghost",
        ButtonVariant::Link => "ds-button--link",
        ButtonVariant::Premium => "ds-button--premium",
    };
    let size_class = match props.size {
        ButtonSize::Sm => "ds-button--sm",
        ButtonSize::Md => "ds-button--md",
        ButtonSize::Lg => "ds-button--lg",
        ButtonSize::Icon => "ds-button--icon",
    };
    let loading_class = if props.loading {
        " ds-button--loading"
    } else {
        ""
    };
    let class = format!("ds-button {variant_class} {size_class}{loading_class}");

    rsx! {
        button {
            class: "{class}",
            r#type: "{props.button_type}",
            disabled: props.disabled || props.loading,
            "aria-busy": if props.loading { "true" } else { "false" },
            onclick: move |event| {
                if !props.loading {
                    props.on_click.call(event);
                }
            },
            if props.loading {
                span { class: "ds-button-spinner", "aria-hidden": "true" }
            } else if let Some(icon) = &props.leading_icon {
                span { class: "ds-button-icon ds-button-icon--leading", {icon.clone()} }
            }
            "{props.label}"
            if !props.loading {
                if let Some(icon) = &props.trailing_icon {
                    span { class: "ds-button-icon ds-button-icon--trailing", {icon.clone()} }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Button, ButtonSize, ButtonVariant};
    use dioxus::prelude::*;

    #[test]
    fn renders_primary_label() {
        fn app() -> Element {
            rsx! {
                Button {
                    label: "Sign in".to_string(),
                    on_click: |_| {},
                }
            }
        }

        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);

        assert!(html.contains("Sign in"));
        assert!(html.contains("ds-button--primary"));
    }

    #[test]
    fn renders_disabled() {
        fn app() -> Element {
            rsx! {
                Button {
                    label: "X".to_string(),
                    disabled: true,
                    on_click: |_| {},
                }
            }
        }

        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);

        assert!(html.contains("disabled"));
    }

    #[test]
    fn ghost_variant_renders_class() {
        fn app() -> Element {
            rsx! {
                Button { label: "G".to_string(), variant: ButtonVariant::Ghost, on_click: |_| {} }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("ds-button--ghost"),
            "ghost class missing: {html}"
        );
    }

    #[test]
    fn link_variant_renders_class() {
        fn app() -> Element {
            rsx! {
                Button { label: "L".to_string(), variant: ButtonVariant::Link, on_click: |_| {} }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("ds-button--link"),
            "link class missing: {html}"
        );
    }

    #[test]
    fn button_premium_variant_renders_class() {
        fn app() -> Element {
            rsx! { Button { label: "Go Pro".to_string(), variant: ButtonVariant::Premium, on_click: |_| {} } }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("ds-button--premium"),
            "premium class missing: {html}"
        );
    }

    #[test]
    fn button_size_sm_renders_class() {
        fn app() -> Element {
            rsx! { Button { label: "x".to_string(), size: ButtonSize::Sm, on_click: |_| {} } }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("ds-button--sm"), "sm class missing: {html}");
    }

    #[test]
    fn button_loading_renders_spinner_marker() {
        fn app() -> Element {
            rsx! { Button { label: "Save".to_string(), loading: true, on_click: |_| {} } }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("ds-button--loading"),
            "loading class missing: {html}"
        );
        assert!(
            html.contains("ds-button-spinner"),
            "spinner marker missing: {html}"
        );
    }

    #[test]
    fn button_trailing_icon_renders() {
        fn app() -> Element {
            rsx! {
                Button {
                    label: "Next".to_string(),
                    trailing_icon: Some(rsx! { span { "→" } }),
                    on_click: |_| {},
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("ds-button-icon--trailing"),
            "trailing icon wrapper missing: {html}"
        );
        assert!(html.contains("→"));
    }
}
