// crates/shell-web/src/routes/admin_branding.rs
//
// Tenant branding admin surface. Restricted to org-admin / platform-admin users
// (the backend `/v1/admin/branding` family 403s everyone else). Renders a form
// for the tenant's logo URL plus primary + accent brand colors (native color
// pickers with a hex text fallback), a live preview swatch, and a sample
// Button/Badge tinted with the chosen primary color so admins can see the
// effect before saving. Saving PATCHes only the provided fields and toasts the
// outcome; the new branding takes effect across the app shell on the next load
// of `/v1/me/branding`.
//
// Follows the project-wide four-state UX: loading, error, empty (handled as the
// "all defaults" loaded state), and loaded.
use design_system::{
    use_toast_sender, Badge, BadgeTone, Button, ButtonVariant, Card, Input, PageHeader,
    SkeletonCard, ToastLevel,
};
use dioxus::prelude::*;
use dioxus_router::use_navigator;
use features_courses::api;
use features_courses::app_shell::{AppShell, ShellUser};

use crate::route_enum::Route;
use crate::routes::{use_api, use_user_context};

/// Default fallbacks shown in the color pickers when the tenant has not set a
/// color yet. These mirror the design-system brand tokens (green-600 primary,
/// gold-600 accent) so the pickers open on-brand rather than on black.
const DEFAULT_PRIMARY: &str = "#244f43";
const DEFAULT_ACCENT: &str = "#b08842";

/// Normalize a free-text hex value for the native color picker, which only
/// accepts a `#rrggbb` string. Anything that doesn't look like a 7-char hex is
/// coerced to `fallback` so the picker stays valid; the text field keeps the
/// raw value the admin typed. Pure so it's unit-testable.
fn picker_value(raw: &str, fallback: &str) -> String {
    let t = raw.trim();
    let is_hex6 =
        t.len() == 7 && t.starts_with('#') && t[1..].chars().all(|c| c.is_ascii_hexdigit());
    if is_hex6 {
        t.to_string()
    } else {
        fallback.to_string()
    }
}

/// The live preview: a color swatch plus a sample Button and Badge tinted with
/// the chosen primary color via an inline `--color-primary` / `--brand-primary`
/// override scoped to the preview wrapper. Pure so it's SSR-testable.
fn preview(primary: &str, accent: &str) -> Element {
    let p = primary.trim();
    let a = accent.trim();
    // Scope the token overrides to the preview block so the sample controls
    // render with the chosen brand without touching the rest of the page.
    let style = format!(
        "--color-primary: {p}; --color-primary-hover: {p}; --brand-primary: {p}; --brand-primary-hover: {p}; --color-accent: {a}; --color-accent-strong: {a};"
    );
    rsx! {
        div { class: "admin-branding-preview", style: "{style}",
            div { class: "admin-branding-swatches",
                div {
                    class: "admin-branding-swatch",
                    style: "background: {p};",
                    title: "Primary {p}",
                }
                div {
                    class: "admin-branding-swatch",
                    style: "background: {a};",
                    title: "Accent {a}",
                }
            }
            div { class: "admin-branding-preview-controls",
                Button {
                    label: "Primary action".to_string(),
                    variant: ButtonVariant::Primary,
                    on_click: move |_| {},
                }
                Badge { label: "Brand".to_string(), tone: BadgeTone::Primary }
            }
        }
    }
}

/// Skeleton placeholder shown while branding loads.
fn loading_grid() -> Element {
    rsx! {
        div { class: "admin-branding-loading",
            SkeletonCard { height: "200px".to_string() }
        }
    }
}

