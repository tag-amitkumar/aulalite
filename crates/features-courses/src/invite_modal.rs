// crates/features-courses/src/invite_modal.rs
use design_system::{Button, ButtonVariant, Field, Input, Modal, Select, SelectOption, Spinner};
use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct InviteModalProps {
    pub open: bool,
    pub on_close: EventHandler<()>,
    pub on_send: EventHandler<(String, String)>,
    pub submitting: bool,
    pub error: Option<String>,
}

#[component]
pub fn InviteModal(props: InviteModalProps) -> Element {
    let mut email = use_signal(String::new);
    let mut role = use_signal(|| "student".to_string());
    let on_send = props.on_send;

    rsx! {
        Modal {
            open: props.open,
            title: "Invite by email".to_string(),
            on_close: props.on_close,
            div { class: "invite-form",
                Field {
                    label: "Email".to_string(),
                    for_id: "course-invite-email".to_string(),
                    Input {
                        id: "course-invite-email".to_string(),
                        name: "email".to_string(),
                        value: email.read().clone(),
                        placeholder: "name@example.com".to_string(),
                        input_type: "email".to_string(),
                        autocomplete: "email".to_string(),
                        required: true,
                        disabled: props.submitting,
                        on_input: move |v| email.set(v),
                    }
                }
                Field {
                    label: "Role".to_string(),
                    for_id: "course-invite-role".to_string(),
                    Select {
                        id: "course-invite-role".to_string(),
                        name: "role".to_string(),
                        value: role.read().clone(),
                        options: vec![
                            SelectOption { value: "student".to_string(), label: "Student".to_string() },
                            SelectOption { value: "ta".to_string(), label: "TA".to_string() },
                            SelectOption { value: "teacher".to_string(), label: "Teacher".to_string() },
                        ],
                        on_change: move |v| role.set(v),
                    }
                }
                if let Some(e) = &props.error {
                    div { class: "form-error", role: "alert", "aria-live": "assertive", "{e}" }
                }
                div { class: "actions",
                    if props.submitting {
                        Spinner {}
                    } else {
                        Button {
                            label: "Send invite".to_string(),
                            variant: ButtonVariant::Primary,
                            on_click: move |_| {
                                on_send.call((email.read().clone(), role.read().clone()));
                            },
                        }
                    }
                }
            }
        }
    }
}
