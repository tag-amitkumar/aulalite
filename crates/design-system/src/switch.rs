// crates/design-system/src/switch.rs
//! Switch primitive (semantic upgrade of the legacy `Toggle`).
//!
//! `Toggle` remains exported from `lib.rs` as `pub use Switch as Toggle;` so
//! existing call sites continue to compile without code changes. The props
//! intentionally mirror `ToggleProps` (`checked`, `label`, `on_change`) and
//! add optional `disabled` state for full state coverage.

use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct SwitchProps {
    pub checked: bool,
    #[props(default)]
    pub disabled: bool,
    /// Optional descriptive label rendered next to the track. An empty
    /// string (the default) hides the label entirely. Kept as `String` —
    /// not `Option<String>` — so legacy `Toggle` callers that pass
    /// `label: "..".to_string()` continue to compile.
    #[props(default)]
    pub label: String,
    #[props(default)]
    pub id: Option<String>,
    #[props(default)]
    pub name: Option<String>,
    #[props(default)]
    pub aria_describedby: Option<String>,
    pub on_change: EventHandler<bool>,
}

#[component]
pub fn Switch(props: SwitchProps) -> Element {
    let field = try_consume_context::<crate::field::FieldContext>();
    let input_id = props
        .id
        .clone()
        .or_else(|| field.as_ref().map(|context| context.control_id.clone()));
    let described_by = props
        .aria_describedby
        .clone()
        .or_else(|| field.as_ref().map(|context| context.description_id.clone()));
    let on_change = props.on_change;
    let label = props.label.clone();
    rsx! {
        label { class: "ds-switch",
            input {
                id: input_id,
                name: props.name.clone(),
                r#type: "checkbox",
                role: "switch",
                class: "ds-switch-input",
                "aria-describedby": described_by,
                checked: props.checked,
                disabled: props.disabled,
                onchange: move |evt| on_change.call(evt.checked()),
            }
            span { class: "ds-switch-track",
                span { class: "ds-switch-thumb" }
            }
            if !label.is_empty() {
                span { class: "ds-switch-label", "{label}" }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switch_renders_checked_state() {
        fn app() -> Element {
            rsx! {
                Switch {
                    checked: true,
                    on_change: |_| {},
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("ds-switch"),
            "ds-switch class missing: {html}"
        );
        assert!(
            html.contains("checked"),
            "checked attribute missing: {html}"
        );
    }

    #[test]
    fn switch_renders_with_label() {
        fn app() -> Element {
            rsx! {
                Switch {
                    checked: false,
                    label: "Notifications".to_string(),
                    on_change: |_| {},
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("Notifications"), "label text missing: {html}");
        assert!(
            html.contains("ds-switch-label"),
            "ds-switch-label class missing: {html}"
        );
    }

    #[test]
    fn switch_omits_label_when_empty() {
        fn app() -> Element {
            rsx! {
                Switch {
                    checked: false,
                    on_change: |_| {},
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            !html.contains("ds-switch-label"),
            "ds-switch-label should be omitted when label is empty: {html}"
        );
    }
}
