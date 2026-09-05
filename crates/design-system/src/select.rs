// crates/design-system/src/select.rs
use dioxus::prelude::*;

#[derive(Clone, PartialEq)]
pub struct SelectOption {
    pub value: String,
    pub label: String,
}

#[derive(Props, Clone, PartialEq)]
pub struct SelectProps {
    pub value: String,
    pub options: Vec<SelectOption>,
    pub on_change: EventHandler<String>,
    #[props(default = false)]
    pub disabled: bool,
    #[props(default)]
    pub error: bool,
    #[props(default)]
    pub name: Option<String>,
    #[props(default)]
    pub id: Option<String>,
    #[props(default)]
    pub aria_describedby: Option<String>,
}

#[component]
pub fn Select(props: SelectProps) -> Element {
    let field = try_consume_context::<crate::field::FieldContext>();
    let select_id = props
        .id
        .clone()
        .or_else(|| field.as_ref().map(|context| context.control_id.clone()));
    let described_by = props
        .aria_describedby
        .clone()
        .or_else(|| field.as_ref().map(|context| context.description_id.clone()));
    let class = if props.error {
        "ds-select ds-select--error"
    } else {
        "ds-select"
    };
    rsx! {
        select {
            class,
            value: "{props.value}",
            disabled: props.disabled,
            "aria-invalid": if props.error { "true" } else { "false" },
            "aria-describedby": described_by,
            name: props.name.clone(),
            id: select_id,
            onchange: move |evt| props.on_change.call(evt.value()),
            for opt in &props.options {
                option { value: "{opt.value}", "{opt.label}" }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_error_state_adds_error_class() {
        fn app() -> Element {
            rsx! {
                Select {
                    value: "".to_string(),
                    options: vec![SelectOption { value: "a".to_string(), label: "A".to_string() }],
                    error: true,
                    on_change: |_| {},
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("ds-select--error") || html.contains("ds-input--error"),
            "error class missing: {html}"
        );
    }
}
