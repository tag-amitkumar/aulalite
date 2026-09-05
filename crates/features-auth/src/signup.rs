use design_system::{
    Button, ButtonVariant, Card, Field, FormError, Input, Loading, PageHeader, PageHeaderVariant,
};
use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct SignupProps {
    pub on_success: EventHandler<String>,
}

#[component]
pub fn Signup(props: SignupProps) -> Element {
    let mut email = use_signal(String::new);
    let mut password = use_signal(String::new);
    let mut confirm = use_signal(String::new);
    let error = use_signal(|| None::<String>);
    let submitting = use_signal(|| false);
    let form_success = props.on_success;

    rsx! {
        div { class: "auth-screen auth-signup-card motion-page",
            Card {
                PageHeader {
                    kicker: "AulaLite Academy".to_string(),
                    title: "Create your AulaLite account".to_string(),
                    subtitle: "Join your courses and live sessions with a secure academy profile.".to_string(),
                    variant: PageHeaderVariant::Hero,
                }
                form {
                    class: "auth-form",
                    "aria-busy": if *submitting.read() { "true" } else { "false" },
                    onsubmit: move |event| {
                        event.prevent_default();
                        submit_signup(
                            email,
                            password,
                            confirm,
                            error,
                            submitting,
                            form_success,
                        );
                    },
                    Field {
                        label: "Email".to_string(),
                        for_id: "signup-email".to_string(),
                        Input {
                            id: "signup-email".to_string(),
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
                    Field {
                        label: "Password".to_string(),
                        for_id: "signup-password".to_string(),
                        helper: "Use at least 8 characters.".to_string(),
                        Input {
                            id: "signup-password".to_string(),
                            name: "password".to_string(),
                            value: password.read().clone(),
                            placeholder: "At least 8 characters".to_string(),
                            input_type: "password".to_string(),
                            autocomplete: "new-password".to_string(),
                            required: true,
                            disabled: *submitting.read(),
                            on_input: move |value| password.set(value),
                        }
                    }
                    Field {
                        label: "Confirm password".to_string(),
                        for_id: "signup-password-confirmation".to_string(),
                        Input {
                            id: "signup-password-confirmation".to_string(),
                            name: "password_confirmation".to_string(),
                            value: confirm.read().clone(),
                            placeholder: "Repeat your password".to_string(),
                            input_type: "password".to_string(),
                            autocomplete: "new-password".to_string(),
                            required: true,
                            disabled: *submitting.read(),
                            on_input: move |value| confirm.set(value),
                        }
                    }
                    FormError {
                        message: error.read().clone(),
                    }
                    p { class: "auth-legal-note",
                        "By creating an account, you agree to the "
                        a { href: "/terms", "Terms of Service" }
                        " and acknowledge the "
                        a { href: "/privacy", "Privacy Policy" }
                        "."
                    }
                    div { class: "actions",
                        if *submitting.read() {
                            Loading { message: "Check your email to verify your account, then keep this page open…".to_string() }
                        } else {
                            Button {
                                label: "Create account".to_string(),
                                variant: ButtonVariant::Primary,
                                button_type: "submit".to_string(),
                                disabled: *submitting.read(),
                                on_click: move |_| {},
                            }
                        }
                    }
                }
                nav { class: "auth-links",
                    a { class: "auth-link", href: "/login", "Already have an account? Sign in" }
                }
            }
        }
    }
}

fn submit_signup(
    email: Signal<String>,
    password: Signal<String>,
    confirm: Signal<String>,
    mut error: Signal<Option<String>>,
    mut submitting: Signal<bool>,
    on_success: EventHandler<String>,
) {
    let email_value = email.read().clone();
    let password_value = password.read().clone();
    let confirm_value = confirm.read().clone();

    if password_value != confirm_value {
        error.set(Some("Passwords do not match".into()));
        return;
    }

    if password_value.len() < 8 {
        error.set(Some("Password must be at least 8 characters".into()));
        return;
    }

    submitting.set(true);
    error.set(None);

    #[cfg(target_arch = "wasm32")]
    {
        use platform_bridge::PlatformBridge;

        wasm_bindgen_futures::spawn_local(async move {
            let bridge = platform_bridge::web::WebBridge;
            match bridge
                .sign_up_email_password(&email_value, &password_value)
                .await
            {
                Ok(token) => on_success.call(token),
                Err(err) => error.set(Some(format!("Sign up failed: {err}"))),
            }
            submitting.set(false);
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        use platform_bridge::PlatformBridge;

        dioxus::prelude::spawn(async move {
            let bridge = platform_bridge::native::NativeBridge;
            match bridge
                .sign_up_email_password(&email_value, &password_value)
                .await
            {
                Ok(token) => on_success.call(token),
                Err(err) => error.set(Some(format!("Sign up failed: {err}"))),
            }
            submitting.set(false);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signup_fields_are_labelled_and_discloses_legal_terms() {
        fn app() -> Element {
            rsx! { Signup { on_success: |_| {} } }
        }

        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        for id in [
            "signup-email",
            "signup-password",
            "signup-password-confirmation",
        ] {
            assert!(html.contains(&format!("for=\"{id}\"")), "{html}");
            assert!(html.contains(&format!("id=\"{id}\"")), "{html}");
        }
        assert!(html.contains("href=\"/terms\""), "{html}");
        assert!(html.contains("href=\"/privacy\""), "{html}");
        assert!(html.contains("aria-busy=\"false\""), "{html}");
        assert!(html.contains("autocomplete=\"email\""), "{html}");
        assert!(html.contains("autocomplete=\"new-password\""), "{html}");
    }
}