#[component]
pub fn AdminBranding() -> Element {
    let nav = use_navigator();
    let api = use_api();
    let user_ctx = use_user_context();
    let toast = use_toast_sender();

    let user = match user_ctx.read().clone() {
        Some(u) => u,
        None => {
            nav.push(Route::Login {});
            return rsx! { p { "Redirecting…" } };
        }
    };

    let is_admin = user.can_manage_organization();
    if !is_admin {
        return rsx! {
            div { class: "container",
                h1 { "Forbidden" }
                p { "You don't have permission to manage branding." }
            }
        };
    }

    let mut branding = use_resource({
        let api = api.clone();
        move || {
            let api = api.clone();
            async move { api::get_admin_branding(&api).await }
        }
    });

    // Editable form state. Seeded from the loaded branding once.
    let mut logo_url = use_signal(String::new);
    let mut primary_color = use_signal(String::new);
    let mut accent_color = use_signal(String::new);
    let mut loaded = use_signal(|| false);
    let mut saving = use_signal(|| false);

    let snap = branding.read_unchecked();

    // Bootstrap the edit signals from the loaded branding exactly once.
    if !*loaded.read() {
        if let Some(Ok(b)) = snap.as_ref() {
            logo_url.set(b.logo_url.clone().unwrap_or_default());
            primary_color.set(b.primary_color.clone().unwrap_or_default());
            accent_color.set(b.accent_color.clone().unwrap_or_default());
            loaded.set(true);
        }
    }

    let content: Element = match snap.as_ref() {
        Some(Ok(_)) => {
            let logo_v = logo_url.read().clone();
            let primary_v = primary_color.read().clone();
            let accent_v = accent_color.read().clone();
            let primary_picker = picker_value(&primary_v, DEFAULT_PRIMARY);
            let accent_picker = picker_value(&accent_v, DEFAULT_ACCENT);
            // The preview tints with the chosen value, falling back to the
            // brand default when a field is blank.
            let preview_primary = if primary_v.trim().is_empty() {
                DEFAULT_PRIMARY.to_string()
            } else {
                primary_v.trim().to_string()
            };
            let preview_accent = if accent_v.trim().is_empty() {
                DEFAULT_ACCENT.to_string()
            } else {
                accent_v.trim().to_string()
            };
            let preview_el = preview(&preview_primary, &preview_accent);

            let api_for_save = api.clone();
            rsx! {
                Card {
                    form {
                        class: "admin-branding-form",
                        onsubmit: move |e: FormEvent| {
                            e.prevent_default();
                            let api = api_for_save.clone();
                            let mut toast = toast;
                            // Send only fields that have a value; blank fields
                            // are omitted so the backend leaves them untouched.
                            let logo = {
                                let v = logo_url.read().trim().to_string();
                                if v.is_empty() { None } else { Some(v) }
                            };
                            let primary = {
                                let v = primary_color.read().trim().to_string();
                                if v.is_empty() { None } else { Some(v) }
                            };
                            let accent = {
                                let v = accent_color.read().trim().to_string();
                                if v.is_empty() { None } else { Some(v) }
                            };
                            saving.set(true);
                            spawn(async move {
                                match api::patch_admin_branding(&api, logo, primary, accent).await {
                                    Ok(_) => {
                                        toast.push(
                                            ToastLevel::Success,
                                            "Branding saved",
                                            "Your workspace branding was updated.",
                                        );
                                        branding.restart();
                                    }
                                    Err(err) => {
                                        toast.push(
                                            ToastLevel::Danger,
                                            "Save failed",
                                            format!("{err}"),
                                        );
                                    }
                                }
                                saving.set(false);
                            });
                        },

                        label { class: "admin-branding-label", "Logo URL" }
                        Input {
                            value: logo_v.clone(),
                            input_type: "url".to_string(),
                            placeholder: "https://cdn.example.com/logo.svg".to_string(),
                            disabled: *saving.read(),
                            on_input: move |v: String| logo_url.set(v),
                        }
                        p { class: "muted admin-branding-help",
                            "Shown as the workspace mark in the sidebar. Leave blank to use the default AulaLite logo."
                        }

                        label { class: "admin-branding-label", "Primary color" }
                        div { class: "admin-branding-color-row",
                            input {
                                class: "admin-branding-color-picker",
                                r#type: "color",
                                value: "{primary_picker}",
                                disabled: *saving.read(),
                                oninput: move |e| primary_color.set(e.value()),
                            }
                            Input {
                                value: primary_v.clone(),
                                placeholder: DEFAULT_PRIMARY.to_string(),
                                disabled: *saving.read(),
                                on_input: move |v: String| primary_color.set(v),
                            }
                        }

                        label { class: "admin-branding-label", "Accent color" }
                        div { class: "admin-branding-color-row",
                            input {
                                class: "admin-branding-color-picker",
                                r#type: "color",
                                value: "{accent_picker}",
                                disabled: *saving.read(),
                                oninput: move |e| accent_color.set(e.value()),
                            }
                            Input {
                                value: accent_v.clone(),
                                placeholder: DEFAULT_ACCENT.to_string(),
                                disabled: *saving.read(),
                                on_input: move |v: String| accent_color.set(v),
                            }
                        }

                        h3 { class: "admin-branding-preview-title", "Live preview" }
                        {preview_el}

                        Button {
                            label: if *saving.read() { "Saving…".to_string() } else { "Save branding".to_string() },
                            button_type: "submit".to_string(),
                            variant: ButtonVariant::Primary,
                            loading: *saving.read(),
                            on_click: move |_| {},
                        }
                    }
                }
            }
        }
        Some(Err(e)) => rsx! {
            p { class: "error", "Could not load branding: {e}" }
        },
        None => rsx! {
            {loading_grid()}
        },
    };
    drop(snap);

    let body = rsx! {
        div { class: "admin-branding-page",
            PageHeader {
                title: "Branding".to_string(),
                kicker: "Workspace administration".to_string(),
                subtitle: "Set your workspace logo and brand colors. Changes apply across the app.".to_string(),
            }
            { content }
        }
    };

    let shell_user = ShellUser {
        display_name: user.display_name.clone(),
        email: user.email.clone(),
        tenant_role: user.tenant_role,
        is_platform_admin: user.is_platform_admin,
    };

    rsx! {
        AppShell {
            user: shell_user,
            on_signout: move |_| {
                #[cfg(target_arch = "wasm32")]
                {
                    use platform_bridge::PlatformBridge;
                    spawn(async move { let _ = platform_bridge::web::WebBridge.sign_out().await; });
                }
                nav.push(Route::Login {});
            },
            { body }
        }
    }
}

