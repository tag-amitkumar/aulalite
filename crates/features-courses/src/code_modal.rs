// crates/features-courses/src/code_modal.rs
use design_system::{Button, ButtonVariant, Card, Input, Modal, Spinner};
use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct CodeModalProps {
    pub open: bool,
    pub on_close: EventHandler<()>,
    pub on_generate: EventHandler<Option<i32>>,
    pub submitting: bool,
    pub generated_code: Option<String>,
    pub error: Option<String>,
}

#[component]
pub fn CodeModal(props: CodeModalProps) -> Element {
    let mut max_uses = use_signal(String::new);
    let on_generate = props.on_generate;

    rsx! {
        Modal {
            open: props.open,
            title: "Generate enrollment code".to_string(),
            on_close: props.on_close,
            if let Some(code) = &props.generated_code {
                Card {
                    div { class: "code-display",
                        p { "Share this code with students. It won't be shown again." }
                        code { class: "big-code", "{code}" }
                    }
                }
            } else {
                div { class: "code-form",
                    div { class: "field",
                        label { "Max uses (leave blank for unlimited)" }
                        Input {
                            value: max_uses.read().clone(),
                            placeholder: "e.g. 30".to_string(),
                            input_type: "number".to_string(),
                            disabled: props.submitting,
                            on_input: move |v| max_uses.set(v),
                        }
                    }
                    if let Some(e) = &props.error {
                        div { class: "form-error", "{e}" }
                    }
                    div { class: "actions",
                        if props.submitting {
                            Spinner {}
                        } else {
                            Button {
                                label: "Generate".to_string(),
                                variant: ButtonVariant::Primary,
                                on_click: move |_| {
                                    let parsed = max_uses.read().parse::<i32>().ok();
                                    on_generate.call(parsed);
                                },
                            }
                        }
                    }
                }
            }
        }
    }
}
