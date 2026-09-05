use design_system::{
    Button, ButtonVariant, Card, Field, FormError, Input, Loading, PageHeader, PageHeaderVariant,
};
use dioxus::prelude::*;

#[component]
pub fn ForgotPassword() -> Element {
    let mut email = use_signal(String::new);
    let sent = use_signal(|| false);
    let error = use_signal(|| None::<String>);
    let submitting = use_signal(|| false);

    rsx! {
        div { class: "auth-screen auth-forgot-card motion-page",
            Card {
                PageHeader {
                    kicker: "AulaLite Academy".to_string(),
                    title: "Reset your password".to_string(),
                    subtitle: "We will send a reset link when your account can receive one.".to_string(),
                    variant: PageHeaderVariant::Hero,
                }
                if *sent.read() {
                    p {
                        class: "auth-success",
                        role: "status",
                        "aria-live": "polite",
                        "If an account exists for that email, a reset link has been sent."
                    }
                } else {
                    form {
                        class: "auth-form",
                        "aria-busy": if *submitting.read() { "true" } else { "false" },
                        onsubmit: move |event| {
                            event.prevent_default();
                            submit_password_reset(email, sent, error, submitting);
                        },
                        Field {
                            label: "Email".to_string(),
                            for_id: "forgot-email".to_string(),
                            Input {
                                id: "forgot-email".to_string(),
                                name: "email".to_string(),
                                value: email.read().clone(),
                                placeholder: "you@example.com".to_string(),
                                input_type: "email".to_string(),
                                autocomplete: "email".to_string(),
                                required: true,
                                disabled: *submitting.read(),
                                on_input: move |value| email.set(value),
                            }
                        }
                        FormError {
                            message: error.read().clone(),
                        }
                        div { class: "actions",
                            if *submitting.read() {
                                Loading { message: "Sending reset link…".to_string() }
                            } else {
                                Button {
                                    label: "Send reset link".to_string(),
                                    variant: ButtonVariant::Primary,
                                    button_type: "submit".to_string(),
                                    disabled: *submitting.read(),
                                    on_click: move |_| {},
                                }
                            }
                        }
                    }
                }
                nav { class: "auth-links",
                    a { class: "auth-link", href: "/login", "Back to sign in" }
                }
            }
        }
    }
}

fn submit_password_reset(
    email: Signal<String>,
    mut sent: Signal<bool>,
    mut error: Signal<Option<String>>,
    mut submitting: Signal<bool>,
) {
    let email_value = email.read().clone();

    submitting.set(true);
    error.set(None);

    #[cfg(target_arch = "wasm32")]
    {
        use platform_bridge::PlatformBridge;

        wasm_bindgen_futures::spawn_local(async move {
            let bridge = platform_bridge::web::WebBridge;
            match bridge.send_password_reset(&email_value).await {
                Ok(()) => sent.set(true),
                Err(err) => error.set(Some(format!("Could not send reset: {err}"))),
            }
            submitting.set(false);
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        use platform_bridge::PlatformBridge;

        dioxus::prelude::spawn(async move {
            let bridge = platform_bridge::native::NativeBridge;
            match bridge.send_password_reset(&email_value).await {
                Ok(()) => sent.set(true),
                Err(err) => error.set(Some(format!("Could not send reset: {err}"))),
            }
            submitting.set(false);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_form_has_browser_and_accessibility_semantics() {
        let mut vdom = VirtualDom::new(ForgotPassword);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("for=\"forgot-email\""), "{html}");
        assert!(html.contains("id=\"forgot-email\""), "{html}");
        assert!(html.contains("name=\"email\""), "{html}");
        assert!(html.contains("autocomplete=\"email\""), "{html}");
        assert!(html.contains("required"), "{html}");
        assert!(html.contains("aria-busy=\"false\""), "{html}");
    }
}
