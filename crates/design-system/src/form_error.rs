use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct FormErrorProps {
    pub message: Option<String>,
}

#[component]
pub fn FormError(props: FormErrorProps) -> Element {
    match &props.message {
        Some(message) => rsx! {
            div { class: "ds-form-error", role: "alert", "{message}" }
        },
        None => rsx! {},
    }
}

#[cfg(test)]
mod tests {
    use super::FormError;
    use dioxus::prelude::*;

    #[test]
    fn renders_when_message_present() {
        fn app() -> Element {
            rsx! { FormError { message: Some("bad email".to_string()) } }
        }

        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);

        assert!(html.contains("bad email"));
    }

    #[test]
    fn renders_nothing_when_message_absent() {
        fn app() -> Element {
            rsx! { FormError { message: None } }
        }

        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);

        assert!(!html.contains("ds-form-error"));
    }
}
