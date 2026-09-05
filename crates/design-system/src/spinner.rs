use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct SpinnerProps {
    #[props(default)]
    pub size: SpinnerSize,
    #[props(default)]
    pub aria_label: Option<String>,
}

#[derive(Clone, PartialEq, Default)]
pub enum SpinnerSize {
    Xs,
    Sm,
    #[default]
    Md,
    Lg,
}

#[component]
pub fn Spinner(props: SpinnerProps) -> Element {
    let size_class = match props.size {
        SpinnerSize::Xs => "ds-spinner ds-spinner--xs",
        SpinnerSize::Sm => "ds-spinner ds-spinner--sm",
        SpinnerSize::Md => "ds-spinner ds-spinner--md",
        SpinnerSize::Lg => "ds-spinner ds-spinner--lg",
    };
    let aria = props.aria_label.unwrap_or_else(|| "Loading".to_string());
    rsx! {
        span {
            class: "{size_class}",
            role: "status",
            "aria-label": "{aria}",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_with_role_status() {
        fn app() -> Element {
            rsx! { Spinner {} }
        }

        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);

        assert!(html.contains("role=\"status\""));
    }

    #[test]
    fn spinner_default_is_md() {
        fn app() -> Element {
            rsx! { Spinner {} }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("ds-spinner--md"),
            "default md class missing: {html}"
        );
    }

    #[test]
    fn spinner_sm_renders_class() {
        fn app() -> Element {
            rsx! { Spinner { size: SpinnerSize::Sm } }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("ds-spinner--sm"), "sm class missing: {html}");
    }
}
