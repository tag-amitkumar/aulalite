// crates/features-courses/src/privacy_settings.rs
//! "Privacy & data" settings panel (GDPR self-service + auth hardening).
//!
//! Three controls, each backed by a `/v1/me/*` endpoint:
//!   * "Download my data"      → GET  /v1/me/export (authenticated Blob download)
//!   * "Sign out everywhere"   → POST /v1/me/sessions/revoke-all
//!   * "Delete my account"     → POST /v1/me/delete, guarded by a confirm modal
//!
//! Designed to drop into the existing notification settings page (mounted by
//! `shell-web/src/routes/notification_settings.rs`). It is shell-agnostic: the
//! parent passes `on_signed_out`, which we invoke after a successful
//! revoke-all / delete so the shell can sign the browser session out and
//! navigate to the login screen.
//!
//! The export download uses the same authenticated-Blob pattern as
//! `gradebook_panel`/`attendance_panel`: a plain `<a href>` can't carry the
//! bearer token, so we fetch with the token, wrap the body in a Blob, and click
//! a synthetic anchor. Browser-only code is behind `cfg(target_arch = "wasm32")`.

use crate::api::{self, ApiContext, ApiError};
use design_system::{use_toast_sender, Button, ButtonVariant, Card, Modal, ModalSize, ToastLevel};
use dioxus::prelude::*;

// ---------------------------------------------------------------------------
// API client (mirrors api.rs wrappers; calls the re-exported fetch_json)
// ---------------------------------------------------------------------------

/// Mirrors the backend `RevokeAllResponse` in `handlers/privacy.rs`. The
/// timestamp serializes as a JSON string.
#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct RevokeAllResponseDto {
    pub tokens_valid_after: String,
}

/// Mirrors the backend `DeleteAccountResponse` in `handlers/privacy.rs`.
#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct DeleteAccountResponseDto {
    pub anonymized: bool,
    pub anonymized_at: Option<String>,
    pub deleted_at: Option<String>,
}

/// `POST /v1/me/sessions/revoke-all` — invalidate every previously-issued token.
pub async fn revoke_all_sessions(cx: &ApiContext) -> Result<RevokeAllResponseDto, ApiError> {
    api::fetch_json(cx, "POST", "/v1/me/sessions/revoke-all", None::<&()>).await
}

/// `POST /v1/me/delete` — right-to-erasure (anonymize the caller).
pub async fn delete_my_account(cx: &ApiContext) -> Result<DeleteAccountResponseDto, ApiError> {
    api::fetch_json(cx, "POST", "/v1/me/delete", None::<&()>).await
}

// ---------------------------------------------------------------------------
// Authenticated export download (wasm-only Blob; mirrors gradebook_panel)
// ---------------------------------------------------------------------------

