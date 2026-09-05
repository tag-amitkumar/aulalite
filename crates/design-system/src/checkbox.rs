// crates/design-system/src/checkbox.rs
use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct CheckboxProps {
    pub checked: bool,
    pub label: String,
    pub on_change: EventHandler<bool>,
    #[props(default = false)]
    pub disabled: bool,
    #[props(default)]
    pub indeterminate: bool,
    #[props(default)]
    pub error: bool,
}

#[component]
pub fn Checkbox(props: CheckboxProps) -> Element {
    let on_change = props.on_change;
    let class = if props.error {
        "ds-checkbox ds-checkbox--error"
    } else {
        "ds-checkbox"
    };
    rsx! {
        label { class: "checkbox",
            input {
                class,
                r#type: "checkbox",
                checked: props.checked,
                disabled: props.disabled,
                "data-indeterminate": "{props.indeterminate}",
                onchange: move |evt| {
                    let v = evt.value() == "true" || evt.checked();
                    on_change.call(v);
                },
            }
            span { "{props.label}" }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct RadioProps {
    pub checked: bool,
    pub name: String,
    pub value: String,
    #[props(default)]
    pub disabled: bool,
    pub on_change: EventHandler<String>,
}

#[component]
pub fn Radio(props: RadioProps) -> Element {
    rsx! {
        input {
            class: "ds-radio",
            r#type: "radio",
            name: "{props.name}",
            value: "{props.value}",
            checked: props.checked,
            disabled: props.disabled,
            onchange: {
                let value = props.value.clone();
                let on_change = props.on_change;
                move |_| on_change.call(value.clone())
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkbox_renders_default() {
        fn app() -> Element {
            rsx! {
                Checkbox {
                    checked: false,
                    label: "Accept".to_string(),
                    on_change: |_| {},
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("ds-checkbox"),
            "checkbox class missing: {html}"
        );
    }

    #[test]
    fn radio_renders_with_name_and_value() {
        fn app() -> Element {
            rsx! {
                Radio {
                    checked: true,
                    name: "fruit".to_string(),
                    value: "apple".to_string(),
                    on_change: |_| {},
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("name=\"fruit\""), "name missing: {html}");
        assert!(html.contains("value=\"apple\""), "value missing: {html}");
        assert!(html.contains("ds-radio"), "ds-radio class missing: {html}");
    }
}