#[cfg(test)]
mod ssr_tests {
    use super::*;

    #[test]
    fn picker_value_coerces_invalid_hex_to_fallback() {
        // Valid 6-digit hex passes through (trimmed).
        assert_eq!(picker_value("#244f43", DEFAULT_PRIMARY), "#244f43");
        assert_eq!(picker_value("  #aabbcc  ", DEFAULT_PRIMARY), "#aabbcc");
        // Blank / partial / named colors fall back so the native picker stays valid.
        assert_eq!(picker_value("", DEFAULT_PRIMARY), DEFAULT_PRIMARY);
        assert_eq!(picker_value("#abc", DEFAULT_PRIMARY), DEFAULT_PRIMARY);
        assert_eq!(
            picker_value("rebeccapurple", DEFAULT_ACCENT),
            DEFAULT_ACCENT
        );
        assert_eq!(picker_value("#gggggg", DEFAULT_PRIMARY), DEFAULT_PRIMARY);
    }

    #[test]
    fn preview_tints_sample_controls_with_chosen_colors() {
        fn app() -> Element {
            preview("#7a00ff", "#00c2a8")
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        // The scoped style carries the chosen tokens.
        assert!(html.contains("--color-primary: #7a00ff"), "got: {html}");
        assert!(html.contains("--color-accent: #00c2a8"), "got: {html}");
        // The sample Button + Badge render inside the preview.
        assert!(html.contains("ds-button--primary"), "got: {html}");
        assert!(html.contains("badge-primary"), "got: {html}");
        // Swatches show the raw color values.
        assert!(html.contains("background: #7a00ff"), "got: {html}");
        assert!(html.contains("background: #00c2a8"), "got: {html}");
    }

    #[test]
    fn loading_grid_renders_skeleton() {
        fn app() -> Element {
            loading_grid()
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("ds-skeleton-card"), "got: {html}");
    }

    #[test]
    fn branding_form_renders_inputs_and_pickers() {
        // Render just the loaded form region with seeded values so the test
        // doesn't need a live ApiContext / use_resource.
        fn app() -> Element {
            let logo = "https://cdn.example.com/logo.svg".to_string();
            let primary = "#244f43".to_string();
            let accent = "#b08842".to_string();
            let primary_picker = picker_value(&primary, DEFAULT_PRIMARY);
            let accent_picker = picker_value(&accent, DEFAULT_ACCENT);
            rsx! {
                form { class: "admin-branding-form",
                    label { class: "admin-branding-label", "Logo URL" }
                    Input {
                        value: logo,
                        input_type: "url".to_string(),
                        on_input: move |_v: String| {},
                    }
                    label { class: "admin-branding-label", "Primary color" }
                    input {
                        class: "admin-branding-color-picker",
                        r#type: "color",
                        value: "{primary_picker}",
                        oninput: move |_e| {},
                    }
                    label { class: "admin-branding-label", "Accent color" }
                    input {
                        class: "admin-branding-color-picker",
                        r#type: "color",
                        value: "{accent_picker}",
                        oninput: move |_e| {},
                    }
                    {preview(&primary, &accent)}
                    Button {
                        label: "Save branding".to_string(),
                        button_type: "submit".to_string(),
                        on_click: move |_| {},
                    }
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        // Form region + labels present.
        assert!(html.contains("admin-branding-form"), "got: {html}");
        assert!(html.contains("Logo URL"));
        assert!(html.contains("Primary color"));
        assert!(html.contains("Accent color"));
        // The logo URL input carries the seeded value.
        assert!(html.contains("https://cdn.example.com/logo.svg"));
        // Native color pickers render with type=color.
        assert!(
            html.contains("type=\"color\""),
            "color picker missing: {html}"
        );
        assert!(html.contains("admin-branding-color-picker"));
        // Save button present.
        assert!(html.contains("Save branding"));
        // Live preview present.
        assert!(html.contains("admin-branding-preview"));
    }
}