/// Trigger an authenticated download of the caller's GDPR data bundle
/// (`GET /v1/me/export`). The endpoint requires the bearer token, so a plain
/// `<a href>` can't reach it — fetch with the token, wrap the body in a Blob,
/// and click a synthetic anchor. wasm-only (no DOM on native).
#[cfg(target_arch = "wasm32")]
fn download_my_export(cx: &ApiContext) {
    use wasm_bindgen::{closure::Closure, JsCast};
    use wasm_bindgen_futures::JsFuture;

    let base_url = cx.base_url.clone();
    let id_token = cx.id_token.clone();

    wasm_bindgen_futures::spawn_local(async move {
        let url = format!("{base_url}/v1/me/export");
        let opts = web_sys::RequestInit::new();
        opts.set_method("GET");
        let Ok(req) = web_sys::Request::new_with_str_and_init(&url, &opts) else {
            return;
        };
        if !id_token.is_empty() {
            let _ = req
                .headers()
                .set("authorization", &format!("Bearer {id_token}"));
        }
        crate::api::apply_workspace_header(&req.headers());
        let Some(window) = web_sys::window() else {
            return;
        };
        let Ok(resp_value) = JsFuture::from(window.fetch_with_request(&req)).await else {
            return;
        };
        let Ok(resp) = resp_value.dyn_into::<web_sys::Response>() else {
            return;
        };
        if !(200..300).contains(&resp.status()) {
            return;
        }
        let Ok(blob_promise) = resp.blob() else {
            return;
        };
        let Ok(blob_value) = JsFuture::from(blob_promise).await else {
            return;
        };
        let Ok(blob) = blob_value.dyn_into::<web_sys::Blob>() else {
            return;
        };
        let Ok(object_url) = web_sys::Url::create_object_url_with_blob(&blob) else {
            return;
        };
        let Some(document) = window.document() else {
            return;
        };
        let Ok(anchor_el) = document.create_element("a") else {
            let _ = web_sys::Url::revoke_object_url(&object_url);
            return;
        };
        let Ok(anchor) = anchor_el.dyn_into::<web_sys::HtmlAnchorElement>() else {
            let _ = web_sys::Url::revoke_object_url(&object_url);
            return;
        };
        anchor.set_href(&object_url);
        anchor.set_download("aulalite-data-export.json");
        anchor.click();

        let revoke_url = object_url.clone();
        let cb = Closure::once_into_js(move || {
            let _ = web_sys::Url::revoke_object_url(&revoke_url);
        });
        let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(cb.unchecked_ref(), 0);
    });
}

