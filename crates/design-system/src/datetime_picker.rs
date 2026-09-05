// crates/design-system/src/datetime_picker.rs
use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct DateTimePickerProps {
    pub value: String,
    pub on_change: EventHandler<String>,
    #[props(default = false)]
    pub disabled: bool,
    #[props(default)]
    pub id: Option<String>,
    #[props(default)]
    pub name: Option<String>,
    #[props(default)]
    pub aria_describedby: Option<String>,
}

#[component]
pub fn DateTimePicker(props: DateTimePickerProps) -> Element {
    let field = try_consume_context::<crate::field::FieldContext>();
    let input_id = props
        .id
        .clone()
        .or_else(|| field.as_ref().map(|context| context.control_id.clone()));
    let described_by = props
        .aria_describedby
        .clone()
        .or_else(|| field.as_ref().map(|context| context.description_id.clone()));
    let h = props.on_change;
    rsx! {
        input {
            id: input_id,
            name: props.name.clone(),
            r#type: "datetime-local",
            class: "ds-datetime",
            "aria-describedby": described_by,
            value: "{props.value}",
            disabled: props.disabled,
            oninput: move |evt| h.call(evt.value()),
        }
    }
}
