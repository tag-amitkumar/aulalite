// crates/features-courses/src/scorm_player.rs
//! SCORM player + package management tab (Tool side).
//!
//! Staff see the registered packages, an "Add package" composer (asset id +
//! title + pasted `imsmanifest.xml`), and can delete; everyone who can read the
//! course can launch a package. Launching resolves the presigned launch URL via
//! `GET /v1/scorm/:id/launch` and loads it in a sandboxed iframe; the SCORM JS
//! runtime is provided by `/assets/scorm-bridge.js`, which this component
//! configures (CMI endpoint + bearer) through `window.__SCORM__` before the SCO
//! loads. CMI round-trips through `GET/PUT /v1/scorm/:id/cmi`.
//!
//! The API client fns live here (like `announcements.rs`) and call the
//! re-exported `api::fetch_json`. Browser-only wiring is behind
//! `cfg(target_arch = "wasm32")` so SSR tests still compile.

use crate::api::{self, ApiContext, ApiError};
use design_system::{
    use_toast_sender, Button, ButtonVariant, Card, CardList, EmptyState, SkeletonLine, ToastLevel,
};
use dioxus::prelude::*;

// ---------------------------------------------------------------------------
// API client (mirrors backend handlers/scorm.rs DTOs)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct ScormPackageDto {
    pub id: String,
    pub course_id: String,
    pub title: String,
    pub asset_id: String,
    pub scorm_version: String,
    pub launch_href: String,
    pub created_at: String,
}

#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct ScormLaunchDto {
    pub base_url: String,
    pub launch_href: String,
    pub scorm_version: String,
    pub launch_url: String,
}

#[derive(serde::Serialize)]
struct RegisterPackageBody<'a> {
    title: &'a str,
    asset_id: &'a str,
    manifest_xml: &'a str,
    launch_href: Option<&'a str>,
}

/// `GET /v1/courses/{cid}/scorm`
pub async fn list_packages(
    ctx: &ApiContext,
    course_id: &str,
) -> Result<Vec<ScormPackageDto>, ApiError> {
    api::fetch_json(
        ctx,
        "GET",
        &format!("/v1/courses/{course_id}/scorm"),
        None::<&()>,
    )
    .await
}

/// `POST /v1/courses/{cid}/scorm` — staff-only.
pub async fn register_package(
    ctx: &ApiContext,
    course_id: &str,
    title: &str,
    asset_id: &str,
    manifest_xml: &str,
    launch_href: Option<&str>,
) -> Result<ScormPackageDto, ApiError> {
    api::fetch_json(
        ctx,
        "POST",
        &format!("/v1/courses/{course_id}/scorm"),
        Some(&RegisterPackageBody {
            title,
            asset_id,
            manifest_xml,
            launch_href,
        }),
    )
    .await
}

/// `DELETE /v1/scorm/{id}` — staff-only.
pub async fn delete_package(ctx: &ApiContext, id: &str) -> Result<(), ApiError> {
    api::fetch_json::<()>(ctx, "DELETE", &format!("/v1/scorm/{id}"), None::<&()>)
        .await
        .map(|_| ())
}

/// `GET /v1/scorm/{id}/launch` — resolve the presigned launch URL.
pub async fn launch_package(ctx: &ApiContext, id: &str) -> Result<ScormLaunchDto, ApiError> {
    api::fetch_json(ctx, "GET", &format!("/v1/scorm/{id}/launch"), None::<&()>).await
}

// ---------------------------------------------------------------------------
// Browser bridge wiring (configures /assets/scorm-bridge.js)
// ---------------------------------------------------------------------------

