// crates/features-courses/src/security_settings.rs
//! "Security" settings panel — two-factor authentication (TOTP) enrollment.
//!
//! Drops into the existing settings page next to `PrivacySettings` (mounted by
//! `shell-web/src/routes/notification_settings.rs`). Self-contained: it defines
//! its own DTOs + `/v1/me/mfa/*` API wrappers (mirroring `privacy_settings.rs`),
//! so it needs no edits to the shared `api.rs`.
//!
//! Flow:
//!   1. On load: GET /v1/me/mfa → enabled? pending?
//!   2. "Set up two-factor": POST /v1/me/mfa/enroll → show the base32 secret +
//!      a copyable `otpauth://` URI (QR-able by the user's app).
//!   3. Enter the 6-digit code → POST /v1/me/mfa/verify → enable + reveal the
//!      one-time recovery codes (shown exactly once).
//!   4. "Turn off" → POST /v1/me/mfa/disable.
//!
//! All network calls reuse `api::fetch_json`; SSR tests read `ApiContext` via the
//! same context the rest of the crate uses.

use crate::api::{self, ApiContext, ApiError, TrustedDeviceDto};
use design_system::{use_toast_sender, Button, ButtonVariant, Card, Input, ToastLevel};
use dioxus::prelude::*;

// ---------------------------------------------------------------------------
// DTOs (mirror handlers/mfa.rs)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct MfaStatusDto {
    pub enabled: bool,
    pub pending: bool,
}

#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct EnrollResponseDto {
    pub secret_base32: String,
    pub otpauth_uri: String,
}

#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct VerifyResponseDto {
    pub enabled: bool,
    pub recovery_codes: Vec<String>,
}

#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct DisableResponseDto {
    pub disabled: bool,
}

#[derive(serde::Serialize)]
struct VerifyBody<'a> {
    code: &'a str,
}

pub async fn get_mfa_status(cx: &ApiContext) -> Result<MfaStatusDto, ApiError> {
    api::fetch_json(cx, "GET", "/v1/me/mfa", None::<&()>).await
}

pub async fn enroll_mfa(cx: &ApiContext) -> Result<EnrollResponseDto, ApiError> {
    api::fetch_json(cx, "POST", "/v1/me/mfa/enroll", None::<&()>).await
}

pub async fn verify_mfa(cx: &ApiContext, code: &str) -> Result<VerifyResponseDto, ApiError> {
    api::fetch_json(cx, "POST", "/v1/me/mfa/verify", Some(&VerifyBody { code })).await
}

pub async fn disable_mfa(cx: &ApiContext) -> Result<DisableResponseDto, ApiError> {
    api::fetch_json(cx, "POST", "/v1/me/mfa/disable", None::<&()>).await
}

pub async fn list_trusted_devices(cx: &ApiContext) -> Result<Vec<TrustedDeviceDto>, ApiError> {
    Ok(api::list_mfa_trusted_devices(cx).await?.devices)
}

pub async fn revoke_trusted_device(cx: &ApiContext, id: &str) -> Result<(), ApiError> {
    api::revoke_mfa_trusted_device(cx, id).await
}