#[cfg(not(target_arch = "wasm32"))]
fn download_my_export(cx: &ApiContext) {
    let cx = cx.clone();
    spawn(async move {
        let _ = api::save_authenticated_download(
            &cx,
            "/v1/me/export",
            "aulalite-data-export.json",
            "application/json",
        )
        .await;
    });
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

/// Privacy & data settings panel. `on_signed_out` is invoked after a successful
/// "sign out everywhere" or account deletion so the host shell can clear the
/// local session and route to login.
#[component]
pub fn PrivacySettings(on_signed_out: EventHandler<()>) -> Element {
    let api = api::use_api();
    let toast = use_toast_sender();

    let mut confirm_open = use_signal(|| false);
    let deleting = use_signal(|| false);
    let revoking = use_signal(|| false);

    // "Download my data"
    let on_download = {
        let api = api.clone();
        let mut toast = toast;
        move |_| {
            download_my_export(&api);
            toast.push(
                ToastLevel::Info,
                "Preparing your download",
                "Your data export will download shortly.",
            );
        }
    };

    // "Sign out everywhere"
    let on_revoke = {
        let api = api.clone();
        move |_| {
            let api = api.clone();
            let mut revoking = revoking;
            let mut toast = toast;
            let on_signed_out = on_signed_out;
            if *revoking.read() {
                return;
            }
            revoking.set(true);
            spawn(async move {
                match revoke_all_sessions(&api).await {
                    Ok(_) => {
                        toast.push(
                            ToastLevel::Success,
                            "Signed out everywhere",
                            "All other sessions have been revoked.",
                        );
                        on_signed_out.call(());
                    }
                    Err(err) => {
                        toast.push(ToastLevel::Danger, "Could not sign out", format!("{err}"));
                    }
                }
                revoking.set(false);
            });
        }
    };

    // "Delete my account" (after confirm)
    let on_confirm_delete = {
        let api = api.clone();
        move |_| {
            let api = api.clone();
            let mut deleting = deleting;
            let mut confirm_open = confirm_open;
            let mut toast = toast;
            let on_signed_out = on_signed_out;
            if *deleting.read() {
                return;
            }
            deleting.set(true);
            spawn(async move {
                match delete_my_account(&api).await {
                    Ok(_) => {
                        confirm_open.set(false);
                        toast.push(
                            ToastLevel::Success,
                            "Account deleted",
                            "Your personal data has been anonymized. Signing you out.",
                        );
                        on_signed_out.call(());
                    }
                    Err(err) => {
                        toast.push(
                            ToastLevel::Danger,
                            "Could not delete account",
                            format!("{err}"),
                        );
                    }
                }
                deleting.set(false);
            });
        }
    };

    let is_revoking = *revoking.read();
    let is_deleting = *deleting.read();

    rsx! {
        h2 { class: "notif-settings-section-title", "Privacy & data" }
        Card {
            div { class: "privacy-pref-list",
                // Download my data
                div { class: "privacy-pref-row",
                    div { class: "privacy-pref-row-text",
                        span { class: "privacy-pref-row-title", "Download my data" }
                        span { class: "privacy-pref-row-help",
                            "Export a copy of your profile, enrollments, submissions, grades, posts, notes, and attendance as a JSON file."
                        }
                    }
                    Button {
                        label: "Download".to_string(),
                        variant: ButtonVariant::Secondary,
                        on_click: on_download,
                    }
                }

                // Sign out everywhere
                div { class: "privacy-pref-row",
                    div { class: "privacy-pref-row-text",
                        span { class: "privacy-pref-row-title", "Sign out everywhere" }
                        span { class: "privacy-pref-row-help",
                            "End every active session on all your devices. You'll need to sign in again."
                        }
                    }
                    Button {
                        label: "Sign out everywhere".to_string(),
                        variant: ButtonVariant::Secondary,
                        loading: is_revoking,
                        on_click: on_revoke,
                    }
                }

                // Delete my account (danger)
                div { class: "privacy-pref-row privacy-pref-row--danger",
                    div { class: "privacy-pref-row-text",
                        span { class: "privacy-pref-row-title", "Delete my account" }
                        span { class: "privacy-pref-row-help",
                            "Permanently anonymize your account. Your name and email are removed; your coursework stays in instructors' records for academic integrity. This cannot be undone."
                        }
                    }
                    Button {
                        label: "Delete my account".to_string(),
                        variant: ButtonVariant::Danger,
                        on_click: move |_| confirm_open.set(true),
                    }
                }
            }
        }

        Modal {
            open: *confirm_open.read(),
            title: "Delete your account?".to_string(),
            size: ModalSize::Small,
            on_close: move |_| confirm_open.set(false),
            div { class: "privacy-delete-confirm",
                p {
                    "This permanently anonymizes your account. Your display name and email will be replaced with a placeholder, and you'll be signed out everywhere. Your submissions and grades remain in instructors' gradebooks for academic-integrity reasons."
                }
                p { class: "privacy-delete-confirm-warn", "This action cannot be undone." }
                div { class: "privacy-delete-confirm-actions",
                    Button {
                        label: "Cancel".to_string(),
                        variant: ButtonVariant::Ghost,
                        on_click: move |_| confirm_open.set(false),
                    }
                    Button {
                        label: "Delete my account".to_string(),
                        variant: ButtonVariant::Danger,
                        loading: is_deleting,
                        on_click: on_confirm_delete,
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod ssr_tests {
    use super::*;

    #[test]
    fn panel_renders_three_controls() {
        fn app() -> Element {
            // Provide the ApiContext signal `use_api` reads.
            use_context_provider(|| {
                Signal::new(ApiContext {
                    base_url: String::new(),
                    id_token: String::new(),
                })
            });
            // PrivacySettings also calls use_toast_sender(), which reads a
            // Signal<ToastQueue> from context — provide it too so the panel
            // renders in the SSR harness.
            use_context_provider(|| Signal::new(design_system::ToastQueue::new()));
            rsx! { PrivacySettings { on_signed_out: |_| {} } }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        // The section header renders as a static `h2.notif-settings-section-title`
        // (Dioxus SSR emits static text verbatim, so the literal "&" is not
        // entity-encoded). Assert the stable class to avoid that ambiguity.
        assert!(
            html.contains("notif-settings-section-title"),
            "section title missing: {html}"
        );
        assert!(
            html.contains("Download my data"),
            "download row missing: {html}"
        );
        assert!(
            html.contains("Sign out everywhere"),
            "revoke row missing: {html}"
        );
        assert!(
            html.contains("Delete my account"),
            "delete row missing: {html}"
        );
    }
}
