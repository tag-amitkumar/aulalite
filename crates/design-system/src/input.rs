use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct InputProps {
    pub value: String,
    #[props(default)]
    pub placeholder: String,
    #[props(default = "text".to_string())]
    pub input_type: String,
    #[props(default)]
    pub disabled: bool,
    #[props(default)]
    pub error: bool,
    #[props(default)]
    pub name: Option<String>,
    #[props(default)]
    pub id: Option<String>,
    /// Id(s) of helper or error text describing the input.
    #[props(default)]
    pub aria_describedby: Option<String>,
    #[props(default)]
    pub autocomplete: Option<String>,
    #[props(default)]
    pub required: bool,
    /// Icon rendered absolutely inside the input on the left edge.
    #[props(default)]
    pub leading_icon: Option<Element>,
    /// Icon rendered absolutely inside the input on the right edge.
    #[props(default)]
    pub trailing_icon: Option<Element>,
    /// Static prefix addon outside the input (e.g. "$").
    #[props(default)]
    pub addon_left: Option<String>,
    /// Static suffix addon outside the input (e.g. "kg").
    #[props(default)]
    pub addon_right: Option<String>,
    pub on_input: EventHandler<String>,
}

#[component]
pub fn Input(props: InputProps) -> Element {
    let field = try_consume_context::<crate::field::FieldContext>();
    let input_id = props
        .id
        .clone()
        .or_else(|| field.as_ref().map(|context| context.control_id.clone()));
    let described_by = props
        .aria_describedby
        .clone()
        .or_else(|| field.as_ref().map(|context| context.description_id.clone()));
    let has_leading = props.leading_icon.is_some();
    let has_trailing = props.trailing_icon.is_some();
    let has_addon_left = props.addon_left.is_some();
    let has_addon_right = props.addon_right.is_some();
    let has_slot = has_leading || has_trailing || has_addon_left || has_addon_right;

    let mut input_class = String::from("ds-input");
    if props.error {
        input_class.push_str(" ds-input--error");
    }
    if has_leading {
        input_class.push_str(" ds-input--has-leading");
    }
    if has_trailing {
        input_class.push_str(" ds-input--has-trailing");
    }

    let raw_input = rsx! {
        input {
            class: "{input_class}",
            r#type: "{props.input_type}",
            value: "{props.value}",
            placeholder: "{props.placeholder}",
            disabled: props.disabled,
            required: props.required,
            "aria-invalid": if props.error { "true" } else { "false" },
            "aria-describedby": described_by,
            autocomplete: props.autocomplete.clone(),
            name: props.name.clone(),
            id: input_id,
            oninput: move |event| props.on_input.call(event.value()),
        }
    };

    if !has_slot {
        return raw_input;
    }

    rsx! {
        div { class: "ds-input-wrap",
            if let Some(text) = &props.addon_left {
                span { class: "ds-input-addon ds-input-addon--left", "{text}" }
            }
            div { class: "ds-input-shell",
                if let Some(icon) = &props.leading_icon {
                    span { class: "ds-input-slot ds-input-slot--leading", {icon.clone()} }
                }
                {raw_input}
                if let Some(icon) = &props.trailing_icon {
                    span { class: "ds-input-slot ds-input-slot--trailing", {icon.clone()} }
                }
            }
            if let Some(text) = &props.addon_right {
                span { class: "ds-input-addon ds-input-addon--right", "{text}" }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Input;
    use dioxus::prelude::*;

    #[test]
    fn renders_with_placeholder() {
        fn app() -> Element {
            rsx! {
                Input {
                    value: "".to_string(),
                    placeholder: "Email".to_string(),
                    on_input: |_| {},
                }
            }
        }

        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);

        assert!(html.contains("placeholder=\"Email\""));
        assert!(html.contains("ds-input"));
        assert!(
            !html.contains("id=\"\""),
            "empty id should be omitted: {html}"
        );
        assert!(
            !html.contains("name=\"\""),
            "empty name should be omitted: {html}"
        );
    }

    #[test]
    fn input_error_state_adds_error_class() {
        fn app() -> Element {
            rsx! {
                Input {
                    value: "".to_string(),
                    placeholder: "Email".to_string(),
                    error: true,
                    on_input: |_| {},
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("ds-input--error"),
            "error class missing: {html}"
        );
    }

    #[test]
    fn input_leading_icon_renders() {
        fn app() -> Element {
            rsx! {
                Input {
                    value: "".to_string(),
                    leading_icon: Some(rsx! { span { class: "i", "🔍" } }),
                    on_input: |_| {},
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("ds-input-slot--leading"),
            "leading slot missing: {html}"
        );
    }

    #[test]
    fn input_addon_left_renders() {
        fn app() -> Element {
            rsx! {
                Input {
                    value: "".to_string(),
                    addon_left: Some("$".to_string()),
                    on_input: |_| {},
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("ds-input-addon--left"),
            "addon left missing: {html}"
        );
        assert!(html.contains("$"));
    }

    #[test]
    fn input_renders_wrapped_when_slot_present() {
        fn app() -> Element {
            rsx! {
                Input {
                    value: "".to_string(),
                    leading_icon: Some(rsx! { span { "x" } }),
                    on_input: |_| {},
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("ds-input-wrap"), "wrap missing: {html}");
    }
}
