use crate::spinner::{Spinner, SpinnerSize};
use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct LoadingProps {
    #[props(default = "Loading…".to_string())]
    pub message: String,
    #[props(default)]
    pub size: SpinnerSize,
    #[props(default)]
    pub layout: LoadingLayout,
}

#[derive(Clone, PartialEq, Default)]
pub enum LoadingLayout {
    #[default]
    Inline,
    Block,
}

#[component]
pub fn Loading(props: LoadingProps) -> Element {
    let layout_class = match props.layout {
        LoadingLayout::Inline => "ds-loading ds-loading--inline",
        LoadingLayout::Block => "ds-loading ds-loading--block",
    };
    rsx! {
        div { class: "{layout_class}", role: "status",
            Spinner { size: props.size, aria_label: None::<String> }
            span { class: "ds-loading-text", "{props.message}" }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loading_default_renders_inline_with_message() {
        fn app() -> Element {
            rsx! { Loading {} }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("ds-loading--inline"));
        assert!(html.contains("Loading"));
        assert!(html.contains("role=\"status\""));
        assert!(html.contains("ds-spinner"));
    }

    #[test]
    fn loading_block_layout_renders_class() {
        fn app() -> Element {
            rsx! { Loading { layout: LoadingLayout::Block, message: "Working".to_string() } }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("ds-loading--block"));
        assert!(html.contains("Working"));
    }
}