/// Configure `window.__SCORM__` with the CMI endpoint + bearer and install the
/// SCORM runtime (`window.API` / `window.API_1484_11`) before the SCO iframe
/// loads. No-op off wasm so SSR/tests compile.
#[cfg(target_arch = "wasm32")]
fn configure_bridge(package_id: &str, token: &str) {
    use wasm_bindgen::{JsCast, JsValue};
    let cmi_url = format!("/v1/scorm/{package_id}/cmi");
    // Build window.__SCORM__ = { cmiUrl, token } via JSON + a tiny eval-free path.
    if let Some(window) = web_sys::window() {
        let win: &JsValue = window.as_ref();
        let obj = js_sys::Object::new();
        let _ = js_sys::Reflect::set(
            &obj,
            &JsValue::from_str("cmiUrl"),
            &JsValue::from_str(&cmi_url),
        );
        let _ = js_sys::Reflect::set(&obj, &JsValue::from_str("token"), &JsValue::from_str(token));
        let _ = js_sys::Reflect::set(win, &JsValue::from_str("__SCORM__"), &obj);
        // Call window.aulaScorm.install() if the bridge script is present.
        if let Ok(scorm) = js_sys::Reflect::get(win, &JsValue::from_str("aulaScorm")) {
            if let Ok(install) = js_sys::Reflect::get(&scorm, &JsValue::from_str("install")) {
                if let Ok(func) = install.dyn_into::<js_sys::Function>() {
                    let _ = func.call0(&scorm);
                }
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn configure_bridge(_package_id: &str, _token: &str) {}

// ---------------------------------------------------------------------------
// Components
// ---------------------------------------------------------------------------

#[derive(Clone, Props, PartialEq)]
pub struct ScormTabProps {
    pub course_id: String,
    /// Staff (course owner / teacher / TA / org-admin) get the composer + delete.
    pub is_teacher: bool,
}

/// SCORM tab: package list (+ staff composer) → in-place player on launch.
#[component]
pub fn ScormTab(props: ScormTabProps) -> Element {
    let api = api::use_api();
    let course_id = props.course_id.clone();
    let is_teacher = props.is_teacher;
    let mut active = use_signal(|| Option::<ScormPackageDto>::None);

    let mut packages = use_resource({
        let api = api.clone();
        let course_id = course_id.clone();
        move || {
            let api = api.clone();
            let course_id = course_id.clone();
            async move { list_packages(&api, &course_id).await }
        }
    });

    // When a package is selected, show the player full-width.
    if let Some(pkg) = active.read().clone() {
        return rsx! {
            ScormPlayer {
                api: api.clone(),
                package: pkg,
                on_back: move |_| active.set(None),
            }
        };
    }

    rsx! {
        div { class: "scorm motion-page",
            div { class: "scorm__header",
                h2 { "SCORM content" }
            }
            if is_teacher {
                ScormComposer {
                    api: api.clone(),
                    course_id: course_id.clone(),
                    on_added: move |_| packages.restart(),
                }
            }
            match &*packages.read_unchecked() {
                Some(Ok(items)) if items.is_empty() => rsx! {
                    EmptyState {
                        title: "No SCORM packages yet".to_string(),
                        description: if is_teacher {
                            "Upload a SCORM .zip, then register it above.".to_string()
                        } else {
                            "Your teachers haven't added any SCORM activities yet.".to_string()
                        },
                    }
                },
                Some(Ok(items)) => rsx! {
                    CardList {
                        for p in items.iter() {
                            ScormRow {
                                key: "{p.id}",
                                api: api.clone(),
                                package: p.clone(),
                                can_delete: is_teacher,
                                on_launch: {
                                    let p = p.clone();
                                    move |_| active.set(Some(p.clone()))
                                },
                                on_deleted: move |_| packages.restart(),
                            }
                        }
                    }
                },
                Some(Err(e)) => rsx! {
                    div { class: "system-state system-state--error", "Couldn't load SCORM packages: {e}" }
                },
                None => rsx! {
                    div { class: "system-state system-state--loading",
                        SkeletonLine { width: "70%".to_string() }
                        SkeletonLine { width: "50%".to_string() }
                    }
                },
            }
        }
    }
}

#[derive(Clone, Props, PartialEq)]
struct ScormRowProps {
    api: ApiContext,
    package: ScormPackageDto,
    can_delete: bool,
    on_launch: EventHandler<()>,
    on_deleted: EventHandler<()>,
}

#[component]
fn ScormRow(props: ScormRowProps) -> Element {
    let api = props.api.clone();
    let p = props.package.clone();
    let id = p.id.clone();
    let mut deleting = use_signal(|| false);
    let mut toast = use_toast_sender();
    let on_launch = props.on_launch;

    let on_delete = move |_| {
        let api = api.clone();
        let id = id.clone();
        let on_deleted = props.on_deleted;
        deleting.set(true);
        spawn(async move {
            match delete_package(&api, &id).await {
                Ok(_) => {
                    toast.push(
                        ToastLevel::Success,
                        "Package removed",
                        "The SCORM package was deleted.",
                    );
                    on_deleted.call(());
                }
                Err(e) => {
                    toast.push(ToastLevel::Danger, "Delete failed", format!("{e}"));
                    deleting.set(false);
                }
            }
        });
    };

    rsx! {
        li { key: "{p.id}", class: "scorm-card",
            Card {
                div { class: "scorm-card__head",
                    h3 { class: "scorm-card__title", "{p.title}" }
                    span { class: "scorm-card__badge muted", "SCORM {p.scorm_version}" }
                }
                div { class: "scorm-card__actions",
                    Button {
                        label: "Launch".to_string(),
                        variant: ButtonVariant::Primary,
                        button_type: "button".to_string(),
                        on_click: move |_| on_launch.call(()),
                    }
                    if props.can_delete {
                        Button {
                            label: if *deleting.read() { "Deleting…".to_string() } else { "Delete".to_string() },
                            variant: ButtonVariant::Ghost,
                            button_type: "button".to_string(),
                            disabled: *deleting.read(),
                            on_click: on_delete,
                        }
                    }
                }
            }
        }
    }
}

#[derive(Clone, Props, PartialEq)]
pub struct ScormPlayerProps {
    pub api: ApiContext,
    pub package: ScormPackageDto,
    pub on_back: EventHandler<()>,
}

/// The player itself: resolves the launch URL, configures the JS bridge, and
/// renders the SCO in a sandboxed iframe.
#[component]
pub fn ScormPlayer(props: ScormPlayerProps) -> Element {
    let api = props.api.clone();
    let package = props.package.clone();
    let on_back = props.on_back;

    let launch = use_resource({
        let api = api.clone();
        let id = package.id.clone();
        move || {
            let api = api.clone();
            let id = id.clone();
            async move { launch_package(&api, &id).await }
        }
    });

    rsx! {
        div { class: "scorm-player motion-page",
            div { class: "scorm-player__bar",
                Button {
                    label: "← Back to packages".to_string(),
                    variant: ButtonVariant::Ghost,
                    button_type: "button".to_string(),
                    on_click: move |_| on_back.call(()),
                }
                h2 { class: "scorm-player__title", "{package.title}" }
            }
            match &*launch.read_unchecked() {
                Some(Ok(info)) => {
                    // Configure the SCORM JS runtime before the iframe loads.
                    configure_bridge(&package.id, &api.id_token);
                    rsx! {
                        ScormFrame { src: info.launch_url.clone() }
                    }
                }
                Some(Err(e)) => rsx! {
                    div { class: "system-state system-state--error", "Couldn't launch package: {e}" }
                },
                None => rsx! {
                    div { class: "system-state system-state--loading",
                        SkeletonLine { width: "90%".to_string() }
                    }
                },
            }
        }
    }
}

#[derive(Clone, Props, PartialEq)]
struct ScormFrameProps {
    src: String,
}

#[component]
fn ScormFrame(props: ScormFrameProps) -> Element {
    rsx! {
        iframe {
            class: "scorm-player__frame",
            src: "{props.src}",
            title: "SCORM content",
            // Allow scripts + same-origin so the SCO can reach the runtime when
            // served same-origin; allow-popups for content that opens glossaries.
            // `sandbox` isn't in dioxus_elements::iframe, so use the string-keyed
            // custom-attribute form.
            "sandbox": "allow-scripts allow-same-origin allow-popups allow-forms",
            allowfullscreen: true,
        }
    }
}

#[derive(Clone, Props, PartialEq)]
struct ScormComposerProps {
    api: ApiContext,
    course_id: String,
    on_added: EventHandler<()>,
}

#[component]
fn ScormComposer(props: ScormComposerProps) -> Element {
    let api = props.api.clone();
    let course_id = props.course_id.clone();
    let on_added = props.on_added;

    let mut title = use_signal(String::new);
    let mut asset_id = use_signal(String::new);
    let mut manifest_xml = use_signal(String::new);
    let mut submitting = use_signal(|| false);
    let mut error = use_signal(|| Option::<String>::None);
    let mut toast = use_toast_sender();

    let on_submit = move |_| {
        let api = api.clone();
        let course_id = course_id.clone();
        let title_value = title.read().trim().to_string();
        let asset_value = asset_id.read().trim().to_string();
        let manifest_value = manifest_xml.read().trim().to_string();
        if title_value.is_empty() {
            error.set(Some("Title is required.".into()));
            return;
        }
        if asset_value.is_empty() {
            error.set(Some("Uploaded asset id is required.".into()));
            return;
        }
        if manifest_value.is_empty() {
            error.set(Some("Paste the package's imsmanifest.xml.".into()));
            return;
        }
        error.set(None);
        submitting.set(true);
        spawn(async move {
            match register_package(
                &api,
                &course_id,
                &title_value,
                &asset_value,
                &manifest_value,
                None,
            )
            .await
            {
                Ok(_) => {
                    toast.push(
                        ToastLevel::Success,
                        "Package registered",
                        "The SCORM package is ready to launch.",
                    );
                    title.set(String::new());
                    asset_id.set(String::new());
                    manifest_xml.set(String::new());
                    on_added.call(());
                }
                Err(e) => {
                    let msg = format!("{e}");
                    toast.push(ToastLevel::Danger, "Couldn't register package", msg.clone());
                    error.set(Some(msg));
                }
            }
            submitting.set(false);
        });
    };

    rsx! {
        Card {
            div { class: "scorm-composer",
                h3 { class: "scorm-composer__title", "Register SCORM package" }
                p { class: "muted scorm-composer__hint",
                    "Upload the SCORM .zip first (purpose \"scorm\"), then register it here with its imsmanifest.xml."
                }
                input {
                    class: "ds-input",
                    r#type: "text",
                    placeholder: "Title",
                    maxlength: "200",
                    value: "{title}",
                    disabled: *submitting.read(),
                    oninput: move |e| title.set(e.value()),
                }
                input {
                    class: "ds-input",
                    r#type: "text",
                    placeholder: "Uploaded asset id (file_assets UUID)",
                    value: "{asset_id}",
                    disabled: *submitting.read(),
                    oninput: move |e| asset_id.set(e.value()),
                }
                textarea {
                    class: "ds-input scorm-composer__manifest",
                    placeholder: "Paste imsmanifest.xml here",
                    rows: "8",
                    value: "{manifest_xml}",
                    disabled: *submitting.read(),
                    oninput: move |e| manifest_xml.set(e.value()),
                }
                if let Some(err) = error.read().as_ref() {
                    p { class: "error", "{err}" }
                }
                div { class: "scorm-composer__actions",
                    Button {
                        label: if *submitting.read() { "Registering…".to_string() } else { "Register package".to_string() },
                        variant: ButtonVariant::Primary,
                        button_type: "button".to_string(),
                        disabled: *submitting.read(),
                        on_click: on_submit,
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dto_deserializes() {
        let json = r#"{
            "id":"p1","course_id":"c1","title":"Intro","asset_id":"a1",
            "scorm_version":"1.2","launch_href":"index.html",
            "created_at":"2026-06-14T00:00:00Z"
        }"#;
        let p: ScormPackageDto = serde_json::from_str(json).unwrap();
        assert_eq!(p.scorm_version, "1.2");
        assert_eq!(p.launch_href, "index.html");
    }

    #[test]
    fn launch_dto_deserializes() {
        let json = r#"{
            "base_url":"https://s3/x/","launch_href":"index.html",
            "scorm_version":"2004","launch_url":"https://s3/x/index.html?sig=1"
        }"#;
        let d: ScormLaunchDto = serde_json::from_str(json).unwrap();
        assert_eq!(d.scorm_version, "2004");
    }
}