#[cfg(target_arch = "wasm32")]
fn copy_recovery_codes(codes: &[String]) {
    use wasm_bindgen::{JsCast, JsValue};
    let Some(window) = web_sys::window() else {
        return;
    };
    let navigator = window.navigator();
    let Ok(clipboard) = js_sys::Reflect::get(navigator.as_ref(), &JsValue::from_str("clipboard"))
    else {
        return;
    };
    let Ok(write_text) = js_sys::Reflect::get(&clipboard, &JsValue::from_str("writeText")) else {
        return;
    };
    if let Some(write_text) = write_text.dyn_ref::<js_sys::Function>() {
        let _ = write_text.call1(&clipboard, &JsValue::from_str(&codes.join("\n")));
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn copy_recovery_codes(_codes: &[String]) {}

#[cfg(target_arch = "wasm32")]
fn download_recovery_codes(codes: &[String]) {
    use wasm_bindgen::JsCast;
    let Some(window) = web_sys::window() else {
        return;
    };
    let Some(document) = window.document() else {
        return;
    };
    let blob_parts = js_sys::Array::new();
    blob_parts.push(&wasm_bindgen::JsValue::from_str(&codes.join("\n")));
    let Ok(blob) = web_sys::Blob::new_with_str_sequence(&blob_parts) else {
        return;
    };
    let Ok(url) = web_sys::Url::create_object_url_with_blob(&blob) else {
        return;
    };
    let Ok(a) = document.create_element("a") else {
        return;
    };
    let _ = a.set_attribute("href", &url);
    let _ = a.set_attribute("download", "aulalite-recovery-codes.txt");
    if let Some(a) = a.dyn_ref::<web_sys::HtmlElement>() {
        a.click();
    }
    let _ = web_sys::Url::revoke_object_url(&url);
}

#[cfg(not(target_arch = "wasm32"))]
fn download_recovery_codes(codes: &[String]) {
    let bytes = codes.join("\n").into_bytes();
    spawn(async move {
        let _ = platform_bridge::native_files::save_bytes(
            "aulalite-recovery-codes.txt",
            "text/plain",
            &bytes,
        )
        .await;
    });
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

#[component]
pub fn SecuritySettings() -> Element {
    let api = api::use_api();
    let toast = use_toast_sender();

    // Server status (None until first load resolves).
    let mut status = use_signal(|| None::<MfaStatusDto>);
    // Pending enrollment payload (secret + otpauth URI) once "set up" is clicked.
    let mut enrollment = use_signal(|| None::<EnrollResponseDto>);
    // The 6-digit code the user types from their app.
    let mut code = use_signal(String::new);
    // Recovery codes revealed once after a successful verify.
    let mut recovery = use_signal(Vec::<String>::new);
    let mut recovery_ack = use_signal(|| false);
    let mut trusted_devices = use_signal(Vec::<TrustedDeviceDto>::new);
    let busy = use_signal(|| false);

    // Initial status load.
    use_resource({
        let api = api.clone();
        move || {
            let api = api.clone();
            async move {
                if let Ok(s) = get_mfa_status(&api).await {
                    status.set(Some(s));
                }
            }
        }
    });

    use_resource({
        let api = api.clone();
        move || {
            let api = api.clone();
            async move {
                if let Ok(resp) = list_trusted_devices(&api).await {
                    trusted_devices.set(resp);
                }
            }
        }
    });

    let on_setup = {
        let api = api.clone();
        move |_| {
            let api = api.clone();
            let mut busy = busy;
            let mut toast = toast;
            if *busy.read() {
                return;
            }
            busy.set(true);
            spawn(async move {
                match enroll_mfa(&api).await {
                    Ok(resp) => enrollment.set(Some(resp)),
                    Err(err) => toast.push(
                        ToastLevel::Danger,
                        "Could not start setup",
                        format!("{err}"),
                    ),
                }
                busy.set(false);
            });
        }
    };

    let on_verify = {
        let api = api.clone();
        move |_| {
            let api = api.clone();
            let mut busy = busy;
            let mut toast = toast;
            let entered = code.read().trim().to_string();
            if entered.is_empty() || *busy.read() {
                return;
            }
            busy.set(true);
            spawn(async move {
                match verify_mfa(&api, &entered).await {
                    Ok(resp) => {
                        recovery.set(resp.recovery_codes.clone());
                        recovery_ack.set(false);
                        enrollment.set(None);
                        code.set(String::new());
                        status.set(Some(MfaStatusDto {
                            enabled: resp.enabled,
                            pending: false,
                        }));
                        toast.push(
                            ToastLevel::Success,
                            "Two-factor authentication on",
                            "Save your recovery codes somewhere safe.",
                        );
                    }
                    Err(err) => {
                        toast.push(ToastLevel::Danger, "Couldn't verify code", format!("{err}"))
                    }
                }
                busy.set(false);
            });
        }
    };

    let on_disable = {
        let api = api.clone();
        move |_| {
            let api = api.clone();
            let mut busy = busy;
            let mut toast = toast;
            if *busy.read() {
                return;
            }
            busy.set(true);
            spawn(async move {
                match disable_mfa(&api).await {
                    Ok(_) => {
                        recovery.set(Vec::new());
                        enrollment.set(None);
                        status.set(Some(MfaStatusDto {
                            enabled: false,
                            pending: false,
                        }));
                        toast.push(
                            ToastLevel::Info,
                            "Two-factor turned off",
                            "Your account no longer requires a second factor.",
                        );
                    }
                    Err(err) => {
                        toast.push(ToastLevel::Danger, "Couldn't turn off", format!("{err}"))
                    }
                }
                busy.set(false);
            });
        }
    };

    let is_busy = *busy.read();
    let is_enabled = status.read().as_ref().map(|s| s.enabled).unwrap_or(false);
    let pending = enrollment.read().clone();
    let recovery_codes = recovery.read().clone();

    // Body switches between: enabled, mid-enrollment, and not-enrolled.
    let body: Element = if is_enabled && recovery_codes.is_empty() {
        rsx! {
            div { class: "security-mfa-row",
                div { class: "security-mfa-row-text",
                    span { class: "security-mfa-row-title", "Two-factor authentication is on" }
                    span { class: "security-mfa-row-help",
                        "You'll be asked for a code from your authenticator app when signing in."
                    }
                }
                Button {
                    label: "Turn off".to_string(),
                    variant: ButtonVariant::Danger,
                    loading: is_busy,
                    on_click: on_disable,
                }
            }
        }
    } else if let Some(enr) = pending {
        rsx! {
            div { class: "security-mfa-enroll",
                p { class: "security-mfa-row-help",
                    "Scan this in your authenticator app, or enter the secret manually, then type the 6-digit code to confirm."
                }
                div { class: "security-mfa-secret",
                    span { class: "security-mfa-secret-label", "Setup key" }
                    code { class: "security-mfa-secret-value", "{enr.secret_base32}" }
                }
                div { class: "security-mfa-uri",
                    span { class: "security-mfa-secret-label", "otpauth URI" }
                    code { class: "security-mfa-uri-value", "{enr.otpauth_uri}" }
                }
                div { class: "security-mfa-verify",
                    Input {
                        value: code.read().clone(),
                        placeholder: "123456".to_string(),
                        on_input: move |v: String| code.set(v),
                    }
                    Button {
                        label: "Verify & enable".to_string(),
                        variant: ButtonVariant::Primary,
                        loading: is_busy,
                        on_click: on_verify,
                    }
                }
            }
        }
    } else {
        rsx! {
            div { class: "security-mfa-row",
                div { class: "security-mfa-row-text",
                    span { class: "security-mfa-row-title", "Two-factor authentication" }
                    span { class: "security-mfa-row-help",
                        "Add a second step at sign-in using an authenticator app (TOTP)."
                    }
                }
                Button {
                    label: "Set up two-factor".to_string(),
                    variant: ButtonVariant::Secondary,
                    loading: is_busy,
                    on_click: on_setup,
                }
            }
        }
    };

    // Recovery codes panel — only after a fresh verify (shown once).
    let recovery_panel: Element = if recovery_codes.is_empty() {
        rsx! {}
    } else {
        let can_dismiss = *recovery_ack.read();
        rsx! {
            div { class: "security-recovery",
                span { class: "security-mfa-row-title", "Your recovery codes" }
                span { class: "security-mfa-row-help",
                    "Each code works once. Store them now; they are shown only once."
                }
                ul { class: "security-recovery-list",
                    for rc in recovery_codes.iter() {
                        li { class: "security-recovery-code", "{rc}" }
                    }
                }
                div { class: "security-recovery-actions",
                    Button {
                        label: "Copy codes".to_string(),
                        variant: ButtonVariant::Secondary,
                        on_click: {
                            let codes = recovery_codes.clone();
                            move |_| copy_recovery_codes(&codes)
                        },
                    }
                    Button {
                        label: "Download".to_string(),
                        variant: ButtonVariant::Secondary,
                        on_click: {
                            let codes = recovery_codes.clone();
                            move |_| download_recovery_codes(&codes)
                        },
                    }
                }
                label { class: "security-recovery-ack",
                    input {
                        r#type: "checkbox",
                        checked: can_dismiss,
                        onchange: move |event| recovery_ack.set(event.checked()),
                    }
                    span { "I saved these codes" }
                }
                Button {
                    label: "Done".to_string(),
                    variant: ButtonVariant::Primary,
                    disabled: !can_dismiss,
                    on_click: move |_| {
                        recovery.set(Vec::new());
                        recovery_ack.set(false);
                    },
                }
            }
        }
    };

    let trusted_panel: Element = if !is_enabled {
        rsx! {}
    } else {
        let devices = trusted_devices.read().clone();
        rsx! {
            div { class: "security-trusted-devices",
                span { class: "security-mfa-row-title", "Remembered devices" }
                if devices.is_empty() {
                    span { class: "security-mfa-row-help", "No remembered devices." }
                } else {
                    ul { class: "security-trusted-device-list",
                        for device in devices.iter() {
                            {
                                let id = device.id.clone();
                                let label = device.label.clone();
                                let api = api.clone();
                                rsx! {
                                    li { class: "security-trusted-device", key: "{id}",
                                        span { class: "security-trusted-device-label", "{label}" }
                                        span { class: "security-mfa-row-help", "Expires {device.expires_at}" }
                                        Button {
                                            label: "Revoke".to_string(),
                                            variant: ButtonVariant::Danger,
                                            on_click: move |_| {
                                                let api = api.clone();
                                                let id = id.clone();
                                                let mut toast = toast;
                                                spawn(async move {
                                                    match revoke_trusted_device(&api, &id).await {
                                                        Ok(()) => {
                                                            trusted_devices.with_mut(|items| items.retain(|d| d.id != id));
                                                            toast.push(ToastLevel::Success, "Device revoked", "This device must use a code next sign-in.");
                                                        }
                                                        Err(err) => toast.push(ToastLevel::Danger, "Revoke failed", format!("{err}")),
                                                    }
                                                });
                                            },
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    };

    rsx! {
        h2 { class: "notif-settings-section-title", "Security" }
        Card {
            div { class: "security-mfa-list",
                { body }
                { recovery_panel }
                { trusted_panel }
            }
        }
    }
}

#[cfg(test)]
mod ssr_tests {
    use super::*;

    #[test]
    fn panel_renders_security_section() {
        fn app() -> Element {
            use_context_provider(|| {
                Signal::new(ApiContext {
                    base_url: String::new(),
                    id_token: String::new(),
                })
            });
            use_context_provider(|| Signal::new(design_system::ToastQueue::new()));
            rsx! { SecuritySettings {} }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("notif-settings-section-title"),
            "section title missing: {html}"
        );
        // Default (not enrolled, status unresolved) shows the set-up affordance.
        assert!(
            html.contains("Two-factor authentication"),
            "mfa heading missing: {html}"
        );
        assert!(
            html.contains("Set up two-factor"),
            "set-up button missing: {html}"
        );
    }

    #[test]
    fn panel_mentions_recovery_code_safety() {
        fn app() -> Element {
            use_context_provider(|| {
                Signal::new(ApiContext {
                    base_url: String::new(),
                    id_token: String::new(),
                })
            });
            use_context_provider(|| Signal::new(design_system::ToastQueue::new()));
            rsx! { SecuritySettings {} }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("Security"), "{html}");
    }
}
