#[allow(unused_imports)]
use design_system::{
    use_toast_sender, Button, ButtonVariant, Card, Field, FormError, Input, Loading, PageHeader,
    PageHeaderVariant, ToastLevel, ToastSender,
};
use dioxus::prelude::*;
#[allow(unused_imports)]
use features_courses::api::{self, ApiContext};

pub mod login_internals {
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum LocalAttempt {
        Ok(String),
        Err(String),
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum LoginOutcome {
        Token {
            token: String,
            persist_local_token: bool,
        },
        Err(String),
    }

    pub fn decide_outcome(
        local: LocalAttempt,
        firebase: Option<Result<String, String>>,
    ) -> LoginOutcome {
        match local {
            LocalAttempt::Ok(token) => LoginOutcome::Token {
                token,
                persist_local_token: true,
            },
            LocalAttempt::Err(_) => match firebase {
                Some(Ok(token)) => LoginOutcome::Token {
                    token,
                    persist_local_token: false,
                },
                Some(Err(msg)) => LoginOutcome::Err(format!("Sign in failed: {msg}")),
                None => LoginOutcome::Err("Sign in failed".into()),
            },
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum MeCheckOutcome {
        Ok,
        MfaRequired,
        Failed(String),
    }

    pub fn classify_me_error(err: &features_courses::api::ApiError) -> MeCheckOutcome {
        match err {
            features_courses::api::ApiError::Status(401, body) if body.contains("mfa_required") => {
                MeCheckOutcome::MfaRequired
            }
            other => MeCheckOutcome::Failed(format!("{other}")),
        }
    }

    pub fn trusted_device_error_should_fall_back(err: &features_courses::api::ApiError) -> bool {
        match err {
            features_courses::api::ApiError::Status(_, body) => {
                body.contains("trusted_device_invalid")
                    || body.contains("trusted_device_expired")
                    || body.contains("trusted_device_revoked")
            }
            _ => false,
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct LoginProps {
    pub on_success: EventHandler<String>,
    #[props(default = "AulaLite Academy".to_string())]
    pub kicker: String,
    #[props(default = "Sign in to the academy".to_string())]
    pub title: String,
    #[props(default = "Live classes, assignments, schedules, and course operations in one polished workspace.".to_string())]
    pub subtitle: String,
    #[props(default = "Create an account".to_string())]
    pub signup_label: String,
}

#[component]
pub fn Login(props: LoginProps) -> Element {
    let mut email = use_signal(String::new);
    let mut password = use_signal(String::new);
    let error = use_signal(|| None::<String>);
    let submitting = use_signal(|| false);
    let primary_token = use_signal(|| None::<String>);
    let primary_token_is_local = use_signal(|| false);
    let mut mfa_code = use_signal(String::new);
    let mut remember_device = use_signal(|| true);
    let mfa_required = use_signal(|| false);
    let form_success = props.on_success;
    let toast = use_toast_sender();

    // Read the ApiContext from the Signal at component-render time.
    // Hooks (use_api) must run during render, not inside async blocks.
    #[cfg(target_arch = "wasm32")]
    let api_ctx = api::use_api();
    #[cfg(not(target_arch = "wasm32"))]
    let api_ctx = ApiContext {
        base_url: api::native_api_base_url(),
        id_token: String::new(),
    };

    let mfa_panel: Element = if *mfa_required.read() {
        rsx! {
            div { class: "auth-mfa-panel",
                Field {
                    label: "Authentication code".to_string(),
                    for_id: Some("login-mfa-code".to_string()),
                    Input {
                        id: Some("login-mfa-code".to_string()),
                        name: Some("one-time-code".to_string()),
                        value: mfa_code.read().clone(),
                        placeholder: "123456 or recovery code".to_string(),
                        input_type: "text".to_string(),
                        autocomplete: Some("one-time-code".to_string()),
                        required: true,
                        disabled: *submitting.read(),
                        error: error.read().is_some(),
                        on_input: move |value| mfa_code.set(value),
                    }
                }
                label { class: "auth-mfa-remember",
                    input {
                        r#type: "checkbox",
                        checked: *remember_device.read(),
                        disabled: *submitting.read(),
                        onchange: move |event| remember_device.set(event.checked()),
                    }
                    span { "Remember this device for 30 days" }
                }
                Button {
                    label: "Verify".to_string(),
                    variant: ButtonVariant::Primary,
                    button_type: "button".to_string(),
                    disabled: *submitting.read(),
                    on_click: {
                        let api_ctx = api_ctx.clone();
                        let form_success = form_success;
                        move |_| {
                            submit_mfa_challenge(
                                email,
                                mfa_code,
                                remember_device,
                                primary_token,
                                primary_token_is_local,
                                error,
                                submitting,
                                form_success,
                                api_ctx.clone(),
                                toast,
                            );
                        }
                    },
                }
            }
        }
    } else {
        rsx! {}
    };

    rsx! {
        div { class: "auth-screen auth-login-card",
            Card {
                PageHeader {
                    kicker: props.kicker.clone(),
                    title: props.title.clone(),
                    subtitle: props.subtitle.clone(),
                    variant: PageHeaderVariant::Hero,
                }
                form {
                    class: "auth-form",
                    "aria-busy": if *submitting.read() { "true" } else { "false" },
                    onsubmit: {
                        let api_ctx = api_ctx.clone();
                        move |event| {
                            event.prevent_default();
                            if *mfa_required.read() {
                                submit_mfa_challenge(
                                    email,
                                    mfa_code,
                                    remember_device,
                                    primary_token,
                                    primary_token_is_local,
                                    error,
                                    submitting,
                                    form_success,
                                    api_ctx.clone(),
                                    toast,
                                );
                            } else {
                                submit_login(
                                    email,
                                    password,
                                    primary_token,
                                    primary_token_is_local,
                                    mfa_required,
                                    error,
                                    submitting,
                                    form_success,
                                    api_ctx.clone(),
                                    toast,
                                );
                            }
                        }
                    },
                    Field {
                        label: "Email".to_string(),
                        for_id: Some("login-email".to_string()),
                        Input {
                            id: Some("login-email".to_string()),
                            name: Some("email".to_string()),
                            value: email.read().clone(),
                            placeholder: "you@example.com".to_string(),
                            input_type: "email".to_string(),
                            autocomplete: Some("email".to_string()),
                            required: true,
                            disabled: *submitting.read(),
                            error: error.read().is_some(),
                            on_input: move |value| email.set(value),
                        }
                    }
                    Field {
                        label: "Password".to_string(),
                        for_id: Some("login-password".to_string()),
                        Input {
                            id: Some("login-password".to_string()),
                            name: Some("password".to_string()),
                            value: password.read().clone(),
                            placeholder: "Your password".to_string(),
                            input_type: "password".to_string(),
                            autocomplete: Some("current-password".to_string()),
                            required: true,
                            disabled: *submitting.read(),
                            error: error.read().is_some(),
                            on_input: move |value| password.set(value),
                        }
                    }
                    FormError { message: error.read().clone() }
                    {mfa_panel}
                    if !*mfa_required.read() {
                        div { class: "actions",
                            if *submitting.read() {
                                Loading { message: "Signing in…".to_string() }
                            } else {
                                Button {
                                    label: "Sign in".to_string(),
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
                    a { class: "auth-link", href: "/signup", "{props.signup_label}" }
                    a { class: "auth-link", href: "/forgot", "Forgot your password?" }
                }
            }
        }
    }
}

fn submit_login(
    email: Signal<String>,
    password: Signal<String>,
    mut primary_token: Signal<Option<String>>,
    mut primary_token_is_local: Signal<bool>,
    mut mfa_required: Signal<bool>,
    mut error: Signal<Option<String>>,
    mut submitting: Signal<bool>,
    on_success: EventHandler<String>,
    api_ctx: ApiContext,
    #[allow(unused_mut)] mut toast: ToastSender,
) {
    let email_value = email.read().clone();
    let password_value = password.read().clone();

    primary_token.set(None);
    primary_token_is_local.set(false);
    mfa_required.set(false);
    submitting.set(true);
    error.set(None);

    #[cfg(target_arch = "wasm32")]
    {
        use login_internals::{decide_outcome, LocalAttempt, LoginOutcome};
        use platform_bridge::PlatformBridge;

        wasm_bindgen_futures::spawn_local(async move {
            let local = match api::local_login(&api_ctx, &email_value, &password_value).await {
                Ok(resp) => LocalAttempt::Ok(resp.id_token),
                Err(e) => LocalAttempt::Err(e),
            };
            let firebase = match &local {
                LocalAttempt::Ok(_) => None,
                LocalAttempt::Err(_) => {
                    let bridge = platform_bridge::web::WebBridge;
                    Some(
                        bridge
                            .sign_in_email_password(&email_value, &password_value)
                            .await
                            .map_err(|e| format!("{e}")),
                    )
                }
            };
            match decide_outcome(local, firebase) {
                LoginOutcome::Token {
                    token,
                    persist_local_token,
                } => {
                    let check_ctx = accepted_api_context(&api_ctx, token.clone());
                    match api::get_me(&check_ctx).await {
                        Ok(_) => finish_accepted_login(
                            &email_value,
                            token,
                            persist_local_token,
                            &check_ctx,
                            on_success,
                        ),
                        Err(err) => match login_internals::classify_me_error(&err) {
                            login_internals::MeCheckOutcome::MfaRequired => {
                                match attempt_trusted_device_stepup(&email_value, &check_ctx).await
                                {
                                    Ok(Some(stepup_token)) => finish_accepted_login(
                                        &email_value,
                                        stepup_token,
                                        persist_local_token,
                                        &check_ctx,
                                        on_success,
                                    ),
                                    Ok(None) => {
                                        primary_token.set(Some(token));
                                        primary_token_is_local.set(persist_local_token);
                                        mfa_required.set(true);
                                        error.set(None);
                                    }
                                    Err(stepup_err) => {
                                        let msg = format!("{stepup_err}");
                                        toast.push(
                                            ToastLevel::Danger,
                                            "Sign-in failed",
                                            msg.clone(),
                                        );
                                        error.set(Some(msg));
                                    }
                                }
                            }
                            login_internals::MeCheckOutcome::Failed(msg) => {
                                toast.push(ToastLevel::Danger, "Sign-in failed", msg.clone());
                                error.set(Some(msg));
                            }
                            login_internals::MeCheckOutcome::Ok => {}
                        },
                    }
                }
                LoginOutcome::Err(msg) => {
                    // Surface server-side signin failures as a transient toast in
                    // addition to the inline FormError. Client-side field validation
                    // (e.g. empty inputs) is handled by FormError only.
                    toast.push(ToastLevel::Danger, "Sign-in failed", msg.clone());
                    error.set(Some(msg));
                }
            }
            submitting.set(false);
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        use login_internals::{decide_outcome, LocalAttempt, LoginOutcome};
        use platform_bridge::PlatformBridge;

        dioxus::prelude::spawn(async move {
            // Same precedence as web: try the backend local-login bypass first,
            // then fall back to Firebase (here via the native REST bridge).
            let local = match api::local_login(&api_ctx, &email_value, &password_value).await {
                Ok(resp) => LocalAttempt::Ok(resp.id_token),
                Err(e) => LocalAttempt::Err(e),
            };
            let firebase = match &local {
                LocalAttempt::Ok(_) => None,
                LocalAttempt::Err(_) => {
                    let bridge = platform_bridge::native::NativeBridge;
                    Some(
                        bridge
                            .sign_in_email_password(&email_value, &password_value)
                            .await
                            .map_err(|e| format!("{e}")),
                    )
                }
            };
            match decide_outcome(local, firebase) {
                LoginOutcome::Token {
                    token,
                    persist_local_token,
                } => {
                    let check_ctx = accepted_api_context(&api_ctx, token.clone());
                    match api::get_me(&check_ctx).await {
                        Ok(_) => finish_accepted_login(
                            &email_value,
                            token,
                            persist_local_token,
                            &check_ctx,
                            on_success,
                        ),
                        Err(err) => match login_internals::classify_me_error(&err) {
                            login_internals::MeCheckOutcome::MfaRequired => {
                                match attempt_trusted_device_stepup(&email_value, &check_ctx).await
                                {
                                    Ok(Some(stepup_token)) => finish_accepted_login(
                                        &email_value,
                                        stepup_token,
                                        persist_local_token,
                                        &check_ctx,
                                        on_success,
                                    ),
                                    Ok(None) => {
                                        primary_token.set(Some(token));
                                        primary_token_is_local.set(persist_local_token);
                                        mfa_required.set(true);
                                        error.set(None);
                                    }
                                    Err(stepup_err) => {
                                        let msg = format!("{stepup_err}");
                                        toast.push(
                                            ToastLevel::Danger,
                                            "Sign-in failed",
                                            msg.clone(),
                                        );
                                        error.set(Some(msg));
                                    }
                                }
                            }
                            login_internals::MeCheckOutcome::Failed(msg) => {
                                toast.push(ToastLevel::Danger, "Sign-in failed", msg.clone());
                                error.set(Some(msg));
                            }
                            login_internals::MeCheckOutcome::Ok => {}
                        },
                    }
                }
                LoginOutcome::Err(msg) => {
                    toast.push(ToastLevel::Danger, "Sign-in failed", msg.clone());
                    error.set(Some(msg));
                }
            }
            submitting.set(false);
        });
    }
}

fn accepted_api_context(api_ctx: &ApiContext, token: String) -> ApiContext {
    ApiContext {
        base_url: api_ctx.base_url.clone(),
        id_token: token,
    }
}

fn finish_accepted_login(
    email_value: &str,
    token: String,
    persist_local_token: bool,
    api_ctx: &ApiContext,
    on_success: EventHandler<String>,
) {
    #[cfg(target_arch = "wasm32")]
    {
        if persist_local_token {
            platform_bridge::web::persist_local_token(&token);
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        if persist_local_token {
            // Local/dev credentials have no provider refresh token. Keep the
            // session in the same OS-protected store as Firebase credentials
            // so native development does not log out on every app restart.
            let _ = platform_bridge::native::NativeBridge::persist_local_development_token(&token);
        }
    }
    let _ = persist_local_token;
    let _ = email_value;
    let _ = api_ctx;
    on_success.call(token);
}

fn stored_trusted_device_token(email: &str) -> Option<String> {
    #[cfg(target_arch = "wasm32")]
    {
        platform_bridge::web::trusted_device_token(email)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        platform_bridge::native::NativeBridge::trusted_device_token(email)
            .ok()
            .flatten()
    }
}

fn clear_stored_trusted_device_token(email: &str) {
    #[cfg(target_arch = "wasm32")]
    platform_bridge::web::clear_trusted_device_token(email);
    #[cfg(not(target_arch = "wasm32"))]
    let _ = platform_bridge::native::NativeBridge::clear_trusted_device_token(email);
}

async fn attempt_trusted_device_stepup(
    email_value: &str,
    challenge_ctx: &ApiContext,
) -> Result<Option<String>, features_courses::api::ApiError> {
    let Some(device_token) = stored_trusted_device_token(email_value) else {
        return Ok(None);
    };
    let body = features_courses::api::MfaChallengeBody {
        code: None,
        trusted_device_token: Some(device_token),
        remember_device: false,
        device_label: None,
    };
    match features_courses::api::challenge_mfa(challenge_ctx, &body).await {
        Ok(resp) => Ok(Some(resp.stepup_token)),
        Err(err) if login_internals::trusted_device_error_should_fall_back(&err) => {
            clear_stored_trusted_device_token(email_value);
            Ok(None)
        }
        Err(err) => Err(err),
    }
}

#[allow(clippy::too_many_arguments)]
fn submit_mfa_challenge(
    email: Signal<String>,
    mfa_code: Signal<String>,
    remember_device: Signal<bool>,
    primary_token: Signal<Option<String>>,
    primary_token_is_local: Signal<bool>,
    mut error: Signal<Option<String>>,
    mut submitting: Signal<bool>,
    on_success: EventHandler<String>,
    api_ctx: ApiContext,
    #[allow(unused_mut)] mut toast: ToastSender,
) {
    let Some(primary) = primary_token.read().clone() else {
        error.set(Some("Sign in again to verify MFA.".to_string()));
        return;
    };
    let entered = mfa_code.read().trim().to_string();
    if entered.is_empty() {
        error.set(Some("Enter your authentication code.".to_string()));
        return;
    }
    submitting.set(true);
    error.set(None);

    let email_value = email.read().clone();
    let challenge_ctx = accepted_api_context(&api_ctx, primary);
    spawn(async move {
        let body = features_courses::api::MfaChallengeBody {
            code: Some(entered),
            trusted_device_token: None,
            remember_device: *remember_device.read(),
            device_label: Some("This device".to_string()),
        };
        match features_courses::api::challenge_mfa(&challenge_ctx, &body).await {
            Ok(resp) => {
                if let Some(device_token) = resp.trusted_device_token.as_deref() {
                    #[cfg(target_arch = "wasm32")]
                    platform_bridge::web::persist_trusted_device_token(&email_value, device_token);
                    #[cfg(not(target_arch = "wasm32"))]
                    let _ = platform_bridge::native::NativeBridge::persist_trusted_device_token(
                        &email_value,
                        device_token,
                    );
                }
                finish_accepted_login(
                    &email_value,
                    resp.stepup_token,
                    *primary_token_is_local.read(),
                    &challenge_ctx,
                    on_success,
                );
            }
            Err(err) => {
                let msg = format!("{err}");
                toast.push(ToastLevel::Danger, "MFA verification failed", msg.clone());
                error.set(Some(msg));
            }
        }
        submitting.set(false);
    });
}
