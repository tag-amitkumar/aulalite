// crates/features-courses/src/redeem_code.rs
use design_system::{
    Button, ButtonVariant, Card, Field, FormError, Input, Loading, PageHeader, PageHeaderVariant,
};
use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct RedeemCodeProps {
    pub on_submit: EventHandler<String>,
    pub submitting: bool,
    pub error: Option<String>,
}

#[component]
pub fn RedeemCode(props: RedeemCodeProps) -> Element {
    let mut code = use_signal(String::new);
    let on_submit_form = props.on_submit;

    rsx! {
        div { class: "redeem-page workflow-page motion-page",
            Card {
                PageHeader {
                    kicker: "Enrollment".to_string(),
                    title: "Redeem an enrollment code".to_string(),
                    subtitle: "Paste the code your teacher sent you.".to_string(),
                    variant: PageHeaderVariant::Hero,
                }
                form {
                    onsubmit: move |e| {
                        e.prevent_default();
                        on_submit_form.call(code.read().clone());
                    },
                    Field {
                        label: "Enrollment code".to_string(),
                        Input {
                            value: code.read().clone(),
                            placeholder: "ABCD2345".to_string(),
                            input_type: "text".to_string(),
                            disabled: props.submitting,
                            on_input: move |v: String| code.set(v.to_uppercase()),
                        }
                    }
                    FormError { message: props.error.clone() }
                    div { class: "actions",
                        if props.submitting {
                            Loading { message: "Redeeming…".to_string() }
                        } else {
                            Button {
                                label: "Redeem".to_string(),
                                variant: ButtonVariant::Primary,
                                button_type: "submit".to_string(),
                                disabled: props.submitting,
                                on_click: move |_| {},
                            }
                        }
                    }
                }
            }
        }
    }
}
